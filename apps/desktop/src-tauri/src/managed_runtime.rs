use colossus_sdk::{
    ApiMajor, ApiScope, AppPrivateInstanceDir, Colossus, CreateRunRequest, GetRunRequest,
    IdempotencyKey, InputContentPart, InstanceId, ListRunsRequest, ManagedAccessProfile,
    ManagedExecutionBoundary, ManagedFieldOverride, ManagedJournalPayloadMode,
    ManagedMcpCredentialHeader, ManagedMcpOAuthConfig, ManagedMcpResearchTool,
    ManagedMcpServerConfig, ManagedMcpTransport, ManagedModelCapabilities, ManagedModelConfig,
    ManagedOtlpProtocol, ManagedProviderConfig, ManagedProviderKind, ManagedReasoningEffort,
    ManagedRuntimeConfig, ManagedSearchConfig, ManagedSearchKind, ManagedTelemetryConfig,
    NativeSidecarLifecycle, PageRequest, PageResponse, RunMode, RunStatus, SdkError, Secret,
    SidecarApplicationGrant, SidecarApprovalBrokerGrant, SidecarBootstrapConfig,
    SidecarHostCredential, SidecarOptions, WorkspaceIdentity, scopes,
};
use colossus_worker_protocol::{WorkerControlClient, worker_ipc_endpoint};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    str::FromStr as _,
    time::Duration,
};

use crate::{
    bundle::VerifiedBundle,
    desktop_dto::{ManagedRuntimeStateDto, RuntimeFailureCodeDto},
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ExecutionBoundarySetting, ProviderKindSetting,
        ReasoningEffortSetting, SettingsStore, load_provider_secret, normalized_settings_snapshot,
        revalidate_workspace,
    },
    dto::CommandErrorDto,
    managed_configuration::{
        JournalPayloadSetting, McpTransportSetting, OtlpProtocolSetting,
        ResolvedSpaceConfiguration, SearchProviderKindSetting, resolve_space_configuration,
    },
    run_list,
    state::{
        AppState, MAX_LIVE_MANAGED_SPACES, ManagedConfigurationDrainGuard, ManagedHealth,
        TargetConsentContext,
    },
    terminal::{TerminalWorkerAuthentication, TerminalWorkspace},
};

const APPLICATION_ID: &str = "app:colossus-desktop-managed";
const SELF_TEST_APPLICATION_ID: &str = "app:colossus-desktop-self-test";
const SELF_TEST_INSTANCE_DOMAIN: &[u8] = b"colossus-desktop-self-test-instance-v2\0";
const ACTIVE_RUN_PAGE_SIZE: u32 = 100;
const MAX_ACTIVE_RUN_PAGES: usize = 4_096;
const CONFIGURATION_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(250);
const CONFIGURATION_DRAIN_TIMEOUT: Duration = Duration::from_mins(5);
const PRIMARY_SCOPES: [&str; 6] = [
    scopes::RUNS_EXECUTE,
    scopes::RUNS_READ,
    scopes::RUNS_CONTROL,
    scopes::PROMPTS_RESPOND,
    scopes::ARTIFACTS_READ,
    scopes::ARTIFACTS_WRITE,
];

const TRUSTED_BUILTIN_TOOL_GRANT: &[&str] = &[
    "agent.delegate",
    "agent.list",
    "agent.result",
    "context.compact",
    "context.restore",
    "context.show",
    "context.snapshots",
    "decision.archive",
    "decision.create",
    "decision.list",
    "decision.supersede",
    "decision.update",
    "echo",
    "filesystem.list",
    "filesystem.read",
    "filesystem.replace",
    "filesystem.search",
    "filesystem.write",
    "git.diff",
    "git.show",
    "git.status",
    "goal.show",
    "goal.update",
    "memory.archive",
    "memory.create",
    "memory.list",
    "memory.search",
    "memory.supersede",
    "memory.update",
    "mcp.call",
    "mcp.servers",
    "mcp.tools",
    "network.http",
    "patch.apply",
    "patch.preview",
    "patch.reverse",
    "plan.create",
    "plan.approve_request",
    "plan.show",
    "plan.update",
    "repo.file_summary",
    "repo.map",
    "repo.references",
    "repo.symbol_search",
    "shell.run",
    "skill.install",
    "skill.inspect",
    "skill.read",
    "skill.resource.list",
    "skill.resource.read",
    "skill.scaffold",
    "skill.validate",
    "skill.write",
    "task.create",
    "task.list",
    "task.update",
    "tool.search",
    "trace.export",
    "trace.show",
    "user.ask",
    "web.fetch",
    "web.search",
    "docs.fetch",
];

pub(crate) async fn start(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    restarting: bool,
) -> Result<(), CommandErrorDto> {
    let normalized_settings = normalized_settings_snapshot(settings)?;
    let settings = &normalized_settings;
    let space_id = settings.selected_space_id.as_deref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "spaceId",
            "Select a Workspace before starting Managed Local.",
        )
    })?;
    let lifecycle_generation = state.begin_managed_lifecycle_for(space_id).await;
    let restore_selection = state.selected_target_id().await.as_deref() == Some(space_id);
    if restore_selection {
        // The selection writer waits for native run operations to release their
        // read leases before the managed transport is closed. No request is replayed.
        state.select_target(None).await;
    }
    let result = start_after_operation_drain(
        state,
        store,
        settings,
        space_id,
        restarting,
        lifecycle_generation,
    )
    .await;
    if restore_selection {
        state.select_target(Some(space_id.to_owned())).await;
        if result.is_ok() {
            state.activate_managed_terminal_for(space_id).await;
        }
    }
    result
}

async fn start_after_operation_drain(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    space_id: &str,
    restarting: bool,
    lifecycle_generation: u64,
) -> Result<(), CommandErrorDto> {
    let starting_state = if restarting {
        ManagedRuntimeStateDto::Restarting
    } else {
        ManagedRuntimeStateDto::Starting
    };
    state
        .set_managed_health_for(
            space_id,
            ManagedHealth {
                state: starting_state,
                message: if restarting {
                    "Restarting the managed Colossus runtime…".into()
                } else {
                    "Starting the managed Colossus runtime…".into()
                },
                failure_code: None,
            },
        )
        .await;

    state.clear_terminal_workspace().await;
    state.clear_managed_worker_for(space_id).await;
    if let Some(previous) = state.remove_target(space_id).await
        && let Err(error) = previous.client.close().await
    {
        let (error, failure_code) = classify_sdk(error, RuntimeFailureCodeDto::Transport);
        state
            .set_managed_health_for(
                space_id,
                ManagedHealth {
                    state: ManagedRuntimeStateDto::Failed,
                    message: error.message.clone(),
                    failure_code: Some(failure_code),
                },
            )
            .await;
        return Err(error);
    }

    ensure_managed_capacity(state, space_id).await?;
    let result = start_inner(state, store, settings, space_id, lifecycle_generation).await;
    match result {
        Ok(()) => {
            state.sync_managed_lifecycle_health_for(space_id).await;
            Ok(())
        }
        Err((error, failure_code)) => {
            state
                .clear_managed_lifecycle_for(space_id, lifecycle_generation)
                .await;
            state.clear_terminal_workspace().await;
            state
                .set_managed_health_for(
                    space_id,
                    ManagedHealth {
                        state: ManagedRuntimeStateDto::Failed,
                        message: error.message.clone(),
                        failure_code: Some(failure_code),
                    },
                )
                .await;
            Err(error)
        }
    }
}

async fn ensure_managed_capacity(
    state: &AppState,
    requested_space_id: &str,
) -> Result<(), CommandErrorDto> {
    let live = state.live_managed_target_ids().await;
    if live.iter().any(|target_id| target_id == requested_space_id)
        || live.len() < MAX_LIVE_MANAGED_SPACES
    {
        return Ok(());
    }
    let mut candidates = Vec::with_capacity(live.len());
    for target_id in live {
        let last_used = state.managed_space_last_used(&target_id).await.unwrap_or(0);
        let Some(target) = state.target(&target_id).await else {
            state.remove_managed_space_runtime(&target_id).await;
            continue;
        };
        let active = managed_target_has_active_work(&target.client).await?;
        candidates.push((last_used, target_id, active));
    }
    let Some(target_id) = idle_lru_candidate(&candidates)? else {
        return Ok(());
    };
    if let Some(target) = state.remove_target(&target_id).await {
        target
            .client
            .close()
            .await
            .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Transport).0)?;
    }
    state.remove_managed_space_runtime(&target_id).await;
    Ok(())
}

async fn managed_target_has_active_work(client: &Colossus) -> Result<bool, CommandErrorDto> {
    let mut page_token = String::new();
    let mut seen_tokens = BTreeSet::new();
    for _ in 0..MAX_ACTIVE_RUN_PAGES {
        let response = run_list::list_runs(
            client,
            ListRunsRequest {
                session_id: None,
                statuses: vec![
                    RunStatus::Queued,
                    RunStatus::Running,
                    RunStatus::Waiting,
                    RunStatus::Cancelling,
                ],
                page: Some(PageRequest {
                    page_size: ACTIVE_RUN_PAGE_SIZE,
                    page_token,
                }),
                include_archived: false,
            },
        )
        .await
        .map_err(CommandErrorDto::from_api)?;
        if !response.runs.is_empty() {
            return Ok(true);
        }
        match next_active_run_page_token(response.page.as_ref(), &mut seen_tokens) {
            Ok(Some(next)) => page_token = next,
            Ok(None) => return Ok(false),
            Err(()) => return Ok(true),
        }
    }
    Ok(true)
}

pub(crate) async fn drain_active_runs_for_configuration(
    state: &AppState,
    space_id: &str,
) -> Result<Option<ManagedConfigurationDrainGuard>, CommandErrorDto> {
    let Some(target) = state.target(space_id).await else {
        return Ok(None);
    };
    if !matches!(target.consent, TargetConsentContext::ManagedLocal) {
        return Ok(None);
    }
    let previous_health = state.managed_health_for(space_id).await;
    let drain = state
        .begin_configuration_drain_for(space_id)
        .await
        .ok_or_else(|| {
            CommandErrorDto::busy(
                "This Workspace is already applying a configuration update. Wait for it to finish.",
            )
        })?;
    state
        .set_managed_health_for(
            space_id,
            ManagedHealth {
                state: ManagedRuntimeStateDto::Stopping,
                message: "Draining active work before applying configuration…".into(),
                failure_code: None,
            },
        )
        .await;

    let drained = tokio::time::timeout(CONFIGURATION_DRAIN_TIMEOUT, async {
        loop {
            if !managed_target_has_active_work(&target.client).await? {
                return Ok::<(), CommandErrorDto>(());
            }
            tokio::time::sleep(CONFIGURATION_DRAIN_POLL_INTERVAL).await;
        }
    })
    .await;
    match drained {
        Ok(Ok(())) => Ok(Some(drain)),
        Ok(Err(error)) => {
            state
                .set_managed_health_for(space_id, previous_health)
                .await;
            Err(error)
        }
        Err(_) => {
            state
                .set_managed_health_for(space_id, previous_health)
                .await;
            Err(CommandErrorDto::busy(
                "Active work did not drain before the configuration deadline. The current runtime is still active and no settings were saved.",
            ))
        }
    }
}

fn next_active_run_page_token(
    page: Option<&PageResponse>,
    seen_tokens: &mut BTreeSet<String>,
) -> Result<Option<String>, ()> {
    let Some(token) = page.map(|page| page.next_page_token.clone()) else {
        return Ok(None);
    };
    if token.is_empty() {
        return Ok(None);
    }
    if !seen_tokens.insert(token.clone()) {
        return Err(());
    }
    Ok(Some(token))
}

fn idle_lru_candidate(
    candidates: &[(u64, String, bool)],
) -> Result<Option<String>, CommandErrorDto> {
    if candidates.len() < MAX_LIVE_MANAGED_SPACES {
        return Ok(None);
    }
    if let Some((_, target_id, _)) = candidates
        .iter()
        .filter(|(_, _, active)| !active)
        .min_by_key(|(last_used, _, _)| *last_used)
    {
        return Ok(Some(target_id.clone()));
    }
    Err(CommandErrorDto::busy(
        "Four Workspaces are already running active work. Switch to one of them or finish a run before starting another Workspace.",
    ))
}

pub(crate) async fn self_test(
    state: &AppState,
    store: &SettingsStore,
    _settings: &DesktopSettings,
) -> Result<(), CommandErrorDto> {
    let storage = store.self_test_storage()?;
    let self_test_workspace = crate::desktop_settings::validate_workspace(&storage.workspace)?;
    let canonical_workspace = self_test_workspace.path;
    let workspace_identity = self_test_workspace.identity.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "desktop_storage",
            "The offline self-test workspace is unavailable.",
            false,
        )
    })?;
    let bundle = VerifiedBundle::load()?;
    state
        .terminal_manager()
        .set_verified_colossus_cli(
            &bundle.cli_path,
            bundle.cli_sha256,
            bundle.macos_code_signing_requirement,
        )
        .map_err(CommandErrorDto::from_terminal)?;
    let instance_id = self_test_instance_id(&storage.instance_dir)?;
    let options = SidecarOptions::new(
        instance_id,
        AppPrivateInstanceDir::new(storage.instance_dir).map_err(CommandErrorDto::from_sdk)?,
        bundle.sidecar,
        ApiMajor::new(1).map_err(CommandErrorDto::from_sdk)?,
    )
    .map_err(CommandErrorDto::from_sdk)?;
    let runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal)
        .with_execution_boundary(ManagedExecutionBoundary::OfflineIsolated);
    let colossus_home = store.home_root()?.to_owned();
    // Keep this trusted diagnostic suppression explicit: the desktop security contract
    // audits this exact call site rather than accepting an indirect function pointer.
    #[allow(clippy::redundant_closure_for_method_calls)]
    let bootstrap = SidecarBootstrapConfig::new(
        canonical_workspace,
        runtime,
        self_test_grant().map_err(CommandErrorDto::from_sdk)?,
    )
    .and_then(|bootstrap| bootstrap.with_expected_workspace_identity(workspace_identity))
    .and_then(|bootstrap| bootstrap.with_colossus_home(colossus_home))
    .map(|bootstrap| bootstrap.without_automatic_agent_instructions_for_diagnostics())
    .map_err(CommandErrorDto::from_sdk)?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .map_err(classify_self_test_sdk)?;
    let probe = probe_offline_echo(&client).await;
    let close = client.close().await.map_err(CommandErrorDto::from_sdk);
    probe?;
    close
}

fn self_test_instance_id(instance_dir: &Path) -> Result<InstanceId, CommandErrorDto> {
    let canonical = std::fs::canonicalize(instance_dir).map_err(|_| {
        CommandErrorDto::local_sanitized(
            "desktop_storage",
            "The offline self-test runtime is unavailable.",
            false,
        )
    })?;
    let encoded = canonical.to_str().ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "desktop_storage",
            "The offline self-test runtime is unavailable.",
            false,
        )
    })?;
    let mut digest = Sha256::new();
    digest.update(SELF_TEST_INSTANCE_DOMAIN);
    digest.update(encoded.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 UUIDv8 provides a stable custom namespace for this per-home
    // diagnostic runtime without sharing Windows Credential Manager anchors globally.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let instance_id = InstanceId::from_uuid(uuid::Uuid::from_bytes(bytes));
    if instance_id.as_uuid().is_nil() {
        return Err(CommandErrorDto::local_sanitized(
            "internal",
            "The offline self-test is unavailable.",
            false,
        ));
    }
    Ok(instance_id)
}

async fn probe_offline_echo(client: &Colossus) -> Result<(), CommandErrorDto> {
    let idempotency_key =
        IdempotencyKey::new(format!("desktop-self-test-{}", uuid::Uuid::now_v7()))
            .map_err(CommandErrorDto::from_api)?;
    let run = client
        .create_run(offline_self_test_request(idempotency_key))
        .await
        .map_err(CommandErrorDto::from_api)?
        .run;
    let run_id = run.run_id;
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let run = client
                .get_run(GetRunRequest {
                    run_id: run_id.clone(),
                })
                .await?
                .run;
            if matches!(
                run.status,
                RunStatus::Completed
                    | RunStatus::Failed
                    | RunStatus::Cancelled
                    | RunStatus::Interrupted
                    | RunStatus::OutcomeUnknown
            ) {
                return Ok::<RunStatus, colossus_sdk::ApiError>(run.status);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "self_test_timeout",
            "The offline self-test did not finish in time.",
            true,
        )
    })?
    .map_err(CommandErrorDto::from_api)?;
    if terminal == RunStatus::Completed {
        Ok(())
    } else {
        let message = format!(
            "The offline Colossus self-test ended with status {} before producing an echo result.",
            run_status_name(terminal)
        );
        Err(CommandErrorDto::local_sanitized(
            "self_test_failed",
            &message,
            false,
        ))
    }
}

fn offline_self_test_request(idempotency_key: IdempotencyKey) -> CreateRunRequest {
    CreateRunRequest {
        input: vec![InputContentPart::Text("offline self-test".into())],
        session_id: None,
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        research_depth: None,
        research_sources: Vec::new(),
        selected_skills: Vec::new(),
        plan_action: None,
        branch: None,
        max_turns: 1,
        idempotency_key,
    }
}

async fn start_inner(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    space_id: &str,
    lifecycle_generation: u64,
) -> Result<(), (CommandErrorDto, RuntimeFailureCodeDto)> {
    let workspace = settings.workspace.as_ref().ok_or_else(|| {
        classified(
            "workspace_required",
            "Choose a workspace before starting Managed Local.",
            RuntimeFailureCodeDto::Configuration,
        )
    })?;
    if !settings.managed_configured() {
        return Err(classified(
            "provider_required",
            "Configure at least one provider and a primary model before starting Managed Local.",
            RuntimeFailureCodeDto::Provider,
        ));
    }
    let canonical_workspace = revalidate_workspace(workspace)
        .map_err(|error| (error, RuntimeFailureCodeDto::Permission))?;
    let workspace_identity = expected_workspace_identity(workspace)?;
    let colossus_home = store
        .home_root()
        .map_err(|error| (error, RuntimeFailureCodeDto::Permission))?
        .to_owned();

    // Executable identity is established before the keychain is touched, and is
    // rechecked by the SDK immediately before its no-shell spawn.
    let bundle =
        VerifiedBundle::load().map_err(|error| (error, RuntimeFailureCodeDto::Integrity))?;
    state
        .terminal_manager()
        .set_verified_colossus_cli(
            &bundle.cli_path,
            bundle.cli_sha256,
            bundle.macos_code_signing_requirement,
        )
        .map_err(|error| {
            (
                CommandErrorDto::from_terminal(error),
                RuntimeFailureCodeDto::Integrity,
            )
        })?;

    let managed_storage = store
        .managed_workspace_storage(
            &settings.managed_instance_id,
            &canonical_workspace,
            &workspace_identity,
        )
        .map_err(|error| (error, RuntimeFailureCodeDto::Permission))?;
    let instance_id = InstanceId::from_str(&managed_storage.instance_id).map_err(|_| {
        classified(
            "configuration",
            "Managed Local configuration is invalid.",
            RuntimeFailureCodeDto::Configuration,
        )
    })?;
    let options = SidecarOptions::new(
        instance_id,
        AppPrivateInstanceDir::new(managed_storage.instance_dir)
            .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Permission))?,
        bundle.sidecar,
        ApiMajor::new(1).map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Internal))?,
    )
    .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Configuration))?;
    let PreparedManagedBootstrap {
        bootstrap,
        worker_authentication,
        terminal_enabled,
    } = prepare_managed_bootstrap(
        &canonical_workspace,
        workspace_identity.clone(),
        store,
        settings,
    )?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    install_managed_target(
        state,
        space_id,
        lifecycle_generation,
        lifecycle,
        options,
        TerminalWorkspace {
            id: workspace.id.clone(),
            display_name: workspace.display_name.clone(),
            workspace: canonical_workspace,
            workspace_identity,
            colossus_home,
            config: None,
            worker_authentication: Some(worker_authentication),
        },
        terminal_enabled,
    )
    .await
}

fn provider_host_credentials(
    resolved: &ResolvedSpaceConfiguration,
) -> Result<Vec<SidecarHostCredential>, (CommandErrorDto, RuntimeFailureCodeDto)> {
    let mut credential_ids = resolved
        .providers
        .iter()
        .filter_map(|provider| provider.credential_id.clone())
        .collect::<BTreeSet<_>>();
    for server in &resolved.mcp_servers {
        credential_ids.extend(server.environment_credentials.values().cloned());
        credential_ids.extend(
            server
                .credential_headers
                .values()
                .map(|header| header.credential_id.clone()),
        );
        credential_ids.extend(
            server
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.client_secret_credential_id.clone()),
        );
    }
    credential_ids.extend(
        resolved
            .search_providers
            .iter()
            .filter_map(|search| search.credential_id.clone()),
    );
    credential_ids
        .into_iter()
        .map(|credential_id| {
            let credential = load_provider_secret(&credential_id)
                .map_err(|error| (error, RuntimeFailureCodeDto::Provider))?;
            let provider_secret = Secret::new(credential.to_vec())
                .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Provider))?;
            SidecarHostCredential::new(&credential_id, provider_secret)
                .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Provider))
        })
        .collect()
}

struct PreparedManagedBootstrap {
    bootstrap: SidecarBootstrapConfig,
    worker_authentication: TerminalWorkerAuthentication,
    terminal_enabled: bool,
}

fn prepare_managed_bootstrap(
    workspace: &Path,
    workspace_identity: WorkspaceIdentity,
    store: &SettingsStore,
    settings: &DesktopSettings,
) -> Result<PreparedManagedBootstrap, (CommandErrorDto, RuntimeFailureCodeDto)> {
    let space = settings
        .selected_space_id
        .as_deref()
        .and_then(|space_id| settings.space(space_id))
        .ok_or_else(|| {
            classified(
                "configuration",
                "Managed Local configuration is invalid.",
                RuntimeFailureCodeDto::Configuration,
            )
        })?;
    let resolved = resolve_space_configuration(&settings.global_configuration, space)
        .map_err(|error| (error, RuntimeFailureCodeDto::Configuration))?;
    let host_credentials = provider_host_credentials(&resolved)?;
    let codex_auth_path = codex_auth_path(settings)?;
    let ca_bundle_path = settings
        .additional_ca_bundle
        .as_ref()
        .map(|bundle| store.ca_bundle_path(bundle))
        .transpose()
        .map_err(|error| (error, RuntimeFailureCodeDto::Permission))?;
    let approval_broker_grant = approval_broker_grant()
        .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Configuration))?;
    let worker_authentication = TerminalWorkerAuthentication::random().map_err(|error| {
        (
            CommandErrorDto::from_terminal(error),
            RuntimeFailureCodeDto::Internal,
        )
    })?;
    let worker_bootstrap_secret = worker_authentication.copy_secret();
    let paths = ManagedBootstrapPaths {
        ca_bundle: ca_bundle_path.as_deref(),
        codex_auth: codex_auth_path.as_deref(),
        colossus_home: store
            .home_root()
            .map_err(|error| (error, RuntimeFailureCodeDto::Permission))?,
    };
    let bootstrap = managed_bootstrap(
        workspace,
        workspace_identity,
        &resolved,
        host_credentials,
        approval_broker_grant,
        worker_bootstrap_secret.as_ref(),
        &paths,
    )
    .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Configuration))?;
    Ok(PreparedManagedBootstrap {
        bootstrap,
        worker_authentication,
        terminal_enabled: resolved.terminal_enabled && settings.has_local_terminal_consent(),
    })
}

struct ManagedBootstrapPaths<'a> {
    ca_bundle: Option<&'a Path>,
    codex_auth: Option<&'a Path>,
    colossus_home: &'a Path,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn managed_bootstrap(
    workspace: &Path,
    workspace_identity: WorkspaceIdentity,
    resolved: &ResolvedSpaceConfiguration,
    host_credentials: Vec<SidecarHostCredential>,
    approval_broker_grant: SidecarApprovalBrokerGrant,
    worker_authentication: &[u8],
    paths: &ManagedBootstrapPaths<'_>,
) -> Result<SidecarBootstrapConfig, SdkError> {
    let runtime = managed_runtime_config(resolved);
    let bootstrap = SidecarBootstrapConfig::new(
        workspace,
        runtime,
        application_grant(resolved.access_profile)?,
    )?
    .with_expected_workspace_identity(workspace_identity)?
    .with_colossus_home(paths.colossus_home)?
    .with_approval_broker_grant(approval_broker_grant)?
    .with_host_credentials(host_credentials)?
    .with_worker_ipc_authentication(Secret::new(worker_authentication.to_vec())?)?;
    #[cfg(debug_assertions)]
    let bootstrap = bootstrap.with_plaintext_journal_for_development();
    let bootstrap = match paths.ca_bundle {
        Some(path) => bootstrap.with_additional_ca_bundle_path(path)?,
        None => bootstrap,
    };
    match paths.codex_auth {
        Some(path) => bootstrap.with_codex_auth_path(path),
        None => Ok(bootstrap),
    }
}

#[allow(clippy::too_many_lines)]
fn managed_runtime_config(resolved: &ResolvedSpaceConfiguration) -> ManagedRuntimeConfig {
    let (search_profiles, search_roles) = managed_search(resolved);
    ManagedRuntimeConfig {
        access_profile: access_profile(resolved.access_profile),
        execution_boundary: execution_boundary(resolved.execution_boundary),
        providers: resolved
            .providers
            .iter()
            .map(|provider| ManagedProviderConfig {
                profile: provider.profile.clone(),
                kind: provider_kind(provider.kind),
                base_url: (provider.kind != ProviderKindSetting::Codex)
                    .then(|| provider.base_url.clone()),
                credential_id: provider.credential_id.clone(),
                timeout_ms: provider.effective_timeout_ms(),
                // Desktop settings do not expose the Chat Completions output-token wire
                // parameter yet, so managed desktop providers keep the `max_tokens` default.
                chat_completions_output_token_parameter: None,
            })
            .collect(),
        models: resolved
            .models
            .iter()
            .map(|model| ManagedModelConfig {
                profile: model.profile.clone(),
                provider_profile: model.provider_profile.clone(),
                model: model.model.clone(),
                context_window_tokens: model.context_window_tokens,
                max_output_tokens: model.max_output_tokens,
                capabilities: ManagedModelCapabilities {
                    tool_calls: model.capabilities.tool_calls,
                    streaming: model.capabilities.streaming,
                    image_inputs: model.capabilities.image_inputs,
                },
                reasoning_effort: model.reasoning_effort.map(reasoning_effort),
            })
            .collect(),
        roles: resolved.model_roles.clone(),
        search_profiles,
        search_roles,
        mcp_servers: resolved
            .mcp_servers
            .iter()
            .map(|server| ManagedMcpServerConfig {
                name: server.name.clone(),
                transport: match server.transport {
                    McpTransportSetting::Stdio => ManagedMcpTransport::Stdio,
                    McpTransportSetting::StreamableHttp => ManagedMcpTransport::StreamableHttp,
                },
                command: server.command.clone(),
                args: server.args.clone(),
                working_directory: server.working_directory.clone(),
                environment_credentials: server.environment_credentials.clone(),
                url: server.url.clone(),
                headers: server.headers.clone(),
                credential_headers: server
                    .credential_headers
                    .iter()
                    .map(|(name, header)| {
                        (
                            name.clone(),
                            ManagedMcpCredentialHeader {
                                scheme: header.scheme.clone(),
                                credential_id: header.credential_id.clone(),
                            },
                        )
                    })
                    .collect(),
                allow_stateless: server.allow_stateless,
                oauth: server.oauth.as_ref().map(|oauth| ManagedMcpOAuthConfig {
                    client_id: oauth.client_id.clone(),
                    client_secret_credential_id: oauth.client_secret_credential_id.clone(),
                    callback_port: oauth.callback_port,
                    scopes: oauth.scopes.clone(),
                }),
                allowed_tools: server.allowed_tools.clone(),
                research_tools: server
                    .research_tools
                    .iter()
                    .map(|tool| ManagedMcpResearchTool {
                        tool: tool.tool.clone(),
                        title: tool.title.clone(),
                        arguments: tool.arguments.clone(),
                    })
                    .collect(),
                timeout_ms: server.timeout_ms,
                max_output_bytes: server.max_output_bytes,
            })
            .collect(),
        telemetry: resolved
            .telemetry
            .as_ref()
            .map(|telemetry| ManagedTelemetryConfig {
                name: telemetry.name.clone(),
                endpoint: telemetry.endpoint.clone(),
                protocol: match telemetry.protocol {
                    OtlpProtocolSetting::Grpc => ManagedOtlpProtocol::Grpc,
                    OtlpProtocolSetting::HttpProtobuf => ManagedOtlpProtocol::HttpProtobuf,
                },
                timeout_ms: telemetry.timeout_ms,
                traces_enabled: telemetry.traces_enabled,
                trace_sample_ratio_millionths: telemetry.trace_sample_ratio_millionths,
                metrics_enabled: telemetry.metrics_enabled,
                metric_export_interval_ms: telemetry.metric_export_interval_ms,
                logs_otlp: telemetry.logs_otlp,
                logs_stdout_json: telemetry.logs_stdout_json,
                journal_payloads: match telemetry.journal_payloads {
                    JournalPayloadSetting::Disabled => ManagedJournalPayloadMode::Disabled,
                    JournalPayloadSetting::Metadata => ManagedJournalPayloadMode::Metadata,
                    JournalPayloadSetting::Full => ManagedJournalPayloadMode::Full,
                },
                acknowledge_sensitive_content: telemetry.acknowledge_sensitive_content,
                acknowledge_insecure_transport: telemetry.acknowledge_insecure_transport,
                resource_attributes: telemetry.resource_attributes.clone(),
            }),
        field_overrides: resolved
            .field_overrides
            .iter()
            .map(|field| ManagedFieldOverride {
                field_id: field.field_id.clone(),
                value: field.value.clone(),
            })
            .collect(),
    }
}

pub(crate) fn preflight_runtime_configuration(
    settings: &DesktopSettings,
    space_id: &str,
) -> Result<(), CommandErrorDto> {
    let space = settings
        .space(space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Workspace is unknown."))?;
    let resolved = resolve_space_configuration(&settings.global_configuration, space)?;
    managed_runtime_config(&resolved).validate().map_err(|_| {
        CommandErrorDto::local_sanitized(
            "desktop_configuration",
            "The managed Desktop configuration could not be compiled.",
            false,
        )
    })
}

fn managed_search(
    resolved: &ResolvedSpaceConfiguration,
) -> (Vec<ManagedSearchConfig>, BTreeMap<String, String>) {
    if resolved.execution_boundary == ExecutionBoundarySetting::OfflineIsolated {
        return (Vec::new(), BTreeMap::new());
    }
    let profiles = resolved
        .search_providers
        .iter()
        .map(|search| ManagedSearchConfig {
            profile: search.profile.clone(),
            kind: match search.kind {
                SearchProviderKindSetting::Searxng => ManagedSearchKind::Searxng,
                SearchProviderKindSetting::SerpApi => ManagedSearchKind::SerpApi,
            },
            endpoint: search.endpoint.clone(),
            credential_id: search.credential_id.clone(),
            auth_header: search.auth_header.clone(),
            timeout_ms: search.timeout_ms,
        })
        .collect();
    (profiles, resolved.search_roles.clone())
}

fn codex_auth_path(
    settings: &DesktopSettings,
) -> Result<Option<std::path::PathBuf>, (CommandErrorDto, RuntimeFailureCodeDto)> {
    if !settings
        .providers
        .iter()
        .any(|provider| provider.kind == ProviderKindSetting::Codex)
    {
        return Ok(None);
    }
    crate::codex_auth::require_codex_auth_path()
        .map(Some)
        .map_err(|error| (error, RuntimeFailureCodeDto::Provider))
}

fn expected_workspace_identity(
    workspace: &crate::desktop_settings::WorkspaceSetting,
) -> Result<WorkspaceIdentity, (CommandErrorDto, RuntimeFailureCodeDto)> {
    workspace.identity.clone().ok_or_else(|| {
        classified(
            "workspace_required",
            "Choose the workspace again before starting Managed Local.",
            RuntimeFailureCodeDto::Permission,
        )
    })
}

async fn install_managed_target(
    state: &AppState,
    space_id: &str,
    lifecycle_generation: u64,
    lifecycle: NativeSidecarLifecycle,
    options: SidecarOptions,
    mut terminal_workspace: TerminalWorkspace,
    terminal_enabled: bool,
) -> Result<(), (CommandErrorDto, RuntimeFailureCodeDto)> {
    state
        .observe_managed_lifecycle_for(space_id, lifecycle_generation, lifecycle.subscribe_status())
        .await;

    let config_path = options.managed_config_path();
    let worker_endpoint = worker_ipc_endpoint(&options.instance_dir().as_path().join("state.redb"))
        .map_err(|_| {
            classified(
                "runtime_configuration",
                "Managed Local generated an invalid private worker endpoint.",
                RuntimeFailureCodeDto::Configuration,
            )
        })?;
    terminal_workspace.config = Some(config_path.clone());
    let worker_authentication = terminal_workspace
        .worker_authentication
        .as_ref()
        .ok_or_else(|| {
            classified(
                "runtime_authentication",
                "Managed Local approval-mode control is unavailable.",
                RuntimeFailureCodeDto::Authentication,
            )
        })?
        .copy_secret();
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Internal))?;
    let worker =
        match WorkerControlClient::new(worker_endpoint, worker_authentication).map_err(|_| {
            classified(
                "runtime_authentication",
                "Managed Local approval-mode control is unavailable.",
                RuntimeFailureCodeDto::Authentication,
            )
        }) {
            Ok(worker) => worker,
            Err(error) => {
                let _ = client.close().await;
                return Err(error);
            }
        };
    let previous = state
        .replace_target(
            space_id,
            client,
            crate::state::TargetConsentContext::ManagedLocal,
        )
        .await;
    debug_assert!(previous.is_none());
    state.configure_managed_worker_for(space_id, worker).await;
    state
        .configure_managed_terminal_for(space_id, terminal_workspace, terminal_enabled)
        .await;
    if state.selected_target_id().await.as_deref() == Some(space_id) {
        state.activate_managed_terminal_for(space_id).await;
    }
    state.touch_managed_space(space_id).await;
    Ok(())
}

fn application_grant(profile: AccessProfileSetting) -> Result<SidecarApplicationGrant, SdkError> {
    let scopes = PRIMARY_SCOPES
        .into_iter()
        .map(ApiScope::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SdkError::InvalidConfiguration("managed API scope is invalid"))?;
    let tools = if profile == AccessProfileSetting::Minimal {
        vec!["echo".to_owned()]
    } else {
        TRUSTED_BUILTIN_TOOL_GRANT
            .iter()
            .map(|tool| (*tool).to_owned())
            .collect()
    };
    SidecarApplicationGrant::new(APPLICATION_ID, scopes, ["primary".to_owned()], tools)
}

fn approval_broker_grant() -> Result<SidecarApprovalBrokerGrant, SdkError> {
    SidecarApprovalBrokerGrant::new(APPLICATION_ID, ["primary".to_owned()])
}

fn self_test_grant() -> Result<SidecarApplicationGrant, SdkError> {
    let scopes = [scopes::RUNS_EXECUTE, scopes::RUNS_READ]
        .into_iter()
        .map(ApiScope::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SdkError::InvalidConfiguration("managed API scope is invalid"))?;
    SidecarApplicationGrant::new(
        SELF_TEST_APPLICATION_ID,
        scopes,
        ["primary".to_owned()],
        Vec::new(),
    )
}

const fn access_profile(profile: AccessProfileSetting) -> ManagedAccessProfile {
    match profile {
        AccessProfileSetting::Minimal => ManagedAccessProfile::Minimal,
        AccessProfileSetting::Pinned => ManagedAccessProfile::Pinned,
        AccessProfileSetting::Development => ManagedAccessProfile::Development,
        AccessProfileSetting::AllowAll => ManagedAccessProfile::AllowAll,
    }
}

const fn execution_boundary(boundary: ExecutionBoundarySetting) -> ManagedExecutionBoundary {
    match boundary {
        ExecutionBoundarySetting::FullAccess => ManagedExecutionBoundary::FullAccess,
        ExecutionBoundarySetting::WorkspaceIsolated => ManagedExecutionBoundary::WorkspaceIsolated,
        ExecutionBoundarySetting::OfflineIsolated => ManagedExecutionBoundary::OfflineIsolated,
    }
}

const fn provider_kind(kind: ProviderKindSetting) -> ManagedProviderKind {
    match kind {
        ProviderKindSetting::Responses => ManagedProviderKind::OpenAiResponses,
        ProviderKindSetting::Compatible => ManagedProviderKind::OpenAiCompatible,
        ProviderKindSetting::Codex => ManagedProviderKind::OpenAiCodex,
    }
}

const fn reasoning_effort(effort: ReasoningEffortSetting) -> ManagedReasoningEffort {
    match effort {
        ReasoningEffortSetting::None => ManagedReasoningEffort::None,
        ReasoningEffortSetting::Minimal => ManagedReasoningEffort::Minimal,
        ReasoningEffortSetting::Low => ManagedReasoningEffort::Low,
        ReasoningEffortSetting::Medium => ManagedReasoningEffort::Medium,
        ReasoningEffortSetting::High => ManagedReasoningEffort::High,
        ReasoningEffortSetting::XHigh => ManagedReasoningEffort::XHigh,
        ReasoningEffortSetting::Max => ManagedReasoningEffort::Max,
        ReasoningEffortSetting::Ultra => ManagedReasoningEffort::Ultra,
    }
}

fn classify_sdk(
    error: SdkError,
    fallback: RuntimeFailureCodeDto,
) -> (CommandErrorDto, RuntimeFailureCodeDto) {
    let failure_code = match &error {
        SdkError::IdentityMismatch => RuntimeFailureCodeDto::Integrity,
        SdkError::WorkspaceIdentityChanged => RuntimeFailureCodeDto::Permission,
        SdkError::Authentication => RuntimeFailureCodeDto::Authentication,
        SdkError::Busy => RuntimeFailureCodeDto::WorkspaceBusy,
        SdkError::PlatformEnvironment(_) => RuntimeFailureCodeDto::Configuration,
        SdkError::Transport | SdkError::Unavailable | SdkError::CloseFailed => {
            RuntimeFailureCodeDto::Transport
        }
        SdkError::InvalidConfiguration(_) | SdkError::PathNotAbsolute(_) => {
            RuntimeFailureCodeDto::Configuration
        }
        _ => fallback,
    };
    let projected = match error {
        SdkError::Busy => CommandErrorDto::local_sanitized(
            "workspace_busy",
            "Another Colossus worker already owns this workspace. Leave it running and connect it as an External target.",
            false,
        ),
        SdkError::IdentityMismatch => CommandErrorDto::local_sanitized(
            "runtime_integrity",
            "The bundled Colossus runtime could not be verified. Reinstall a signed desktop build.",
            false,
        ),
        SdkError::WorkspaceIdentityChanged => CommandErrorDto::local_sanitized(
            "workspace_changed",
            "The selected workspace changed. Choose the workspace again.",
            false,
        ),
        SdkError::PlatformEnvironment(_) => CommandErrorDto::local_sanitized(
            "platform_environment",
            platform_environment_message(),
            false,
        ),
        SdkError::Authentication => CommandErrorDto::local_sanitized(
            "runtime_authentication",
            runtime_authentication_message(),
            false,
        ),
        SdkError::LaunchFailed | SdkError::SidecarFailed | SdkError::EmbeddedOpenFailed => {
            CommandErrorDto::local_sanitized(
                "runtime_launch_failed",
                runtime_launch_failed_message(),
                false,
            )
        }
        SdkError::Transport | SdkError::Unavailable => CommandErrorDto::local_sanitized(
            "runtime_transport",
            "Managed Local could not reach its private local API after launch. Restart the runtime and try again.",
            false,
        ),
        other => CommandErrorDto::from_sdk(other),
    };
    (projected, failure_code)
}

fn classify_self_test_sdk(error: SdkError) -> CommandErrorDto {
    match error {
        SdkError::PlatformEnvironment(_) => CommandErrorDto::local_sanitized(
            "platform_environment",
            "The offline self-test could not read a trusted Windows system directory from SystemRoot or WINDIR, so the bundled runtime cannot start.",
            false,
        ),
        SdkError::Authentication => CommandErrorDto::local_sanitized(
            "runtime_authentication",
            "The offline self-test could not create or activate its private local API credentials. Restart desktop development and retry with a private COLOSSUS_HOME.",
            false,
        ),
        SdkError::LaunchFailed | SdkError::SidecarFailed | SdkError::EmbeddedOpenFailed => {
            CommandErrorDto::local_sanitized(
                "runtime_launch_failed",
                offline_self_test_runtime_launch_failed_message(),
                false,
            )
        }
        other => CommandErrorDto::from_sdk(other),
    }
}

fn run_status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Waiting => "waiting",
        RunStatus::Cancelling => "cancelling",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
        RunStatus::OutcomeUnknown => "outcome_unknown",
    }
}

fn platform_environment_message() -> &'static str {
    "Managed Local could not read a trusted Windows system directory from SystemRoot or WINDIR, so the bundled runtime cannot start."
}

fn runtime_authentication_message() -> &'static str {
    "Managed Local could not create or activate its private local API credentials. Restart the runtime and try again."
}

#[cfg(debug_assertions)]
fn runtime_launch_failed_message() -> &'static str {
    "Managed Local could not start the bundled runtime. Run cargo xtask desktop prepare --profile debug, then restart desktop development."
}

#[cfg(not(debug_assertions))]
fn runtime_launch_failed_message() -> &'static str {
    "Managed Local could not start the bundled runtime. Reinstall a signed desktop build and try again."
}

#[cfg(debug_assertions)]
fn offline_self_test_runtime_launch_failed_message() -> &'static str {
    "The offline self-test could not start the bundled runtime. Run cargo xtask desktop prepare --profile debug, then restart desktop development."
}

#[cfg(not(debug_assertions))]
fn offline_self_test_runtime_launch_failed_message() -> &'static str {
    "The offline self-test could not start the bundled runtime. Reinstall a signed desktop build and try again."
}

fn classified(
    code: &str,
    message: &str,
    failure_code: RuntimeFailureCodeDto,
) -> (CommandErrorDto, RuntimeFailureCodeDto) {
    (
        CommandErrorDto::local_sanitized(code, message, false),
        failure_code,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_grant_includes_every_declared_builtin_without_inventing_capabilities() {
        let grant = application_grant(AccessProfileSetting::Development).expect("grant");
        let debug = format!("{grant:?}");
        for required in [
            "agent.delegate",
            "filesystem.read",
            "shell.run",
            "skill.install",
            "mcp.call",
            "web.search",
            "network.http",
            "plan.approve_request",
        ] {
            assert!(debug.contains(required));
        }
        assert!(!debug.contains("worker.admin"));
        assert!(!debug.contains(scopes::APPROVALS_RESPOND));
        assert_eq!(PRIMARY_SCOPES.len(), 6);
        for required in PRIMARY_SCOPES {
            assert!(debug.contains(required));
        }
    }

    #[test]
    fn desktop_approval_broker_is_an_explicit_separate_grant() {
        let broker = approval_broker_grant().expect("approval broker grant");
        let debug = format!("{broker:?}");
        assert!(debug.contains(APPLICATION_ID));
        assert!(debug.contains("primary"));
        assert!(!debug.contains("shell.run"));
    }

    #[test]
    fn managed_search_uses_selected_profiles_and_is_disabled_offline() {
        let mut resolved = ResolvedSpaceConfiguration {
            access_profile: AccessProfileSetting::Development,
            execution_boundary: ExecutionBoundarySetting::WorkspaceIsolated,
            terminal_enabled: false,
            field_overrides: Vec::new(),
            providers: Vec::new(),
            models: Vec::new(),
            model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
            search_providers: vec![crate::managed_configuration::SearchProviderSetting {
                profile: "local-search".into(),
                kind: SearchProviderKindSetting::Searxng,
                endpoint: "http://127.0.0.1:8888/search".into(),
                credential_id: None,
                auth_header: Some("X-Search-Key".into()),
                timeout_ms: 30_000,
            }],
            search_roles: BTreeMap::from([("research".into(), "local-search".into())]),
            mcp_servers: Vec::new(),
            telemetry: None,
        };
        let (profiles, roles) = managed_search(&resolved);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].endpoint, "http://127.0.0.1:8888/search");
        assert_eq!(
            roles.get("research").map(String::as_str),
            Some("local-search")
        );

        resolved.execution_boundary = ExecutionBoundarySetting::OfflineIsolated;
        let (profiles, roles) = managed_search(&resolved);
        assert!(profiles.is_empty());
        assert!(roles.is_empty());
    }

    #[test]
    fn minimal_grant_only_exposes_echo() {
        let grant = application_grant(AccessProfileSetting::Minimal).expect("grant");
        let debug = format!("{grant:?}");
        assert!(debug.contains("echo"));
        assert!(!debug.contains("filesystem.read"));
    }

    #[test]
    fn allow_all_maps_independently_from_the_execution_boundary() {
        assert_eq!(
            access_profile(AccessProfileSetting::Pinned),
            ManagedAccessProfile::Pinned
        );
        assert_eq!(
            access_profile(AccessProfileSetting::AllowAll),
            ManagedAccessProfile::AllowAll
        );
        assert_eq!(
            execution_boundary(ExecutionBoundarySetting::FullAccess),
            ManagedExecutionBoundary::FullAccess
        );
        assert_eq!(
            execution_boundary(ExecutionBoundarySetting::WorkspaceIsolated),
            ManagedExecutionBoundary::WorkspaceIsolated
        );
    }

    #[test]
    fn self_test_grant_cannot_answer_prompts_or_use_tools() {
        let grant = self_test_grant().expect("grant");
        let debug = format!("{grant:?}");
        assert!(debug.contains(scopes::RUNS_EXECUTE));
        assert!(debug.contains(scopes::RUNS_READ));
        assert!(!debug.contains(scopes::PROMPTS_RESPOND));
        assert!(!debug.contains("echo"));
    }

    #[test]
    fn offline_self_test_probe_uses_executable_echo_run() {
        let request = offline_self_test_request(
            IdempotencyKey::new("desktop-self-test-unit").expect("idempotency key"),
        );
        assert_eq!(
            request.input,
            vec![InputContentPart::Text("offline self-test".into())]
        );
        assert_eq!(request.role, "primary");
        assert_eq!(request.mode, RunMode::Execute);
        assert_eq!(request.max_turns, 1);
        assert!(request.plan_action.is_none());
        assert!(request.selected_skills.is_empty());
    }

    #[test]
    fn offline_self_test_instance_id_is_namespaced_to_runtime_directory() {
        let root = tempfile::tempdir().expect("self-test root");
        let first = root.path().join("first").join("runtime");
        let second = root.path().join("second").join("runtime");
        std::fs::create_dir_all(&first).expect("first runtime");
        std::fs::create_dir_all(&second).expect("second runtime");

        let first_id = self_test_instance_id(&first).expect("first instance");
        let first_alias =
            self_test_instance_id(&first.join("..").join("runtime")).expect("first alias instance");
        let second_id = self_test_instance_id(&second).expect("second instance");

        assert_eq!(first_id, first_alias);
        assert_ne!(first_id, second_id);
        let encoded = first_id.to_string();
        assert_eq!(&encoded[14..15], "8");
        assert!(matches!(&encoded[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn workspace_drift_prompts_reselection_without_mislabeling_bundle_integrity() {
        let (error, failure_code) = classify_sdk(
            SdkError::WorkspaceIdentityChanged,
            RuntimeFailureCodeDto::Internal,
        );
        assert_eq!(failure_code, RuntimeFailureCodeDto::Permission);
        assert_eq!(error.code, "workspace_changed");
        assert!(error.message.contains("Choose the workspace again"));
        assert!(!error.message.contains("Reinstall"));

        let (integrity, failure_code) =
            classify_sdk(SdkError::IdentityMismatch, RuntimeFailureCodeDto::Internal);
        assert_eq!(failure_code, RuntimeFailureCodeDto::Integrity);
        assert_eq!(integrity.code, "runtime_integrity");
    }

    #[test]
    fn managed_launch_errors_explain_the_next_development_step() {
        let (launch, _) = classify_sdk(SdkError::SidecarFailed, RuntimeFailureCodeDto::Internal);
        assert_eq!(launch.code, "runtime_launch_failed");
        assert!(launch.message.contains("bundled runtime"));
        #[cfg(debug_assertions)]
        assert!(
            launch
                .message
                .contains("cargo xtask desktop prepare --profile debug")
        );

        let (environment, failure_code) = classify_sdk(
            SdkError::PlatformEnvironment("windows_system_root_unavailable"),
            RuntimeFailureCodeDto::Internal,
        );
        assert_eq!(failure_code, RuntimeFailureCodeDto::Configuration);
        assert_eq!(environment.code, "platform_environment");
        assert!(environment.message.contains("SystemRoot"));
        assert!(environment.message.contains("WINDIR"));

        let (authentication, failure_code) =
            classify_sdk(SdkError::Authentication, RuntimeFailureCodeDto::Internal);
        assert_eq!(failure_code, RuntimeFailureCodeDto::Authentication);
        assert_eq!(authentication.code, "runtime_authentication");
        assert!(
            authentication
                .message
                .contains("private local API credentials")
        );
    }

    #[test]
    fn offline_self_test_errors_are_specific_to_the_probe_path() {
        let launch = classify_self_test_sdk(SdkError::SidecarFailed);
        assert_eq!(launch.code, "runtime_launch_failed");
        assert!(launch.message.contains("offline self-test"));
        #[cfg(debug_assertions)]
        assert!(
            launch
                .message
                .contains("cargo xtask desktop prepare --profile debug")
        );

        let environment = classify_self_test_sdk(SdkError::PlatformEnvironment(
            "windows_system_root_unavailable",
        ));
        assert_eq!(environment.code, "platform_environment");
        assert!(environment.message.contains("SystemRoot"));
        assert!(environment.message.contains("WINDIR"));

        assert_eq!(run_status_name(RunStatus::Failed), "failed");
        assert_eq!(
            run_status_name(RunStatus::OutcomeUnknown),
            "outcome_unknown"
        );
    }

    #[test]
    fn fifth_space_evicts_the_least_recently_used_idle_runtime() {
        let candidates = vec![
            (40, "busy-old".into(), true),
            (30, "idle-newer".into(), false),
            (10, "idle-oldest".into(), false),
            (50, "busy-new".into(), true),
        ];

        assert_eq!(
            idle_lru_candidate(&candidates).expect("capacity plan"),
            Some("idle-oldest".into())
        );
    }

    #[test]
    fn fifth_space_is_refused_when_all_live_spaces_are_busy() {
        let candidates = (0..MAX_LIVE_MANAGED_SPACES)
            .map(|index| (index as u64, format!("space-{index}"), true))
            .collect::<Vec<_>>();

        let error = idle_lru_candidate(&candidates).expect_err("all busy");
        assert_eq!(error.code, "busy");
        assert!(error.message.contains("Four Workspaces"));
    }

    #[test]
    fn active_run_pagination_follows_empty_pages_and_fails_closed_on_replay() {
        let mut seen = BTreeSet::new();
        let page = PageResponse {
            next_page_token: "next-page".into(),
        };
        assert_eq!(
            next_active_run_page_token(Some(&page), &mut seen).expect("next page"),
            Some("next-page".into())
        );
        assert!(next_active_run_page_token(Some(&page), &mut seen).is_err());
        assert_eq!(
            next_active_run_page_token(
                Some(&PageResponse {
                    next_page_token: String::new(),
                }),
                &mut seen,
            )
            .expect("terminal page"),
            None
        );
    }
}
