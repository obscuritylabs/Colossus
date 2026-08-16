use colossus_sdk::{
    ApiMajor, ApiScope, AppPrivateInstanceDir, Colossus, CreateRunRequest, GetRunRequest,
    IdempotencyKey, InputContentPart, InstanceId, ListRunsRequest, ManagedAccessProfile,
    ManagedExecutionBoundary, ManagedModelCapabilities, ManagedModelConfig, ManagedProviderConfig,
    ManagedProviderKind, ManagedReasoningEffort, ManagedRuntimeConfig, ManagedSearchConfig,
    NativeSidecarLifecycle, RunMode, RunStatus, SdkError, Secret, SidecarApplicationGrant,
    SidecarApprovalBrokerGrant, SidecarBootstrapConfig, SidecarHostCredential, SidecarOptions,
    WorkspaceIdentity, scopes,
};
use colossus_worker_protocol::{WorkerControlClient, worker_ipc_endpoint};
use std::{collections::BTreeMap, path::Path, str::FromStr as _};

use crate::{
    bundle::VerifiedBundle,
    desktop_dto::{ManagedRuntimeStateDto, RuntimeFailureCodeDto},
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ExecutionBoundarySetting, ProviderKindSetting,
        ReasoningEffortSetting, SettingsStore, load_provider_secret, revalidate_workspace,
    },
    dto::CommandErrorDto,
    state::{AppState, MAX_LIVE_MANAGED_SPACES, ManagedHealth},
    terminal::{TerminalWorkerAuthentication, TerminalWorkspace},
};

const APPLICATION_ID: &str = "app:colossus-desktop-managed";
const SELF_TEST_APPLICATION_ID: &str = "app:colossus-desktop-self-test";
const SELF_TEST_INSTANCE_ID: &str = "00000000-0000-7000-8000-000000000001";
const LOCAL_DEV_SEARCH_PROFILE: &str = "local-dev-searxng";
const LOCAL_DEV_SEARCH_ENDPOINT: &str = "http://127.0.0.1:8888/search";
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
    let space_id = settings.selected_space_id.as_deref().ok_or_else(|| {
        CommandErrorDto::invalid("spaceId", "Select a Space before starting Managed Local.")
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
        let active = target
            .client
            .list_runs(ListRunsRequest {
                session_id: None,
                statuses: vec![
                    RunStatus::Queued,
                    RunStatus::Running,
                    RunStatus::Waiting,
                    RunStatus::Cancelling,
                ],
                page: Some(colossus_sdk::PageRequest {
                    page_size: 1,
                    page_token: String::new(),
                }),
                include_archived: false,
            })
            .await
            .map_err(CommandErrorDto::from_api)?;
        candidates.push((last_used, target_id, !active.runs.is_empty()));
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
        "Four Spaces are already running active work. Switch to one of them or finish a run before starting another Space.",
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
    let instance_id = InstanceId::from_str(SELF_TEST_INSTANCE_ID).map_err(|_| {
        CommandErrorDto::local_sanitized("internal", "The offline self-test is unavailable.", false)
    })?;
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
        .map_err(CommandErrorDto::from_sdk)?;
    let probe = probe_offline_echo(&client).await;
    let close = client.close().await.map_err(CommandErrorDto::from_sdk);
    probe?;
    close
}

async fn probe_offline_echo(client: &Colossus) -> Result<(), CommandErrorDto> {
    let idempotency_key =
        IdempotencyKey::new(format!("desktop-self-test-{}", uuid::Uuid::now_v7()))
            .map_err(CommandErrorDto::from_api)?;
    let run = client
        .create_run(CreateRunRequest {
            input: vec![InputContentPart::Text("offline self-test".into())],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Plan,
            research_depth: None,
            research_sources: Vec::new(),
            selected_skills: Vec::new(),
            plan_action: None,
            branch: None,
            max_turns: 1,
            idempotency_key,
        })
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
        Err(CommandErrorDto::local_sanitized(
            "self_test_failed",
            "The offline Colossus self-test did not complete successfully.",
            false,
        ))
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
        settings.local_terminal_enabled(),
    )
    .await
}

fn provider_host_credentials(
    settings: &DesktopSettings,
) -> Result<Vec<SidecarHostCredential>, (CommandErrorDto, RuntimeFailureCodeDto)> {
    settings
        .provider_credential_ids()
        .into_iter()
        .map(|credential_id| {
            let credential = load_provider_secret(credential_id)
                .map_err(|error| (error, RuntimeFailureCodeDto::Provider))?;
            let provider_secret = Secret::new(credential.to_vec())
                .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Provider))?;
            SidecarHostCredential::new(credential_id, provider_secret)
                .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Provider))
        })
        .collect()
}

struct PreparedManagedBootstrap {
    bootstrap: SidecarBootstrapConfig,
    worker_authentication: TerminalWorkerAuthentication,
}

fn prepare_managed_bootstrap(
    workspace: &Path,
    workspace_identity: WorkspaceIdentity,
    store: &SettingsStore,
    settings: &DesktopSettings,
) -> Result<PreparedManagedBootstrap, (CommandErrorDto, RuntimeFailureCodeDto)> {
    let host_credentials = provider_host_credentials(settings)?;
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
        settings,
        host_credentials,
        approval_broker_grant,
        worker_bootstrap_secret.as_ref(),
        &paths,
    )
    .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Configuration))?;
    Ok(PreparedManagedBootstrap {
        bootstrap,
        worker_authentication,
    })
}

struct ManagedBootstrapPaths<'a> {
    ca_bundle: Option<&'a Path>,
    codex_auth: Option<&'a Path>,
    colossus_home: &'a Path,
}

fn managed_bootstrap(
    workspace: &Path,
    workspace_identity: WorkspaceIdentity,
    settings: &DesktopSettings,
    host_credentials: Vec<SidecarHostCredential>,
    approval_broker_grant: SidecarApprovalBrokerGrant,
    worker_authentication: &[u8],
    paths: &ManagedBootstrapPaths<'_>,
) -> Result<SidecarBootstrapConfig, SdkError> {
    let (search_profiles, search_roles) = managed_search(settings.execution_boundary);
    let runtime = ManagedRuntimeConfig {
        access_profile: access_profile(settings.access_profile),
        execution_boundary: execution_boundary(settings.execution_boundary),
        providers: settings
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
        models: settings
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
                },
                reasoning_effort: model.reasoning_effort.map(reasoning_effort),
            })
            .collect(),
        roles: settings.model_roles.clone(),
        search_profiles,
        search_roles,
    };
    let bootstrap = SidecarBootstrapConfig::new(
        workspace,
        runtime,
        application_grant(settings.access_profile)?,
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

#[cfg(debug_assertions)]
fn managed_search(
    boundary: ExecutionBoundarySetting,
) -> (Vec<ManagedSearchConfig>, BTreeMap<String, String>) {
    if boundary == ExecutionBoundarySetting::OfflineIsolated {
        return (Vec::new(), BTreeMap::new());
    }
    (
        vec![ManagedSearchConfig {
            profile: LOCAL_DEV_SEARCH_PROFILE.into(),
            endpoint: LOCAL_DEV_SEARCH_ENDPOINT.into(),
            timeout_ms: 30_000,
        }],
        BTreeMap::from([
            ("agent".into(), LOCAL_DEV_SEARCH_PROFILE.into()),
            ("research".into(), LOCAL_DEV_SEARCH_PROFILE.into()),
        ]),
    )
}

#[cfg(not(debug_assertions))]
fn managed_search(
    _boundary: ExecutionBoundarySetting,
) -> (Vec<ManagedSearchConfig>, BTreeMap<String, String>) {
    (Vec::new(), BTreeMap::new())
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
        SdkError::Authentication => CommandErrorDto::local_sanitized(
            "runtime_authentication",
            "Managed Local could not establish its private authenticated connection.",
            false,
        ),
        other => CommandErrorDto::from_sdk(other),
    };
    (projected, failure_code)
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

    #[cfg(debug_assertions)]
    #[test]
    fn debug_managed_search_is_loopback_only_and_disabled_offline() {
        let (profiles, roles) = managed_search(ExecutionBoundarySetting::WorkspaceIsolated);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].endpoint, LOCAL_DEV_SEARCH_ENDPOINT);
        assert_eq!(
            roles.get("research").map(String::as_str),
            Some(LOCAL_DEV_SEARCH_PROFILE)
        );

        let (profiles, roles) = managed_search(ExecutionBoundarySetting::OfflineIsolated);
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
        assert!(error.message.contains("Four Spaces"));
    }
}
