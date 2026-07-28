use colossus_sdk::{ApiErrorCode, ListRunsRequest, PageRequest, RunStatus, ServerCapabilities};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use uuid::Uuid;

use crate::{
    connection,
    desktop_dto::{
        ApplyManagedModelConfigurationInput, ConfigureManagedRuntimeInput, CredentialActionInput,
        DesktopCapabilitiesDto, DesktopReleaseChannelDto, DesktopStatusDto,
        ManagedModelConfigurationDto, ManagedRuntimeStateDto, ProviderSummaryDto, RuntimeTargetDto,
        RuntimeTargetKindDto, WorkspaceSummaryDto,
    },
    desktop_settings::{
        DesktopSettings, ExternalTargetSetting, MAX_EXTERNAL_TARGETS,
        MAX_PENDING_PROVIDER_CLEANUPS, ModelCapabilitiesSetting, ModelSetting, ProviderSetting,
        SettingsStore, WorkspaceSetting, application_support_root, delete_provider_secret,
        load_provider_secret, provider_base_url, revalidate_workspace, store_provider_secret,
        validate_workspace,
    },
    dto::{CommandErrorDto, ConnectionStateDto, ConnectionStatusDto},
    managed_runtime, provider_enrollment,
    state::{
        AppState, ExternalHealth, MANAGED_TARGET_ID, ManagedHealth, TargetConsentContext,
        TargetHandle,
    },
};

const EXTERNAL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[tauri::command]
pub(crate) fn desktop_release_channel() -> DesktopReleaseChannelDto {
    DesktopReleaseChannelDto::current()
}

#[tauri::command]
pub(crate) async fn initialize_desktop(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let had_settings = store.has_persisted_settings();
    let mut settings = store.load()?;
    cleanup_pending_provider_credentials(&store, &mut settings)?;
    if migrate_legacy_connection(&mut settings) {
        store.save(&settings)?;
    }
    state.set_terminal_enabled(settings.terminal_enabled).await;
    set_unstarted_health(&state, &settings).await;

    let selected = settings
        .selected_target_id
        .as_deref()
        .filter(|target| target_exists(&settings, target))
        .map(str::to_owned)
        .or_else(|| has_managed_configuration(&settings).then(|| MANAGED_TARGET_ID.to_owned()))
        .or_else(|| {
            (!had_settings)
                .then(|| settings.external_targets.first())
                .flatten()
                .map(|target| target.target_id.clone())
        })
        .or_else(|| {
            settings
                .external_targets
                .first()
                .map(|target| target.target_id.clone())
        });
    if settings.selected_target_id != selected {
        settings.selected_target_id.clone_from(&selected);
        store.save(&settings)?;
    }
    state.select_target(selected.clone()).await;

    if should_start_managed_on_initialize(
        has_managed_configuration(&settings),
        state.connected(MANAGED_TARGET_ID).await,
    ) {
        let _ = managed_runtime::start(&state, &store, &settings, false).await;
    }
    if let Some(target) = selected
        .as_deref()
        .and_then(|target_id| external_target(&settings, target_id))
        && !external_target_ready(&state, &target.target_id).await
    {
        let _ = connect_external(&state, target).await;
    }
    let status = desktop_status_from(&state, &settings).await?;
    spawn_external_health_probes(&app, settings.external_targets).await;
    Ok(status)
}

#[tauri::command]
pub(crate) async fn desktop_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    let status = desktop_status_from(&state, &settings).await?;
    spawn_external_health_probes(&app, settings.external_targets).await;
    Ok(status)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn connect_colossus(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: Option<String>,
) -> Result<ConnectionStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    cleanup_pending_provider_credentials(&store, &mut settings)?;
    let selected = state.selected_target_id().await;
    let target_id = target_id
        .or(selected)
        .or_else(|| {
            settings
                .external_targets
                .first()
                .map(|target| target.target_id.clone())
        })
        .ok_or_else(CommandErrorDto::not_configured)?;
    validate_target(&settings, &target_id)?;
    if target_id == MANAGED_TARGET_ID {
        if !state.connected(MANAGED_TARGET_ID).await {
            managed_runtime::start(&state, &store, &settings, false).await?;
        }
    } else {
        let target =
            external_target(&settings, &target_id).ok_or_else(CommandErrorDto::not_configured)?;
        if !confirm_external_target(&app, target, ExternalConsentAction::Connect).await? {
            return Ok(desktop_status_from(&state, &settings).await?.connection);
        }
        if !external_target_ready(&state, &target_id).await {
            connect_external(&state, target).await?;
        }
    }
    settings.selected_target_id = Some(target_id.clone());
    store.save(&settings)?;
    state.select_target(Some(target_id.clone())).await;
    Ok(ConnectionStatusDto::connected(target_id))
}

#[tauri::command]
pub(crate) async fn connection_status(
    state: State<'_, AppState>,
) -> Result<ConnectionStatusDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    Ok(desktop_status_from(&state, &settings).await?.connection)
}

#[tauri::command]
pub(crate) async fn import_ca_bundle(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<DesktopStatusDto>, CommandErrorDto> {
    let selected = app
        .dialog()
        .file()
        .add_filter("PEM CA certificate bundle", &["pem", "crt", "cer"])
        .blocking_pick_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| {
        CommandErrorDto::invalid(
            "caBundle",
            "The selected CA certificate bundle is unavailable.",
        )
    })?;
    let _guard = connect_guard(&state)?;
    reject_active_managed_runs(&state).await?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous = settings.additional_ca_bundle.clone();
    let staged = store.stage_ca_bundle(&path)?;
    settings.additional_ca_bundle = Some(staged.clone());
    if let Err(error) = store.save(&settings) {
        let _ = store.delete_ca_bundle(&staged);
        return Err(error);
    }
    if has_managed_configuration(&settings)
        && let Err(start_error) = managed_runtime::start(&state, &store, &settings, true).await
    {
        settings.additional_ca_bundle = previous.clone();
        store.save(&settings)?;
        let _ = store.delete_ca_bundle(&staged);
        restore_managed_after_rollback(&state, &store, &settings).await?;
        return Err(start_error);
    }
    if let Some(previous) = previous {
        store.delete_ca_bundle(&previous)?;
    }
    desktop_status_from(&state, &settings).await.map(Some)
}

#[tauri::command]
pub(crate) async fn remove_ca_bundle(
    state: State<'_, AppState>,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    reject_active_managed_runs(&state).await?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let Some(previous) = settings.additional_ca_bundle.take() else {
        return desktop_status_from(&state, &settings).await;
    };
    store.save(&settings)?;
    if has_managed_configuration(&settings)
        && let Err(start_error) = managed_runtime::start(&state, &store, &settings, true).await
    {
        settings.additional_ca_bundle = Some(previous);
        store.save(&settings)?;
        restore_managed_after_rollback(&state, &store, &settings).await?;
        return Err(start_error);
    }
    store.delete_ca_bundle(&previous)?;
    desktop_status_from(&state, &settings).await
}

#[tauri::command]
pub(crate) async fn add_external_target(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<DesktopStatusDto>, CommandErrorDto> {
    let selected = app
        .dialog()
        .file()
        .add_filter("Colossus connection", &["json"])
        .blocking_pick_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| {
        CommandErrorDto::invalid(
            "connectionFile",
            "The selected connection file is unavailable.",
        )
    })?;
    let mut imported = connection::import_target(&path)?;
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let existing_target_index = settings
        .external_targets
        .iter()
        .position(|target| connection::same_connection(target, &imported));
    if existing_target_index.is_none() && settings.external_targets.len() >= MAX_EXTERNAL_TARGETS {
        return Err(CommandErrorDto::invalid(
            "connectionFile",
            "Remove a saved daemon before adding another one.",
        ));
    }
    if !confirm_external_target(&app, &imported, ExternalConsentAction::Import).await? {
        return Ok(None);
    }
    let target_id = if let Some(index) = existing_target_index {
        let target_id = settings.external_targets[index].target_id.clone();
        imported.target_id.clone_from(&target_id);
        settings.external_targets[index] = imported;
        target_id
    } else {
        let target_id = imported.target_id.clone();
        settings.external_targets.push(imported);
        target_id
    };
    settings.selected_target_id = Some(target_id.clone());
    store.save(&settings)?;
    state.select_target(Some(target_id.clone())).await;
    let target =
        external_target(&settings, &target_id).ok_or_else(CommandErrorDto::not_configured)?;
    connect_external(&state, target).await?;
    desktop_status_from(&state, &settings).await.map(Some)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn remove_external_target(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let Some(target) = settings
        .external_targets
        .iter()
        .find(|target| target.target_id == target_id)
    else {
        return Err(CommandErrorDto::invalid(
            "targetId",
            "The external runtime target is unknown.",
        ));
    };
    if !confirm_external_target(&app, target, ExternalConsentAction::Remove).await? {
        return desktop_status_from(&state, &settings).await;
    }
    settings
        .external_targets
        .retain(|target| target.target_id != target_id);
    if settings.selected_target_id.as_deref() == Some(target_id.as_str()) {
        settings.selected_target_id = if settings.managed_configured() {
            Some(MANAGED_TARGET_ID.to_owned())
        } else {
            None
        };
    }
    store.save(&settings)?;
    state
        .select_target(settings.selected_target_id.clone())
        .await;
    state.clear_external_health(&target_id).await;
    if let Some(previous) = state.remove_target(&target_id).await {
        let _ = previous.client.close().await;
    }
    desktop_status_from(&state, &settings).await
}

#[tauri::command]
pub(crate) async fn choose_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<WorkspaceSummaryDto>, CommandErrorDto> {
    let selected = app.dialog().file().blocking_pick_folder();
    let Some(path) = selected else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| {
        CommandErrorDto::invalid("workspace", "The selected folder is unavailable.")
    })?;
    let workspace = validate_workspace(&path)?;
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous_settings =
        persist_workspace_change(&mut settings, workspace.clone(), |settings| {
            store.save(settings)
        })?;
    if settings.managed_configured() {
        let selected = settings
            .selected_target_id
            .clone()
            .expect("workspace persistence selects Managed Local when a provider exists");
        state.select_target(Some(selected)).await;
        if let Err(start_error) = managed_runtime::start(&state, &store, &settings, true).await {
            // Restore the durable workspace before restoring renderer-visible target state.
            // If persistence itself fails, keep the new selection visible rather than
            // claiming that the prior runtime is active.
            rollback_workspace_change(&mut settings, previous_settings, |settings| {
                store.save(settings)
            })?;
            restore_managed_after_rollback(&state, &store, &settings).await?;
            return Err(start_error);
        }
    } else {
        state.clear_terminal_workspace().await;
        state
            .set_managed_health(ManagedHealth {
                state: ManagedRuntimeStateDto::NeedsProvider,
                message: "Configure a provider to start Managed Local.".into(),
                failure_code: None,
            })
            .await;
    }
    Ok(Some(WorkspaceSummaryDto::from(&workspace)))
}

fn persist_workspace_change(
    settings: &mut DesktopSettings,
    workspace: WorkspaceSetting,
    save_settings: impl FnOnce(&DesktopSettings) -> Result<(), CommandErrorDto>,
) -> Result<DesktopSettings, CommandErrorDto> {
    let previous = settings.clone();
    settings.workspace = Some(workspace);
    if settings.managed_configured() && settings.selected_target_id.is_none() {
        settings.selected_target_id = Some(MANAGED_TARGET_ID.to_owned());
    }
    if let Err(error) = save_settings(settings) {
        *settings = previous;
        return Err(error);
    }
    Ok(previous)
}

fn rollback_workspace_change(
    settings: &mut DesktopSettings,
    previous: DesktopSettings,
    save_settings: impl FnOnce(&DesktopSettings) -> Result<(), CommandErrorDto>,
) -> Result<(), CommandErrorDto> {
    save_settings(&previous)?;
    *settings = previous;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn configure_managed_runtime(
    app: AppHandle,
    state: State<'_, AppState>,
    mut request: ConfigureManagedRuntimeInput,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    request.validate()?;
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    cleanup_pending_provider_credentials(&store, &mut settings)?;
    let workspace = settings.workspace.as_ref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "workspaceId",
            "Choose a workspace before configuring a provider.",
        )
    })?;
    reject_active_managed_runs(&state).await?;
    if workspace.id != request.workspace_id {
        return Err(CommandErrorDto::invalid(
            "workspaceId",
            "The workspace selection changed. Review it and retry.",
        ));
    }
    revalidate_workspace(workspace)?;
    if development_access_elevation(&settings, &request)
        && !confirm_development_access(&app).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "access_profile_confirmation",
            "Development access was not enabled.",
            false,
        ));
    }
    if reusable_provider_credential(&settings, &request) {
        // Verify native keychain access before mutating settings or stopping the
        // working runtime. The value is dropped from zeroizing memory immediately;
        // start_inner resolves it again only after the new configuration is durable.
        verify_reused_provider_credential(&settings, load_provider_secret)?;
        let previous_settings =
            persist_reused_provider_configuration(&mut settings, &mut request, |settings| {
                store.save(settings)
            })?;
        state
            .select_target(Some(MANAGED_TARGET_ID.to_owned()))
            .await;
        if let Err(start_error) = managed_runtime::start(&state, &store, &settings, true).await {
            rollback_workspace_change(&mut settings, previous_settings, |settings| {
                store.save(settings)
            })?;
            restore_managed_after_rollback(&state, &store, &settings).await?;
            return Err(start_error);
        }
        return desktop_status_from(&state, &settings).await;
    }
    let previous_settings = settings.clone();
    let secret = provider_enrollment::request_provider_secret().await?;
    let rotation = persist_provider_rotation(
        &mut settings,
        &mut request,
        &secret,
        store_provider_secret,
        |settings| store.save(settings),
    )?;
    state
        .select_target(Some(MANAGED_TARGET_ID.to_owned()))
        .await;
    if let Err(start_error) = managed_runtime::start(&state, &store, &settings, true).await {
        let rollback = rollback_provider_rotation(
            &mut settings,
            previous_settings,
            &rotation,
            |settings| store.save(settings),
            delete_provider_secret,
        )?;
        restore_managed_after_rollback(&state, &store, &settings).await?;
        if let Some(cleanup_error) = rollback.cleanup_error {
            return Err(cleanup_error);
        }
        return Err(start_error);
    }
    if let Some(previous_credential_id) = rotation.previous_credential_id {
        retire_pending_provider_credential(&store, &mut settings, &previous_credential_id)?;
    }
    desktop_status_from(&state, &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn apply_managed_model_configuration(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ApplyManagedModelConfigurationInput,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    request.validate()?;
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    cleanup_pending_provider_credentials(&store, &mut settings)?;
    confirm_managed_model_configuration(&app, &state, &settings, &request).await?;

    let previous_settings = settings.clone();
    let credentials = plan_provider_credentials(&settings, &request)?;
    stage_provider_credentials(
        &store,
        &mut settings,
        &previous_settings,
        &credentials.fresh_ids,
    )
    .await?;
    settings.providers = request.providers_with_credentials(&credentials.by_profile);
    settings.models = request.model_settings();
    settings.model_roles = request.roles.clone();
    settings.access_profile = request.access_profile;
    settings.selected_target_id = Some(MANAGED_TARGET_ID.to_owned());
    settings
        .pending_provider_cleanup_ids
        .retain(|credential_id| !credentials.fresh_ids.contains(credential_id));
    for credential_id in &credentials.retired_ids {
        if !settings
            .pending_provider_cleanup_ids
            .contains(credential_id)
        {
            settings
                .pending_provider_cleanup_ids
                .push(credential_id.clone());
        }
    }
    if let Err(error) = store.save(&settings) {
        rollback_staged_provider_credentials(
            &store,
            &mut settings,
            previous_settings,
            &credentials.fresh_ids,
        )?;
        return Err(error);
    }
    restart_after_model_configuration(
        &state,
        &store,
        &mut settings,
        previous_settings,
        &credentials,
    )
    .await
}

async fn confirm_managed_model_configuration(
    app: &AppHandle,
    state: &AppState,
    settings: &DesktopSettings,
    request: &ApplyManagedModelConfigurationInput,
) -> Result<(), CommandErrorDto> {
    let workspace = settings.workspace.as_ref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "workspaceId",
            "Choose a workspace before configuring model providers.",
        )
    })?;
    if workspace.id != request.workspace_id {
        return Err(CommandErrorDto::invalid(
            "workspaceId",
            "The workspace selection changed. Review it and retry.",
        ));
    }
    revalidate_workspace(workspace)?;
    reject_active_managed_runs(state).await?;

    if request.access_profile == crate::desktop_settings::AccessProfileSetting::Development
        && (!settings.managed_configured()
            || settings.access_profile
                != crate::desktop_settings::AccessProfileSetting::Development)
        && !confirm_development_access(app).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "access_profile_confirmation",
            "Development access was not enabled.",
            false,
        ));
    }
    let changed_origins = request
        .providers
        .iter()
        .filter(|provider| {
            settings
                .providers
                .iter()
                .find(|current| current.profile == provider.profile)
                .is_none_or(|current| current.base_url != provider.base_url)
        })
        .map(|provider| format!("{}: {}", provider.profile, provider.base_url))
        .collect::<Vec<_>>();
    if !changed_origins.is_empty() && !confirm_provider_origins(app, &changed_origins).await? {
        return Err(CommandErrorDto::local_sanitized(
            "provider_origin_confirmation",
            "The model provider origin change was not approved.",
            false,
        ));
    }
    Ok(())
}

struct ProviderCredentialPlan {
    by_profile: BTreeMap<String, Option<String>>,
    fresh_ids: Vec<String>,
    retired_ids: Vec<String>,
}

fn plan_provider_credentials(
    settings: &DesktopSettings,
    request: &ApplyManagedModelConfigurationInput,
) -> Result<ProviderCredentialPlan, CommandErrorDto> {
    let old_credentials = settings
        .providers
        .iter()
        .map(|provider| (provider.profile.clone(), provider.credential_id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut by_profile = BTreeMap::new();
    let mut fresh_ids = Vec::new();
    for provider in &request.providers {
        let credential_id = match provider.credential_action {
            CredentialActionInput::None => None,
            CredentialActionInput::Reuse => {
                let credential_id = old_credentials
                    .get(&provider.profile)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| {
                        CommandErrorDto::invalid(
                            "credentialAction",
                            "A provider without a stored credential cannot reuse one.",
                        )
                    })?;
                drop(load_provider_secret(&credential_id)?);
                Some(credential_id)
            }
            CredentialActionInput::Replace => {
                let credential_id = Uuid::now_v7().to_string();
                fresh_ids.push(credential_id.clone());
                Some(credential_id)
            }
        };
        by_profile.insert(provider.profile.clone(), credential_id);
    }
    let referenced_credentials = by_profile
        .values()
        .filter_map(Option::as_deref)
        .collect::<BTreeSet<_>>();
    let retired_ids = settings
        .provider_credential_ids()
        .into_iter()
        .filter(|credential_id| !referenced_credentials.contains(credential_id))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if settings
        .pending_provider_cleanup_ids
        .len()
        .saturating_add(fresh_ids.len())
        .saturating_add(retired_ids.len())
        > MAX_PENDING_PROVIDER_CLEANUPS
    {
        return Err(CommandErrorDto::busy(
            "Pending provider credential cleanup must finish before applying this configuration.",
        ));
    }
    Ok(ProviderCredentialPlan {
        by_profile,
        fresh_ids,
        retired_ids,
    })
}

async fn stage_provider_credentials(
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    previous_settings: &DesktopSettings,
    fresh_ids: &[String],
) -> Result<(), CommandErrorDto> {
    // Persist cleanup intent before creating any native keychain entries. A crash
    // during enrollment can therefore be repaired on the next Desktop startup.
    settings
        .pending_provider_cleanup_ids
        .extend(fresh_ids.iter().cloned());
    store.save(settings)?;
    for credential_id in fresh_ids {
        let enrollment = provider_enrollment::request_provider_secret().await;
        let result = enrollment.and_then(|secret| store_provider_secret(credential_id, &secret));
        if let Err(error) = result {
            rollback_staged_provider_credentials(
                store,
                settings,
                previous_settings.clone(),
                fresh_ids,
            )?;
            return Err(error);
        }
    }
    Ok(())
}

async fn restart_after_model_configuration(
    state: &AppState,
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    previous_settings: DesktopSettings,
    credentials: &ProviderCredentialPlan,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    state
        .select_target(Some(MANAGED_TARGET_ID.to_owned()))
        .await;
    if let Err(start_error) = managed_runtime::start(state, store, settings, true).await {
        rollback_staged_provider_credentials(
            store,
            settings,
            previous_settings,
            &credentials.fresh_ids,
        )?;
        restore_managed_after_rollback(state, store, settings).await?;
        return Err(start_error);
    }
    for credential_id in &credentials.retired_ids {
        retire_pending_provider_credential(store, settings, credential_id)?;
    }
    desktop_status_from(state, settings).await
}

fn rollback_staged_provider_credentials(
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    mut previous: DesktopSettings,
    fresh_ids: &[String],
) -> Result<(), CommandErrorDto> {
    previous
        .pending_provider_cleanup_ids
        .extend(fresh_ids.iter().cloned());
    previous.pending_provider_cleanup_ids.sort();
    previous.pending_provider_cleanup_ids.dedup();
    store.save(&previous)?;
    *settings = previous;
    for credential_id in fresh_ids {
        retire_pending_provider_credential(store, settings, credential_id)?;
    }
    Ok(())
}

async fn reject_active_managed_runs(state: &AppState) -> Result<(), CommandErrorDto> {
    let Some(target) = state.target(MANAGED_TARGET_ID).await else {
        return Ok(());
    };
    let runs = target
        .client
        .list_runs(ListRunsRequest {
            session_id: None,
            statuses: vec![
                RunStatus::Queued,
                RunStatus::Running,
                RunStatus::Waiting,
                RunStatus::Cancelling,
            ],
            page: Some(PageRequest {
                page_size: 1,
                page_token: String::new(),
            }),
        })
        .await
        .map_err(CommandErrorDto::from_api)?;
    if runs.runs.is_empty() {
        Ok(())
    } else {
        Err(CommandErrorDto::busy(
            "Finish or cancel active Managed Local runs before changing model configuration.",
        ))
    }
}

async fn confirm_provider_origins(
    app: &AppHandle,
    origins: &[String],
) -> Result<bool, CommandErrorDto> {
    let message = format!(
        "Managed Local will send model requests to these provider endpoints:\n\n{}\n\nApprove these native network destinations?",
        origins.join("\n")
    );
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("Approve model provider origins")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Approve origins".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "provider_origin_confirmation",
            "The native provider-origin confirmation could not be opened.",
            true,
        )
    })
}

fn reusable_provider_credential(
    settings: &DesktopSettings,
    request: &ConfigureManagedRuntimeInput,
) -> bool {
    !request.replace_credential
        && settings
            .primary_provider()
            .is_some_and(|provider| provider.kind == request.provider_kind)
}

fn development_access_elevation(
    settings: &DesktopSettings,
    request: &ConfigureManagedRuntimeInput,
) -> bool {
    request.access_profile == crate::desktop_settings::AccessProfileSetting::Development
        && (!settings.managed_configured()
            || settings.access_profile
                != crate::desktop_settings::AccessProfileSetting::Development)
}

fn verify_reused_provider_credential(
    settings: &DesktopSettings,
    load: impl FnOnce(&str) -> Result<zeroize::Zeroizing<Vec<u8>>, CommandErrorDto>,
) -> Result<(), CommandErrorDto> {
    let credential_id = settings
        .primary_provider()
        .and_then(|provider| provider.credential_id.as_deref())
        .ok_or_else(CommandErrorDto::not_configured)?;
    drop(load(credential_id)?);
    Ok(())
}

async fn confirm_development_access(app: &AppHandle) -> Result<bool, CommandErrorDto> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(
                "Development access lets Colossus read and change files in the folder you selected and request sandboxed shell effects. Policy, audit, and per-effect approval still apply.\n\nEnable Development access for Managed Local?",
            )
            .title("Enable Colossus Development access")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Enable Development".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "access_profile_confirmation",
            "The native access-profile confirmation could not be opened.",
            true,
        )
    })
}

fn persist_reused_provider_configuration(
    settings: &mut DesktopSettings,
    request: &mut ConfigureManagedRuntimeInput,
    save_settings: impl FnOnce(&DesktopSettings) -> Result<(), CommandErrorDto>,
) -> Result<DesktopSettings, CommandErrorDto> {
    let previous = settings.clone();
    let provider_profile = settings
        .primary_provider()
        .filter(|provider| provider.kind == request.provider_kind)
        .map(|provider| provider.profile.clone())
        .ok_or_else(CommandErrorDto::not_configured)?;
    let model_profile = settings
        .primary_model()
        .map(|model| model.profile.clone())
        .ok_or_else(CommandErrorDto::not_configured)?;
    let provider = settings
        .providers
        .iter_mut()
        .find(|provider| provider.profile == provider_profile)
        .ok_or_else(CommandErrorDto::not_configured)?;
    if provider.kind != request.provider_kind {
        return Err(CommandErrorDto::not_configured());
    }
    provider_base_url(provider.kind).clone_into(&mut provider.base_url);
    settings
        .models
        .iter_mut()
        .find(|model| model.profile == model_profile)
        .ok_or_else(CommandErrorDto::not_configured)?
        .model = std::mem::take(&mut request.model);
    settings.access_profile = request.access_profile;
    settings.selected_target_id = Some(MANAGED_TARGET_ID.to_owned());
    if let Err(error) = save_settings(settings) {
        *settings = previous;
        return Err(error);
    }
    Ok(previous)
}

struct ProviderCredentialRotation {
    previous_credential_id: Option<String>,
    fresh_credential_id: String,
}

struct ProviderRollbackResult {
    cleanup_error: Option<CommandErrorDto>,
}

fn persist_provider_rotation(
    settings: &mut DesktopSettings,
    request: &mut ConfigureManagedRuntimeInput,
    secret: &zeroize::Zeroizing<String>,
    store_secret: impl FnOnce(&str, &zeroize::Zeroizing<String>) -> Result<(), CommandErrorDto>,
    mut save_settings: impl FnMut(&DesktopSettings) -> Result<(), CommandErrorDto>,
) -> Result<ProviderCredentialRotation, CommandErrorDto> {
    if settings.pending_provider_cleanup_ids.len() >= MAX_PENDING_PROVIDER_CLEANUPS {
        return Err(CommandErrorDto::busy(
            "Pending provider key cleanup must finish before rotating credentials again.",
        ));
    }
    let previous_credential_id = settings
        .primary_provider()
        .and_then(|provider| provider.credential_id.clone());
    // Never overwrite a credential still referenced by durable settings. If a later
    // persistence step fails, the old provider continues to resolve only its old key.
    let credential_id = Uuid::now_v7().to_string();
    let original = settings.clone();
    settings
        .pending_provider_cleanup_ids
        .push(credential_id.clone());
    if let Err(error) = save_settings(settings) {
        *settings = original;
        return Err(error);
    }
    let cleanup_staged = settings.clone();
    store_secret(&credential_id, secret)?;
    settings.providers = vec![ProviderSetting {
        profile: "primary-provider".into(),
        kind: request.provider_kind,
        base_url: provider_base_url(request.provider_kind).to_owned(),
        credential_id: Some(credential_id.clone()),
        timeout_ms: 120_000,
    }];
    settings.models = vec![ModelSetting {
        profile: "primary".into(),
        provider_profile: "primary-provider".into(),
        model: std::mem::take(&mut request.model),
        context_window_tokens: 128_000,
        max_output_tokens: 16_000,
        capabilities: ModelCapabilitiesSetting {
            tool_calls: true,
            streaming: true,
        },
    }];
    settings.model_roles = std::collections::BTreeMap::from([("primary".into(), "primary".into())]);
    settings.access_profile = request.access_profile;
    settings.selected_target_id = Some(MANAGED_TARGET_ID.to_owned());
    settings
        .pending_provider_cleanup_ids
        .retain(|pending| pending != &credential_id);
    if let Some(previous) = previous_credential_id.as_ref() {
        settings.pending_provider_cleanup_ids.push(previous.clone());
    }
    if let Err(error) = save_settings(settings) {
        // Durable settings still reference the previous provider and retain the
        // cleanup marker for the fresh key. Preserve that same view in memory.
        *settings = cleanup_staged;
        return Err(error);
    }
    Ok(ProviderCredentialRotation {
        previous_credential_id,
        fresh_credential_id: credential_id,
    })
}

fn rollback_provider_rotation(
    settings: &mut DesktopSettings,
    mut previous_settings: DesktopSettings,
    rotation: &ProviderCredentialRotation,
    mut save_settings: impl FnMut(&DesktopSettings) -> Result<(), CommandErrorDto>,
    delete_secret: impl FnOnce(&str) -> Result<(), CommandErrorDto>,
) -> Result<ProviderRollbackResult, CommandErrorDto> {
    // Restore the durable reference and record cleanup before deleting the fresh key.
    // A crash or keychain failure therefore leaves a retryable, app-private marker.
    if previous_settings.pending_provider_cleanup_ids.len() >= MAX_PENDING_PROVIDER_CLEANUPS {
        return Err(CommandErrorDto::busy(
            "Pending provider key cleanup must finish before rollback can complete.",
        ));
    }
    previous_settings
        .pending_provider_cleanup_ids
        .push(rotation.fresh_credential_id.clone());
    save_settings(&previous_settings)?;
    *settings = previous_settings.clone();
    let cleanup_error = delete_secret(&rotation.fresh_credential_id).err();
    if cleanup_error.is_none() {
        previous_settings
            .pending_provider_cleanup_ids
            .retain(|pending| pending != &rotation.fresh_credential_id);
        if let Err(error) = save_settings(&previous_settings) {
            return Ok(ProviderRollbackResult {
                cleanup_error: Some(error),
            });
        }
        *settings = previous_settings;
    }
    Ok(ProviderRollbackResult { cleanup_error })
}

fn cleanup_pending_provider_credentials(
    store: &SettingsStore,
    settings: &mut DesktopSettings,
) -> Result<(), CommandErrorDto> {
    if settings.pending_provider_cleanup_ids.is_empty() {
        return Ok(());
    }
    let before = settings.pending_provider_cleanup_ids.len();
    settings
        .pending_provider_cleanup_ids
        .retain(|credential_id| delete_provider_secret(credential_id).is_err());
    if settings.pending_provider_cleanup_ids.len() != before {
        store.save(settings)?;
    }
    Ok(())
}

fn retire_pending_provider_credential(
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    credential_id: &str,
) -> Result<(), CommandErrorDto> {
    debug_assert!(
        settings
            .pending_provider_cleanup_ids
            .iter()
            .any(|pending| pending == credential_id),
        "retired provider credential must be durably queued"
    );
    delete_provider_secret(credential_id)?;
    let previous = settings.pending_provider_cleanup_ids.clone();
    settings
        .pending_provider_cleanup_ids
        .retain(|pending| pending != credential_id);
    if let Err(error) = store.save(settings) {
        settings.pending_provider_cleanup_ids = previous;
        return Err(error);
    }
    Ok(())
}

async fn restore_managed_after_rollback(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
) -> Result<(), CommandErrorDto> {
    state
        .select_target(settings.selected_target_id.clone())
        .await;
    if has_managed_configuration(settings) {
        managed_runtime::start(state, store, settings, true).await
    } else {
        set_unstarted_health(state, settings).await;
        Ok(())
    }
}

fn has_managed_configuration(settings: &DesktopSettings) -> bool {
    settings.workspace.is_some() && settings.managed_configured()
}

#[tauri::command]
pub(crate) async fn restart_managed_runtime(
    state: State<'_, AppState>,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let settings = store.load()?;
    managed_runtime::start(&state, &store, &settings, true).await?;
    desktop_status_from(&state, &settings).await
}

#[tauri::command]
pub(crate) async fn run_managed_self_test(
    state: State<'_, AppState>,
) -> Result<(), CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let settings = store.load()?;
    managed_runtime::self_test(&state, &store, &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn select_target(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    validate_target(&settings, &target_id)?;
    if target_id == MANAGED_TARGET_ID {
        if !state.connected(MANAGED_TARGET_ID).await && settings.managed_configured() {
            managed_runtime::start(&state, &store, &settings, false).await?;
        }
    } else {
        let target =
            external_target(&settings, &target_id).ok_or_else(CommandErrorDto::not_configured)?;
        if !confirm_external_target(&app, target, ExternalConsentAction::Select).await? {
            return desktop_status_from(&state, &settings).await;
        }
        if !external_target_ready(&state, &target_id).await {
            connect_external(&state, target).await?;
        }
    }
    settings.selected_target_id = Some(target_id.clone());
    store.save(&settings)?;
    state.select_target(Some(target_id)).await;
    desktop_status_from(&state, &settings).await
}

#[derive(Clone, Copy)]
enum ExternalConsentAction {
    Import,
    Connect,
    Select,
    Remove,
}

impl ExternalConsentAction {
    const fn title(self) -> &'static str {
        match self {
            Self::Import => "Trust external Colossus daemon",
            Self::Connect | Self::Select => "Connect to external Colossus daemon",
            Self::Remove => "Remove external Colossus daemon",
        }
    }

    const fn prompt(self) -> &'static str {
        match self {
            Self::Import => "Import, save, and connect to this authenticated daemon?",
            Self::Connect => "Connect Work to this authenticated daemon?",
            Self::Select => "Select this authenticated daemon for Work?",
            Self::Remove => "Remove this saved daemon from Colossus Desktop?",
        }
    }

    const fn accept_label(self) -> &'static str {
        match self {
            Self::Import => "Trust and connect",
            Self::Connect => "Connect",
            Self::Select => "Select",
            Self::Remove => "Remove",
        }
    }
}

async fn confirm_external_target(
    app: &AppHandle,
    target: &ExternalTargetSetting,
    action: ExternalConsentAction,
) -> Result<bool, CommandErrorDto> {
    let message = external_consent_message(target, action)?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title(action.title())
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                action.accept_label().into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "external_target_confirmation",
            "The native external-target confirmation could not be opened.",
            true,
        )
    })
}

fn external_consent_message(
    target: &ExternalTargetSetting,
    action: ExternalConsentAction,
) -> Result<String, CommandErrorDto> {
    if !crate::desktop_settings::valid_external_label(&target.label)
        || !uuid::Uuid::parse_str(&target.instance_id).is_ok_and(|value| !value.is_nil())
        || target.certificate_sha256.len() != 64
        || !target
            .certificate_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CommandErrorDto::local_sanitized(
            "external_target_confirmation",
            "The external target identity could not be displayed safely.",
            false,
        ));
    }
    Ok(format!(
        "{}\n\nName: {}\nInstance: {}\nCertificate SHA-256: {}\n\nVerify these values match the daemon operator before continuing.",
        action.prompt(),
        target.label,
        target.instance_id,
        target.certificate_sha256.to_ascii_lowercase(),
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_terminal_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    settings.terminal_enabled = enabled;
    store.save(&settings)?;
    state.set_terminal_enabled(enabled).await;
    desktop_status_from(&state, &settings).await
}

async fn connect_external(
    state: &AppState,
    target: &ExternalTargetSetting,
) -> Result<(), CommandErrorDto> {
    let generation = state.begin_external_probe(&target.target_id).await;
    let client = match connection::connect(target).await {
        Ok(client) => client,
        Err(error) => {
            state
                .finish_external_probe(
                    &target.target_id,
                    generation,
                    Some(ExternalHealth::connection_failed(&error.code)),
                )
                .await;
            return Err(error);
        }
    };
    let consent = TargetConsentContext::External {
        label: target.label.clone(),
        instance_id: target.instance_id.clone(),
        certificate_sha256: target.certificate_sha256.clone(),
    };
    let restore_selection = state.selected_target_id().await.as_deref() == Some(&target.target_id);
    if restore_selection {
        // Acquiring the native selection writer waits for every selected-target
        // operation to finish before the old transport can be closed. This prevents
        // a committed create/control request from losing its response during reconnect.
        state.select_target(None).await;
    }
    if let Some(previous) = state
        .replace_target(&target.target_id, client, consent)
        .await
    {
        let _ = previous.client.close().await;
    }
    if restore_selection {
        state.select_target(Some(target.target_id.clone())).await;
    }
    state
        .finish_external_probe(
            &target.target_id,
            generation,
            Some(ExternalHealth::connected()),
        )
        .await;
    Ok(())
}

async fn spawn_external_health_probes(app: &AppHandle, targets: Vec<ExternalTargetSetting>) {
    for target in targets {
        let Some(generation) = app
            .state::<AppState>()
            .try_begin_external_probe(&target.target_id)
            .await
        else {
            continue;
        };
        let app = app.clone();
        tauri::async_runtime::spawn(run_external_health_probe(app, target, generation));
    }
}

async fn run_external_health_probe(app: AppHandle, target: ExternalTargetSetting, generation: u64) {
    let state = app.state::<AppState>();
    let Some(permit) = state.acquire_external_probe_slot().await else {
        state
            .finish_external_probe(&target.target_id, generation, None)
            .await;
        return;
    };
    if !state
        .external_probe_is_current(&target.target_id, generation)
        .await
    {
        return;
    }
    if !saved_external_target_matches(&target) {
        state
            .finish_external_probe(&target.target_id, generation, None)
            .await;
        return;
    }
    if let Some(existing) = state.target(&target.target_id).await {
        probe_connected_external(&state, &target.target_id, generation, existing, permit).await;
    } else {
        probe_disconnected_external(&state, &target, generation, permit).await;
    }
}

fn saved_external_target_matches(target: &ExternalTargetSetting) -> bool {
    settings_store()
        .and_then(|store| store.load())
        .ok()
        .and_then(|settings| external_target(&settings, &target.target_id).cloned())
        .is_some_and(|saved| connection::same_connection(&saved, target))
}

async fn probe_connected_external(
    state: &AppState,
    target_id: &str,
    generation: u64,
    existing: TargetHandle,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let request = ListRunsRequest {
        session_id: None,
        statuses: Vec::new(),
        page: Some(PageRequest {
            page_size: 1,
            page_token: String::new(),
        }),
    };
    let mut probe = tauri::async_runtime::spawn(async move {
        // The permit lives with the actual request task. If the health deadline
        // expires, a non-cancellable platform-keychain read remains globally bounded.
        let _permit = permit;
        existing.client.list_runs(request).await
    });
    let result = tokio::time::timeout(EXTERNAL_PROBE_TIMEOUT, &mut probe).await;
    let health = match result {
        Ok(Ok(Ok(_))) => Some(ExternalHealth::connected()),
        Ok(Ok(Err(error))) if error.code == ApiErrorCode::Unavailable => {
            Some(ExternalHealth::unreachable())
        }
        Ok(Ok(Err(error))) if error.code == ApiErrorCode::Unauthenticated => {
            Some(ExternalHealth::authentication_failed())
        }
        Ok(Ok(Err(_))) => {
            // An authenticated server response proves transport liveness; workload-
            // specific denial or pressure is not a connection loss.
            Some(ExternalHealth::connected())
        }
        Ok(Err(_)) => Some(ExternalHealth::connection_failed("internal")),
        Err(_) => Some(ExternalHealth::stalled()),
    };
    state
        .finish_external_probe(target_id, generation, health)
        .await;
}

async fn probe_disconnected_external(
    state: &AppState,
    target: &ExternalTargetSetting,
    generation: u64,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let target_for_probe = target.clone();
    let mut probe = tauri::async_runtime::spawn(async move {
        let _permit = permit;
        match connection::connect(&target_for_probe).await {
            Ok(client) => {
                let _ = client.close().await;
                ExternalHealth::available()
            }
            Err(error) => ExternalHealth::connection_failed(&error.code),
        }
    });
    let health = match tokio::time::timeout(EXTERNAL_PROBE_TIMEOUT, &mut probe).await {
        Ok(Ok(health)) => health,
        Ok(Err(_)) => ExternalHealth::connection_failed("internal"),
        Err(_) => ExternalHealth::stalled(),
    };
    state
        .finish_external_probe(&target.target_id, generation, Some(health))
        .await;
}

async fn set_unstarted_health(state: &AppState, settings: &DesktopSettings) {
    if state.connected(MANAGED_TARGET_ID).await {
        return;
    }
    let health = if settings.workspace.is_none() {
        ManagedHealth::default()
    } else if !settings.managed_configured() {
        ManagedHealth {
            state: ManagedRuntimeStateDto::NeedsProvider,
            message: "Configure a provider to start Managed Local.".into(),
            failure_code: None,
        }
    } else {
        ManagedHealth {
            state: ManagedRuntimeStateDto::Starting,
            message: "Managed Local is waiting to start.".into(),
            failure_code: None,
        }
    };
    state.set_managed_health(health).await;
}

async fn external_target_ready(state: &AppState, target_id: &str) -> bool {
    let (connected, health) = state.external_target_snapshot(target_id).await;
    connected && health.is_none_or(|health| health.state != "unreachable")
}

async fn desktop_status_from(
    state: &AppState,
    settings: &DesktopSettings,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    state.sync_managed_lifecycle_health().await;
    let selected = state.selected_target_id().await;
    let managed_health = state.managed_health().await;
    let managed_connected = state.connected(MANAGED_TARGET_ID).await;
    let managed_closed = state.target_is_closed(MANAGED_TARGET_ID).await;
    let health_was_ready = managed_health.state == ManagedRuntimeStateDto::Ready;
    let managed_health = managed_health_for_status(managed_health, managed_closed);
    if managed_closed && health_was_ready {
        state.set_managed_health(managed_health.clone()).await;
    }
    let workspace = settings.workspace.as_ref().map(WorkspaceSummaryDto::from);
    let managed_state = managed_health.state;
    let managed_ready = managed_connected && managed_state == ManagedRuntimeStateDto::Ready;
    let mut targets = vec![RuntimeTargetDto {
        target_id: MANAGED_TARGET_ID.into(),
        kind: RuntimeTargetKindDto::ManagedLocal,
        label: "Managed Local".into(),
        state: managed_state_name(managed_state).into(),
        message: managed_health.message.clone(),
        selected: selected.as_deref() == Some(MANAGED_TARGET_ID),
        terminal_available: managed_ready && cfg!(any(target_os = "macos", target_os = "windows")),
        workspace: workspace.clone(),
        failure_code: managed_health.failure_code,
    }];
    let mut selected_external_ready = false;
    for target in &settings.external_targets {
        let (status, ready) = external_target_status(state, target, selected.as_deref()).await;
        if status.selected {
            selected_external_ready = ready;
        }
        targets.push(status);
    }
    let connection = match selected.as_deref() {
        Some(MANAGED_TARGET_ID) if managed_ready => {
            ConnectionStatusDto::connected(MANAGED_TARGET_ID)
        }
        Some(MANAGED_TARGET_ID) => managed_connection_status(&managed_health),
        Some(target_id)
            if settings
                .external_targets
                .iter()
                .any(|target| target.target_id == target_id)
                && selected_external_ready =>
        {
            ConnectionStatusDto::connected(target_id)
        }
        Some(target_id)
            if settings
                .external_targets
                .iter()
                .any(|target| target.target_id == target_id) =>
        {
            ConnectionStatusDto::disconnected(Some(target_id.into()))
        }
        _ => ConnectionStatusDto::not_configured(),
    };
    let selected_managed = selected.as_deref() == Some(MANAGED_TARGET_ID) && managed_ready;
    let advertised = if connection.state == ConnectionStateDto::Connected {
        match selected.as_deref() {
            Some(target_id) => state
                .target(target_id)
                .await
                .map(|target| target.client.capabilities())
                .unwrap_or_default(),
            None => ServerCapabilities::default(),
        }
    } else {
        ServerCapabilities::default()
    };
    let capabilities = DesktopCapabilitiesDto {
        delegation: advertised.contains("agent_runs.delegation"),
        skills: advertised.contains("skills.select"),
        tui: selected_managed && cfg!(any(target_os = "macos", target_os = "windows")),
        files: selected_managed
            && workspace.is_some()
            && settings.access_profile
                == crate::desktop_settings::AccessProfileSetting::Development,
        artifacts: advertised.contains("artifacts.read"),
        update_available: state.update_available(),
        agent_workflows: advertised.contains("automation.workflows"),
        attachments: advertised.contains("attachments.run_input"),
    };
    Ok(DesktopStatusDto {
        release_channel: DesktopReleaseChannelDto::current(),
        connection,
        targets,
        selected_target_id: selected,
        managed_state,
        workspace,
        provider: ProviderSummaryDto::from_settings(settings),
        managed_model_configuration: ManagedModelConfigurationDto::from_settings(settings),
        access_profile: settings.access_profile,
        terminal_enabled: settings.terminal_enabled,
        additional_ca_bundle: crate::desktop_dto::CaBundleStatusDto::from_settings(settings),
        capabilities,
    })
}

pub(crate) async fn diagnostics_status(
    state: &AppState,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    desktop_status_from(state, &settings).await
}

async fn external_target_status(
    state: &AppState,
    target: &ExternalTargetSetting,
    selected: Option<&str>,
) -> (RuntimeTargetDto, bool) {
    let (connected, health) = state.external_target_snapshot(&target.target_id).await;
    let ready = connected
        && health
            .as_ref()
            .is_none_or(|health| health.state != "unreachable");
    let (target_state, message, failure_code) = if let Some(health) = health
        .as_ref()
        .filter(|health| health.state == "unreachable")
    {
        (health.state, health.message.clone(), health.failure_code)
    } else if connected {
        (
            "ready",
            "Authenticated daemon connection is ready.".to_owned(),
            None,
        )
    } else if let Some(health) = health {
        if health.state == "ready" {
            (
                "unreachable",
                "The authenticated daemon connection closed.".to_owned(),
                Some(crate::desktop_dto::RuntimeFailureCodeDto::Transport),
            )
        } else {
            (health.state, health.message, health.failure_code)
        }
    } else {
        (
            "disconnected",
            "Saved daemon is not connected.".to_owned(),
            None,
        )
    };
    let status = RuntimeTargetDto {
        target_id: target.target_id.clone(),
        kind: RuntimeTargetKindDto::ExternalDaemon,
        label: target.label.clone(),
        state: target_state.into(),
        message,
        selected: selected == Some(target.target_id.as_str()),
        terminal_available: false,
        workspace: None,
        failure_code,
    };
    (status, ready)
}

fn managed_health_for_status(health: ManagedHealth, client_closed: bool) -> ManagedHealth {
    if client_closed && health.state == ManagedRuntimeStateDto::Ready {
        ManagedHealth {
            state: ManagedRuntimeStateDto::Failed,
            message:
                "Managed Local stopped after repeated restart failures. Restart it to continue."
                    .into(),
            failure_code: Some(crate::desktop_dto::RuntimeFailureCodeDto::CrashLoop),
        }
    } else {
        health
    }
}

fn managed_connection_status(health: &ManagedHealth) -> ConnectionStatusDto {
    let state = match health.state {
        ManagedRuntimeStateDto::NeedsWorkspace | ManagedRuntimeStateDto::NeedsProvider => {
            ConnectionStateDto::NotConfigured
        }
        ManagedRuntimeStateDto::Starting => ConnectionStateDto::Starting,
        ManagedRuntimeStateDto::Ready => ConnectionStateDto::Disconnected,
        ManagedRuntimeStateDto::Restarting => ConnectionStateDto::Restarting,
        ManagedRuntimeStateDto::Stopping => ConnectionStateDto::Stopping,
        ManagedRuntimeStateDto::Failed => ConnectionStateDto::Failed,
    };
    ConnectionStatusDto::managed(state, &health.message)
}

const fn managed_state_name(state: ManagedRuntimeStateDto) -> &'static str {
    match state {
        ManagedRuntimeStateDto::NeedsWorkspace => "needs_workspace",
        ManagedRuntimeStateDto::NeedsProvider => "needs_provider",
        ManagedRuntimeStateDto::Starting => "starting",
        ManagedRuntimeStateDto::Ready => "ready",
        ManagedRuntimeStateDto::Restarting => "restarting",
        ManagedRuntimeStateDto::Stopping => "stopping",
        ManagedRuntimeStateDto::Failed => "failed",
    }
}

fn settings_store() -> Result<SettingsStore, CommandErrorDto> {
    SettingsStore::open(application_support_root()?)
}

fn connect_guard(state: &AppState) -> Result<tokio::sync::MutexGuard<'_, ()>, CommandErrorDto> {
    state.try_connect_guard().ok_or_else(|| {
        CommandErrorDto::busy("A Colossus connection or restart is already in progress.")
    })
}

fn validate_target(settings: &DesktopSettings, target_id: &str) -> Result<(), CommandErrorDto> {
    if target_exists(settings, target_id) {
        Ok(())
    } else {
        Err(CommandErrorDto::invalid(
            "targetId",
            "The runtime target is unknown.",
        ))
    }
}

fn target_exists(settings: &DesktopSettings, target_id: &str) -> bool {
    target_id == MANAGED_TARGET_ID
        || settings
            .external_targets
            .iter()
            .any(|target| target.target_id == target_id)
}

fn external_target<'a>(
    settings: &'a DesktopSettings,
    target_id: &str,
) -> Option<&'a ExternalTargetSetting> {
    settings
        .external_targets
        .iter()
        .find(|target| target.target_id == target_id)
}

fn migrate_legacy_connection(settings: &mut DesktopSettings) -> bool {
    if settings.legacy_connection_migrated {
        return false;
    }
    let Some(compiled) = connection::compiled_target() else {
        return false;
    };
    if !settings
        .external_targets
        .iter()
        .any(|target| connection::same_connection(target, &compiled))
    {
        if settings.external_targets.len() >= MAX_EXTERNAL_TARGETS {
            return false;
        }
        settings.external_targets.push(compiled);
    }
    settings.legacy_connection_migrated = true;
    true
}

const fn should_start_managed_on_initialize(managed_configured: bool, connected: bool) -> bool {
    managed_configured && !connected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace_identity() -> colossus_sdk::WorkspaceIdentity {
        colossus_sdk::WorkspaceIdentity::from_macos_parts(1, 2, 1_700_000_000, 0)
            .expect("current workspace identity")
    }

    fn provider_request() -> ConfigureManagedRuntimeInput {
        ConfigureManagedRuntimeInput {
            workspace_id: Uuid::now_v7().to_string(),
            provider_kind: crate::desktop_settings::ProviderKindSetting::OpenAiCompatible,
            model: "new-model".into(),
            access_profile: crate::desktop_settings::AccessProfileSetting::Development,
            replace_credential: false,
        }
    }

    fn provider_secret() -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new("sk-or-v1-test-secret".into())
    }

    fn settings_with_provider(credential_id: &str) -> DesktopSettings {
        DesktopSettings {
            providers: vec![ProviderSetting {
                profile: "primary-provider".into(),
                kind: crate::desktop_settings::ProviderKindSetting::OpenAiCompatible,
                base_url: crate::desktop_settings::OPENROUTER_BASE_URL.into(),
                credential_id: Some(credential_id.into()),
                timeout_ms: 120_000,
            }],
            models: vec![ModelSetting {
                profile: "primary".into(),
                provider_profile: "primary-provider".into(),
                model: "old-model".into(),
                context_window_tokens: 128_000,
                max_output_tokens: 16_000,
                capabilities: ModelCapabilitiesSetting {
                    tool_calls: true,
                    streaming: true,
                },
            }],
            model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
            ..DesktopSettings::default()
        }
    }

    #[test]
    fn target_ids_are_native_constants_not_renderer_paths() {
        let external_id = Uuid::now_v7().to_string();
        let instance_id = Uuid::now_v7().to_string();
        let certificate_sha256 = "a".repeat(64);
        let credential_account =
            crate::desktop_settings::external_credential_account(&instance_id, &certificate_sha256)
                .expect("credential binding");
        let settings = DesktopSettings {
            external_targets: vec![ExternalTargetSetting {
                target_id: external_id.clone(),
                label: "Lab daemon".into(),
                instance_id,
                certificate_sha256,
                public_api_dir: "/private/tmp/colossus-api".into(),
                credential_service: crate::desktop_settings::EXTERNAL_KEYRING_SERVICE.into(),
                credential_account,
                requires_credential_enrollment: false,
            }],
            ..DesktopSettings::default()
        };
        assert!(validate_target(&settings, MANAGED_TARGET_ID).is_ok());
        assert!(validate_target(&settings, &external_id).is_ok());
        assert!(validate_target(&settings, "/private/tmp/runtime").is_err());
        assert!(validate_target(&settings, "managed-local/../../other").is_err());
    }

    #[test]
    fn managed_state_names_are_stable_renderer_values() {
        assert_eq!(
            managed_state_name(ManagedRuntimeStateDto::NeedsWorkspace),
            "needs_workspace"
        );
        assert_eq!(managed_state_name(ManagedRuntimeStateDto::Ready), "ready");
    }

    #[test]
    fn external_consent_names_the_complete_native_identity() {
        let instance_id = Uuid::now_v7().to_string();
        let certificate_sha256 = "a".repeat(64);
        let target = ExternalTargetSetting {
            target_id: Uuid::now_v7().to_string(),
            label: "Production daemon".into(),
            instance_id: instance_id.clone(),
            certificate_sha256: certificate_sha256.clone(),
            public_api_dir: "/private/tmp/colossus-api".into(),
            credential_service: crate::desktop_settings::EXTERNAL_KEYRING_SERVICE.into(),
            credential_account: "unused-by-display".into(),
            requires_credential_enrollment: false,
        };

        let message = external_consent_message(&target, ExternalConsentAction::Select)
            .expect("safe native identity");
        assert!(message.contains("Production daemon"));
        assert!(message.contains(&instance_id));
        assert!(message.contains(&certificate_sha256));

        let mut unsafe_target = target;
        unsafe_target.label = "Production\nspoofed".into();
        assert!(external_consent_message(&unsafe_target, ExternalConsentAction::Remove).is_err());
    }

    #[test]
    fn workspace_change_is_durable_and_rolls_back_as_one_native_transaction() {
        let old_credential = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&old_credential);
        let original = settings.clone();
        let workspace = WorkspaceSetting {
            id: Uuid::now_v7().to_string(),
            path: "/tmp/new-workspace".into(),
            identity: Some(test_workspace_identity()),
            display_name: "new-workspace".into(),
            display_path: "/tmp/new-workspace".into(),
        };

        let previous = persist_workspace_change(&mut settings, workspace.clone(), |persisted| {
            assert_eq!(persisted.workspace.as_ref(), Some(&workspace));
            assert_eq!(
                persisted.selected_target_id.as_deref(),
                Some(MANAGED_TARGET_ID)
            );
            Ok(())
        })
        .expect("persist workspace");
        assert_eq!(previous, original);

        rollback_workspace_change(&mut settings, previous, |persisted| {
            assert_eq!(persisted, &original);
            Ok(())
        })
        .expect("rollback workspace");
        assert_eq!(settings, original);
    }

    #[test]
    fn failed_workspace_persistence_does_not_mutate_native_settings() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        let original = settings.clone();
        let error = CommandErrorDto::local_sanitized(
            "test_failure",
            "The test persistence step failed.",
            false,
        );
        let result = persist_workspace_change(
            &mut settings,
            WorkspaceSetting {
                id: Uuid::now_v7().to_string(),
                path: "/tmp/new-workspace".into(),
                identity: Some(test_workspace_identity()),
                display_name: "new-workspace".into(),
                display_path: "/tmp/new-workspace".into(),
            },
            |_| Err(error),
        );

        assert!(result.is_err());
        assert_eq!(settings, original);
    }

    #[test]
    fn exhausted_sidecar_restarts_fail_closed_with_a_sanitized_code() {
        let health = managed_health_for_status(
            ManagedHealth {
                state: ManagedRuntimeStateDto::Ready,
                message: "Managed Local is ready.".into(),
                failure_code: None,
            },
            true,
        );

        assert_eq!(health.state, ManagedRuntimeStateDto::Failed);
        assert_eq!(
            health.failure_code,
            Some(crate::desktop_dto::RuntimeFailureCodeDto::CrashLoop)
        );
        assert!(!health.message.contains('/'));
    }

    #[test]
    fn managed_initialize_requires_workspace_provider_and_disconnected_runtime() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        assert!(settings.primary_provider().is_some());
        assert!(!has_managed_configuration(&settings));
        assert!(!should_start_managed_on_initialize(
            has_managed_configuration(&settings),
            false
        ));

        settings.workspace = Some(WorkspaceSetting {
            id: Uuid::now_v7().to_string(),
            path: "/tmp/selected-workspace".into(),
            identity: Some(test_workspace_identity()),
            display_name: "selected-workspace".into(),
            display_path: "/tmp/selected-workspace".into(),
        });
        assert!(should_start_managed_on_initialize(
            has_managed_configuration(&settings),
            false
        ));
        assert!(!should_start_managed_on_initialize(
            has_managed_configuration(&settings),
            true
        ));
    }

    #[test]
    fn rollback_restores_a_runtime_only_for_a_complete_managed_configuration() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        assert!(!has_managed_configuration(&settings));
        settings.workspace = Some(WorkspaceSetting {
            id: Uuid::now_v7().to_string(),
            path: "/tmp/previous-workspace".into(),
            identity: Some(test_workspace_identity()),
            display_name: "previous-workspace".into(),
            display_path: "/tmp/previous-workspace".into(),
        });
        assert!(has_managed_configuration(&settings));
        settings.providers.clear();
        settings.models.clear();
        settings.model_roles.clear();
        assert!(!has_managed_configuration(&settings));
    }

    #[test]
    fn provider_reuse_is_native_and_never_crosses_provider_kinds() {
        let settings = settings_with_provider(&Uuid::now_v7().to_string());
        let mut request = provider_request();
        assert!(reusable_provider_credential(&settings, &request));

        request.replace_credential = true;
        assert!(!reusable_provider_credential(&settings, &request));
        request.replace_credential = false;
        request.provider_kind = crate::desktop_settings::ProviderKindSetting::OpenAiResponses;
        assert!(!reusable_provider_credential(&settings, &request));
        assert!(!reusable_provider_credential(
            &DesktopSettings::default(),
            &provider_request(),
        ));
    }

    #[test]
    fn development_authority_requires_native_confirmation_only_on_elevation() {
        let request = provider_request();
        assert!(development_access_elevation(
            &DesktopSettings::default(),
            &request,
        ));

        let mut minimal = settings_with_provider(&Uuid::now_v7().to_string());
        minimal.access_profile = crate::desktop_settings::AccessProfileSetting::Minimal;
        assert!(development_access_elevation(&minimal, &request));

        let development = settings_with_provider(&Uuid::now_v7().to_string());
        assert!(!development_access_elevation(&development, &request));

        let mut narrower = request;
        narrower.access_profile = crate::desktop_settings::AccessProfileSetting::Minimal;
        assert!(!development_access_elevation(&development, &narrower));
    }

    #[test]
    fn reused_provider_update_preserves_key_identity_and_is_transactional() {
        let credential_id = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&credential_id);
        let original = settings.clone();
        let mut request = provider_request();
        request.model = "updated-model".into();
        request.access_profile = crate::desktop_settings::AccessProfileSetting::Minimal;

        verify_reused_provider_credential(&settings, |loaded_id| {
            assert_eq!(loaded_id, credential_id);
            Ok(zeroize::Zeroizing::new(b"existing-key".to_vec()))
        })
        .expect("existing native key");
        let previous =
            persist_reused_provider_configuration(&mut settings, &mut request, |_| Ok(()))
                .expect("reuse provider configuration");
        assert_eq!(previous, original);
        let provider = settings.primary_provider().expect("provider");
        assert_eq!(
            provider.credential_id.as_deref(),
            Some(credential_id.as_str())
        );
        assert_eq!(
            settings.primary_model().expect("model").model,
            "updated-model"
        );
        assert_eq!(
            settings.access_profile,
            crate::desktop_settings::AccessProfileSetting::Minimal,
        );
        assert!(settings.pending_provider_cleanup_ids.is_empty());

        rollback_workspace_change(&mut settings, previous, |_| Ok(()))
            .expect("rollback reused settings");
        assert_eq!(settings, original);
    }

    #[test]
    fn failed_reused_provider_persistence_restores_the_original_configuration() {
        let credential_id = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&credential_id);
        let original = settings.clone();
        let mut request = provider_request();
        request.model = "updated-model".into();
        request.access_profile = crate::desktop_settings::AccessProfileSetting::Minimal;

        let error = CommandErrorDto::local_sanitized(
            "test_failure",
            "The test persistence step failed.",
            false,
        );
        let result =
            persist_reused_provider_configuration(&mut settings, &mut request, |_| Err(error));

        assert!(result.is_err());
        assert_eq!(settings, original);
        assert_eq!(
            settings
                .primary_provider()
                .and_then(|provider| provider.credential_id.as_deref()),
            Some(credential_id.as_str()),
        );
    }

    #[test]
    fn missing_reused_key_fails_before_settings_or_runtime_mutation() {
        let settings = settings_with_provider(&Uuid::now_v7().to_string());
        let error = verify_reused_provider_credential(&settings, |_| {
            Err(CommandErrorDto::local_sanitized(
                "provider_credential",
                "stored key unavailable; replace it",
                false,
            ))
        })
        .expect_err("missing native key");
        assert_eq!(error.code, "provider_credential");
    }

    #[test]
    fn provider_rotation_never_overwrites_the_durable_old_credential() {
        let old_id = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&old_id);
        let mut request = provider_request();
        let secret = provider_secret();
        let stored = std::cell::RefCell::new(Vec::<String>::new());

        let rotation = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |credential_id, _| {
                stored.borrow_mut().push(credential_id.into());
                Ok(())
            },
            |_| Ok(()),
        )
        .expect("stage rotation");

        assert_eq!(
            rotation.previous_credential_id.as_deref(),
            Some(old_id.as_str())
        );
        assert_eq!(stored.borrow().len(), 1);
        assert_ne!(stored.borrow()[0], old_id);
        assert_eq!(rotation.fresh_credential_id, stored.borrow()[0]);
        assert_eq!(settings.pending_provider_cleanup_ids, [old_id]);
        assert_eq!(
            settings
                .primary_provider()
                .expect("new provider")
                .credential_id
                .as_deref(),
            Some(stored.borrow()[0].as_str())
        );
    }

    #[test]
    fn failed_final_settings_commit_keeps_a_durable_fresh_key_cleanup_marker() {
        let old_id = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&old_id);
        let mut request = provider_request();
        let secret = provider_secret();
        let stored = std::cell::RefCell::new(String::new());
        let save_count = std::cell::Cell::new(0_u8);

        let result = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |credential_id, _| {
                stored.replace(credential_id.into());
                Ok(())
            },
            |_| {
                let call = save_count.get();
                save_count.set(call + 1);
                if call == 0 {
                    Ok(())
                } else {
                    Err(CommandErrorDto::local_sanitized(
                        "test_storage",
                        "test storage failure",
                        false,
                    ))
                }
            },
        );

        assert!(result.is_err());
        assert_ne!(stored.borrow().as_str(), old_id);
        assert_eq!(
            settings
                .primary_provider()
                .expect("old provider")
                .credential_id
                .as_deref(),
            Some(old_id.as_str())
        );
        assert_eq!(
            settings.pending_provider_cleanup_ids,
            [stored.borrow().clone()]
        );
    }

    #[test]
    fn failed_keychain_write_retains_the_durable_cleanup_marker() {
        let old_id = Uuid::now_v7().to_string();
        let mut settings = settings_with_provider(&old_id);
        let mut request = provider_request();
        let secret = provider_secret();

        let result = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |_, _| {
                Err(CommandErrorDto::local_sanitized(
                    "test_keychain",
                    "test keychain failure",
                    false,
                ))
            },
            |_| Ok(()),
        );

        assert!(result.is_err());
        assert_eq!(
            settings
                .primary_provider()
                .expect("old provider")
                .credential_id
                .as_deref(),
            Some(old_id.as_str())
        );
        assert_eq!(settings.pending_provider_cleanup_ids.len(), 1);
    }

    #[test]
    fn failed_runtime_start_rolls_back_settings_before_deleting_the_fresh_key() {
        let old_id = Uuid::now_v7().to_string();
        let original = settings_with_provider(&old_id);
        let mut settings = original.clone();
        let mut request = provider_request();
        let secret = provider_secret();
        let rotation = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .expect("stage rotation");
        let operations = std::cell::RefCell::new(Vec::new());

        rollback_provider_rotation(
            &mut settings,
            original.clone(),
            &rotation,
            |_| {
                operations.borrow_mut().push("save");
                Ok(())
            },
            |credential_id| {
                assert_eq!(credential_id, rotation.fresh_credential_id.as_str());
                operations.borrow_mut().push("delete");
                Ok(())
            },
        )
        .expect("rollback");

        assert_eq!(settings, original);
        assert_eq!(*operations.borrow(), ["save", "delete", "save"]);
    }

    #[test]
    fn failed_rotation_rollback_keeps_the_referenced_fresh_key() {
        let old_id = Uuid::now_v7().to_string();
        let original = settings_with_provider(&old_id);
        let mut settings = original.clone();
        let mut request = provider_request();
        let secret = provider_secret();
        let rotation = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .expect("stage rotation");
        let staged = settings.clone();

        let result = rollback_provider_rotation(
            &mut settings,
            original,
            &rotation,
            |_| {
                Err(CommandErrorDto::local_sanitized(
                    "test_storage",
                    "test storage failure",
                    false,
                ))
            },
            |_| panic!("a referenced fresh key must not be deleted"),
        );

        assert!(result.is_err());
        assert_eq!(settings, staged);
    }

    #[test]
    fn rollback_cleanup_failure_still_restores_the_previous_provider() {
        let old_id = Uuid::now_v7().to_string();
        let original = settings_with_provider(&old_id);
        let mut settings = original.clone();
        let mut request = provider_request();
        let secret = provider_secret();
        let rotation = persist_provider_rotation(
            &mut settings,
            &mut request,
            &secret,
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .expect("stage rotation");

        let rollback = rollback_provider_rotation(
            &mut settings,
            original.clone(),
            &rotation,
            |_| Ok(()),
            |_| {
                Err(CommandErrorDto::local_sanitized(
                    "test_keychain",
                    "test keychain cleanup failure",
                    false,
                ))
            },
        )
        .expect("durable rollback");

        assert_eq!(settings.providers, original.providers);
        assert_eq!(
            settings.pending_provider_cleanup_ids,
            [rotation.fresh_credential_id]
        );
        assert!(rollback.cleanup_error.is_some());
    }
}
