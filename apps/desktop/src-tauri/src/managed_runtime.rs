use colossus_sdk::{
    ApiMajor, ApiScope, AppPrivateInstanceDir, Colossus, CreateRunRequest, GetRunRequest,
    IdempotencyKey, InputContentPart, InstanceId, ManagedAccessProfile, ManagedModelCapabilities,
    ManagedModelConfig, ManagedProviderConfig, ManagedProviderKind, ManagedRuntimeConfig,
    NativeSidecarLifecycle, RunMode, RunStatus, SdkError, Secret, SidecarApplicationGrant,
    SidecarApprovalBrokerGrant, SidecarBootstrapConfig, SidecarHostCredential, SidecarOptions,
    WorkspaceIdentity, scopes,
};
use std::{path::Path, str::FromStr as _};

use crate::{
    bundle::VerifiedBundle,
    desktop_dto::{ManagedRuntimeStateDto, RuntimeFailureCodeDto},
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ProviderKindSetting, SettingsStore,
        load_provider_secret, revalidate_workspace,
    },
    dto::CommandErrorDto,
    state::{AppState, MANAGED_TARGET_ID, ManagedHealth},
    terminal::{TerminalWorkerAuthentication, TerminalWorkspace},
};

const APPLICATION_ID: &str = "app:colossus-desktop-managed";
const SELF_TEST_APPLICATION_ID: &str = "app:colossus-desktop-self-test";
const SELF_TEST_INSTANCE_ID: &str = "00000000-0000-7000-8000-000000000001";
const PRIMARY_SCOPES: [&str; 6] = [
    scopes::RUNS_EXECUTE,
    scopes::RUNS_READ,
    scopes::RUNS_CONTROL,
    scopes::PROMPTS_RESPOND,
    scopes::ARTIFACTS_READ,
    scopes::ARTIFACTS_WRITE,
];

const DEVELOPMENT_TOOL_GRANT: &[&str] = &[
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
    "patch.apply",
    "patch.preview",
    "patch.reverse",
    "plan.create",
    "plan.show",
    "repo.file_summary",
    "repo.map",
    "repo.references",
    "repo.symbol_search",
    "shell.run",
    "task.create",
    "task.list",
    "task.update",
    "tool.search",
    "trace.export",
    "trace.show",
    "user.ask",
];

pub(crate) async fn start(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    restarting: bool,
) -> Result<(), CommandErrorDto> {
    let lifecycle_generation = state.begin_managed_lifecycle();
    let restore_selection = state.selected_target_id().await.as_deref() == Some(MANAGED_TARGET_ID);
    if restore_selection {
        // The selection writer waits for native run operations to release their
        // read leases before the managed transport is closed. No request is replayed.
        state.select_target(None).await;
    }
    let result =
        start_after_operation_drain(state, store, settings, restarting, lifecycle_generation).await;
    if restore_selection {
        state
            .select_target(Some(MANAGED_TARGET_ID.to_owned()))
            .await;
    }
    result
}

async fn start_after_operation_drain(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    restarting: bool,
    lifecycle_generation: u64,
) -> Result<(), CommandErrorDto> {
    let starting_state = if restarting {
        ManagedRuntimeStateDto::Restarting
    } else {
        ManagedRuntimeStateDto::Starting
    };
    state
        .set_managed_health(ManagedHealth {
            state: starting_state,
            message: if restarting {
                "Restarting the managed Colossus runtime…".into()
            } else {
                "Starting the managed Colossus runtime…".into()
            },
            failure_code: None,
        })
        .await;

    state.clear_terminal_workspace().await;
    if let Some(previous) = state.remove_target(MANAGED_TARGET_ID).await
        && let Err(error) = previous.client.close().await
    {
        let (error, failure_code) = classify_sdk(error, RuntimeFailureCodeDto::Transport);
        state
            .set_managed_health(ManagedHealth {
                state: ManagedRuntimeStateDto::Failed,
                message: error.message.clone(),
                failure_code: Some(failure_code),
            })
            .await;
        return Err(error);
    }

    let result = start_inner(state, store, settings, lifecycle_generation).await;
    match result {
        Ok(()) => {
            state.sync_managed_lifecycle_health().await;
            Ok(())
        }
        Err((error, failure_code)) => {
            state.clear_managed_lifecycle(lifecycle_generation);
            state.clear_terminal_workspace().await;
            state
                .set_managed_health(ManagedHealth {
                    state: ManagedRuntimeStateDto::Failed,
                    message: error.message.clone(),
                    failure_code: Some(failure_code),
                })
                .await;
            Err(error)
        }
    }
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
    let runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal);
    let bootstrap = SidecarBootstrapConfig::new(
        canonical_workspace,
        runtime,
        self_test_grant().map_err(CommandErrorDto::from_sdk)?,
    )
    .and_then(|bootstrap| bootstrap.with_expected_workspace_identity(workspace_identity))
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
            role: "primary".into(),
            mode: RunMode::Plan,
            selected_skills: Vec::new(),
            plan_action: None,
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
    let host_credentials = provider_host_credentials(settings)?;
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
    let bootstrap = managed_bootstrap(
        &canonical_workspace,
        workspace_identity.clone(),
        settings,
        host_credentials,
        approval_broker_grant,
        worker_bootstrap_secret.as_ref(),
        ca_bundle_path.as_deref(),
    )
    .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Configuration))?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    install_managed_target(
        state,
        lifecycle_generation,
        lifecycle,
        options,
        TerminalWorkspace {
            id: workspace.id.clone(),
            display_name: workspace.display_name.clone(),
            workspace: canonical_workspace,
            workspace_identity,
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

fn managed_bootstrap(
    workspace: &Path,
    workspace_identity: WorkspaceIdentity,
    settings: &DesktopSettings,
    host_credentials: Vec<SidecarHostCredential>,
    approval_broker_grant: SidecarApprovalBrokerGrant,
    worker_authentication: &[u8],
    ca_bundle_path: Option<&Path>,
) -> Result<SidecarBootstrapConfig, SdkError> {
    let runtime = ManagedRuntimeConfig {
        access_profile: access_profile(settings.access_profile),
        providers: settings
            .providers
            .iter()
            .map(|provider| ManagedProviderConfig {
                profile: provider.profile.clone(),
                kind: provider_kind(provider.kind),
                base_url: Some(provider.base_url.clone()),
                credential_id: provider.credential_id.clone(),
                timeout_ms: provider.timeout_ms,
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
            })
            .collect(),
        roles: settings.model_roles.clone(),
    };
    let bootstrap = SidecarBootstrapConfig::new(
        workspace,
        runtime,
        application_grant(settings.access_profile)?,
    )?
    .with_expected_workspace_identity(workspace_identity)?
    .with_approval_broker_grant(approval_broker_grant)?
    .with_host_credentials(host_credentials)?
    .with_worker_ipc_authentication(Secret::new(worker_authentication.to_vec())?)?;
    if let Some(path) = ca_bundle_path {
        bootstrap.with_additional_ca_bundle_path(path)
    } else {
        Ok(bootstrap)
    }
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
    lifecycle_generation: u64,
    lifecycle: NativeSidecarLifecycle,
    options: SidecarOptions,
    mut terminal_workspace: TerminalWorkspace,
    terminal_enabled: bool,
) -> Result<(), (CommandErrorDto, RuntimeFailureCodeDto)> {
    state.observe_managed_lifecycle(lifecycle_generation, lifecycle.subscribe_status());

    terminal_workspace.config = Some(options.managed_config_path());
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .map_err(|error| classify_sdk(error, RuntimeFailureCodeDto::Internal))?;
    let previous = state
        .replace_target(
            MANAGED_TARGET_ID,
            client,
            crate::state::TargetConsentContext::ManagedLocal,
        )
        .await;
    debug_assert!(previous.is_none());
    state.configure_terminal_workspace(terminal_workspace).await;
    state.set_terminal_enabled(terminal_enabled).await;
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
        DEVELOPMENT_TOOL_GRANT
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
        AccessProfileSetting::Development | AccessProfileSetting::LegacyAllowAll => {
            ManagedAccessProfile::Development
        }
    }
}

const fn provider_kind(kind: ProviderKindSetting) -> ManagedProviderKind {
    match kind {
        ProviderKindSetting::OpenAiResponses => ManagedProviderKind::OpenAiResponses,
        ProviderKindSetting::OpenAiCompatible => ManagedProviderKind::OpenAiCompatible,
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
    fn desktop_grant_includes_bounded_delegation_and_excludes_worker_admin() {
        let grant = application_grant(AccessProfileSetting::Development).expect("grant");
        let debug = format!("{grant:?}");
        for denied in ["skill.install", "mcp.call"] {
            assert!(!debug.contains(denied));
        }
        assert!(debug.contains("agent.delegate"));
        assert!(debug.contains("filesystem.read"));
        assert!(debug.contains("shell.run"));
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
    fn minimal_grant_only_exposes_echo() {
        let grant = application_grant(AccessProfileSetting::Minimal).expect("grant");
        let debug = format!("{grant:?}");
        assert!(debug.contains("echo"));
        assert!(!debug.contains("filesystem.read"));
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
}
