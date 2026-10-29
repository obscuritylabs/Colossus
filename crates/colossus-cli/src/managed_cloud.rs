//! CLI-owned runtime composition for the URL/token Cloud enrollment path.
//! The connector retains transport/enrollment logic; this module owns only the
//! selected local worker, its dedicated application grant and protected binding.
use super::public_api_admin::{
    EnrollmentRequest, PublicApiEnvironment, credential_store, enroll_application,
};
use colossus_api::scopes;
use colossus_connector::{ConnectorCommand, LocalArguments, StorageArguments};
use colossus_home::detect_workspace_identity;
use colossus_runtime::{RuntimeConfig, RuntimeOpenOptions, StorageAdapter};
use colossus_worker::{WorkerApprovalMode, WorkerClient, WorkerServer};
use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

mod metadata;
use metadata::{Binding, Grant, Metadata, Stage, digest};

fn local_arguments(command: &ConnectorCommand) -> Result<&LocalArguments, Box<dyn Error>> {
    match command {
        ConnectorCommand::Enroll(arguments) => Ok(&arguments.local),
        ConnectorCommand::Run(arguments) | ConnectorCommand::Policy(arguments) => Ok(arguments),
        _ => Err("this command does not require a managed Cloud runtime".into()),
    }
}

fn bind_connection(command: &mut ConnectorCommand, path: &Path) -> Result<(), Box<dyn Error>> {
    let local = match command {
        ConnectorCommand::Enroll(arguments) => &mut arguments.local,
        ConnectorCommand::Run(arguments) | ConnectorCommand::Policy(arguments) => arguments,
        _ => return Err("this command does not use a managed Cloud connection".into()),
    };
    local.local_config = Some(path.to_owned());
    // These switches belong to native composition. The generated connection now
    // has an independently retained credential grant; transport flags cannot widen it.
    local.allow_approvals = false;
    local.enable_human_terminals = false;
    Ok(())
}

fn validate_enrollment_url(command: &ConnectorCommand) -> Result<(), Box<dyn Error>> {
    if let ConnectorCommand::Enroll(arguments) = command {
        let url = url::Url::parse(arguments.selected_url()?)
            .map_err(|_| "enter a valid Cloud enrollment URL")?;
        let loopback = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
            || !(url.scheme() == "https"
                || (url.scheme() == "http" && loopback && arguments.allow_loopback_http))
        {
            return Err("Cloud enrollment requires verified HTTPS without embedded credentials, query or fragment".into());
        }
    }
    Ok(())
}

fn requested_scopes(arguments: &LocalArguments) -> Vec<String> {
    let mut scopes = [
        scopes::RUNS_EXECUTE,
        scopes::RUNS_READ,
        scopes::RUNS_CONTROL,
        scopes::PROMPTS_RESPOND,
        scopes::AGENT_MESSAGES_READ,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if arguments.allow_approvals {
        scopes.push(scopes::APPROVALS_RESPOND.to_owned());
    }
    if arguments.enable_human_terminals {
        scopes.push(colossus_api::HUMAN_TERMINAL_SCOPE.to_owned());
    }
    scopes.sort();
    scopes
}

fn retain_consented_access(
    arguments: &mut LocalArguments,
    binding: &Binding,
) -> Result<(), Box<dyn Error>> {
    if (arguments.allow_approvals && !binding.grant.allow_approvals)
        || (arguments.enable_human_terminals && !binding.grant.enable_human_terminals)
    {
        return Err("this named enrollment has a narrower saved grant; review a new enrollment name to add approvals or human terminals".into());
    }
    arguments.allow_approvals = binding.grant.allow_approvals;
    arguments.enable_human_terminals = binding.grant.enable_human_terminals;
    Ok(())
}

fn grant(server: &WorkerServer, arguments: &LocalArguments) -> Grant {
    let mut tools = server.active_tool_names();
    // The public worker deliberately does not expose private subagent dispatch.
    tools.retain(|tool| tool != "agent.delegate");
    tools.sort();
    tools.dedup();
    Grant {
        scopes: requested_scopes(arguments),
        roles: vec!["primary".into()],
        tools,
        allow_approvals: arguments.allow_approvals,
        enable_human_terminals: arguments.enable_human_terminals,
    }
}

fn disclosure(binding: &Binding, owns_worker: bool) {
    eprintln!("Local runtime: {}", binding.workspace.display());
    eprintln!("Configuration: {}", binding.config_path.display());
    eprintln!("Dedicated Cloud application: {}", binding.application_id);
    eprintln!(
        "Access: scopes [{}], roles [{}], {} existing runtime tools; policy and sandbox remain in force.",
        binding.grant.scopes.join(", "),
        binding.grant.roles.join(", "),
        binding.grant.tools.len()
    );
    eprintln!(
        "Approvals: {}; human terminals: {}.",
        binding.grant.allow_approvals, binding.grant.enable_human_terminals
    );
    if owns_worker {
        eprintln!(
            "This command owns the local worker until Ctrl-C. Disconnect only closes its Cloud connection; accepted work remains with the worker."
        );
    } else {
        eprintln!(
            "Using the existing managed CLI worker. Ctrl-C disconnects this connection; the original foreground command owns the worker lifetime."
        );
    }
}

/// Resolve ordinary CLI configuration into an owned worker without accepting
/// connection JSON, descriptor fingerprints or bearer material from the user.
#[allow(clippy::too_many_lines)]
pub(super) async fn run(
    mut command: ConnectorCommand,
    config: &RuntimeConfig,
    runtime_options: RuntimeOpenOptions,
    config_path: &Path,
    home_workspace: &Path,
) -> Result<(), Box<dyn Error>> {
    validate_enrollment_url(&command)?;
    if config.storage.adapter == StorageAdapter::Ephemeral {
        return Err("Cloud connections need persistent redb or PostgreSQL storage; ephemeral storage cannot host a reachable runtime".into());
    }
    if !config.models.roles.contains_key("primary") {
        return Err("managed Cloud requires a configured primary model role".into());
    }
    let mut arguments = local_arguments(&command)?.clone();
    if arguments.local_config.is_some() {
        return Err("external daemon connections use the advanced connector path".into());
    }
    let identity = detect_workspace_identity(&runtime_options.workspace)?;
    if identity.canonical_path() != runtime_options.workspace {
        return Err("workspace identity changed before Cloud setup".into());
    }
    identity.revalidate()?;
    let home = runtime_options
        .colossus_home
        .as_deref()
        .ok_or("managed Cloud requires the selected CLI home")?;
    let mut metadata = Metadata::open(home, home_workspace, &arguments.storage.name)?;
    let retained = metadata.load()?;
    let fresh = retained.is_none();
    if fresh && !matches!(command, ConnectorCommand::Enroll(_)) {
        return Err("enroll this workspace first with `colossus cloud enroll URL`".into());
    }
    if let Some(binding) = &retained {
        retain_consented_access(&mut arguments, binding)?;
    }
    let planned = Binding::planned(
        &arguments,
        &identity,
        config_path,
        digest(config.to_resolved_yaml()?.as_bytes()),
        metadata.api_directory()?,
    );
    let mut binding = match retained {
        Some(binding) => {
            binding.validate_context(&planned)?;
            binding
        }
        None => planned,
    };

    let active = match WorkerClient::discover(config)? {
        Some(worker) => match worker.ping().await {
            Ok(status) => Some(status),
            Err(error) if super::worker_shell::worker_probe_allows_embedded_fallback(&error, false) => None,
            Err(_) => return Err("the configured worker cannot be authenticated; inspect it before retrying Cloud setup".into()),
        },
        None => None,
    };
    if let Some(status) = active {
        super::worker_dispatch::validate_worker_workspace(&status, &runtime_options.workspace)?;
        if fresh || binding.stage != Stage::Ready {
            return Err("an existing worker owns this workspace without this managed Cloud binding; stop it explicitly when idle, then retry enrollment".into());
        }
        if status
            .get("managed_configuration_identity")
            .and_then(serde_json::Value::as_str)
            != Some(binding.config_sha256.as_str())
        {
            return Err("the active worker was not started with this managed configuration; stop it explicitly when idle and restart through `colossus cloud run`".into());
        }
        metadata.connection(&mut binding)?;
        disclosure(&binding, false);
        bind_connection(&mut command, &metadata.connection)?;
        let invitation = prepare_invitation(&command)?;
        drop(metadata.lock.take());
        return delegate(command, invitation).await;
    }

    // Opening the normal runtime also fences an embedded CLI/TUI owner that has
    // no worker endpoint. It never stops that owner or changes its configuration.
    if fresh {
        metadata.save(&binding)?;
    }
    let mode = if arguments.allow_approvals {
        WorkerApprovalMode::Ask
    } else {
        WorkerApprovalMode::Deny
    };
    #[cfg(target_os = "linux")]
    let browser = if config.browser.enabled {
        super::browser_bundle::discover(home_workspace).await?
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let browser = None;
    let observability = colossus_observability::ObservabilityGuard::install(&config.observability)?;
    let diagnostics = observability
        .as_ref()
        .map(colossus_observability::ObservabilityGuard::diagnostics)
        .unwrap_or_default();
    let server = WorkerServer::open_with_verified_browser(config, mode, runtime_options,
        Arc::new(colossus_runtime::EnvironmentCredentialResolver), None, None, browser)?
        .with_managed_configuration_identity(binding.config_sha256.clone())?
        .with_observability_diagnostics(move || {
            serde_json::to_value(diagnostics.force_flush()).unwrap_or_else(|_| serde_json::json!({
                "ready": false, "checks": [{"name": "host", "status": "fail", "detail": "The exporter diagnostic failed safely."}]
            }))
        });
    let actual_grant = grant(&server, &arguments);
    if binding.stage != Stage::Planned && binding.grant != actual_grant {
        return Err("the runtime's active grant changed; reconnect with the original configuration or review a new named enrollment".into());
    }
    let store = credential_store(
        &binding.public_api_directory,
        arguments.storage.vault_key_variable.as_deref(),
    )?;
    let environment = if binding.stage == Stage::Planned {
        PublicApiEnvironment::open(&binding.public_api_directory, store.as_ref())?
    } else {
        PublicApiEnvironment::open_existing(&binding.public_api_directory, store.as_ref())?
    };
    let (instance, pin) = environment.public_identity()?;
    if binding
        .instance_id
        .as_ref()
        .is_some_and(|id| id != &instance.to_string())
        || binding
            .certificate_sha256
            .as_ref()
            .is_some_and(|expected| expected != &pin)
    {
        return Err("the managed local instance or independent TLS pin changed; restore its exact native custody".into());
    }
    let manager = environment.credential_manager(&server);
    if binding.stage == Stage::Planned {
        binding.instance_id = Some(instance.to_string());
        binding.certificate_sha256 = Some(pin);
        binding.grant = actual_grant;
        binding.stage = Stage::CredentialIntent;
        metadata.save(&binding)?;
        let enrolled = enroll_application(
            &environment,
            &server,
            store.as_ref(),
            EnrollmentRequest {
                application_id: &binding.application_id,
                scopes: &binding.grant.scopes,
                roles: &binding.grant.roles,
                tools: &binding.grant.tools,
                destination_service: &binding.keyring_service,
                destination_account: &binding.keyring_account,
                replace_destination: false,
                retirement_source: None,
            },
        )?;
        binding.credential_id = Some(enrolled.credential_id);
        binding.stage = Stage::Ready;
        metadata.save(&binding)?;
    } else {
        // Native delivery may have committed before a prior process lost its
        // response. An active exact-application credential can be adopted; absence
        // never authorizes issuing another credential or consuming an invitation.
        let token = store.read(&binding.keyring_service, &binding.keyring_account)?
            .ok_or("local credential activation is unresolved or native custody is missing; inspect the retained binding before retrying")?;
        let token =
            std::str::from_utf8(&token).map_err(|_| "native connector credential is invalid")?;
        let credential_id = manager
            .validate_rotation_source(token, &binding.application_id)
            .map_err(|_| "native connector credential no longer matches this managed runtime")?;
        if binding
            .credential_id
            .as_ref()
            .is_some_and(|expected| expected != &credential_id)
        {
            return Err("the dedicated connector credential identity changed".into());
        }
        binding.credential_id = Some(credential_id);
        binding.stage = Stage::Ready;
        metadata.save(&binding)?;
    }
    metadata.connection(&mut binding)?;
    identity.revalidate()?;
    let server = server
        .prepare_worker_ipc()
        .await?
        .enable_public_api(
            environment
                .host_options(&manager)?
                .with_human_terminals(arguments.enable_human_terminals),
        )
        .await?;
    // Match ordinary worker composition: startup custody must not retain a
    // separate headless authority lease while the SDK loads its dedicated bearer.
    drop(manager);
    drop(store);
    disclosure(&binding, true);
    bind_connection(&mut command, &metadata.connection)?;
    // No worker is serving in this process while the native terminal prompt is
    // blocking. The opaque preparation retains the reviewed trust bytes and token.
    let invitation = prepare_invitation(&command)?;
    drop(metadata.lock.take());
    let result = serve_owned(command, invitation, server, environment, arguments.storage).await;
    if let Some(observability) = observability {
        observability.shutdown();
    }
    result
}

fn prepare_invitation(
    command: &ConnectorCommand,
) -> Result<Option<colossus_connector::PreparedEnrollment>, Box<dyn Error>> {
    match command {
        ConnectorCommand::Enroll(arguments) => {
            colossus_connector::prepare_enrollment_invitation(arguments).map(Some)
        }
        _ => Ok(None),
    }
}

async fn delegate(
    command: ConnectorCommand,
    invitation: Option<colossus_connector::PreparedEnrollment>,
) -> Result<(), Box<dyn Error>> {
    let start = match &command {
        ConnectorCommand::Enroll(arguments) => Some(arguments.local.clone()),
        _ => None,
    };
    colossus_connector::run_cli_with_invitation(command, invitation).await?;
    if let Some(arguments) = start {
        colossus_connector::run_cli(ConnectorCommand::Run(arguments)).await?;
    }
    Ok(())
}

async fn serve_owned(
    command: ConnectorCommand,
    invitation: Option<colossus_connector::PreparedEnrollment>,
    server: WorkerServer,
    environment: PublicApiEnvironment,
    storage: StorageArguments,
) -> Result<(), Box<dyn Error>> {
    let keep_worker = !matches!(command, ConnectorCommand::Policy(_));
    let (shutdown, mut receiver) = tokio::sync::watch::channel(false);
    let interrupt = shutdown.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = interrupt.send(true);
        }
    });
    let mut host = tokio::spawn(async move {
        // The API directory lease and stable native keys live exactly as long as
        // this owned worker, including after the connector has disconnected.
        let _environment = environment;
        server
            .serve_until(async move {
                loop {
                    let stopping = *receiver.borrow_and_update();
                    if stopping {
                        break;
                    }
                    if receiver.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
    });
    let connector = delegate(command, invitation);
    tokio::pin!(connector);
    let result: Result<(), Box<dyn Error>> = tokio::select! {
        result = &mut connector => {
            if result.is_err() || !keep_worker { let _ = shutdown.send(true); }
            else if !*shutdown.borrow() {
                eprintln!("Cloud connection ended. The owned local worker remains available; Ctrl-C checkpoints and stops it.");
            }
            let stopped = (&mut host).await;
            result.and_then(|()| stopped.map_err(|_| -> Box<dyn Error> { "managed worker task failed".into() })?
                .map_err(|error| Box::new(error) as Box<dyn Error>))
        }
        stopped = &mut host => {
            // Stop only this named connector. The lifecycle command checks its
            // run identity and never stops an unrelated local worker.
            let _ = colossus_connector::run_cli(ConnectorCommand::Disconnect(storage)).await;
            let drained = tokio::time::timeout(std::time::Duration::from_secs(12), connector.as_mut()).await;
            let worker = stopped.map_err(|_| -> Box<dyn Error> { "managed worker task failed".into() })
                .and_then(|result| result.map_err(|error| Box::new(error) as Box<dyn Error>));
            worker.and_then(|()| drained.map_err(|_| -> Box<dyn Error> {
                "the owned worker stopped but connector shutdown was not confirmed; inspect its retained status before retrying".into()
            })?)
        }
    };
    signal.abort();
    result
}

#[cfg(test)]
mod tests;
