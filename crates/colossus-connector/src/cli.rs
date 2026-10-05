use crate::cli_control::{Control, Report};
use crate::{ConnectorStatus, EnrollmentStore, RuntimeConnector};
use clap::{Args, Subcommand};
use colossus_credentials::{EnvironmentKeyStore, PlatformCredentialVault};
use colossus_home::{ColossusHome, ConfinedRoot};
use colossus_ports::CredentialVault;
use colossus_sdk::{
    ApiMajor, Colossus, DaemonConnectOptions, InstanceId, KeyringCredentialProvider, TlsFingerprint,
};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};
use zeroize::Zeroizing;

/// Portable CLI and standalone connector lifecycle operations.
#[derive(Subcommand)]
pub enum ConnectorCommand {
    /// Enroll one independently verified local daemon; invitation is read from stdin.
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
/// Secret-storage references shared by all connector commands.
#[derive(Args)]
pub struct StorageArguments {
    /// Opaque local enrollment name, permitting multiple explicitly selected nodes.
    #[arg(long, default_value = "active")]
    pub name: String,
    /// Explicit headless authority; variable must hold a secret 32-byte hex wrapping key.
    #[arg(long)]
    pub vault_key_variable: Option<String>,
}
/// Independently enrolled local daemon connection references.
#[derive(Args)]
pub struct LocalArguments {
    /// Owner-private JSON containing exact descriptor, instance, pin, and dedicated keyring references.
    #[arg(long)]
    pub local_config: PathBuf,
    /// Native credential authority and enrollment identity.
    #[command(flatten)]
    pub storage: StorageArguments,
}
/// Single-use cloud enrollment references; no invitation is accepted through argv or environment.
#[derive(Args)]
pub struct EnrollArguments {
    /// Exact HTTPS enrollment endpoint obtained from the authenticated web fleet view.
    #[arg(long)]
    pub enrollment_url: String,
    /// Explicitly permit HTTP enrollment on loopback for local acceptance only.
    #[arg(long)]
    pub allow_loopback_http: bool,
    /// Independently verified local daemon and native vault references.
    #[command(flatten)]
    pub local: LocalArguments,
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
    let root = ConfinedRoot::bind(
        home.confined_root()
            .prepare_directory(std::path::Path::new("cloud-connector"))?,
    )?;
    let vault: Arc<dyn CredentialVault> = match &arguments.vault_key_variable {
        Some(variable) => Arc::new(PlatformCredentialVault::with_key_store(
            root.clone(),
            "cloud-connector",
            Arc::new(EnvironmentKeyStore::new(root, variable.clone())?),
        )?),
        None => Arc::new(PlatformCredentialVault::new(root, "cloud-connector")?),
    };
    Ok(EnrollmentStore::new(vault, &arguments.name)?)
}
async fn local(arguments: &LocalArguments) -> Result<Colossus, Box<dyn std::error::Error>> {
    let parent = arguments
        .local_config
        .parent()
        .ok_or("local config must be an absolute protected file")?;
    let root = ConfinedRoot::bind(parent)?;
    let file = root.open_existing_file(
        arguments
            .local_config
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
    file.revalidate(&root)?;
    let credential: Arc<dyn colossus_sdk::CredentialProvider> = match config.headless_authority {
        Some(authority) => Arc::new(crate::HeadlessCredentialProvider::new(
            ConfinedRoot::bind(authority.directory)?,
            authority.key_variable,
            &config.keyring_service,
            &config.keyring_account,
        )?),
        None => Arc::new(KeyringCredentialProvider::new(
            config.keyring_service,
            config.keyring_account,
        )?),
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
/// Execute the shared `colossus cloud` / `colossus-connector` command surface.
/// Credentials and tokens never appear in status output or process arguments.
pub async fn run_cli(command: ConnectorCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ConnectorCommand::Renew(arguments) => {
            Control::new(&arguments.name)?.disconnect().await?;
            let enrollment = store(&arguments)?;
            enrollment.renew(true).await?;
            println!("connector certificate renewed");
        }
        ConnectorCommand::Enroll(arguments) => {
            let client = local(&arguments.local).await?;
            eprint!("One-use cloud invitation: ");
            std::io::stderr().flush()?;
            let mut token = Zeroizing::new(String::new());
            BufReader::new(std::io::stdin())
                .take(66)
                .read_line(&mut token)?;
            let token = Zeroizing::new(token.trim().to_owned());
            let enrollment = store(&arguments.local.storage)?;
            let config = enrollment
                .enroll(
                    arguments.enrollment_url,
                    token,
                    client
                        .instance_id()
                        .ok_or("local instance unavailable")?
                        .to_string(),
                    client.capabilities().iter().map(str::to_owned).collect(),
                    arguments.allow_loopback_http,
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
            let (config, key) = enrollment.load()?.ok_or("enroll this connector first")?;
            if client.instance_id().map(|id| id.to_string()).as_deref() != Some(&config.instance_id)
            {
                return Err("local enrolled instance changed".into());
            }
            let mut report = Report::saved(Some(&config));
            report.run_instance = Some(uuid::Uuid::now_v7().simple().to_string());
            let connector = RuntimeConnector::new(config, key, client.agent_runs())?
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
