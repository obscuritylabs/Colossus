use super::*;
use colossus_contracts::{BrowserActionKind, BrowserLimits, BrowserMode};
use colossus_contracts::{
    BrowserDocumentId, BrowserOpenOptions, BrowserOrigin, BrowserScope, BrowserSessionBinding,
    BrowserTabId,
};
use std::{os::unix::fs::PermissionsExt as _, path::Path};

#[path = "tests/profile_lifetime.rs"]
mod profile_lifetime;

fn request() -> BrowserDriverOpenRequest {
    BrowserDriverOpenRequest {
        binding: BrowserSessionBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope: BrowserScope::Conversation {
                id: "conversation".into(),
            },
        },
        run_id: Some("run".into()),
        session_id: BrowserSessionId::parse(format!("bs_{}", "a".repeat(32))).unwrap(),
        tab_id: BrowserTabId::parse(format!("bt_{}", "b".repeat(32))).unwrap(),
        document_id: BrowserDocumentId::parse(format!("bd_{}", "c".repeat(32))).unwrap(),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Headless,
            allowed_origins: vec![BrowserOrigin::parse("https://allowed.example").unwrap()],
            initial_url: None,
        },
    }
}

fn resources(installation: Arc<Installation>) -> Resources {
    let path = installation
        .root
        .prepare_directory(Path::new("session-test"))
        .unwrap();
    installation.artifacts.admit();
    let request = request();
    Resources {
        installation,
        binding: request.binding.clone(),
        request,
        name: "colossus-browser-test".into(),
        nonce: "trustednonce".into(),
        control_handle: std::fs::File::open(&path).unwrap(),
        control: ConfinedRoot::bind(path).unwrap(),
        quarantine: None,
        control_removed: false,
        container: None,
        create_attempted: false,
        process: None,
        egress: None,
        relay: None,
        pki: None,
        profile: None,
        presentation: None,
        closed: false,
        counted: true,
        fenced: Arc::new(AtomicBool::new(false)),
        startup_phase: "prepared",
        shutdown_receipt: OciBrowserShutdownReceipt::Unknown,
    }
}

fn installation(root: &Path) -> Installation {
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).expect("private root");
    Installation {
        root: ConfinedRoot::bind(root).expect("root"),
        component: root.join("component"),
        seccomp: root.join("seccomp.json"),
        docker_config: root.join("docker"),
        docker: PathBuf::from("/usr/bin/docker"),
        docker_identity: engine::ExecutableIdentity::default(),
        diagnostic_owner: false,
        image: format!("sha256:{}", "a".repeat(64)),
        digest: [1; 32],
        uid: rustix::process::geteuid().as_raw(),
        gid: rustix::process::getegid().as_raw(),
        limits: OciBrowserLimits::default(),
        capabilities: BrowserCapabilities {
            available: true,
            engine_version: Some("accepted".into()),
            modes: vec![BrowserMode::Headless],
            actions: vec![BrowserActionKind::Snapshot],
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: true,
        },
        pki: None,
        pki_enrollment: None,
        profile_store: None,
        presentation: false,
        artifacts: Arc::new(installation::StageGuard::new(root).expect("artifact ownership")),
    }
}

#[test]
fn container_command_has_no_host_network_or_secret_configuration() {
    let temporary = tempfile::tempdir().expect("state");
    let installation = installation(temporary.path());
    let arguments = engine::arguments(
        &installation,
        "colossus-browser-accepted",
        "accepted",
        &temporary.path().join("session"),
        None,
    )
    .expect("arguments");
    for expected in [
        "--network=none",
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pull=never",
        "--ipc=private",
        "--cgroupns=private",
        "--log-driver=none",
        "--oci-sockets",
    ] {
        assert!(
            arguments.iter().any(|argument| argument == expected),
            "missing {expected}"
        );
    }
    let text = arguments.join(" ");
    assert!(text.contains("/var/colossus-browser:rw,noexec,nosuid,nodev,"));
    assert!(text.contains("dst=/run/colossus-browser-control,readonly"));
    assert!(text.contains("dst=/opt/colossus-browser,readonly"));
    for forbidden in [
        "--privileged",
        "seccomp=unconfined",
        "--init",
        "--no-sandbox",
        "password",
        "bootstrap-key",
        "--network=host",
    ] {
        assert!(!text.contains(forbidden), "unsafe argument {forbidden}");
    }
}

#[tokio::test]
async fn native_private_channels_require_exact_process_uid_gid() {
    let (left, right) = tokio::net::UnixStream::pair().expect("private pair");
    let pid = rustix::process::getpid().as_raw_pid() as u32;
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    assert_eq!(relay::verify_peer(&left, pid, uid, gid), Ok(()));
    assert_eq!(
        relay::verify_peer(&right, pid + 1, uid, gid),
        Err(BrowserDriverError::Denied)
    );
    assert_eq!(
        relay::verify_peer(&right, pid, uid + 1, gid),
        Err(BrowserDriverError::Denied)
    );
    assert_eq!(
        relay::verify_peer(&right, pid, uid, gid + 1),
        Err(BrowserDriverError::Denied)
    );
}

#[test]
fn seccomp_namespace_support_is_curated_and_digestable() {
    let policy: serde_json::Value =
        serde_json::from_slice(include_bytes!("seccomp.json")).expect("policy");
    assert_eq!(policy["defaultAction"], "SCMP_ACT_ERRNO");
    assert_eq!(policy["defaultErrnoRet"], 1);
    let rules = policy["syscalls"].as_array().expect("syscall rules");
    assert_eq!(rules.len(), 1, "additional allow rules require review");
    assert_eq!(rules[0]["action"], "SCMP_ACT_ALLOW");
    let names = rules[0]["names"].as_array().expect("syscalls");
    // Chromium 154.0.8037.98 PreSandboxInit queries physical memory before the
    // zygote boot message. glibc 2.41 __get_phys_pages uses sysinfo without
    // checking failure; denying this read-only query breaks sandbox startup.
    for required in ["clone", "unshare", "seccomp", "prctl", "sysinfo"] {
        assert!(names.iter().any(|name| name == required));
    }
    for forbidden in [
        "bpf",
        "ptrace",
        "process_vm_readv",
        "process_vm_writev",
        "keyctl",
        "add_key",
        "request_key",
        "kexec_load",
        "kexec_file_load",
        "perf_event_open",
        "init_module",
        "finit_module",
        "delete_module",
        "mount",
        "setns",
    ] {
        assert!(!names.iter().any(|name| name == forbidden));
    }
}

#[tokio::test]
async fn private_socket_binding_uses_retained_directory_for_deep_state_paths() {
    use std::os::fd::AsRawFd as _;
    let temporary = tempfile::tempdir().unwrap();
    let deep = temporary.path().join("a".repeat(100)).join("b".repeat(100));
    std::fs::create_dir_all(&deep).unwrap();
    let directory = std::fs::File::open(&deep).unwrap();
    let listener = launch::listener(&directory, "bootstrap.sock").unwrap();
    assert!(deep.join("bootstrap.sock").exists());
    let endpoint = format!("/proc/self/fd/{}/bootstrap.sock", directory.as_raw_fd());
    let client = tokio::net::UnixStream::connect(endpoint).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    assert_eq!(
        relay::verify_peer(
            &server,
            rustix::process::getpid().as_raw_pid() as u32,
            rustix::process::geteuid().as_raw(),
            rustix::process::getegid().as_raw()
        ),
        Ok(())
    );
    drop(client);
}

#[test]
fn lost_create_response_cannot_acknowledge_early_absence_before_late_daemon_allocation() {
    let temporary = tempfile::tempdir().unwrap();
    let mut resources = resources(Arc::new(installation(temporary.path())));
    resources.create_attempted = true;
    assert_eq!(
        resources.reconcile_creation(None),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(resources.container.is_none());
    assert!(!resources.closed);
    assert!(resources.control.path().is_dir());
    let cid = "b".repeat(64);
    let late = engine::Inspection {
        id: cid.clone(),
        pid: 42,
        running: true,
        exit_code: 0,
        oom_killed: false,
        image: resources.installation.image.clone(),
        label: resources.nonce.clone(),
        network: "none".into(),
        user: format!(
            "{}:{}",
            resources.installation.uid, resources.installation.gid
        ),
    };
    assert_eq!(resources.reconcile_creation(Some(late)), Ok(()));
    assert_eq!(resources.container, Some(cid));
    assert!(!resources.closed);
}

#[tokio::test]
async fn cleanup_preserves_replaced_control_directory_and_retains_obligation() {
    let temporary = tempfile::tempdir().unwrap();
    let mut resources = resources(Arc::new(installation(temporary.path())));
    let original = resources.control.path().to_owned();
    let retained = temporary.path().join("actual-owned-directory");
    std::fs::rename(&original, &retained).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
    let marker = original.join("replacement-must-survive");
    std::fs::write(&marker, b"unowned replacement").unwrap();
    assert_eq!(
        resources.cleanup().await,
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(
        resources.fenced.load(Ordering::Acquire),
        "failed physical cleanup still fences new browser dispatch"
    );
    assert!(marker.is_file());
    assert!(retained.is_dir());
    assert!(!resources.closed);
    assert_eq!(
        resources.shutdown_receipt,
        OciBrowserShutdownReceipt::Unknown
    );
}

#[tokio::test]
async fn known_no_launch_cleanup_is_idempotent_and_releases_owned_artifacts() {
    let temporary = tempfile::tempdir().unwrap();
    let installation = Arc::new(installation(temporary.path()));
    let mut resources = resources(Arc::clone(&installation));
    let path = resources.control.path().to_owned();
    assert_eq!(resources.cleanup().await, Ok(()));
    assert_eq!(resources.cleanup().await, Ok(()));
    assert_eq!(
        resources.shutdown_receipt,
        OciBrowserShutdownReceipt::NoProcess
    );
    assert!(!path.exists());
    drop(resources);
    drop(installation);
    assert!(!temporary.path().exists());
}

#[test]
fn installation_cleanup_refuses_active_or_replaced_ownership_and_can_retry() {
    let temporary = tempfile::tempdir().unwrap();
    let original = temporary.path().join("installation");
    std::fs::create_dir(&original).unwrap();
    let installation = installation(&original);
    installation.artifacts.admit();
    assert_eq!(
        installation::discard(&installation),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(original.is_dir());
    installation.artifacts.release();
    let retained = temporary.path().join("retained");
    std::fs::rename(&original, &retained).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
    let marker = original.join("unowned-marker");
    std::fs::write(&marker, b"preserve").unwrap();
    assert_eq!(
        installation::discard(&installation),
        Err(BrowserDriverError::OutcomeUnknown)
    );
    assert!(marker.is_file());
    std::fs::remove_dir_all(&original).unwrap();
    std::fs::rename(retained, &original).unwrap();
    assert_eq!(installation::discard(&installation), Ok(()));
    assert_eq!(installation::discard(&installation), Ok(()));
    assert!(!original.exists());
}
