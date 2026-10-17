use crate::wire::{self, Result};
use colossus_api::{ApiScope, scopes};
use colossus_home::{ColossusHome, HomeSurface, detect_workspace_identity};
use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, Colossus, HostSecret, InstanceId, ManagedAccessProfile,
    ManagedRuntimeConfig, NativeSidecarLifecycle, NativeSidecarStatus, SdkError, Sha256Digest,
    SidecarApplicationGrant, SidecarApprovalBrokerGrant, SidecarBootstrapConfig,
    SidecarConnectionObserver, SidecarHostCredential, SidecarOptions, VerifiedExecutable,
};
use colossus_sidecar_protocol::{ReadyResponse, SecretString, WorkspaceIdentity};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    version: u8,
    workspace: PathBuf,
    home: PathBuf,
    executable: PathBuf,
    sha256: String,
    macos_developer_preview: bool,
    expected_workspace_identity: Option<WorkspaceBinding>,
    runtime: ManagedRuntimeConfig,
    credentials: Vec<Credential>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    id: String,
    secret: SecretString,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceBinding {
    version: u16,
    sha256: String,
}

struct EditorConnection {
    workspace_identity: WorkspaceBinding,
}
#[derive(serde::Serialize)]
struct ConnectionFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    connection: &'a ReadyResponse,
    workspace_identity: &'a WorkspaceBinding,
}
impl SidecarConnectionObserver for EditorConnection {
    fn connected(&self, connection: &ReadyResponse) -> colossus_sdk::SdkResult<()> {
        // Exactly one trusted pipe consumer: the native extension host. No credential
        // is persisted, logged, or included in any webview projection.
        wire::write(&ConnectionFrame {
            kind: "connection",
            connection,
            workspace_identity: &self.workspace_identity,
        })
        .map_err(|_| SdkError::SidecarFailed)
    }
}

pub(crate) async fn run() -> Result<()> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--managed-v1"] {
        return Err("private host invocation required".into());
    }
    let mut input = tokio::io::stdin();
    let frame = tokio::time::timeout(Duration::from_secs(30), wire::read(&mut input)).await??;
    let start: Start = serde_json::from_slice(&frame)?;
    drop(frame);
    if start.version != 1 || start.credentials.len() > 64 {
        return Err("invalid managed configuration".into());
    }
    start.runtime.validate()?;
    let identity = detect_workspace_identity(&start.workspace)?;
    if start
        .expected_workspace_identity
        .as_ref()
        .is_some_and(|expected| {
            expected.version != identity.version() || expected.sha256 != identity.sha256()
        })
    {
        return Err(SdkError::WorkspaceIdentityChanged.into());
    }
    let home = ColossusHome::ensure_at(&start.home)?;
    let instance = home.workspace_surface_dir(
        identity.canonical_path(),
        identity.as_ref(),
        HomeSurface::Vscode,
    )?;
    let mut instance_hash = Sha256::new();
    instance_hash.update(b"colossus-vscode-managed-instance-v1\0");
    instance_hash.update(instance.to_string_lossy().as_bytes());
    let digest = instance_hash.finalize();
    let mut instance_bytes = [0; 16];
    instance_bytes.copy_from_slice(&digest[..16]);
    let instance_id = InstanceId::from_uuid(Uuid::from_bytes(instance_bytes));
    let executable =
        VerifiedExecutable::new(start.executable, Sha256Digest::from_hex(&start.sha256)?)?;
    let executable = if cfg!(target_os = "macos") && start.macos_developer_preview {
        executable.with_macos_code_signing_requirement(
            colossus_sdk::MacosCodeSigningRequirement::AdHocDeveloperPreview,
        )
    } else {
        executable
    };
    let options = SidecarOptions::new(
        instance_id,
        AppPrivateInstanceDir::new(instance)?,
        executable,
        ApiMajor::new(1)?,
    )?;
    let minimal = start.runtime.access_profile == ManagedAccessProfile::Minimal;
    let tools = if minimal {
        vec!["echo".to_owned()]
    } else {
        [
            "echo",
            "filesystem.list",
            "filesystem.read",
            "filesystem.search",
            "git.status",
            "git.diff",
            "patch.preview",
            "patch.apply",
            "shell.run",
            "user.ask",
            "plan.create",
            "plan.update",
            "plan.show",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    };
    let grant = SidecarApplicationGrant::new(
        "app:colossus-vscode",
        [
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::PROMPTS_RESPOND,
        ]
        .into_iter()
        .map(ApiScope::new)
        .collect::<std::result::Result<Vec<_>, _>>()?,
        ["primary".to_owned()],
        tools,
    )?;
    let mut bootstrap =
        SidecarBootstrapConfig::new(identity.canonical_path(), start.runtime, grant)?
            .with_colossus_home(home.root())?
            .with_approval_broker_grant(SidecarApprovalBrokerGrant::new(
                "app:colossus-vscode",
                ["primary".to_owned()],
            )?)?
            .with_host_credentials(
                start
                    .credentials
                    .into_iter()
                    .map(|credential| {
                        SidecarHostCredential::new(
                            credential.id,
                            HostSecret::new(credential.secret.expose().to_owned())
                                .map_err(|_| SdkError::SidecarFailed)?,
                        )
                    })
                    .collect::<colossus_sdk::SdkResult<Vec<_>>>()?,
            )?
            .with_connection_observer(Arc::new(EditorConnection {
                workspace_identity: WorkspaceBinding {
                    version: identity.version(),
                    sha256: identity.sha256().to_owned(),
                },
            }));
    // Windows and macOS persist the same birthtime/file-ID authority as Desktop.
    // Linux's SDK independently retains its descriptor-lifetime identity on launch.
    if matches!(identity.version(), 2 | 3) {
        let expected: WorkspaceIdentity = serde_json::from_value(serde_json::json!({
            "version": identity.version(), "sha256": identity.sha256()
        }))?;
        bootstrap = bootstrap.with_expected_workspace_identity(expected)?;
    }
    #[cfg(target_os = "linux")]
    let workspace_directory = std::fs::File::open(identity.canonical_path())?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt as _;
        let captured = colossus_home::capture_linux_workspace_identity(&workspace_directory)?;
        // The current SDK bootstrap cannot represent NFS v5. Keep the existing
        // fail-closed boundary instead of assigning a new object to old state.
        if captured.version() != 4
            || captured.digest() != *Sha256Digest::from_hex(identity.sha256())?.as_bytes()
        {
            return Err("unsupported or changed workspace identity".into());
        }
        let metadata = workspace_directory.metadata()?;
        bootstrap = bootstrap.with_expected_workspace_identity(
            WorkspaceIdentity::from_unix_parts(metadata.dev(), metadata.ino()),
        )?;
    }
    identity.revalidate()?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let mut status = lifecycle.subscribe_status();
    // EOF while bootstrapping cancels launch; SDK RAII closes its provisional child.
    let client = tokio::select! {
        result = Colossus::start_sidecar(&lifecycle, options) => result?,
        _ = wire::read(&mut input) => return Ok(()),
    };
    let mut health = tokio::time::interval(Duration::from_secs(3));
    let mut health_failures = 0;
    loop {
        tokio::select! {
            _ = wire::read(&mut input) => break,
            _ = health.tick() => {
                if lifecycle.status() != NativeSidecarStatus::Ready { health_failures = 0; continue; }
                let request = colossus_sdk::ListRunsRequest {
                    session_id: None, statuses: Vec::new(),
                    page: Some(colossus_sdk::PageRequest {page_size: 1, page_token: String::new()}),
                    include_archived: false,
                };
                if !matches!(tokio::time::timeout(Duration::from_secs(3), client.list_runs(request)).await, Ok(Ok(_))) {
                    if lifecycle.status() == NativeSidecarStatus::Ready { health_failures += 1; }
                    if health_failures >= 3 {
                        wire::write(&serde_json::json!({"type":"failed","code":"managed_runtime_stopped"}))?;
                        break;
                    }
                } else { health_failures = 0; }
            }
            changed = status.changed() => {
                if changed.is_err() { break; }
                let state = *status.borrow_and_update();
                if matches!(state, NativeSidecarStatus::Failed(_)) {
                    wire::write(&serde_json::json!({"type":"failed","code":"managed_runtime_stopped"}))?;
                    break;
                }
            }
        }
    }
    client.close().await?;
    wire::write(&serde_json::json!({"type":"stopped"}))?;
    Ok(())
}
