//! Run the real Rust SDK against an isolated local worker without a credential store.
//!
//! This is a development acceptance harness, not an enrollment mechanism. The public
//! API authentication root, TLS seed, instance identity, and application bearer exist
//! only in this process. The bearer is never written to a file, argument, environment
//! variable, descriptor, log, or renderer.

use async_trait::async_trait;
use colossus_api::{ApiScope, ApplicationKind, scopes};
use colossus_grpc::{TlsIdentity, TlsKeySeed};
use colossus_runtime::{RuntimeConfig, RuntimeOpenOptions};
use colossus_sdk::{
    ApiMajor, Colossus, CreateRunRequest, CredentialProvider, DaemonConnectOptions, IdempotencyKey,
    InputContentPart, InstanceId, RunMode, RunUpdateKind, Secret, TlsFingerprint, WatchRunRequest,
};
use colossus_worker::{
    ApplicationGrant, PublicApiAuthenticationKey, PublicApiHostOptions, WorkerApprovalMode,
    WorkerServer,
};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt,
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};
#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt as _};
use tempfile::TempDir;
use tokio::sync::oneshot;
use uuid::Uuid;
use zeroize::Zeroizing;

struct MemoryCredential {
    bytes: Zeroizing<Vec<u8>>,
}

impl MemoryCredential {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }
}

impl fmt::Debug for MemoryCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MemoryCredential([REDACTED])")
    }
}

#[async_trait]
impl CredentialProvider for MemoryCredential {
    async fn load(&self) -> colossus_sdk::SdkResult<Secret> {
        Secret::new(self.bytes.as_slice().to_vec())
    }
}

struct Options {
    config: PathBuf,
    client: ClientKind,
    prompt: String,
}

#[derive(Clone, Copy)]
enum ClientKind {
    Rust,
    Python,
    TypeScript,
    Go,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let config = RuntimeConfig::from_path(&options.config)?;
    let workspace = RuntimeOpenOptions::for_workspace(env::current_dir()?)?;
    let server =
        WorkerServer::open_with_mode_at_workspace(&config, WorkerApprovalMode::Deny, workspace)?;

    let mut authentication_root = [0_u8; 32];
    getrandom::fill(&mut authentication_root)?;
    let credentials =
        server.public_api_credential_manager(PublicApiAuthenticationKey::new(authentication_root));
    let grant = ApplicationGrant::new(
        "app:sdk-ephemeral-local",
        ApplicationKind::Enrolled,
        [
            ApiScope::new(scopes::RUNS_EXECUTE)?,
            ApiScope::new(scopes::RUNS_READ)?,
            ApiScope::new(scopes::RUNS_CONTROL)?,
            ApiScope::new(scopes::PROMPTS_RESPOND)?,
        ],
        ["primary".to_owned()],
        Vec::<String>::new(),
    )?;
    let issued = credentials.issue_pending(&grant)?;
    let credential_id = issued.credential_id().to_owned();
    let bearer = Zeroizing::new(issued.expose_token().as_bytes().to_vec());
    if !credentials.activate(&credential_id)? {
        return Err("ephemeral SDK credential activation failed".into());
    }
    drop(issued);

    let discovery = TempDir::new()?;
    #[cfg(unix)]
    fs::set_permissions(discovery.path(), fs::Permissions::from_mode(0o700))?;
    let descriptor_path = discovery.path().join("endpoint.json");
    let certificate_path = discovery.path().join("certificate.pem");
    let mut tls_seed = [0_u8; 32];
    getrandom::fill(&mut tls_seed)?;
    let tls_identity = TlsIdentity::from_seed(TlsKeySeed::new(tls_seed))?;
    let instance_uuid = Uuid::now_v7();
    let host = PublicApiHostOptions::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        instance_uuid,
        &descriptor_path,
        &certificate_path,
        tls_identity,
        &credentials,
    )?;
    let server = server.enable_public_api(host).await?;
    let ready = server
        .public_api_ready_metadata()
        .ok_or("ephemeral public API did not publish ready metadata")?;
    let instance_id = InstanceId::from_uuid(ready.instance_id());
    let certificate_pin = TlsFingerprint::from_hex(ready.certificate_sha256())?;
    let expected_instance_id = ready.instance_id().to_string();
    let expected_certificate_sha256 = ready.certificate_sha256().to_owned();

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .serve_until(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    let outcome = match options.client {
        ClientKind::Rust => {
            run_rust_prompt(
                &descriptor_path,
                &certificate_path,
                instance_id,
                certificate_pin,
                Arc::new(MemoryCredential::new(bearer.as_slice().to_vec())),
                options.prompt,
            )
            .await
        }
        client => run_external_client(
            client,
            &descriptor_path,
            &certificate_path,
            &expected_instance_id,
            &expected_certificate_sha256,
            bearer.as_slice(),
            &options.prompt,
        ),
    };
    let _ = shutdown_tx.send(());
    server_task.await??;
    outcome
}

async fn run_rust_prompt(
    descriptor_path: &Path,
    certificate_path: &Path,
    instance_id: InstanceId,
    certificate_pin: TlsFingerprint,
    credential: Arc<MemoryCredential>,
    prompt: String,
) -> Result<(), Box<dyn Error>> {
    let connect = DaemonConnectOptions::new(
        instance_id,
        descriptor_path,
        certificate_pin,
        ApiMajor::new(1)?,
        credential,
    )?
    .with_certificate_path(certificate_path)?;
    let client = Colossus::connect_installed(connect).await?;
    let created = client
        .create_run(CreateRunRequest {
            input: vec![InputContentPart::Text(prompt)],
            session_id: None,
            role: "primary".to_owned(),
            mode: RunMode::Execute,
            selected_skills: Vec::new(),
            plan_action: None,
            max_turns: 12,
            idempotency_key: IdempotencyKey::new(format!(
                "sdk-ephemeral-create-{}",
                Uuid::now_v7()
            ))?,
        })
        .await?;
    let run_id = created.run.run_id;
    let mut updates = client
        .watch_run(WatchRunRequest {
            run_id,
            after_sequence: 0,
        })
        .await?;
    while let Some(update) = updates.next_update().await {
        match update?.update {
            RunUpdateKind::ToolActivity(activity) => {
                eprintln!(
                    "tool {} {:?}: {}",
                    activity.tool_name, activity.state, activity.summary
                );
            }
            RunUpdateKind::Result(result) => {
                println!("{}", result.output);
                client.close().await?;
                return Ok(());
            }
            RunUpdateKind::Failure { failure, .. } => {
                return Err(format!(
                    "run failed: {} (reason={}, recoverable={}, outcome={:?}, http_status={:?}, retry_after_ms={:?})",
                    failure.message,
                    failure.reason,
                    failure.recoverable,
                    failure.outcome_certainty,
                    failure.http_status,
                    failure.retry_after_ms,
                )
                .into());
            }
            RunUpdateKind::Cancellation(cancellation) => {
                return Err(format!("run cancelled: {}", cancellation.message).into());
            }
            RunUpdateKind::Interaction(interaction) if interaction.respondable_by_caller => {
                return Err(
                    "ephemeral smoke run requested an interaction; use the enrolled durable example for interactive scenarios"
                        .into(),
                );
            }
            _ => {}
        }
    }
    Err("run watch ended without an exact terminal update".into())
}

fn run_external_client(
    client: ClientKind,
    descriptor_path: &Path,
    certificate_path: &Path,
    expected_instance_id: &str,
    expected_certificate_sha256: &str,
    bearer: &[u8],
    prompt: &str,
) -> Result<(), Box<dyn Error>> {
    let repository = env::current_dir()?;
    let (program, script) = match client {
        ClientKind::Python => (
            repository.join("sdk/python/.codegen/bin/python"),
            repository.join("sdk/python/examples/live_run.py"),
        ),
        ClientKind::TypeScript => (
            PathBuf::from("node"),
            repository.join("sdk/typescript/.live-dist/examples/live-run.js"),
        ),
        ClientKind::Go => (
            repository.join("target/sdk-go-live"),
            repository.join("target/sdk-go-live"),
        ),
        ClientKind::Rust => return Err("Rust client must run in process".into()),
    };
    if client_requires_script(client) && !script.is_file() {
        return Err(format!(
            "{} live runner is not built; follow examples/sdk/README.md",
            client_name(client)
        )
        .into());
    }
    if !client_requires_script(client) && !program.is_file() {
        return Err("Go live runner is not built; follow examples/sdk/README.md".into());
    }

    let mut command = Command::new(program);
    if client_requires_script(client) {
        command.arg(script);
    }
    command.args([
        descriptor_path.as_os_str(),
        certificate_path.as_os_str(),
        OsStr::new(expected_instance_id),
        OsStr::new(expected_certificate_sha256),
        OsStr::new(prompt),
    ]);
    command
        .current_dir(&repository)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    restrict_child_environment(&mut command);
    if matches!(client, ClientKind::Python) {
        command.env(
            "PYTHONPATH",
            env::join_paths([
                repository.join("sdk/python/src"),
                repository.join("sdk/python/generated"),
            ])?,
        );
        if env::var_os("COLOSSUS_SDK_E2E_GRPC_TRACE").is_some() {
            command.env("GRPC_VERBOSITY", "DEBUG");
            command.env("GRPC_TRACE", "tsi");
        }
    }
    let mut child = command.spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("SDK child did not receive its anonymous credential pipe")?;
    stdin.write_all(bearer)?;
    stdin.flush()?;
    drop(stdin);
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("{} SDK runner failed", client_name(client)).into());
    }
    Ok(())
}

fn client_requires_script(client: ClientKind) -> bool {
    matches!(client, ClientKind::Python | ClientKind::TypeScript)
}

fn client_name(client: ClientKind) -> &'static str {
    match client {
        ClientKind::Rust => "Rust",
        ClientKind::Python => "Python",
        ClientKind::TypeScript => "TypeScript",
        ClientKind::Go => "Go",
    }
}

fn restrict_child_environment(command: &mut Command) {
    let allowed = ["PATH", "TMPDIR", "LANG", "LC_ALL", "SYSTEMROOT"];
    let retained = allowed
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|value| (name, value)))
        .collect::<Vec<_>>();
    command.env_clear();
    command.envs(retained);
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let config = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage().to_owned())?;
    let client = match arguments.next().as_deref() {
        Some("rust") => ClientKind::Rust,
        Some("python") => ClientKind::Python,
        Some("typescript") => ClientKind::TypeScript,
        Some("go") => ClientKind::Go,
        _ => return Err(usage().into()),
    };
    let prompt = arguments.collect::<Vec<_>>().join(" ");
    if prompt.is_empty() {
        return Err(usage().into());
    }
    Ok(Options {
        config,
        client,
        prompt,
    })
}

fn usage() -> &'static str {
    "usage: sdk_ephemeral_local CONFIG {rust|python|typescript|go} PROMPT..."
}
