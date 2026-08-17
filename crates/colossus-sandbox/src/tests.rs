use super::{
    AllowlistProxy, BASE64, FilesystemExecutor, HttpExecutor, OCI_PROXY_CONFIG_VARIABLE,
    SandboxJob, SignedSandboxJob, atomic_create, atomic_write, authority,
    host_process_limits_apply, inherit_ambient_environment, non_public_ip, oci_command,
    oci_proxy_run_arguments, oci_remove_arguments, oci_resource_names, proposed_write_bytes,
    redact_proxy_credential, resolve_oci_origins, sandbox_helper_budget, search_files, sha256_hex,
    tls_server_name, validate_process_spec,
};
#[cfg(unix)]
use super::{ProcessSpec, execute_sandbox_job, normalize_path_arguments};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::{native_helper_diagnostics, native_target_pid};
use base64::Engine as _;
#[cfg(unix)]
use colossus_contracts::FilesystemGrant;
use colossus_contracts::{
    DecisionOutcome, EffectPhase, EffectRequest, PolicyDecision, PolicyObligations,
    ResourceAuthority, SandboxBoundaryMode,
};
use colossus_policy::{
    BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, SandboxBoundaryGate, effect_request,
    system_actor,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint};
use colossus_testkit::InMemoryEventJournal;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[cfg(target_os = "windows")]
#[test]
fn canonical_drive_paths_use_process_compatible_syntax_after_authorization() {
    assert_eq!(
        super::windows_process_value(r"\\?\C:\workspace\allowed.txt"),
        r"C:\workspace\allowed.txt"
    );
    assert_eq!(
        super::windows_process_value(r"\\?\UNC\server\share"),
        r"\\?\UNC\server\share"
    );
}

struct AdapterPostDenyPolicy(BuiltInPolicy);

#[cfg(unix)]
#[test]
fn oci_control_timeout_allows_bounded_cold_runtime_startup() {
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", "sleep 2; printf ready"]);

    let (status, stdout, stderr) =
        super::bounded_control_command(command).expect("bounded command completes");
    assert!(status.success());
    assert_eq!(stdout, b"ready");
    assert!(stderr.is_empty());
}

#[test]
fn helper_budgets_reserve_backend_cleanup_before_the_outer_deadline() {
    let mut obligations = PolicyObligations {
        sandbox_backend: "windows_job".into(),
        timeout_ms: 10_000,
        ..PolicyObligations::default()
    };
    assert_eq!(sandbox_helper_budget(&obligations, 10_000), 3_000);

    obligations.sandbox_backend = "oci".into();
    obligations.timeout_ms = 5_000;
    assert_eq!(sandbox_helper_budget(&obligations, 5_000), 3_000);

    obligations.timeout_ms = 10_000;
    obligations.network_destinations = vec!["https://example.com".into()];
    assert_eq!(sandbox_helper_budget(&obligations, 10_000), 5_000);

    obligations.sandbox_backend = "native".into();
    obligations.network_destinations.clear();
    obligations.timeout_ms = 1_000;
    assert_eq!(sandbox_helper_budget(&obligations, 800), 550);
}

#[async_trait::async_trait]
impl PolicyDecisionPoint for AdapterPostDenyPolicy {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<PolicyDecision, colossus_ports::PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.phase == EffectPhase::PostEffect {
            decision.outcome = DecisionOutcome::Deny;
            decision.reason = "adapter content denied by post-effect policy".into();
        }
        Ok(decision)
    }

    async fn doctor(&self) -> Result<Value, colossus_ports::PolicyError> {
        self.0.doctor().await
    }
}

#[test]
fn atomic_write_replaces_content_without_following_leaf_symlinks() {
    let directory = tempdir().expect("tempdir");
    let target = directory.path().join("target");
    atomic_write(&target, b"first").expect("first");
    atomic_write(&target, b"second").expect("second");
    assert_eq!(std::fs::read(target).expect("read"), b"second");
}

#[test]
fn atomic_create_never_clobbers_a_concurrent_target() {
    let directory = tempdir().expect("tempdir");
    let target = directory.path().join("target");
    atomic_create(&target, b"first").expect("create");
    assert!(atomic_create(&target, b"second").is_err());
    assert_eq!(std::fs::read(target).expect("read"), b"first");
}

#[test]
fn write_payload_is_strict_and_bounded() {
    assert_eq!(
        proposed_write_bytes(&json!({"text": "ok"}), 2).expect("text"),
        b"ok"
    );
    assert!(proposed_write_bytes(&json!({"text": "too large"}), 2).is_err());
    assert!(proposed_write_bytes(&json!({"unknown": true}), 20).is_err());
}

#[test]
fn proxy_credentials_are_redacted_from_captured_process_output() {
    let credential = "a".repeat(64);
    let basic = BASE64.encode(format!("colossus:{credential}"));
    let output = format!("HTTP_PROXY=http://colossus:{credential}@127.0.0.1:42\n{basic}\n");
    let redacted = redact_proxy_credential(output.as_bytes(), Some(&credential));
    assert!(
        !redacted
            .windows(credential.len())
            .any(|value| value == credential.as_bytes())
    );
    assert!(
        !redacted
            .windows(basic.len())
            .any(|value| value == basic.as_bytes())
    );
    assert!(
        String::from_utf8(redacted)
            .expect("UTF-8")
            .contains("[REDACTED]")
    );
}

#[cfg(unix)]
#[test]
fn proxy_environment_overrides_both_unix_spellings() {
    let proxy = "http://colossus:credential@127.0.0.1:42";
    let mut command = std::process::Command::new("/bin/true");
    command
        .env("http_proxy", "http://attacker.invalid")
        .env("no_proxy", "*");
    super::configure_proxy_environment(&mut command, proxy);
    let environment = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        assert_eq!(
            environment.get(name).and_then(Option::as_deref),
            Some(proxy)
        );
    }
    assert_eq!(
        environment.get("NO_PROXY").and_then(Option::as_deref),
        Some("")
    );
    assert_eq!(
        environment.get("no_proxy").and_then(Option::as_deref),
        Some("")
    );
}

#[test]
fn authenticated_helper_job_rejects_tampering_and_expiry() {
    let job = SandboxJob {
        schema_version: 2,
        job_id: "018f0f9b-7b6e-7cc0-8000-000000000001".into(),
        request_id: "request".into(),
        request_hash: "hash".into(),
        decision_id: "decision".into(),
        permit_nonce: "nonce".into(),
        permit_expires_at_unix_ms: i128::MAX,
        executable: PathBuf::from("/bin/echo"),
        process: super::ProcessSpec {
            cwd: PathBuf::from("/tmp"),
            args: Vec::new(),
            environment: BTreeMap::new(),
            stdin_base64: None,
            timeout_ms: None,
            max_output_bytes: None,
        },
        obligations: PolicyObligations::default(),
        timeout_ms: 1,
        proxy_port: None,
        proxy_credential: None,
        oci_runtime: None,
        oci_image: None,
        oci_proxy_image: None,
        temporary_root: None,
    };
    let key = [7_u8; 32];
    let signed = SignedSandboxJob::sign(job.clone(), &key).expect("sign");
    assert!(signed.clone().verify(&key).is_ok());
    assert!(signed.verify(&[8_u8; 32]).is_err());

    let mut legacy = job.clone();
    legacy.schema_version = 1;
    assert!(
        SignedSandboxJob::sign(legacy, &key)
            .expect("sign legacy job")
            .verify(&key)
            .is_err()
    );

    let mut mismatched = job;
    mismatched.proxy_port = Some(42);
    assert!(
        SignedSandboxJob::sign(mismatched.clone(), &key)
            .expect("sign mismatched proxy")
            .verify(&key)
            .is_err()
    );

    let mut missing_temporary_root = mismatched;
    missing_temporary_root.proxy_port = None;
    missing_temporary_root.obligations.sandbox_backend = "windows_job".into();
    assert!(
        SignedSandboxJob::sign(missing_temporary_root, &key)
            .expect("sign Windows job without temporary root")
            .verify(&key)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn explicit_direct_backends_execute_without_the_native_kernel_sandbox() {
    for backend in ["external", "danger_full_access"] {
        let job = SandboxJob {
            schema_version: 2,
            job_id: format!("direct-{backend}"),
            request_id: "request".into(),
            request_hash: "hash".into(),
            decision_id: "decision".into(),
            permit_nonce: "nonce".into(),
            permit_expires_at_unix_ms: i128::MAX,
            executable: PathBuf::from("/bin/echo"),
            process: super::ProcessSpec {
                cwd: PathBuf::from("/tmp"),
                args: vec!["direct".into()],
                environment: BTreeMap::new(),
                stdin_base64: None,
                timeout_ms: None,
                max_output_bytes: None,
            },
            obligations: PolicyObligations {
                sandbox_backend: backend.into(),
                sandbox_profile: "test".into(),
                timeout_ms: 5_000,
                max_output_bytes: 4096,
                max_processes: 2,
                max_memory_bytes: 64 * 1024 * 1024,
                max_concurrency: 1,
                retention: "test".into(),
                ..PolicyObligations::default()
            },
            timeout_ms: 2_000,
            proxy_port: None,
            proxy_credential: None,
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
            temporary_root: None,
        };
        let result = execute_sandbox_job(job, &[7_u8; 32]).expect("direct execution");
        assert_eq!(result.backend, backend);
        assert!(result.success);
        assert_eq!(
            BASE64.decode(result.stdout_base64).expect("stdout"),
            b"direct\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn direct_backends_do_not_claim_filesystem_confinement_for_cwd_or_argv() {
    let cwd = tempdir().expect("cwd");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    for backend in ["external", "danger_full_access"] {
        let obligations = PolicyObligations {
            sandbox_backend: backend.into(),
            filesystem: vec![FilesystemGrant {
                root: executable.display().to_string(),
                mode: "execute".into(),
            }],
            timeout_ms: 5_000,
            max_output_bytes: 4096,
            ..PolicyObligations::default()
        };
        let mut process = ProcessSpec {
            cwd: cwd.path().into(),
            args: vec!["/path/outside/declared/filesystem".into()],
            environment: BTreeMap::new(),
            stdin_base64: None,
            timeout_ms: None,
            max_output_bytes: None,
        };

        validate_process_spec(&process, &executable.display().to_string(), &obligations)
            .expect("direct cwd requires no unenforced filesystem declaration");
        normalize_path_arguments(&mut process, &obligations)
            .expect("direct argv paths require no unenforced filesystem declaration");
        assert_eq!(process.args, ["/path/outside/declared/filesystem"]);
    }
}

#[cfg(unix)]
#[test]
fn danger_full_access_requires_no_process_resource_allowlists() {
    let cwd = tempdir().expect("cwd");
    let executable = std::env::current_exe()
        .expect("executable")
        .canonicalize()
        .expect("canonical executable");
    let process = ProcessSpec {
        cwd: cwd.path().into(),
        args: Vec::new(),
        environment: BTreeMap::from([("UNDECLARED_ENVIRONMENT".into(), "available".into())]),
        stdin_base64: None,
        timeout_ms: None,
        max_output_bytes: None,
    };
    let obligations = |backend: &str| PolicyObligations {
        sandbox_backend: backend.into(),
        timeout_ms: 5_000,
        max_output_bytes: 4096,
        ..PolicyObligations::default()
    };

    validate_process_spec(
        &process,
        &executable.display().to_string(),
        &obligations("danger_full_access"),
    )
    .expect("danger full access accepts ambient process resources");
    assert!(
        validate_process_spec(
            &process,
            &executable.display().to_string(),
            &obligations("external"),
        )
        .is_err(),
        "external execution still requires declared resources"
    );
}

#[test]
fn danger_full_access_ambient_environment_keeps_explicit_overrides_and_hides_control_state() {
    let mut environment = BTreeMap::from([
        ("PATH".into(), "/explicit/bin".into()),
        ("EXPLICIT_ONLY".into(), "explicit".into()),
    ]);
    inherit_ambient_environment(
        &mut environment,
        [
            ("PATH".into(), "/ambient/bin".into()),
            ("AMBIENT_ONLY".into(), "ambient".into()),
            ("COLOSSUS_SANDBOX_JOB_KEY".into(), "private".into()),
            ("colossus_sandbox_native_inner".into(), "private".into()),
            (OCI_PROXY_CONFIG_VARIABLE.into(), "private".into()),
        ],
    );

    assert_eq!(environment["PATH"], "/explicit/bin");
    assert_eq!(environment["EXPLICIT_ONLY"], "explicit");
    assert_eq!(environment["AMBIENT_ONLY"], "ambient");
    assert!(!environment.contains_key("COLOSSUS_SANDBOX_JOB_KEY"));
    assert!(!environment.contains_key("colossus_sandbox_native_inner"));
    assert!(!environment.contains_key(OCI_PROXY_CONFIG_VARIABLE));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_supervisor_accepts_only_its_strict_target_announcement() {
    let announcement = b"colossus-native-target-pid:42\n";
    assert_eq!(
        native_target_pid(announcement).map(|pid| pid.as_u32()),
        Some(42)
    );
    assert_eq!(
        native_helper_diagnostics(announcement).expect("strip announcement"),
        Vec::<u8>::new()
    );
    assert!(native_target_pid(b"colossus-native-target-pid:nope\n").is_none());
    assert!(native_helper_diagnostics(b"colossus-native-target-pid:nope\n").is_err());
    assert_eq!(
        native_helper_diagnostics(b"setup failed\n").expect("preserve diagnostics"),
        b"setup failed\n"
    );
}

#[test]
fn proxy_authorities_and_private_ranges_are_strict() {
    assert_eq!(
        authority("example.com:8443", 443).expect("authority"),
        ("example.com".into(), 8443)
    );
    assert!(non_public_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    assert!(!non_public_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    let resolved =
        resolve_oci_origins(&["http://127.0.0.1:18080".into()]).expect("explicit IP origin");
    assert_eq!(
        resolved["http://127.0.0.1:18080"],
        [SocketAddr::from(([127, 0, 0, 1], 18_080))]
    );
}

#[test]
fn tls_client_hello_server_name_is_extracted_for_connect_enforcement() {
    let records = tls_client_hello("api.example.com");
    assert_eq!(
        tls_server_name(&records).expect("server name"),
        Some("api.example.com".into())
    );
    let mut truncated = records;
    truncated.pop();
    assert!(tls_server_name(&truncated).is_err());
}

fn tls_client_hello(server_name: &str) -> Vec<u8> {
    let mut server_name_extension = Vec::new();
    let name_len = u16::try_from(server_name.len()).expect("name length");
    server_name_extension.extend_from_slice(&(name_len + 3).to_be_bytes());
    server_name_extension.push(0);
    server_name_extension.extend_from_slice(&name_len.to_be_bytes());
    server_name_extension.extend_from_slice(server_name.as_bytes());

    let mut extensions = Vec::new();
    extensions.extend_from_slice(&0_u16.to_be_bytes());
    extensions.extend_from_slice(
        &u16::try_from(server_name_extension.len())
            .expect("extension length")
            .to_be_bytes(),
    );
    extensions.extend_from_slice(&server_name_extension);

    let mut body = Vec::new();
    body.extend_from_slice(&[3, 3]);
    body.extend_from_slice(&[7; 32]);
    body.push(0);
    body.extend_from_slice(&2_u16.to_be_bytes());
    body.extend_from_slice(&[0x13, 0x01]);
    body.push(1);
    body.push(0);
    body.extend_from_slice(
        &u16::try_from(extensions.len())
            .expect("extensions length")
            .to_be_bytes(),
    );
    body.extend_from_slice(&extensions);

    let mut handshake = vec![
        1,
        u8::try_from((body.len() >> 16) & 0xff).expect("length"),
        u8::try_from((body.len() >> 8) & 0xff).expect("length"),
        u8::try_from(body.len() & 0xff).expect("length"),
    ];
    handshake.extend_from_slice(&body);

    let mut record = vec![22, 3, 1];
    record.extend_from_slice(
        &u16::try_from(handshake.len())
            .expect("record length")
            .to_be_bytes(),
    );
    record.extend_from_slice(&handshake);
    record
}

#[test]
fn oci_profile_applies_resource_and_privilege_limits_without_argv_secrets() {
    let directory = tempdir().expect("directory");
    let mut obligations = PolicyObligations {
        sandbox_backend: "oci".into(),
        sandbox_profile: "test".into(),
        max_output_bytes: 1024,
        max_processes: 2,
        max_memory_bytes: 64 * 1024 * 1024,
        max_concurrency: 1,
        timeout_ms: 1000,
        retention: "test".into(),
        ..PolicyObligations::default()
    };
    obligations
        .filesystem
        .push(colossus_contracts::FilesystemGrant {
            root: directory.path().display().to_string(),
            mode: "write".into(),
        });
    obligations
        .filesystem
        .push(colossus_contracts::FilesystemGrant {
            root: "/usr/bin/example".into(),
            mode: "execute".into(),
        });
    obligations.allowed_environment.push("TOKEN".into());
    let mut job = SandboxJob {
        schema_version: 1,
        job_id: "018f0f9b-7b6e-7cc0-8000-000000000002".into(),
        request_id: "request".into(),
        request_hash: "hash".into(),
        decision_id: "decision".into(),
        permit_nonce: "nonce".into(),
        permit_expires_at_unix_ms: i128::MAX,
        executable: PathBuf::from("/usr/bin/example"),
        process: super::ProcessSpec {
            cwd: directory.path().into(),
            args: vec!["check".into()],
            environment: BTreeMap::from([("TOKEN".into(), "secret-value".into())]),
            stdin_base64: None,
            timeout_ms: None,
            max_output_bytes: None,
        },
        obligations,
        timeout_ms: 1000,
        proxy_port: None,
        proxy_credential: None,
        oci_runtime: Some(PathBuf::from("/usr/bin/docker")),
        oci_image: Some(format!("example@sha256:{}", "a".repeat(64))),
        oci_proxy_image: None,
        temporary_root: None,
    };
    validate_process_spec(&job.process, "/usr/bin/example", &job.obligations)
        .expect("exact OCI image executable");
    let mut oversized_request = job.process.clone();
    oversized_request.timeout_ms = Some(job.obligations.timeout_ms.saturating_add(1));
    assert!(
        validate_process_spec(&oversized_request, "/usr/bin/example", &job.obligations,).is_err()
    );
    oversized_request.timeout_ms = None;
    oversized_request.max_output_bytes = Some(job.obligations.max_output_bytes.saturating_add(1));
    assert!(
        validate_process_spec(&oversized_request, "/usr/bin/example", &job.obligations,).is_err()
    );
    let command = oci_command(&job, None).expect("OCI command");
    let args = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(args.contains(&"--network=none".into()));
    assert!(args.contains(&"--pull=never".into()));
    assert!(args.contains(&"--read-only".into()));
    assert!(args.contains(&"--cap-drop=ALL".into()));
    assert!(args.contains(&"--user".into()));
    assert!(!args.contains(&"--userns=keep-id".into()));
    assert!(args.contains(&"--pids-limit=2".into()));
    assert!(args.contains(&format!("--memory={}", 64 * 1024 * 1024)));
    assert!(!host_process_limits_apply("oci"));
    assert!(host_process_limits_apply("native"));
    assert!(host_process_limits_apply("broker"));
    assert!(host_process_limits_apply("external"));
    assert!(host_process_limits_apply("danger_full_access"));
    assert!(args.contains(&"--entrypoint".into()));
    assert!(args.contains(&"colossus-018f0f9b7b6e7cc08000000000000002".into()));
    assert!(
        !args
            .iter()
            .any(|argument| argument.contains("secret-value"))
    );
    assert!(command.get_envs().any(|(name, value)| {
        name == "TOKEN" && value.is_some_and(|value| value == "secret-value")
    }));
    assert!(
        command.get_envs().any(|(name, value)| {
            name == "PATH" && value.is_some_and(|value| value == "/usr/bin")
        })
    );
    assert_eq!(
        oci_remove_arguments(PathBuf::from("/usr/bin/docker").as_path(), "job")
            .expect("Docker cleanup"),
        ["container", "rm", "--force", "job"]
    );
    assert_eq!(
        oci_remove_arguments(PathBuf::from("/usr/bin/podman").as_path(), "job")
            .expect("Podman cleanup"),
        ["container", "rm", "--force", "--time", "0", "job"]
    );
    assert_eq!(
        oci_remove_arguments(PathBuf::from("/usr/bin/podman-remote").as_path(), "job")
            .expect("Podman remote cleanup"),
        ["container", "rm", "--force", "--time", "0", "job"]
    );
    assert!(oci_remove_arguments(PathBuf::from("/usr/bin/unknown").as_path(), "job").is_none());

    let names = oci_resource_names(&job.job_id);
    let proxy_image = format!("sha256:{}", "b".repeat(64));
    let docker_proxy =
        oci_proxy_run_arguments(&job, &names, &proxy_image).expect("Docker proxy arguments");
    assert!(!docker_proxy.contains(&"--userns=keep-id".into()));
    assert!(!docker_proxy.contains(&"--user".into()));

    job.oci_runtime = Some(PathBuf::from("/usr/bin/podman"));
    let podman = oci_command(&job, None).expect("Podman command");
    assert!(
        podman
            .get_args()
            .any(|argument| argument == "--userns=keep-id")
    );
    assert!(
        podman.get_envs().any(|(name, value)| {
            name == "PATH" && value.is_some_and(|value| value == "/usr/bin")
        })
    );
    let podman_proxy =
        oci_proxy_run_arguments(&job, &names, &proxy_image).expect("Podman proxy arguments");
    assert!(!podman_proxy.contains(&"--userns=keep-id".into()));
    assert!(!podman_proxy.contains(&"--user".into()));

    job.obligations
        .network_destinations
        .push("https://example.com".into());
    job.oci_proxy_image = Some(format!("sha256:{}", "b".repeat(64)));
    let proxy_address = SocketAddr::from(([10, 88, 0, 2], super::OCI_PROXY_PORT));
    let command = oci_command(&job, Some(proxy_address)).expect("networked OCI command");
    let args = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(!args.contains(&"--network=none".into()));
    assert!(args.contains(&"--dns=127.0.0.1".into()));
    assert!(args.contains(&super::oci_resource_names(&job.job_id).internal_network));
    assert!(!args.iter().any(|argument| argument.contains("10.88.0.2")));
    assert!(command.get_envs().any(|(name, value)| {
        name == "HTTPS_PROXY" && value.is_some_and(|value| value == "http://10.88.0.2:18080")
    }));
}

#[tokio::test]
async fn filesystem_and_http_content_denied_post_effect_never_reaches_the_caller() {
    let directory = tempdir().expect("directory");
    let file_secret = "filesystem-private-content";
    let file = directory.path().join("private.txt");
    std::fs::write(&file, file_secret).expect("file fixture");
    let file_journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let file_policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.read", DecisionOutcome::Allow)
        .with_filesystem_read_root(directory.path().display().to_string())
        .with_post_effect(true);
    let file_gateway = EffectGateway::new(
        Arc::clone(&file_journal),
        Arc::new(AdapterPostDenyPolicy(file_policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["filesystem.read".into()]),
        [51_u8; 32],
    );
    let mut file_request = effect_request(
        system_actor("filesystem-post-deny"),
        "filesystem.read",
        file.display().to_string(),
        json!({}),
    );
    file_request.capabilities = vec!["filesystem.read".into()];
    let file_error = file_gateway
        .execute(file_request, &FilesystemExecutor::new())
        .await
        .expect_err("filesystem post-effect denial");
    assert!(
        file_error
            .to_string()
            .contains("post-effect release denied")
    );
    assert!(!file_error.to_string().contains(file_secret));
    let file_events = file_journal.read_global(1, 30).expect("file events");
    assert!(
        file_events
            .iter()
            .any(|event| event.event_type == "effect.release_denied.v1")
    );
    assert!(
        file_events
            .iter()
            .all(|event| event.event_type != "effect.completed.v1")
    );

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = listener.local_addr().expect("address");
    let http_secret = "network-private-content";
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await.expect("read");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{http_secret}",
            http_secret.len()
        );
        stream.write_all(response.as_bytes()).await.expect("write");
    });
    let origin = format!("http://{address}");
    let url = format!("{origin}/private");
    let http_journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let http_policy = BuiltInPolicy::offline_default()
        .with_action("network.http", DecisionOutcome::Allow)
        .with_network_destination(&origin)
        .with_post_effect(true);
    let http_gateway = EffectGateway::new(
        Arc::clone(&http_journal),
        Arc::new(AdapterPostDenyPolicy(http_policy)),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]),
        [52_u8; 32],
    );
    let mut http_request = effect_request(
        system_actor("http-post-deny"),
        "network.http",
        url,
        json!({"method": "GET", "headers": {}}),
    );
    http_request.capabilities = vec!["network.http".into()];
    let http_error = http_gateway
        .execute(http_request, &HttpExecutor::new())
        .await
        .expect_err("HTTP post-effect denial");
    assert!(
        http_error
            .to_string()
            .contains("post-effect release denied")
    );
    assert!(!http_error.to_string().contains(http_secret));
    let http_events = http_journal.read_global(1, 30).expect("HTTP events");
    assert!(
        http_events
            .iter()
            .any(|event| event.event_type == "effect.release_denied.v1")
    );
    assert!(
        http_events
            .iter()
            .all(|event| event.event_type != "effect.completed.v1")
    );
    server.await.expect("server");
}

#[cfg(unix)]
#[tokio::test]
async fn filesystem_symlink_escape_is_denied_before_release() {
    use std::os::unix::fs::symlink;

    let allowed = tempdir().expect("allowed");
    let denied = tempdir().expect("denied");
    let secret = denied.path().join("secret");
    std::fs::write(&secret, "secret").expect("secret");
    let escape = allowed.path().join("escape");
    symlink(&secret, &escape).expect("symlink");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.read", DecisionOutcome::Allow)
        .with_filesystem_read_root(allowed.path().display().to_string());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["filesystem.read".into()]),
        [4_u8; 32],
    );
    let mut request = effect_request(
        system_actor("test"),
        "filesystem.read",
        escape.display().to_string(),
        json!({}),
    );
    request.capabilities = vec!["filesystem.read".into()];
    assert!(
        gateway
            .execute(request, &FilesystemExecutor::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn filesystem_write_is_permit_bound_and_atomic() {
    let directory = tempdir().expect("directory");
    let target = directory.path().join("created.txt");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.write", DecisionOutcome::Allow)
        .with_filesystem_root(directory.path().display().to_string(), "write");
    let gateway = EffectGateway::new(
        journal,
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["filesystem.write".into()]),
        [4_u8; 32],
    );
    let mut request = effect_request(
        system_actor("test"),
        "filesystem.write",
        target.display().to_string(),
        json!({
            "operation": "write",
            "display_path": "created.txt",
            "text": "hello hello",
            "mode": "create",
        }),
    );
    request.capabilities = vec!["filesystem.write".into()];
    let written = gateway
        .execute(request, &FilesystemExecutor::new())
        .await
        .expect("write");
    let written: serde_json::Value = serde_json::from_slice(&written.bytes).expect("write JSON");
    assert_eq!(written["path"], "created.txt");
    assert_eq!(written["changed_line_ranges"][0]["start"], 1);
    assert!(
        written["diff"]
            .as_str()
            .is_some_and(|diff| diff.contains("+hello hello"))
    );

    let mut request = effect_request(
        system_actor("test"),
        "filesystem.write",
        target.display().to_string(),
        json!({
            "operation": "replace",
            "display_path": "created.txt",
            "old": "hello",
            "new": "hi",
            "replace_all": true,
        }),
    );
    request.capabilities = vec!["filesystem.write".into()];
    let replaced = gateway
        .execute(request, &FilesystemExecutor::new())
        .await
        .expect("replace");
    let replaced: serde_json::Value =
        serde_json::from_slice(&replaced.bytes).expect("replace JSON");
    assert_eq!(replaced["replacements"], 2);
    assert!(
        replaced["diff"]
            .as_str()
            .is_some_and(|diff| { diff.contains("-hello hello") && diff.contains("+hi hi") })
    );
    assert_eq!(std::fs::read_to_string(target).expect("read"), "hi hi");
}

#[tokio::test]
async fn patch_preview_apply_and_reverse_are_permit_bound_and_atomic() {
    let directory = tempdir().expect("directory");
    let target = directory.path().join("note.txt");
    std::fs::write(&target, "alpha\nbeta\n").expect("fixture");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_post_effect(true)
        .with_action("patch.preview", DecisionOutcome::Allow)
        .with_action("patch.apply", DecisionOutcome::RequireApproval)
        .with_action("patch.reverse", DecisionOutcome::RequireApproval)
        .with_filesystem_root(directory.path().display().to_string(), "write");
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test-operator".into(),
        }),
        SafetyKernel::new([
            "patch.preview".into(),
            "patch.apply".into(),
            "patch.reverse".into(),
        ]),
        [41_u8; 32],
    );
    let effect = |action: &str, old: &str, new: &str| {
        let mut request = effect_request(
            system_actor("test"),
            action,
            target.display().to_string(),
            json!({
                "operation": "replace",
                "display_path": "note.txt",
                "old": old,
                "new": new,
                "replace_all": false,
            }),
        );
        request.capabilities = vec![action.into()];
        request
    };
    let preview = gateway
        .execute(
            effect("patch.preview", "beta", "gamma"),
            &FilesystemExecutor::new(),
        )
        .await
        .expect("preview");
    let preview: serde_json::Value = serde_json::from_slice(&preview.bytes).expect("preview JSON");
    assert!(
        preview["diff"]
            .as_str()
            .is_some_and(|diff| diff.contains("+gamma"))
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        "alpha\nbeta\n"
    );

    gateway
        .execute(
            effect("patch.apply", "beta", "gamma"),
            &FilesystemExecutor::new(),
        )
        .await
        .expect("apply");
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        "alpha\ngamma\n"
    );
    gateway
        .execute(
            effect("patch.reverse", "gamma", "beta"),
            &FilesystemExecutor::new(),
        )
        .await
        .expect("reverse");
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        "alpha\nbeta\n"
    );
    let event_types = journal
        .read_global(1, 100)
        .expect("events")
        .into_iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"approval.granted.v1".into()));
    assert!(event_types.contains(&"effect.release_requested.v1".into()));
}

#[tokio::test]
async fn filesystem_search_is_bounded_utf8_only_and_skips_control_state() {
    let directory = tempdir().expect("directory");
    std::fs::create_dir_all(directory.path().join("src")).expect("src");
    std::fs::create_dir_all(directory.path().join(".colossus")).expect("control");
    std::fs::write(
        directory.path().join("src/example.rs"),
        "first\nNeedle here\nneedle again\n",
    )
    .expect("fixture");
    std::fs::write(directory.path().join("src/blob.bin"), b"needle\0hidden")
        .expect("binary fixture");
    std::fs::write(directory.path().join(".colossus/secret"), "needle secret")
        .expect("control fixture");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("filesystem.search", DecisionOutcome::Allow)
        .with_filesystem_read_root(directory.path().display().to_string());
    let gateway = EffectGateway::new(
        journal,
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["filesystem.search".into()]),
        [4_u8; 32],
    );
    let mut request = effect_request(
        system_actor("test"),
        "filesystem.search",
        directory.path().display().to_string(),
        json!({
            "pattern": "needle",
            "regex": false,
            "case_sensitive": false,
            "glob": "**/*.rs",
            "max_matches": 1,
        }),
    );
    request.capabilities = vec!["filesystem.search".into()];
    let result = gateway
        .execute(request, &FilesystemExecutor::new())
        .await
        .expect("search");
    let value: serde_json::Value = serde_json::from_slice(&result.bytes).expect("JSON");
    assert_eq!(value["matches"][0]["path"], "src/example.rs");
    assert_eq!(value["matches"][0]["line"], 2);
    assert_eq!(value["matches"][0]["column"], 1);
    assert_eq!(value["truncated"], true);
    assert_eq!(value["matches"].as_array().map(Vec::len), Some(1));
}

#[test]
fn ambient_workspace_search_respects_repository_ignores_and_releases_context() {
    let directory = tempdir().expect("directory");
    std::fs::create_dir_all(directory.path().join(".git")).expect("git marker");
    std::fs::create_dir_all(directory.path().join("target")).expect("ignored directory");
    std::fs::create_dir_all(directory.path().join(".colossus")).expect("control directory");
    std::fs::write(directory.path().join(".gitignore"), "target/\n").expect("ignore file");
    std::fs::write(
        directory.path().join("visible.md"),
        "before\nunique needle\nafter\n",
    )
    .expect("visible fixture");
    std::fs::write(directory.path().join("target/ignored.md"), "unique needle")
        .expect("ignored fixture");
    std::fs::write(directory.path().join(".colossus/secret"), "unique needle")
        .expect("control fixture");

    let result = search_files(
        directory.path(),
        &json!({
            "pattern": "unique needle",
            "regex": false,
            "case_sensitive": true,
            "max_matches": 10,
            "context_lines": 1,
            "workspace_scoped": true,
        }),
        1024 * 1024,
        true,
    )
    .expect("workspace-scoped ambient search");
    let value: serde_json::Value = serde_json::from_slice(&result.bytes).expect("JSON");
    let matches = value["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["path"], "visible.md");
    assert_eq!(matches[0]["text"], "1: before\n2: unique needle\n3: after");
}

#[tokio::test]
async fn worm_http_requires_body_and_object_key_hash_binding() {
    let origin = "https://127.0.0.1:1";
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("audit.export.worm.write", DecisionOutcome::Allow)
        .with_network_destination(origin);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["audit.export.worm.write".into()]),
        [90_u8; 32],
    );
    let body = br#"{"event":"redacted"}"#;
    let content_hash = sha256_hex(body);
    let request = |resource: &str, hash: &str| {
        let mut request = effect_request(
            system_actor("test"),
            "audit.export.worm.write",
            resource,
            json!({
                "method": "PUT",
                "create_only": true,
                "body_base64": BASE64.encode(body),
                "content_sha256": hash,
            }),
        );
        request.capabilities = vec!["audit.export.worm.write".into()];
        request
    };

    let mismatch = gateway
        .execute(
            request(
                &format!(
                    "{origin}/00000000000000000001-event-{}.json",
                    "0".repeat(64)
                ),
                &"0".repeat(64),
            ),
            &HttpExecutor::new(),
        )
        .await
        .expect_err("mismatched body hash must fail");
    assert!(mismatch.to_string().contains("content hash"));

    let unbound = gateway
        .execute(
            request(&format!("{origin}/event.json"), &content_hash),
            &HttpExecutor::new(),
        )
        .await
        .expect_err("unbound object key must fail");
    assert!(unbound.to_string().contains("object key"));
}

#[tokio::test]
async fn brokered_http_is_exact_origin_bounded_and_post_authorized() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await.expect("read");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nok",
            )
            .await
            .expect("write");
    });
    let origin = format!("http://{address}");
    let url = format!("{origin}/health");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let policy = BuiltInPolicy::offline_default()
        .with_action("network.http", DecisionOutcome::Allow)
        .with_network_destination(&origin);
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]),
        [4_u8; 32],
    );
    let mut request = effect_request(
        system_actor("test"),
        "network.http",
        &url,
        json!({"method": "GET", "headers": {}}),
    );
    request.capabilities = vec!["network.http".into()];
    let result = gateway
        .execute(request, &HttpExecutor::new())
        .await
        .expect("request");
    assert_eq!(result.bytes, b"ok");
    assert!(
        journal
            .read_global(1, 30)
            .expect("events")
            .iter()
            .any(|event| event.event_type == "effect.release_requested.v1")
    );
    server.await.expect("server");
}

#[tokio::test]
async fn ambient_http_accepts_loopback_without_a_destination_grant() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await.expect("read");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nambient")
            .await
            .expect("write");
    });
    let url = format!("http://{address}/ambient");
    let policy = BuiltInPolicy::offline_default()
        .with_action("network.http", DecisionOutcome::Allow)
        .with_sandbox("danger_full_access", "test", false)
        .with_resource_authority(ResourceAuthority::Ambient);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::DangerFullAccess), true),
        )),
        [5_u8; 32],
    );
    let mut request = effect_request(
        system_actor("ambient-http"),
        "network.http",
        &url,
        json!({"method": "GET", "headers": {}}),
    );
    request.capabilities = vec!["network.http".into()];
    let result = gateway
        .execute(request, &HttpExecutor::new())
        .await
        .expect("ambient loopback request");
    assert_eq!(result.bytes, b"ambient");
    server.await.expect("server");
}

#[tokio::test]
async fn remote_plaintext_http_requires_ambient_authority_in_the_permit() {
    let endpoint = "http://192.0.2.1:9/plaintext";
    let request = || {
        let mut request = effect_request(
            system_actor("plaintext-http"),
            "network.http",
            endpoint,
            json!({"method": "GET", "headers": {}}),
        );
        request.capabilities = vec!["network.http".into()];
        request
    };

    let declared = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("network.http", DecisionOutcome::Allow)
                .with_network_destination("http://192.0.2.1:9"),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]),
        [69_u8; 32],
    );
    let error = declared
        .execute(request(), &HttpExecutor::new())
        .await
        .expect_err("declared exact origin must not authorize remote plaintext HTTP");
    assert!(
        error
            .to_string()
            .contains("requires ambient resource authority")
    );

    let ambient = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("network.http", DecisionOutcome::Allow)
                .with_sandbox("danger_full_access", "test", false)
                .with_resource_authority(ResourceAuthority::Ambient)
                .with_limits(25, 1024 * 1024, 1, 64 * 1024 * 1024, 1),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["network.http".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::DangerFullAccess), true),
        )),
        [70_u8; 32],
    );
    let result = ambient.execute(request(), &HttpExecutor::new()).await;
    assert!(result.as_ref().err().is_none_or(|error| {
        !error
            .to_string()
            .contains("requires ambient resource authority")
    }));
}

#[tokio::test]
async fn allowlist_proxy_rejects_an_unlisted_origin_without_connecting_upstream() {
    let proxy = AllowlistProxy::start(vec!["https://example.com".into()])
        .await
        .expect("proxy");
    let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("connect");
    stream
        .write_all(b"CONNECT denied.example:443 HTTP/1.1\r\nHost: denied.example\r\n\r\n")
        .await
        .expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("response");
    assert!(response.starts_with(b"HTTP/1.1 403"));
    assert!(proxy.observed_origins().is_empty());
}

#[tokio::test]
async fn public_wildcard_proxy_rejects_loopback_without_an_exact_origin() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = upstream.local_addr().expect("address");
    let proxy = AllowlistProxy::start(vec!["*".into()])
        .await
        .expect("proxy");
    let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("connect");
    stream
        .write_all(
            format!(
                "GET http://{address}/metadata HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("response");
    assert!(response.starts_with(b"HTTP/1.1 403"));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), upstream.accept())
            .await
            .is_err(),
        "wildcard request reached a loopback upstream"
    );
}

#[tokio::test]
async fn authenticated_allowlist_proxy_rejects_missing_credentials_and_strips_valid_ones() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = upstream.local_addr().expect("address");
    let origin = format!("http://{address}");
    let credential = "a".repeat(64);
    let proxy = AllowlistProxy::start_authenticated(vec![origin.clone()], &credential)
        .await
        .expect("authenticated proxy");

    let mut unauthorized = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("unauthorized connect");
    unauthorized
        .write_all(format!("GET {origin}/denied HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .await
        .expect("unauthorized request");
    let mut response = Vec::new();
    unauthorized
        .read_to_end(&mut response)
        .await
        .expect("unauthorized response");
    assert!(response.starts_with(b"HTTP/1.1 407"));

    let server = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.expect("authorized accept");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.expect("authorized read");
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        assert!(
            !String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("proxy-authorization:")
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .expect("authorized response");
    });
    let authorization = format!("Basic {}", BASE64.encode(format!("colossus:{credential}")));
    let mut authorized = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("authorized connect");
    authorized
            .write_all(
                format!(
                    "GET {origin}/allowed HTTP/1.1\r\nHost: {address}\r\nProxy-Authorization: {authorization}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("authorized request");
    let mut response = Vec::new();
    authorized
        .read_to_end(&mut response)
        .await
        .expect("authorized response");
    assert!(response.starts_with(b"HTTP/1.1 200"));
    server.await.expect("server");
}

#[tokio::test]
async fn allowlist_proxy_forwards_an_exact_http_origin() {
    let upstream = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
    let address = upstream.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.expect("read");
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        assert!(request.starts_with(b"GET /health HTTP/1.1\r\n"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .expect("response");
    });
    let origin = format!("http://{address}");
    let proxy = AllowlistProxy::start(vec![origin.clone()])
        .await
        .expect("proxy");
    let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("connect");
    client
        .write_all(
            format!("GET {origin}/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("request");
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.expect("response");
    assert!(response.starts_with(b"HTTP/1.1 200 OK"));
    assert!(response.ends_with(b"ok"));
    server.await.expect("server");
    assert_eq!(proxy.observed_origins(), [origin]);
}

#[tokio::test]
async fn allowlist_proxy_rejects_conflicting_http_host_and_tls_server_name() {
    let proxy = AllowlistProxy::start(vec![
        "http://example.com".into(),
        "https://example.com".into(),
    ])
    .await
    .expect("proxy");

    let mut http = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("connect HTTP");
    http.write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: attacker.example\r\n\r\n")
        .await
        .expect("write HTTP");
    let mut response = Vec::new();
    http.read_to_end(&mut response).await.expect("HTTP close");
    assert!(response.is_empty());

    let mut tls = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .expect("connect TLS");
    tls.write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
        .await
        .expect("write CONNECT");
    let expected = b"HTTP/1.1 200 Connection Established\r\n\r\n";
    let mut established = vec![0_u8; expected.len()];
    tls.read_exact(&mut established)
        .await
        .expect("CONNECT response");
    assert_eq!(&established, expected);
    tls.write_all(&tls_client_hello("attacker.example"))
        .await
        .expect("write ClientHello");
    let mut response = Vec::new();
    tls.read_to_end(&mut response).await.expect("TLS close");
    assert!(response.is_empty());
}
