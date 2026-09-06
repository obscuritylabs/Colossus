//! Runtime-to-Distribution acceptance: permits, helper isolation, and released evidence.

use super::plugin_authorization_tests::{open, replace_policy};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const SECRET: &str = "runtime-registry-fixture-secret";

struct DenyHelperRelease(BuiltInPolicy);

#[async_trait]
impl PolicyDecisionPoint for DenyHelperRelease {
    async fn decide(
        &self,
        request: &EffectRequest,
    ) -> Result<colossus_contracts::PolicyDecision, PolicyError> {
        let mut decision = self.0.decide(request).await?;
        if request.action == "plugin.registry.credential_helper"
            && request.phase == colossus_contracts::EffectPhase::PostEffect
        {
            decision.outcome = DecisionOutcome::Deny;
        }
        Ok(decision)
    }
    async fn doctor(&self) -> Result<Value, PolicyError> {
        self.0.doctor().await
    }
}

struct Helper {
    calls: AtomicUsize,
    config: PathBuf,
}

#[async_trait]
impl EffectExecutor for Helper {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        assert_eq!(request.action, "plugin.registry.credential_helper");
        assert_eq!(request.content["args"], json!(["get"]));
        self.calls.fetch_add(1, Ordering::SeqCst);
        // If transport rereads config after helper selection, this operation will fail.
        fs::write(&self.config, b"changed after the single authorized parse")
            .expect("mutate fixture");
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(), effect_succeeded: true,
            bytes: serde_json::to_vec(&json!({"success":true,"exit_code":0,"output_truncated":false,
                "stdout_base64": BASE64.encode(serde_json::to_vec(&json!({"Username":"fixture", "Secret":SECRET})).expect("credential"))})).expect("process result"),
        })
    }
}

struct Registry {
    origin: String,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Registry {
    async fn start(artifact: colossus_plugins::BuiltPluginArtifact) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback registry");
        let origin = format!("http://{}", listener.local_addr().expect("address"));
        let calls = Arc::new(AtomicUsize::new(0));
        let recorded = Arc::clone(&calls);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let (head, body) = loop {
                    let mut chunk = [0; 8192];
                    let count = stream.read(&mut chunk).await.expect("request");
                    assert_ne!(count, 0, "complete request");
                    bytes.extend_from_slice(&chunk[..count]);
                    assert!(bytes.len() < 1024 * 1024, "bounded fixture");
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let head = String::from_utf8(bytes[..end].to_vec()).expect("head");
                        let length = head
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().expect("length"))
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break (head, bytes[end + 4..end + 4 + length].to_vec());
                        }
                    }
                };
                recorded.fetch_add(1, Ordering::SeqCst);
                let expected = format!("Basic {}", BASE64.encode(format!("fixture:{SECRET}")));
                assert!(
                    head.lines()
                        .any(|line| line.split_once(':').is_some_and(|(name, value)| name
                            .eq_ignore_ascii_case("authorization")
                            && value.trim() == expected)),
                    "authorized fixture request"
                );
                let mut line = head
                    .lines()
                    .next()
                    .expect("request line")
                    .split_whitespace();
                let method = line.next().expect("method");
                let path = line.next().expect("path");
                let (status, body) = if method == "HEAD" {
                    (200, vec![])
                } else if method == "PUT" {
                    assert_eq!(body, artifact.manifest);
                    (201, vec![])
                } else if path.contains("/manifests/") {
                    (200, artifact.manifest.clone())
                } else if path.contains("/referrers/") {
                    (200, br#"{"schemaVersion":2,"manifests":[]}"#.to_vec())
                } else if path.ends_with(&artifact.parsed_manifest.config.digest) {
                    (200, artifact.config.clone())
                } else if path.ends_with(&artifact.parsed_manifest.layers[0].digest) {
                    (200, artifact.layer.clone())
                } else {
                    panic!("unexpected registry request {method} {path}");
                };
                let media_type = if path.contains("/manifests/") {
                    colossus_plugins::OCI_IMAGE_MANIFEST_MEDIA_TYPE
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: {media_type}\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("response");
                stream.write_all(&body).await.expect("body");
            }
        });
        Self {
            origin,
            calls,
            task,
        }
    }
}

#[tokio::test]
async fn plugin_runtime_registry_transfers_enforce_helper_permits_and_redact_credentials() {
    let temporary = crate::test_support::private_tempdir();
    let root = temporary.path().canonicalize().expect("root");
    let artifact = colossus_bundled_plugins::core_artifact().expect("artifact");
    let digest = artifact.manifest_digest.clone();
    let registry = Registry::start(artifact).await;
    let config_path = root.join("docker.json");
    let executable = fs::canonicalize(std::env::current_exe().expect("test executable"))
        .expect("canonical executable");
    fs::write(&config_path, br#"{"credsStore":"fixture"}"#).expect("config");
    let config = PluginsConfig {
        registries: BTreeMap::from([(
            "local".into(),
            PluginRegistryProfile {
                origin: registry.origin.clone(),
                trust_profile: "default".into(),
                allow_non_public: true,
                auth: RegistryAuthConfig::Docker {
                    config_path: Some(config_path.clone()),
                    helper_executables: BTreeMap::from([("fixture".into(), executable.clone())]),
                },
                ..PluginRegistryProfile::default()
            },
        )]),
        ..PluginsConfig::default()
    };
    let mut runtime = open(&root, config);
    let helper = Arc::new(Helper {
        calls: AtomicUsize::new(0),
        config: config_path.clone(),
    });
    runtime.process_executor = helper.clone();
    let workspace = runtime.workspace.clone();
    let policy = || {
        BuiltInPolicy::offline_default()
            .with_sandbox("native", "plugin-registry-fixture", false)
            .with_action("plugin.pull", DecisionOutcome::Allow)
            .with_action("plugin.push", DecisionOutcome::Allow)
            .with_filesystem_root(workspace.display().to_string(), "write")
            .with_filesystem_root(config_path.display().to_string(), "read")
            .with_network_destination(&registry.origin)
    };
    let reference = format!(
        "{}/team/core:latest",
        registry.origin.trim_start_matches("http://")
    );
    let output = workspace.join("pulled");
    replace_policy(&mut runtime, Arc::new(policy()));
    assert!(
        runtime
            .pull_plugin("local", &reference, &output)
            .await
            .is_err()
    );
    assert_eq!(helper.calls.load(Ordering::SeqCst), 0);
    assert_eq!(registry.calls.load(Ordering::SeqCst), 0);
    replace_policy(
        &mut runtime,
        Arc::new(policy().with_action("plugin.registry.credential_helper", DecisionOutcome::Allow)),
    );
    let error = runtime
        .pull_plugin("local", &reference, &output)
        .await
        .expect_err("helper executable grant");
    assert!(
        error.to_string().contains("not explicitly granted"),
        "{error}"
    );
    assert_eq!(helper.calls.load(Ordering::SeqCst), 0);
    replace_policy(
        &mut runtime,
        Arc::new(DenyHelperRelease(
            policy()
                .with_action("plugin.registry.credential_helper", DecisionOutcome::Allow)
                .with_filesystem_root(executable.display().to_string(), "execute"),
        )),
    );
    assert!(
        runtime
            .pull_plugin("local", &reference, &output)
            .await
            .is_err()
    );
    assert_eq!(helper.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.calls.load(Ordering::SeqCst),
        0,
        "denied helper output must not reach the transport"
    );
    fs::write(&config_path, br#"{"credsStore":"fixture"}"#)
        .expect("restore config after denied release");
    replace_policy(
        &mut runtime,
        Arc::new(
            policy()
                .with_action("plugin.registry.credential_helper", DecisionOutcome::Allow)
                .with_filesystem_root(executable.display().to_string(), "execute"),
        ),
    );
    let pulled = runtime
        .pull_plugin("local", &reference, &output)
        .await
        .expect("runtime pull");
    assert_eq!(pulled.manifest_digest, digest);
    fs::write(&config_path, br#"{"credsStore":"fixture"}"#).expect("restore helper config");
    assert_eq!(
        runtime
            .push_plugin("local", &output, &reference)
            .await
            .expect("runtime push")
            .manifest_digest,
        digest
    );
    assert_eq!(helper.calls.load(Ordering::SeqCst), 3);
    fs::write(&config_path, serde_json::to_vec(&json!({"auths":{registry.origin.trim_start_matches("http://"): {"auth":BASE64.encode(format!("fixture:{SECRET}"))}}})).expect("auths")).expect("inline Docker credential");
    runtime
        .pull_plugin("local", &reference, workspace.join("auths-pull"))
        .await
        .expect("authorized Docker auths");
    assert_eq!(helper.calls.load(Ordering::SeqCst), 3);
    assert!(registry.calls.load(Ordering::SeqCst) >= 10);
    let events = runtime.journal.read_global(1, 1024).expect("audit");
    let payloads: Vec<_> = events
        .iter()
        .map(|event| runtime.journal.decrypt_payload(event).expect("payload"))
        .collect();
    let evidence = serde_json::to_string(&payloads).expect("evidence");
    assert!(evidence.contains("plugin.registry.credential_helper"));
    assert!(!evidence.contains(SECRET));
    assert!(!evidence.contains("stdout_base64"));
    assert!(!evidence.contains(&BASE64.encode(format!("fixture:{SECRET}"))));
}
