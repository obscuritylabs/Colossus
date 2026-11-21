//! Opt-in integration against a real local OPA process.

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, DecisionOutcome, EffectRequest, QuarantinedEffectResult,
};
use colossus_policy::{
    AllowApproval, EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit, GatewayError,
    OpaConfig, OpaPolicy, SafetyKernel, effect_request, system_actor,
};
use colossus_ports::{EventJournal, PolicyDecisionPoint, PolicyError};
use colossus_testkit::InMemoryEventJournal;
use serde_json::{Value, json};
use std::{
    env, io,
    net::{SocketAddr, TcpListener},
    path::Path,
    process::{Command as StdCommand, Stdio},
    sync::Arc,
    time::Duration,
};
use tempfile::tempdir;
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
    process::{Child, Command},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout, timeout_at},
};

const POLICY: &str = r#"
package colossus

base_obligations := {
  "sandbox_backend": "native",
  "sandbox_profile": "opa-live-v1",
  "filesystem": [],
  "network_destinations": [],
  "allowed_environment": [],
  "allow_sandbox_downgrade": false,
  "timeout_ms": 2000,
  "max_output_bytes": 4096,
  "max_processes": 4,
  "max_memory_bytes": 67108864,
  "max_concurrency": 1,
  "required_redactions": [],
  "require_post_effect": false,
  "audit_labels": {"suite": "opa-live"},
  "retention": "test"
}

decision(outcome, reason, post_effect) := result if {
  result := {
    "decision_id": sprintf("live-%s-%s", [input.action, input.phase]),
    "policy_revision": "opa-live-v1",
    "outcome": outcome,
    "reason": reason,
    "obligations": object.union(base_obligations, {"require_post_effect": post_effect})
  }
}

default effect := null

effect := decision("allow", "explicit allow", false) if {
  input.action == "provider.echo"
}

effect := decision("allow", "complete redacted disclosure accepted", false) if {
  input.action == "test.disclosure"
  input.content.prompt == "classify the complete request"
  input.content.options.mode == "strict"
  input.content.options.limit == 7
  input.content.api_key.redacted == true
  input.content.api_key.sha256 != ""
  input.content.api_key.size > 0
  input.content.credential_reference == "env:OPENROUTER_API_KEY"
  input.credential_references[0].reference == "env:OPENROUTER_API_KEY"
  input.credential_references[0].value_hash == "sha256:live-reference"
  not contains(json.marshal(input), "live-raw-secret")
}

effect := decision("deny", "explicit deny", false) if {
  input.action == "test.deny"
}

effect := decision("require_approval", "operator approval required", false) if {
  input.action == "test.approval"
  input.approval == null
}

effect := decision("allow", "approval proof accepted", false) if {
  input.action == "test.approval"
  input.approval != null
}

effect := decision("allow", "quarantine before release", true) if {
  input.action == "test.post"
  input.phase == "pre_effect"
}

effect := decision("deny", "post-effect content denied", true) if {
  input.action == "test.post"
  input.phase == "post_effect"
}

effect := {"decision_id": "invalid-live-response"} if {
  input.action == "test.invalid"
}
"#;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(50);
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_millis(250);
const REAP_TIMEOUT: Duration = Duration::from_secs(1);
const STARTUP_STDERR_LIMIT: usize = 4096;

struct OpaProcess {
    child: Child,
    address: SocketAddr,
    started: Instant,
    stderr: JoinHandle<io::Result<(Vec<u8>, bool)>>,
}

impl OpaProcess {
    fn spawn(command: &mut Command, address: SocketAddr) -> Self {
        let started = Instant::now();
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start OPA");
        let mut child_stderr = child.stderr.take().expect("OPA stderr pipe");
        let stderr = tokio::spawn(async move {
            let mut retained = Vec::with_capacity(STARTUP_STDERR_LIMIT);
            let mut buffer = [0; 1024];
            let mut truncated = false;
            loop {
                let count = child_stderr.read(&mut buffer).await?;
                if count == 0 {
                    return Ok((retained, truncated));
                }
                // Keep draining after reaching the cap so OPA cannot block on stderr.
                let copy = count.min(STARTUP_STDERR_LIMIT - retained.len());
                retained.extend_from_slice(&buffer[..copy]);
                truncated |= copy < count;
            }
        });
        Self {
            child,
            address,
            started,
            stderr,
        }
    }

    async fn wait_until_ready(&mut self, policy: &OpaPolicy) {
        let deadline = self.started + STARTUP_TIMEOUT;
        let mut last_error = None;
        let mut polls = 0;
        let reason = loop {
            if let Some(failure) = self.child_failure() {
                break failure;
            }
            if Instant::now() >= deadline {
                break "startup deadline elapsed".into();
            }
            polls += 1;
            match timeout_at(deadline, PolicyDecisionPoint::doctor(policy)).await {
                Ok(Ok(_)) => match self.child_failure() {
                    Some(failure) => break failure,
                    None => return,
                },
                Ok(Err(error)) => last_error = Some(error.to_string()),
                Err(_) => {
                    break self
                        .child_failure()
                        .unwrap_or_else(|| "health request exceeded startup deadline".into());
                }
            }
            sleep_until((Instant::now() + STARTUP_POLL_INTERVAL).min(deadline)).await;
        };

        let elapsed = self.started.elapsed();
        // A direct TCP probe can diagnose transport failures, but readiness still requires doctor.
        let tcp = match timeout(DIAGNOSTIC_TIMEOUT, TcpStream::connect(self.address)).await {
            Ok(Ok(_)) => "connected".into(),
            Ok(Err(error)) => format!("failed ({error})"),
            Err(_) => "timed out".into(),
        };
        let cleanup = timeout(REAP_TIMEOUT, self.child.kill()).await;
        let stderr = match timeout(DIAGNOSTIC_TIMEOUT, &mut self.stderr).await {
            Ok(Ok(Ok((retained, truncated)))) => {
                let suffix = if truncated { " (truncated)" } else { "" };
                format!("{:?}{suffix}", String::from_utf8_lossy(&retained))
            }
            Ok(Ok(Err(error))) => format!("read failed ({error})"),
            Ok(Err(error)) => format!("capture failed ({error})"),
            Err(_) => "capture timed out".into(),
        };
        panic!(
            "OPA did not become ready at {} after {elapsed:?} ({polls} health requests): {reason}; last health error: {last_error:?}; direct TCP: {tcp}; cleanup: {cleanup:?}; startup stderr: {stderr}",
            self.address
        );
    }

    fn child_failure(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => Some(format!("child exited ({status})")),
            Err(error) => Some(format!("could not read child status ({error})")),
        }
    }
}

impl Drop for OpaProcess {
    fn drop(&mut self) {
        self.stderr.abort();
    }
}

fn opa_command(binary: &Path, address: SocketAddr) -> Command {
    let mut command = Command::new(binary);
    command
        .args(["run", "--server", "--addr"])
        .arg(address.to_string());
    command
}

fn start_opa_mtls(
    binary: &Path,
    policy: &Path,
    address: SocketAddr,
    ca: &Path,
    certificate: &Path,
    private_key: &Path,
) -> OpaProcess {
    OpaProcess::spawn(
        opa_command(binary, address)
            .args(["--authentication=tls", "--tls-ca-cert-file"])
            .arg(ca)
            .arg("--tls-cert-file")
            .arg(certificate)
            .arg("--tls-private-key-file")
            .arg(private_key)
            .arg(policy),
        address,
    )
}

fn openssl(binary: &Path, arguments: &[&str], directory: &Path) {
    let output = StdCommand::new(binary)
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run openssl");
    assert!(
        output.status.success(),
        "openssl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn generate_mtls_identity(openssl_binary: &Path, directory: &Path) {
    openssl(
        openssl_binary,
        &[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "ca.key",
            "-out",
            "ca.crt",
            "-days",
            "1",
            "-subj",
            "/CN=Colossus Test CA",
        ],
        directory,
    );
    openssl(
        openssl_binary,
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "server.key",
            "-out",
            "server.csr",
            "-subj",
            "/CN=127.0.0.1",
            "-addext",
            "subjectAltName=IP:127.0.0.1",
            "-addext",
            "extendedKeyUsage=serverAuth",
        ],
        directory,
    );
    openssl(
        openssl_binary,
        &[
            "x509",
            "-req",
            "-in",
            "server.csr",
            "-CA",
            "ca.crt",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "server.crt",
            "-days",
            "1",
            "-copy_extensions",
            "copy",
        ],
        directory,
    );
    openssl(
        openssl_binary,
        &[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "client.key",
            "-out",
            "client.csr",
            "-subj",
            "/CN=colossus-client",
            "-addext",
            "extendedKeyUsage=clientAuth",
        ],
        directory,
    );
    openssl(
        openssl_binary,
        &[
            "x509",
            "-req",
            "-in",
            "client.csr",
            "-CA",
            "ca.crt",
            "-CAkey",
            "ca.key",
            "-CAcreateserial",
            "-out",
            "client.crt",
            "-days",
            "1",
            "-copy_extensions",
            "copy",
        ],
        directory,
    );
}

fn opa_policy(base_url: String) -> OpaPolicy {
    OpaPolicy::new(OpaConfig {
        base_url,
        decision_path: "colossus/effect".into(),
        ca_pem: None,
        tls_roots: Default::default(),
        identity_pem: None,
        full_content_disclosure_acknowledged: true,
        decision_log_masking_verified: false,
        timeout: Duration::from_secs(2),
    })
    .expect("OPA policy")
}

struct StaticExecutor(&'static [u8]);

#[async_trait]
impl EffectExecutor for StaticExecutor {
    async fn execute(
        &self,
        _request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        Ok(QuarantinedEffectResult {
            media_type: "text/plain".into(),
            bytes: self.0.to_vec(),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
#[ignore = "requires COLOSSUS_OPA_BIN pointing to a real OPA binary"]
async fn live_opa_enforces_decisions_approval_release_readiness_and_outage() {
    let binary = Path::new(&env::var("COLOSSUS_OPA_BIN").expect("OPA binary path")).to_owned();
    let directory = tempdir().expect("directory");
    let policy_path = directory.path().join("policy.rego");
    std::fs::write(&policy_path, POLICY).expect("policy");
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve OPA port");
    let address = listener.local_addr().expect("OPA address");
    drop(listener);
    let base_url = format!("http://{address}/");
    let mut process = OpaProcess::spawn(opa_command(&binary, address).arg(&policy_path), address);
    let policy = opa_policy(base_url.clone());
    process.wait_until_ready(&policy).await;

    let doctor = PolicyDecisionPoint::doctor(&policy)
        .await
        .expect("OPA doctor");
    assert_eq!(doctor["ready"], Value::Bool(true));
    assert!(doctor["warning"].as_str().is_some());

    let allow = PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("opa-live"),
            "provider.echo",
            "provider:echo",
            json!({"message": "ok"}),
        ),
    )
    .await
    .expect("allow decision");
    assert_eq!(allow.outcome, DecisionOutcome::Allow);
    assert_eq!(allow.policy_revision, "opa-live-v1");

    let deny = PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("opa-live"),
            "test.deny",
            "test:deny",
            json!({}),
        ),
    )
    .await
    .expect("deny decision");
    assert_eq!(deny.outcome, DecisionOutcome::Deny);

    let invalid = PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("opa-live"),
            "test.invalid",
            "test:invalid",
            json!({}),
        ),
    )
    .await
    .expect_err("invalid live response");
    assert!(matches!(invalid, PolicyError::InvalidDecision(_)));

    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let gateway = EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(opa_policy(base_url.clone())),
        Arc::new(AllowApproval {
            approved_by: "operator".into(),
        }),
        SafetyKernel::new([
            "test.approval".into(),
            "test.disclosure".into(),
            "test.post".into(),
            "provider.echo".into(),
        ]),
        [9_u8; 32],
    );
    let mut disclosure = effect_request(
        system_actor("opa-live"),
        "test.disclosure",
        "test:disclosure",
        json!({
            "prompt": "classify the complete request",
            "options": {"mode": "strict", "limit": 7},
            "api_key": "live-raw-secret",
            "credential_reference": "env:OPENROUTER_API_KEY"
        }),
    );
    disclosure.capabilities = vec!["test.disclosure".into()];
    disclosure.credential_references = vec![CredentialReference {
        reference: "env:OPENROUTER_API_KEY".into(),
        value_hash: Some("sha256:live-reference".into()),
    }];
    let disclosed = gateway
        .execute(disclosure, &StaticExecutor(b"disclosed"))
        .await
        .expect("OPA must receive complete content with hard secrets redacted");
    assert_eq!(disclosed.bytes, b"disclosed");

    let mut approval = effect_request(
        system_actor("opa-live"),
        "test.approval",
        "test:approval",
        json!({"operation": "approved"}),
    );
    approval.capabilities = vec!["test.approval".into()];
    let approved = gateway
        .execute(approval, &StaticExecutor(b"approved"))
        .await
        .expect("OPA approval re-evaluation");
    assert_eq!(approved.bytes, b"approved");

    let mut post = effect_request(
        system_actor("opa-live"),
        "test.post",
        "test:post",
        json!({"operation": "quarantined"}),
    );
    post.capabilities = vec!["test.post".into()];
    let denied_release = gateway
        .execute(post, &StaticExecutor(b"never release this"))
        .await
        .expect_err("post-effect deny");
    assert!(matches!(denied_release, GatewayError::Denied(_)));
    let events = journal.read_global(1, 100).expect("journal events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "effect.release_denied.v1")
    );

    process.child.kill().await.expect("stop and reap OPA");
    let outage = PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("opa-live"),
            "provider.echo",
            "provider:echo",
            json!({"message": "outage"}),
        ),
    )
    .await
    .expect_err("OPA outage");
    assert!(matches!(outage, PolicyError::Unavailable(_)));
}

#[tokio::test]
#[ignore = "requires COLOSSUS_OPA_BIN and COLOSSUS_OPENSSL_BIN"]
async fn live_opa_mtls_requires_and_accepts_a_pinned_client_identity() {
    let opa_binary = Path::new(&env::var("COLOSSUS_OPA_BIN").expect("OPA binary path")).to_owned();
    let openssl_binary =
        Path::new(&env::var("COLOSSUS_OPENSSL_BIN").expect("openssl binary path")).to_owned();
    let directory = tempdir().expect("directory");
    let policy_path = directory.path().join("policy.rego");
    std::fs::write(&policy_path, POLICY).expect("policy");
    generate_mtls_identity(&openssl_binary, directory.path());
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve OPA TLS port");
    let address = listener.local_addr().expect("OPA TLS address");
    drop(listener);
    let mut process = start_opa_mtls(
        &opa_binary,
        &policy_path,
        address,
        &directory.path().join("ca.crt"),
        &directory.path().join("server.crt"),
        &directory.path().join("server.key"),
    );
    let ca = std::fs::read(directory.path().join("ca.crt")).expect("CA");
    let mut identity = std::fs::read(directory.path().join("client.crt")).expect("client cert");
    identity.extend_from_slice(b"\n");
    identity.extend_from_slice(
        &std::fs::read(directory.path().join("client.key")).expect("client key"),
    );
    let base_url = format!("https://{address}/");
    let policy = OpaPolicy::new(OpaConfig {
        base_url: base_url.clone(),
        decision_path: "colossus/effect".into(),
        ca_pem: Some(ca.clone()),
        tls_roots: Default::default(),
        identity_pem: Some(identity),
        full_content_disclosure_acknowledged: true,
        decision_log_masking_verified: true,
        timeout: Duration::from_secs(2),
    })
    .expect("mTLS OPA policy");
    process.wait_until_ready(&policy).await;
    let decision = PolicyDecisionPoint::decide(
        &policy,
        &effect_request(
            system_actor("opa-mtls"),
            "provider.echo",
            "provider:echo",
            json!({"message": "mTLS"}),
        ),
    )
    .await
    .expect("mTLS decision");
    assert_eq!(decision.outcome, DecisionOutcome::Allow);

    let missing_identity = OpaPolicy::new(OpaConfig {
        base_url,
        decision_path: "colossus/effect".into(),
        ca_pem: Some(ca),
        tls_roots: Default::default(),
        identity_pem: None,
        full_content_disclosure_acknowledged: true,
        decision_log_masking_verified: true,
        timeout: Duration::from_millis(500),
    })
    .expect("local TLS policy without identity");
    let error = PolicyDecisionPoint::doctor(&missing_identity)
        .await
        .expect_err("OPA must reject a client without an identity");
    assert!(matches!(error, PolicyError::Unavailable(_)));
    process.child.kill().await.expect("stop and reap mTLS OPA");
}
