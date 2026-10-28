use crate::cli_control::{Control, Report};
use crate::{ConnectorStatus, EnrollmentStore, EnrollmentTrust, RuntimeConnector};
use clap::{Args, Subcommand};
use colossus_credentials::{EnvironmentKeyStore, PlatformCredentialVault};
use colossus_home::{ColossusHome, ConfinedRoot};
use colossus_ports::CredentialVault;
use colossus_sdk::{
    ApiMajor, Colossus, DaemonConnectOptions, InstanceId, KeyringCredentialProvider, TlsFingerprint,
};
use serde::Deserialize;
use std::{io::Read, path::PathBuf, str::FromStr, sync::Arc};
use zeroize::Zeroizing;

/// Portable CLI and standalone connector lifecycle operations.
#[derive(Clone, Subcommand)]
pub enum ConnectorCommand {
    /// Read metadata-only policy beneath the dedicated local application's authority.
    Policy(LocalArguments),
    /// Explicitly share the local application's workspace sessions with a dedicated cloud application.
    ShareWorkspace(ShareWorkspaceArguments),
    /// Enroll a runtime using a Cloud URL and a hidden invitation prompt.
    Enroll(EnrollArguments),
    /// Maintain its outbound connection until Ctrl-C; accepted runs survive disconnect.
    Run(LocalArguments),
    /// Show the saved non-secret enrollment identity.
    Status(StorageArguments),
    /// Rotate the native-held certificate/key; retries reconcile the saved exchange.
    Renew(StorageArguments),
    /// Stop this CLI connection without cancelling accepted runtime work.
    Disconnect(StorageArguments),
    /// Disconnect and revoke this exact native enrollment in the cloud.
    Revoke(StorageArguments),
    /// Forget local enrollment after stopping the connector; remote authority remains revoked separately.
    Forget(StorageArguments),
}
/// Local operator disclosure beneath its own authenticated application credential.
#[derive(Clone, Args)]
pub struct ShareWorkspaceArguments {
    /// Exact independently enrolled cloud application ID, such as app:colossus-cloud.
    #[arg(long)]
    pub recipient_application_id: String,
    /// Revoke future reads and continuation; synchronized cloud history remains retained.
    #[arg(long)]
    pub disable: bool,
    /// Allow new recipient-owned runs to continue source conversations under its own grant.
    #[arg(long, conflicts_with = "disable")]
    pub allow_continuation: bool,
    /// Source application's protected native connection references.
    #[command(flatten)]
    pub local: LocalArguments,
}
/// Secret-storage references shared by all connector commands.
#[derive(Clone, Args)]
pub struct StorageArguments {
    /// Opaque local enrollment name, permitting multiple explicitly selected nodes.
    #[arg(long, default_value = "active")]
    pub name: String,
    /// Explicit headless authority; variable must hold a secret 32-byte hex wrapping key.
    #[arg(long)]
    pub vault_key_variable: Option<String>,
}
/// Independently enrolled local daemon connection references.
#[derive(Clone, Args)]
pub struct LocalArguments {
    /// Advanced: protected connection references for an independently enrolled external daemon.
    #[arg(long)]
    pub local_config: Option<PathBuf>,
    /// Explicitly permit this managed connector to answer policy approvals.
    #[arg(long, conflicts_with = "local_config")]
    pub allow_approvals: bool,
    /// Explicitly enable human shells in the managed worker and its connector grant.
    #[arg(long, conflicts_with = "local_config")]
    pub enable_human_terminals: bool,
    /// Native credential authority and enrollment identity.
    #[command(flatten)]
    pub storage: StorageArguments,
}
/// Single-use cloud enrollment references; no invitation is accepted through argv or environment.
#[derive(Clone, Args)]
pub struct EnrollArguments {
    /// Exact HTTPS enrollment endpoint obtained from the authenticated web fleet view.
    #[arg(value_name = "URL", conflicts_with = "enrollment_url")]
    pub url: Option<String>,
    /// Advanced-compatible spelling of the enrollment URL.
    #[arg(long, required_unless_present = "url")]
    pub enrollment_url: Option<String>,
    /// Explicit PEM trust anchor for private/local HTTPS; hostname verification stays enabled.
    #[arg(long, value_name = "PEM")]
    pub enrollment_ca_file: Option<PathBuf>,
    /// Explicitly permit HTTP enrollment on loopback for local acceptance only.
    #[arg(long)]
    pub allow_loopback_http: bool,
    /// Independently verified local daemon and native vault references.
    #[command(flatten)]
    pub local: LocalArguments,
}

impl ConnectorCommand {
    /// Whether the full CLI must bind its ordinary workspace/configuration and own a worker.
    pub fn requires_managed_runtime(&self) -> bool {
        match self {
            Self::Enroll(arguments) => arguments.local.local_config.is_none(),
            Self::Run(arguments) | Self::Policy(arguments) => arguments.local_config.is_none(),
            _ => false,
        }
    }
}

impl EnrollArguments {
    /// Exactly one URL, independent of its positional or legacy flag spelling.
    pub fn selected_url(&self) -> Result<&str, &'static str> {
        match (&self.url, &self.enrollment_url) {
            (Some(url), None) | (None, Some(url)) => Ok(url),
            _ => Err("supply one enrollment URL"),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalConfig {
    descriptor: PathBuf,
    certificate: PathBuf,
    instance_id: String,
    certificate_sha256: String,
    keyring_service: String,
    keyring_account: String,
    #[serde(default)]
    headless_authority: Option<HeadlessAuthority>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadlessAuthority {
    directory: PathBuf,
    key_variable: String,
}
fn store(arguments: &StorageArguments) -> Result<EnrollmentStore, Box<dyn std::error::Error>> {
    let home = ColossusHome::resolve_and_ensure()?;
    let development =
        colossus_credentials::DevelopmentAuthority::selected(home.confined_root(), &[])?;
    if development.is_some() && arguments.vault_key_variable.is_some() {
        return Err("development authority conflicts with an explicit vault-key variable".into());
    }
    let root = ConfinedRoot::bind(
        home.confined_root()
            .prepare_directory(std::path::Path::new("cloud-connector"))?,
    )?;
    let vault: Arc<dyn CredentialVault> = if let Some(authority) = development {
        Arc::new(PlatformCredentialVault::with_key_store(
            root,
            "cloud-connector",
            Arc::new(
                authority.store(colossus_credentials::DevelopmentStoreScope::ControlPlaneVault)?,
            ),
        )?)
    } else {
        match &arguments.vault_key_variable {
            Some(variable) => Arc::new(PlatformCredentialVault::with_key_store(
                root.clone(),
                "cloud-connector",
                Arc::new(EnvironmentKeyStore::new(root, variable.clone())?),
            )?),
            None => Arc::new(PlatformCredentialVault::new(root, "cloud-connector")?),
        }
    };
    Ok(EnrollmentStore::new(vault, &arguments.name)?)
}
async fn local(arguments: &LocalArguments) -> Result<Colossus, Box<dyn std::error::Error>> {
    let local_config = arguments.local_config.as_ref().ok_or(
        "managed enrollment requires `colossus cloud`; standalone/external connections require --local-config",
    )?;
    let parent = local_config
        .parent()
        .ok_or("local config must be an absolute protected file")?;
    let root = ConfinedRoot::bind(parent)?;
    let file = root.open_existing_file(
        local_config
            .file_name()
            .ok_or("invalid local config")?
            .as_ref(),
    )?;
    let mut bytes = Vec::new();
    file.file().take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err("local config exceeds bound".into());
    }
    let config: LocalConfig = serde_json::from_slice(&bytes)?;
    if config.headless_authority.is_some()
        && std::env::var_os(colossus_credentials::DEVELOPMENT_AUTHORITY_VARIABLE).is_some()
    {
        return Err(
            "development authority conflicts with an explicit headless credential authority".into(),
        );
    }
    file.revalidate(&root)?;
    let credential: Arc<dyn colossus_sdk::CredentialProvider> = match config.headless_authority {
        Some(authority) => Arc::new(crate::HeadlessCredentialProvider::new(
            ConfinedRoot::bind(authority.directory)?,
            authority.key_variable,
            &config.keyring_service,
            &config.keyring_account,
        )?),
        None => {
            let home = ColossusHome::resolve_and_ensure()?;
            if let Some(authority) =
                colossus_credentials::DevelopmentAuthority::selected(home.confined_root(), &[])?
            {
                Arc::new(crate::HeadlessCredentialProvider::from_store(
                    Arc::new(
                        authority.store(colossus_credentials::DevelopmentStoreScope::PublicApi)?,
                    ),
                    &config.keyring_service,
                    &config.keyring_account,
                )?)
            } else {
                Arc::new(KeyringCredentialProvider::new(
                    config.keyring_service,
                    config.keyring_account,
                )?)
            }
        }
    };
    let options = DaemonConnectOptions::new(
        InstanceId::from_str(&config.instance_id)?,
        config.descriptor,
        TlsFingerprint::from_hex(&config.certificate_sha256)?,
        ApiMajor::new(1)?,
        credential,
    )?
    .with_certificate_path(config.certificate)?;
    Ok(Colossus::connect_installed(options).await?)
}
/// Execute the shared `colossus control-plane` / `colossus-connector` command surface.
/// Credentials and tokens never appear in status output or process arguments.
pub async fn run_cli(command: ConnectorCommand) -> Result<(), Box<dyn std::error::Error>> {
    run_cli_with_invitation(command, None).await
}

/// Native-only, one-use invitation input and the exact trust selection reviewed with it.
/// This value must never be logged, serialized, or sent through a renderer/argument channel.
pub struct PreparedEnrollment {
    url: String,
    name: String,
    vault_key_variable: Option<String>,
    ca_file: Option<PathBuf>,
    trust: EnrollmentTrust,
    token: Zeroizing<String>,
}

/// Read invitation input before a managed host starts serving accepted work.
/// Retained exchanges reuse their encrypted invitation instead of prompting again.
pub fn prepare_enrollment_invitation(
    arguments: &EnrollArguments,
) -> Result<PreparedEnrollment, Box<dyn std::error::Error>> {
    let endpoint = enrollment_endpoint(arguments)?;
    let trust = enrollment_trust(arguments.enrollment_ca_file.as_deref())?;
    eprintln!("Cloud origin: {}", endpoint.origin().ascii_serialization());
    let enrollment = store(&arguments.local.storage)?;
    let token = if enrollment.has_saved_exchange()? {
        eprintln!("Using this name's retained enrollment exchange; no new invitation is consumed.");
        Zeroizing::new(String::new())
    } else {
        crate::cli_prompt::read_invitation()?
    };
    Ok(PreparedEnrollment {
        url: arguments.selected_url()?.to_owned(),
        name: arguments.local.storage.name.clone(),
        vault_key_variable: arguments.local.storage.vault_key_variable.clone(),
        ca_file: arguments.enrollment_ca_file.clone(),
        trust,
        token,
    })
}

fn enrollment_endpoint(arguments: &EnrollArguments) -> Result<url::Url, &'static str> {
    let endpoint =
        url::Url::parse(arguments.selected_url()?).map_err(|_| "invalid enrollment URL")?;
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.host_str().is_none()
        || !(endpoint.scheme() == "https"
            || (arguments.allow_loopback_http
                && endpoint.scheme() == "http"
                && matches!(
                    endpoint.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]")
                )))
    {
        return Err("enrollment requires HTTPS without URL credentials, query, or fragment");
    }
    Ok(endpoint)
}

/// Execute a native CLI operation with invitation input already collected in memory.
/// Managed hosts use this to keep blocking terminal input outside their worker lifetime.
pub async fn run_cli_with_invitation(
    command: ConnectorCommand,
    invitation: Option<PreparedEnrollment>,
) -> Result<(), Box<dyn std::error::Error>> {
    if command.requires_managed_runtime() {
        return Err("use `colossus cloud` for a managed runtime; standalone connectors require --local-config".into());
    }
    if invitation.is_some() && !matches!(command, ConnectorCommand::Enroll(_)) {
        return Err("prepared invitation requires an enrollment command".into());
    }
    match command {
        ConnectorCommand::Policy(arguments) => {
            let client = local(&arguments).await?;
            let result = client.agent_runs().get_runtime_policy_posture().await;
            client.close().await?;
            println!("{}", serde_json::to_string(&result?)?);
        }
        ConnectorCommand::ShareWorkspace(arguments) => {
            let client = local(&arguments.local).await?;
            let state = client
                .agent_runs()
                .set_workspace_sharing(colossus_sdk::SetWorkspaceSharingRequest {
                    recipient_application_id: arguments.recipient_application_id,
                    enabled: !arguments.disable,
                    allow_continuation: arguments.allow_continuation,
                })
                .await?;
            println!("{}", serde_json::to_string(&state)?);
            client.close().await?;
        }
        ConnectorCommand::Renew(arguments) => {
            Control::new(&arguments.name)?.disconnect().await?;
            let enrollment = store(&arguments)?;
            enrollment.renew(true).await?;
            println!("connector certificate renewed");
        }
        ConnectorCommand::Enroll(arguments) => {
            let enrollment_url = arguments.selected_url()?.to_owned();
            let prepared = match invitation {
                Some(prepared) => prepared,
                None => prepare_enrollment_invitation(&arguments)?,
            };
            if prepared.url != enrollment_url
                || prepared.name != arguments.local.storage.name
                || prepared.vault_key_variable != arguments.local.storage.vault_key_variable
                || prepared.ca_file != arguments.enrollment_ca_file
            {
                return Err("prepared invitation belongs to another enrollment selection".into());
            }
            let client = local(&arguments.local).await?;
            let enrollment = store(&arguments.local.storage)?;
            let config = enrollment
                .enroll_with_trust(
                    enrollment_url,
                    prepared.token,
                    client
                        .instance_id()
                        .ok_or("local instance unavailable")?
                        .to_string(),
                    client.capabilities().iter().map(str::to_owned).collect(),
                    arguments.allow_loopback_http,
                    prepared.trust,
                )
                .await?;
            Control::new(&arguments.local.storage.name)?.save(&Report::saved(Some(&config)))?;
            println!(
                "{}",
                serde_json::json!({"node_id":config.node_id,"project_id":config.project_id,"endpoint":config.endpoint,"status":"enrolled"})
            );
            client.close().await?;
        }
        ConnectorCommand::Run(arguments) => {
            let control = Control::new(&arguments.storage.name)?;
            eprintln!(
                "connecting to the pinned local runtime; the native credential store may require unlocking"
            );
            let client = local(&arguments).await?;
            let enrollment = store(&arguments.storage)?;
            let (mut config, key) = enrollment.load()?.ok_or("enroll this connector first")?;
            if client.instance_id().map(|id| id.to_string()).as_deref() != Some(&config.instance_id)
            {
                return Err("local enrolled instance changed".into());
            }
            if config.inventory.is_none() {
                let inventory = crate::native_inventory(
                    format!("workspace:{}", config.instance_id),
                    "CLI workspace".into(),
                    crate::DeploymentKind::Cli,
                )?;
                config = enrollment.set_inventory(inventory).await?;
            }
            let mut report = Report::saved(Some(&config));
            report.run_instance = Some(uuid::Uuid::now_v7().simple().to_string());
            let connector = RuntimeConnector::new(config, key, client.agent_runs())?
                .with_resources(crate::ConnectorResources {
                    workflows: client.workflows(),
                    plugins: client.plugins(),
                    capabilities: client.capabilities().iter().map(str::to_owned).collect(),
                    model_profiles: client.model_profiles(),
                })
                .with_enrollment_store(enrollment);
            control.heartbeat(&mut report, ConnectorStatus::Connecting)?;
            let (shutdown, receiver) = tokio::sync::watch::channel(false);
            let (status, mut updates) = tokio::sync::watch::channel(ConnectorStatus::Connecting);
            let final_status = updates.clone();
            let interrupt = shutdown.clone();
            let interrupt_task = tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = interrupt.send(true);
            });
            let monitor_control = control.clone();
            let mut monitor_report = report.clone();
            let monitor = tokio::spawn(async move {
                let mut poll = tokio::time::interval(std::time::Duration::from_millis(200));
                let mut ticks = 0;
                loop {
                    tokio::select! {
                        result = updates.changed() => {
                            if result.is_err() { break; }
                            let state = *updates.borrow_and_update();
                            eprintln!("connector {}", serde_json::to_string(&state).unwrap_or_default());
                            if let Err(error) = monitor_control.heartbeat(&mut monitor_report, state) {
                                eprintln!("connector status unavailable: {error}");
                                let _ = shutdown.send(true);
                                break;
                            }
                        }
                        _ = poll.tick() => {
                            let stop = monitor_control.stopped(monitor_report.run_instance.as_deref().unwrap_or_default());
                            if !matches!(stop, Ok(false)) { let _ = shutdown.send(true); break; }
                            ticks += 1;
                            if ticks % 10 == 0 && monitor_control.heartbeat(&mut monitor_report, *updates.borrow()).is_err() {
                                let _ = shutdown.send(true); break;
                            }
                        }
                    }
                }
            });
            let result = connector.run(receiver, status).await;
            monitor.abort();
            interrupt_task.abort();
            client.close().await?;
            report.run_instance = None;
            control.heartbeat(
                &mut report,
                if *final_status.borrow() == ConnectorStatus::Revoked {
                    ConnectorStatus::Revoked
                } else {
                    ConnectorStatus::Disconnected
                },
            )?;
            result?;
        }
        ConnectorCommand::Status(arguments) => {
            let control = Control::new(&arguments.name)?;
            if let Some(report) = control.report()? {
                println!("{}", Control::output(&report));
                return Ok(());
            }
            let saved = store(&arguments)?.load()?;
            println!(
                "{}",
                Control::output(&Report::saved(saved.as_ref().map(|(config, _)| config)))
            );
        }
        ConnectorCommand::Disconnect(arguments) => {
            let control = Control::new(&arguments.name)?;
            control.disconnect().await?;
            if let Some(report) = control.report()? {
                println!("{}", Control::output(&report));
            } else {
                println!("{}", serde_json::json!({"status":"disconnected"}));
            }
        }
        ConnectorCommand::Revoke(arguments) => {
            let control = Control::new(&arguments.name)?;
            control.disconnect().await?;
            let enrollment = store(&arguments)?;
            enrollment.revoke().await?;
            let saved = enrollment.load()?;
            let report = Report::saved(saved.as_ref().map(|(config, _)| config));
            control.save(&report)?;
            println!("{}", Control::output(&report));
        }
        ConnectorCommand::Forget(arguments) => {
            let control = Control::new(&arguments.name)?;
            control.disconnect().await?;
            store(&arguments)?.forget()?;
            control.save(&Report::saved(None))?;
            println!("{}", serde_json::json!({"enrolled":false}));
        }
    }
    Ok(())
}

fn enrollment_trust(
    path: Option<&std::path::Path>,
) -> Result<EnrollmentTrust, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(EnrollmentTrust::public_roots());
    };
    let file = std::fs::File::open(path).map_err(|_| "enrollment trust file unavailable")?;
    let mut bytes = Vec::new();
    file.take((EnrollmentTrust::MAX_PEM_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "enrollment trust file unavailable")?;
    Ok(EnrollmentTrust::from_pem(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn standalone_managed_enrollment_is_rejected_before_input_or_custody() {
        let error = run_cli(ConnectorCommand::Enroll(EnrollArguments {
            url: Some("https://cloud.example.test/api/enroll".into()),
            enrollment_url: None,
            enrollment_ca_file: Some("absent-private-ca.pem".into()),
            allow_loopback_http: false,
            local: LocalArguments {
                local_config: None,
                allow_approvals: false,
                enable_human_terminals: false,
                storage: StorageArguments {
                    name: "fixture".into(),
                    vault_key_variable: None,
                },
            },
        }))
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "use `colossus cloud` for a managed runtime; standalone connectors require --local-config"
        );
    }
}
