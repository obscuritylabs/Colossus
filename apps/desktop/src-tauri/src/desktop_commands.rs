use colossus_sdk::{
    ApiErrorCode, GetRunRequest, ListRunsRequest, PLAN_CONTINUATION_CAPABILITY, PageRequest,
    PageResponse, RunStatus, SESSION_ACTIVITY_CAPABILITY, ServerCapabilities,
};
use colossus_worker_protocol::{WorkerSessionMap, WorkerThreadDelegateInspection};
use futures_util::future::join_all;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter as _, Manager as _, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use uuid::Uuid;

use crate::{
    connection,
    desktop_dto::{
        ApplyManagedModelConfigurationInput, ConfigureManagedRuntimeInput, CredentialActionInput,
        DesktopApprovalModeDto, DesktopCapabilitiesDto, DesktopReleaseChannelDto, DesktopStatusDto,
        ManagedModelConfigurationDto, ManagedRuntimeStateDto, ProviderSummaryDto, RuntimeTargetDto,
        RuntimeTargetKindDto, SpaceAttentionDto, SpaceSearchPageDto, SpaceStatusEventDto,
        SpaceSummaryDto, WorkspaceSummaryDto,
    },
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, ExecutionBoundarySetting, ExternalTargetSetting,
        LOCAL_TERMINAL_CONSENT_VERSION, MAX_EXTERNAL_TARGETS, MAX_PENDING_PROVIDER_CLEANUPS,
        ModelCapabilitiesSetting, ModelSetting, ProviderKindSetting, ProviderSetting,
        SettingsStore, WorkspaceSetting, delete_provider_secret, load_provider_secret,
        provider_base_url, revalidate_workspace, store_provider_secret, validate_workspace,
    },
    dto::{CommandErrorDto, ConnectionStateDto, ConnectionStatusDto, RunDto},
    managed_runtime, provider_enrollment, run_list, space_search,
    state::{AppState, ExternalHealth, ManagedHealth, TargetConsentContext, TargetHandle},
};

const EXTERNAL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const SPACE_SUMMARY_REFRESH_TIMEOUT: Duration = Duration::from_secs(2);
const SPACE_SEARCH_INDEX_PAGE_SIZE: u32 = 100;
const MAX_SPACE_SEARCH_INDEX_RUNS: usize = 4_096;

fn emit_space_events(app: &AppHandle, status: &DesktopStatusDto) {
    for summary in &status.spaces {
        let _ = app.emit(
            "space-status-changed",
            SpaceStatusEventDto {
                space_id: summary.space_id.clone(),
                target_id: summary.target_id.clone(),
                display_name: summary.display_name.clone(),
                archived: summary.archived,
                state: summary.state.clone(),
                selected: summary.selected,
                attention_count: summary.attention_count,
                last_activity_at: summary.last_activity_at.clone(),
            },
        );
        if summary.attention_count > 0 {
            let _ = app.emit(
                "space-attention",
                SpaceAttentionDto {
                    space_id: summary.space_id.clone(),
                    target_id: summary.target_id.clone(),
                    attention_count: summary.attention_count,
                },
            );
        }
    }
}

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
    state
        .set_terminal_enabled(settings.local_terminal_enabled())
        .await;
    set_unstarted_health(&state, &settings).await;

    let selected = settings
        .selected_target_id
        .as_deref()
        .filter(|target| target_exists(&settings, target))
        .map(str::to_owned)
        .or_else(|| {
            has_managed_configuration(&settings)
                .then(|| settings.selected_space_id.clone())
                .flatten()
        })
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
        match settings.selected_space_id.as_deref() {
            Some(space_id) => state.connected(space_id).await,
            None => false,
        },
    ) {
        spawn_managed_start_on_initialize(app.clone());
    }
    if let Some(target) = selected
        .as_deref()
        .and_then(|target_id| external_target(&settings, target_id))
        && !external_target_ready(&state, &target.target_id).await
    {
        let _ = connect_external(&state, target).await;
    }
    let status = desktop_status_from(&state, &settings).await?;
    emit_space_events(&app, &status);
    spawn_external_health_probes(&app, settings.external_targets).await;
    Ok(status)
}

#[tauri::command]
pub(crate) async fn desktop_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    refresh_live_space_search_index(&state, &settings).await;
    let status = desktop_status_from(&state, &settings).await?;
    emit_space_events(&app, &status);
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
    if settings.spaces.iter().any(|space| space.id == target_id) {
        settings.activate_space(&target_id)?;
        if !state.connected(&target_id).await {
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
    if settings.selected_space_id.as_deref() == Some(target_id.as_str()) {
        state.activate_managed_terminal_for(&target_id).await;
    }
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
        settings.selected_target_id = settings.selected_space_id.clone();
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
    create_space_from_picker(&app, &state)
        .await
        .map(|result| result.map(|(workspace, _)| workspace))
}

#[tauri::command]
pub(crate) async fn create_space(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<DesktopStatusDto>, CommandErrorDto> {
    create_space_from_picker(&app, &state)
        .await
        .map(|result| result.map(|(_, status)| status))
}

async fn create_space_from_picker(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<(WorkspaceSummaryDto, DesktopStatusDto)>, CommandErrorDto> {
    let Some(workspace) = pick_workspace_from_dialog(app)? else {
        return Ok(None);
    };
    let _guard = connect_guard(state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous_settings = settings.clone();
    let rebound_space_id =
        apply_workspace_picker_selection(app, state, &mut settings, workspace.clone()).await?;
    let space_id = selected_space_id(&settings)?;
    let rebound_runtime_was_connected =
        rebound_runtime_was_connected(state, rebound_space_id.as_deref()).await;

    stop_rebound_runtime_for_selection(
        state,
        &store,
        &previous_settings,
        rebound_space_id.as_deref(),
        rebound_runtime_was_connected,
    )
    .await?;
    save_space_selection(
        state,
        &store,
        &settings,
        &previous_settings,
        rebound_space_id.as_deref(),
        rebound_runtime_was_connected,
    )
    .await?;
    start_selected_space_runtime(
        state,
        &store,
        &settings,
        &previous_settings,
        rebound_space_id.as_deref(),
        rebound_runtime_was_connected,
        &space_id,
    )
    .await?;

    state.select_target(Some(space_id.clone())).await;
    state.activate_managed_terminal_for(&space_id).await;
    let status = desktop_status_from(state, &settings).await?;
    Ok(Some((WorkspaceSummaryDto::from(&workspace), status)))
}

fn pick_workspace_from_dialog(
    app: &AppHandle,
) -> Result<Option<WorkspaceSetting>, CommandErrorDto> {
    let Some(path) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|_| {
        CommandErrorDto::invalid("workspace", "The selected folder is unavailable.")
    })?;
    Ok(Some(validate_workspace(&path)?))
}

async fn apply_workspace_picker_selection(
    app: &AppHandle,
    state: &AppState,
    settings: &mut DesktopSettings,
    workspace: WorkspaceSetting,
) -> Result<Option<String>, CommandErrorDto> {
    let identity_match = workspace
        .identity
        .as_ref()
        .and_then(|identity| settings.space_for_workspace_identity(identity))
        .cloned();
    let path_match = settings.space_for_workspace_path(&workspace.path).cloned();
    let mut rebound_space_id = None;
    if let Some(existing) = identity_match {
        if !existing.archived {
            return Err(CommandErrorDto::invalid(
                "workspace",
                "That folder already belongs to an active Workspace.",
            ));
        }
        if !confirm_restore_space(app, &existing.display_name).await? {
            return Ok(None);
        }
        if let Some(space) = settings
            .spaces
            .iter_mut()
            .find(|space| space.id == existing.id)
        {
            space.archived = false;
        }
        settings.activate_space(&existing.id)?;
    } else if let Some(existing) = path_match {
        if existing.archived && !confirm_restore_space(app, &existing.display_name).await? {
            return Ok(None);
        }
        reject_active_managed_runs_for(state, &existing.id).await?;
        if settings.selected_space_id.as_deref() == Some(existing.id.as_str())
            && state.terminal_session_active()
        {
            return Err(CommandErrorDto::busy(
                "Close this Workspace's terminal sessions before selecting its folder again.",
            ));
        }
        settings.rebind_space_workspace(&existing.id, workspace.clone())?;
        if let Some(space) = settings
            .spaces
            .iter_mut()
            .find(|space| space.id == existing.id)
        {
            space.archived = false;
        }
        settings.activate_space(&existing.id)?;
        rebound_space_id = Some(existing.id);
    } else {
        settings.add_space(workspace)?;
    }
    Ok(rebound_space_id)
}

fn selected_space_id(settings: &DesktopSettings) -> Result<String, CommandErrorDto> {
    settings
        .selected_space_id
        .clone()
        .ok_or_else(CommandErrorDto::not_configured)
}

async fn rebound_runtime_was_connected(state: &AppState, rebound_space_id: Option<&str>) -> bool {
    if let Some(rebound_space_id) = rebound_space_id {
        state.connected(rebound_space_id).await
    } else {
        false
    }
}

async fn stop_rebound_runtime_for_selection(
    state: &AppState,
    store: &SettingsStore,
    previous_settings: &DesktopSettings,
    rebound_space_id: Option<&str>,
    rebound_runtime_was_connected: bool,
) -> Result<(), CommandErrorDto> {
    let Some(rebound_space_id) = rebound_space_id else {
        return Ok(());
    };
    if let Some(target) = state.target(rebound_space_id).await
        && target.client.close().await.is_err()
    {
        let close_error =
            CommandErrorDto::busy("The previous Workspace runtime is still stopping.");
        restore_space_rebind_after_failure(
            state,
            store,
            previous_settings,
            rebound_space_id,
            rebound_runtime_was_connected,
        )
        .await?;
        return Err(close_error);
    }
    state.remove_target(rebound_space_id).await;
    state.remove_managed_space_runtime(rebound_space_id).await;
    Ok(())
}

async fn save_space_selection(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    previous_settings: &DesktopSettings,
    rebound_space_id: Option<&str>,
    rebound_runtime_was_connected: bool,
) -> Result<(), CommandErrorDto> {
    if let Err(error) = store.save(settings) {
        let settings_restore_result = store.save(previous_settings);
        let runtime_restore_result = restore_rebound_runtime_after_failure(
            state,
            store,
            previous_settings,
            rebound_space_id,
            rebound_runtime_was_connected,
        )
        .await;
        settings_restore_result?;
        runtime_restore_result?;
        return Err(error);
    }
    Ok(())
}

async fn start_selected_space_runtime(
    state: &AppState,
    store: &SettingsStore,
    settings: &DesktopSettings,
    previous_settings: &DesktopSettings,
    rebound_space_id: Option<&str>,
    rebound_runtime_was_connected: bool,
    space_id: &str,
) -> Result<(), CommandErrorDto> {
    state.select_target(None).await;
    if settings.managed_configured() {
        if let Err(error) = managed_runtime::start(state, store, settings, false).await {
            let settings_restore_result = store.save(previous_settings);
            let runtime_restore_result = restore_runtime_after_start_failure(
                state,
                store,
                previous_settings,
                rebound_space_id,
                rebound_runtime_was_connected,
            )
            .await;
            settings_restore_result?;
            runtime_restore_result?;
            return Err(error);
        }
    } else {
        state.clear_terminal_workspace().await;
        state
            .set_managed_health_for(
                space_id,
                ManagedHealth {
                    state: ManagedRuntimeStateDto::NeedsProvider,
                    message: "Configure a provider to start this Workspace.".into(),
                    failure_code: None,
                },
            )
            .await;
    }
    Ok(())
}

async fn restore_rebound_runtime_after_failure(
    state: &AppState,
    store: &SettingsStore,
    previous_settings: &DesktopSettings,
    rebound_space_id: Option<&str>,
    rebound_runtime_was_connected: bool,
) -> Result<(), CommandErrorDto> {
    if let Some(rebound_space_id) = rebound_space_id {
        restore_space_rebind_after_failure(
            state,
            store,
            previous_settings,
            rebound_space_id,
            rebound_runtime_was_connected,
        )
        .await
    } else {
        Ok(())
    }
}

async fn restore_runtime_after_start_failure(
    state: &AppState,
    store: &SettingsStore,
    previous_settings: &DesktopSettings,
    rebound_space_id: Option<&str>,
    rebound_runtime_was_connected: bool,
) -> Result<(), CommandErrorDto> {
    if rebound_space_id.is_some() {
        restore_rebound_runtime_after_failure(
            state,
            store,
            previous_settings,
            rebound_space_id,
            rebound_runtime_was_connected,
        )
        .await
    } else {
        restore_previous_space_selection(state, previous_settings).await;
        Ok(())
    }
}

fn rebound_runtime_rollback_settings(
    previous_settings: &DesktopSettings,
    rebound_space_id: &str,
) -> Result<Option<DesktopSettings>, CommandErrorDto> {
    if previous_settings
        .space(rebound_space_id)
        .is_none_or(|space| space.archived)
    {
        return Ok(None);
    }
    let mut rollback_settings = previous_settings.clone();
    rollback_settings.activate_space(rebound_space_id)?;
    Ok(Some(rollback_settings))
}

async fn restore_space_rebind_after_failure(
    state: &AppState,
    store: &SettingsStore,
    previous_settings: &DesktopSettings,
    rebound_space_id: &str,
    runtime_was_connected: bool,
) -> Result<(), CommandErrorDto> {
    let runtime_restore_result = async {
        if runtime_was_connected && !state.connected(rebound_space_id).await {
            state.remove_target(rebound_space_id).await;
            state.remove_managed_space_runtime(rebound_space_id).await;
            match rebound_runtime_rollback_settings(previous_settings, rebound_space_id)? {
                Some(rollback_settings) if has_managed_configuration(&rollback_settings) => {
                    state.select_target(Some(rebound_space_id.to_owned())).await;
                    managed_runtime::start(state, store, &rollback_settings, true).await
                }
                Some(rollback_settings) => {
                    set_unstarted_health(state, &rollback_settings).await;
                    Ok(())
                }
                None => Ok(()),
            }
        } else {
            Ok(())
        }
    }
    .await;
    restore_previous_space_selection(state, previous_settings).await;
    runtime_restore_result
}

async fn restore_previous_space_selection(state: &AppState, settings: &DesktopSettings) {
    state
        .select_target(settings.selected_target_id.clone())
        .await;
    if let Some(previous_space_id) = settings.selected_space_id.as_deref() {
        state.activate_managed_terminal_for(previous_space_id).await;
    }
}

async fn confirm_restore_space(app: &AppHandle, name: &str) -> Result<bool, CommandErrorDto> {
    let name = name.to_owned();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "{name} is archived. Restore and open this Workspace?"
            ))
            .title("Restore Workspace")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Restore Workspace".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "space_confirmation",
            "The native Workspace confirmation could not be opened.",
            true,
        )
    })
}

#[tauri::command]
pub(crate) async fn list_spaces(
    state: State<'_, AppState>,
) -> Result<Vec<SpaceSummaryDto>, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    Ok(space_summaries(&state, &settings).await)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchSpaceThreadsInput {
    query: String,
    #[serde(default)]
    space_id: Option<String>,
    #[serde(default)]
    include_archived: bool,
    #[serde(default)]
    cursor: String,
    #[serde(default = "default_search_page_size")]
    page_size: usize,
}

const fn default_search_page_size() -> usize {
    50
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn search_space_threads(
    state: State<'_, AppState>,
    request: SearchSpaceThreadsInput,
) -> Result<SpaceSearchPageDto, CommandErrorDto> {
    if request.query.len() > 128 || request.query.chars().any(char::is_control) {
        return Err(CommandErrorDto::invalid(
            "query",
            "Search queries must be at most 128 visible characters.",
        ));
    }
    if request.page_size == 0 || request.page_size > 100 {
        return Err(CommandErrorDto::invalid(
            "pageSize",
            "Search pages must contain between 1 and 100 results.",
        ));
    }
    let settings = settings_store()?.load()?;
    if request
        .space_id
        .as_ref()
        .is_some_and(|space_id| settings.space(space_id).is_none())
    {
        return Err(CommandErrorDto::invalid(
            "spaceId",
            "The Workspace is unknown.",
        ));
    }
    refresh_live_space_search_index(&state, &settings).await;
    let offset = if request.cursor.is_empty() {
        0
    } else {
        request
            .cursor
            .parse::<usize>()
            .map_err(|_| CommandErrorDto::invalid("cursor", "The search cursor is invalid."))?
    };
    space_search::search(
        &settings,
        &request.query,
        request.space_id.as_deref(),
        request.include_archived,
        offset,
        request.page_size,
    )
}

async fn refresh_live_space_search_index(state: &AppState, settings: &DesktopSettings) {
    let mut requests = Vec::new();
    for space_id in state.live_managed_target_ids().await {
        let Some(target) = state.target(&space_id).await else {
            continue;
        };
        requests.push(async move {
            let started = Instant::now();
            let mut runs = Vec::new();
            let mut page_token = String::new();
            let mut seen_tokens = BTreeSet::new();
            loop {
                let remaining = SPACE_SUMMARY_REFRESH_TIMEOUT.checked_sub(started.elapsed())?;
                let response = tokio::time::timeout(
                    remaining,
                    run_list::list_runs(
                        &target.client,
                        ListRunsRequest {
                            session_id: None,
                            statuses: Vec::new(),
                            page: Some(PageRequest {
                                page_size: SPACE_SEARCH_INDEX_PAGE_SIZE,
                                page_token,
                            }),
                            include_archived: false,
                        },
                    ),
                )
                .await
                .ok()?
                .ok()?;
                let available = MAX_SPACE_SEARCH_INDEX_RUNS.saturating_sub(runs.len());
                runs.extend(response.runs.into_iter().take(available).map(RunDto::from));
                let Some(next_page_token) = next_space_search_page_token(
                    response.page.as_ref(),
                    &mut seen_tokens,
                    runs.len(),
                ) else {
                    return Some((space_id, runs));
                };
                page_token = next_page_token;
            }
        });
    }
    for result in join_all(requests).await.into_iter().flatten() {
        let (space_id, runs) = result;
        let _ = space_search::index_runs(settings, &space_id, &runs);
    }
}

fn next_space_search_page_token(
    page: Option<&PageResponse>,
    seen_tokens: &mut BTreeSet<String>,
    indexed_runs: usize,
) -> Option<String> {
    if indexed_runs >= MAX_SPACE_SEARCH_INDEX_RUNS {
        return None;
    }
    let token = page?.next_page_token.clone();
    if token.is_empty() || !seen_tokens.insert(token.clone()) {
        return None;
    }
    Some(token)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn select_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let previous_settings = settings.clone();
    settings.activate_space(&space_id)?;
    store.save(&settings)?;
    state.select_target(None).await;
    if settings.managed_configured()
        && !state.connected(&space_id).await
        && let Err(error) = managed_runtime::start(&state, &store, &settings, false).await
    {
        store.save(&previous_settings)?;
        state
            .select_target(previous_settings.selected_target_id.clone())
            .await;
        if let Some(previous_space_id) = previous_settings.selected_space_id.as_deref() {
            state.activate_managed_terminal_for(previous_space_id).await;
        }
        return Err(error);
    }
    state.select_target(Some(space_id.clone())).await;
    state.touch_managed_space(&space_id).await;
    state.activate_managed_terminal_for(&space_id).await;
    desktop_status_from(&state, &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn rename_space(
    state: State<'_, AppState>,
    space_id: String,
    display_name: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let name = display_name.trim();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err(CommandErrorDto::invalid(
            "displayName",
            "Workspace names must be 1–80 visible characters.",
        ));
    }
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Workspace is unknown."))?;
    name.clone_into(&mut space.display_name);
    store.save(&settings)?;
    desktop_status_from(&state, &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn archive_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    if settings.space(&space_id).is_none() {
        return Err(CommandErrorDto::invalid(
            "spaceId",
            "The Workspace is unknown.",
        ));
    }
    reject_active_managed_runs_for(&state, &space_id).await?;
    if settings.selected_space_id.as_deref() == Some(&space_id) && state.terminal_session_active() {
        return Err(CommandErrorDto::busy(
            "Close this Workspace's terminal sessions before archiving it.",
        ));
    }
    if settings.selected_space_id.as_deref() == Some(&space_id) {
        state.select_target(None).await;
    }
    if let Some(target) = state.remove_target(&space_id).await {
        target
            .client
            .close()
            .await
            .map_err(|_| CommandErrorDto::busy("The Workspace runtime is still stopping."))?;
    }
    state.remove_managed_space_runtime(&space_id).await;
    if let Some(space) = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
    {
        space.archived = true;
    }
    if settings.selected_space_id.as_deref() == Some(&space_id) {
        let next = settings
            .spaces
            .iter()
            .filter(|space| !space.archived)
            .max_by_key(|space| space.last_opened_at_ms)
            .map(|space| space.id.clone());
        settings.selected_space_id = next.clone();
        settings.selected_target_id = next;
        settings.project_selected_space();
    }
    store.save(&settings)?;
    if let Some(selected) = settings.selected_space_id.clone() {
        if settings.managed_configured() && !state.connected(&selected).await {
            managed_runtime::start(&state, &store, &settings, false).await?;
        }
        state.select_target(Some(selected.clone())).await;
        state.activate_managed_terminal_for(&selected).await;
    }
    desktop_status_from(&state, &settings).await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn restore_space(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    let space = settings
        .spaces
        .iter_mut()
        .find(|space| space.id == space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Workspace is unknown."))?;
    space.archived = false;
    store.save(&settings)?;
    desktop_status_from(&state, &settings).await
}

#[cfg(test)]
fn persist_workspace_change(
    settings: &mut DesktopSettings,
    workspace: WorkspaceSetting,
    save_settings: impl FnOnce(&DesktopSettings) -> Result<(), CommandErrorDto>,
) -> Result<DesktopSettings, CommandErrorDto> {
    let previous = settings.clone();
    settings.add_space(workspace)?;
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
    if access_profile_elevation(&settings, request.access_profile)
        && !confirm_access_profile(&app, request.access_profile).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "access_profile_confirmation",
            "The requested access profile was not enabled.",
            false,
        ));
    }
    if execution_boundary_elevation(&settings, request.execution_boundary)
        && !confirm_execution_boundary(&app, request.execution_boundary).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "execution_boundary_confirmation",
            "The requested execution boundary was not enabled.",
            false,
        ));
    }
    if request.provider_kind == ProviderKindSetting::Codex {
        return configure_managed_codex_runtime(
            &app,
            state.inner(),
            &store,
            &mut settings,
            &mut request,
        )
        .await;
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
            .select_target(settings.selected_space_id.clone())
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
        .select_target(settings.selected_space_id.clone())
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

async fn configure_managed_codex_runtime(
    app: &AppHandle,
    state: &AppState,
    store: &SettingsStore,
    settings: &mut DesktopSettings,
    request: &mut ConfigureManagedRuntimeInput,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let provider_changed = settings
        .primary_provider()
        .is_none_or(|provider| provider.kind != ProviderKindSetting::Codex);
    let codex_origins = [
        "primary-provider: https://chatgpt.com".to_owned(),
        "primary-provider refresh: https://auth.openai.com".to_owned(),
    ];
    if provider_changed && !confirm_provider_origins(app, &codex_origins).await? {
        return Err(CommandErrorDto::local_sanitized(
            "provider_origin_confirmation",
            "The Codex provider origin change was not approved.",
            false,
        ));
    }
    crate::codex_auth::require_codex_auth_path()?;
    let previous_settings = settings.clone();
    let retired_ids = settings
        .provider_credential_ids()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if settings
        .pending_provider_cleanup_ids
        .len()
        .saturating_add(retired_ids.len())
        > MAX_PENDING_PROVIDER_CLEANUPS
    {
        return Err(CommandErrorDto::busy(
            "Pending provider credential cleanup must finish before selecting Codex.",
        ));
    }
    settings.providers = vec![ProviderSetting {
        profile: "primary-provider".into(),
        kind: ProviderKindSetting::Codex,
        base_url: provider_base_url(ProviderKindSetting::Codex).into(),
        credential_id: None,
        timeout_ms: None,
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
            image_inputs: false,
        },
        reasoning_effort: None,
    }];
    settings.model_roles = BTreeMap::from([("primary".into(), "primary".into())]);
    settings.access_profile = request.access_profile;
    settings.execution_boundary = request.execution_boundary;
    settings
        .selected_target_id
        .clone_from(&settings.selected_space_id);
    for credential_id in &retired_ids {
        if !settings
            .pending_provider_cleanup_ids
            .contains(credential_id)
        {
            settings
                .pending_provider_cleanup_ids
                .push(credential_id.clone());
        }
    }
    store.save(settings)?;
    state
        .select_target(settings.selected_space_id.clone())
        .await;
    if let Err(start_error) = managed_runtime::start(state, store, settings, true).await {
        store.save(&previous_settings)?;
        *settings = previous_settings;
        restore_managed_after_rollback(state, store, settings).await?;
        return Err(start_error);
    }
    for credential_id in retired_ids {
        retire_pending_provider_credential(store, settings, &credential_id)?;
    }
    desktop_status_from(state, settings).await
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
    if request
        .providers
        .iter()
        .any(|provider| provider.provider_kind == ProviderKindSetting::Codex)
    {
        crate::codex_auth::require_codex_auth_path()?;
    }

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
    settings.execution_boundary = request.execution_boundary;
    settings
        .selected_target_id
        .clone_from(&settings.selected_space_id);
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

    if access_profile_elevation(settings, request.access_profile)
        && !confirm_access_profile(app, request.access_profile).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "access_profile_confirmation",
            "The requested access profile was not enabled.",
            false,
        ));
    }
    if execution_boundary_elevation(settings, request.execution_boundary)
        && !confirm_execution_boundary(app, request.execution_boundary).await?
    {
        return Err(CommandErrorDto::local_sanitized(
            "execution_boundary_confirmation",
            "The requested execution boundary was not enabled.",
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
                .is_none_or(|current| {
                    current.kind != provider.provider_kind || current.base_url != provider.base_url
                })
        })
        .flat_map(|provider| {
            if provider.provider_kind == ProviderKindSetting::Codex {
                vec![
                    format!("{}: https://chatgpt.com", provider.profile),
                    format!("{} refresh: https://auth.openai.com", provider.profile),
                ]
            } else {
                vec![format!("{}: {}", provider.profile, provider.base_url)]
            }
        })
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
        .select_target(settings.selected_space_id.clone())
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
    let Some(target_id) = state.selected_target_id().await else {
        return Ok(());
    };
    reject_active_managed_runs_for(state, &target_id).await
}

pub(crate) async fn reject_active_managed_runs_for(
    state: &AppState,
    target_id: &str,
) -> Result<(), CommandErrorDto> {
    let Some(target) = state.target(target_id).await else {
        return Ok(());
    };
    if !matches!(target.consent, TargetConsentContext::ManagedLocal) {
        return Ok(());
    }
    let runs = run_list::list_runs(
        &target.client,
        ListRunsRequest {
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
            include_archived: false,
        },
    )
    .await
    .map_err(CommandErrorDto::from_api)?;
    if runs.runs.is_empty() {
        Ok(())
    } else {
        Err(CommandErrorDto::busy(
            "Finish or cancel active Managed Local runs before changing runtime settings.",
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
    request.provider_kind != ProviderKindSetting::Codex
        && !request.replace_credential
        && settings
            .primary_provider()
            .is_some_and(|provider| provider.kind == request.provider_kind)
}

fn access_profile_elevation(settings: &DesktopSettings, requested: AccessProfileSetting) -> bool {
    (!settings.managed_configured() && requested != AccessProfileSetting::Minimal)
        || access_profile_rank(requested) > access_profile_rank(settings.access_profile)
}

const fn access_profile_rank(profile: AccessProfileSetting) -> u8 {
    match profile {
        AccessProfileSetting::Minimal | AccessProfileSetting::Pinned => 0,
        AccessProfileSetting::Development => 1,
        AccessProfileSetting::AllowAll => 2,
    }
}

fn execution_boundary_elevation(
    settings: &DesktopSettings,
    requested: ExecutionBoundarySetting,
) -> bool {
    (!settings.managed_configured() && requested != ExecutionBoundarySetting::OfflineIsolated)
        || execution_boundary_rank(requested) > execution_boundary_rank(settings.execution_boundary)
}

const fn execution_boundary_rank(boundary: ExecutionBoundarySetting) -> u8 {
    match boundary {
        ExecutionBoundarySetting::OfflineIsolated => 0,
        ExecutionBoundarySetting::WorkspaceIsolated => 1,
        ExecutionBoundarySetting::FullAccess => 2,
    }
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

async fn confirm_access_profile(
    app: &AppHandle,
    profile: AccessProfileSetting,
) -> Result<bool, CommandErrorDto> {
    let (name, message) = match profile {
        AccessProfileSetting::Minimal => return Ok(true),
        AccessProfileSetting::Pinned => (
            "Pinned",
            "Pinned access denies every tool and action except the exact entries configured in Settings. The execution boundary and approval mode are configured separately.",
        ),
        AccessProfileSetting::Development => (
            "Development",
            "Development access lets Colossus use workspace-development tools. The execution boundary and approval mode are configured separately.",
        ),
        AccessProfileSetting::AllowAll => (
            "Allow all",
            "Allow all grants the Managed Local agent every declared built-in tool. The execution boundary and approval mode are configured separately.",
        ),
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "{message}\n\nEnable {name} access for Managed Local?"
            ))
            .title(format!("Enable Colossus {name} access"))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                format!("Enable {name}"),
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

async fn confirm_execution_boundary(
    app: &AppHandle,
    boundary: ExecutionBoundarySetting,
) -> Result<bool, CommandErrorDto> {
    let (name, message) = match boundary {
        ExecutionBoundarySetting::OfflineIsolated => return Ok(true),
        ExecutionBoundarySetting::WorkspaceIsolated => (
            "Workspace isolated",
            "Workspace isolated confines filesystem effects to the selected workspace while allowing explicitly configured provider destinations.",
        ),
        ExecutionBoundarySetting::FullAccess => (
            "Full access",
            "Unsafe: Full access runs commands without Colossus filesystem or network isolation. They can access files, environment variables, and network destinations available to your account. Policy, permits, approvals, and audit remain active, but approval mode is a separate setting.",
        ),
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "{message}\n\nEnable the {name} execution boundary for Managed Local?"
            ))
            .title(format!("Enable {name}"))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                format!("Enable {name}"),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "execution_boundary_confirmation",
            "The native execution-boundary confirmation could not be opened.",
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
    settings.execution_boundary = request.execution_boundary;
    settings
        .selected_target_id
        .clone_from(&settings.selected_space_id);
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
        timeout_ms: None,
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
            image_inputs: false,
        },
        reasoning_effort: None,
    }];
    settings.model_roles = std::collections::BTreeMap::from([("primary".into(), "primary".into())]);
    settings.access_profile = request.access_profile;
    settings.execution_boundary = request.execution_boundary;
    settings
        .selected_target_id
        .clone_from(&settings.selected_space_id);
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

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_thread_delegate(
    state: State<'_, AppState>,
    parent_run_id: String,
    job_id: String,
) -> Result<WorkerThreadDelegateInspection, CommandErrorDto> {
    if parent_run_id.is_empty() || parent_run_id.len() > 128 {
        return Err(CommandErrorDto::invalid(
            "parentRunId",
            "The parent run identifier is invalid.",
        ));
    }
    if job_id.is_empty() || job_id.len() > 128 {
        return Err(CommandErrorDto::invalid(
            "jobId",
            "The delegated agent identifier is invalid.",
        ));
    }
    let settings = settings_store()?.load()?;
    let space_id = settings.selected_space_id.as_deref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "jobId",
            "Select a Workspace before inspecting a delegated agent.",
        )
    })?;
    if state.selected_target_id().await.as_deref() != Some(space_id)
        || !state.managed_lifecycle_ready_for(space_id).await
    {
        return Err(CommandErrorDto::local_sanitized(
            "delegate_inspection_unavailable",
            "Delegated agent details are available only for the selected Managed Local Workspace.",
            false,
        ));
    }
    let worker = state.managed_worker_for(space_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "delegate_inspection_unavailable",
            "Delegated agent details are unavailable. Restart Managed Local and retry.",
            true,
        )
    })?;
    worker
        .inspect_thread_delegate(&parent_run_id, &job_id)
        .await
        .map_err(|_| {
            CommandErrorDto::local_sanitized(
                "delegate_inspection_unavailable",
                "The delegated agent details could not be loaded for this thread.",
                true,
            )
        })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_session_map(
    state: State<'_, AppState>,
    source_run_id: String,
) -> Result<WorkerSessionMap, CommandErrorDto> {
    if source_run_id.is_empty() || source_run_id.len() > 128 {
        return Err(CommandErrorDto::invalid(
            "sourceRunId",
            "The source run identifier is invalid.",
        ));
    }
    let settings = settings_store()?.load()?;
    let space_id = settings.selected_space_id.as_deref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "sourceRunId",
            "Select a Workspace before inspecting its session map.",
        )
    })?;
    if !state.managed_lifecycle_ready_for(space_id).await {
        return Err(CommandErrorDto::local_sanitized(
            "session_map_unavailable",
            "The session map is available only for the selected Managed Local Workspace.",
            false,
        ));
    }
    let target = state.selected_target(space_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "session_map_unavailable",
            "Select this Managed Local Workspace before inspecting its session map.",
            true,
        )
    })?;
    if !state.run_is_bound(&target, &source_run_id).await {
        return Err(CommandErrorDto::invalid(
            "sourceRunId",
            "Refresh this Workspace's work before inspecting the session map.",
        ));
    }
    let _unary_slot = target.target.try_unary_slot().ok_or_else(|| {
        CommandErrorDto::busy("The desktop request limit is active. Wait and retry.")
    })?;
    let response = target
        .target
        .client
        .get_run(GetRunRequest {
            run_id: source_run_id,
        })
        .await
        .map_err(CommandErrorDto::from_api)?;
    let worker = state.managed_worker_for(space_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "session_map_unavailable",
            "The session map is unavailable. Restart Managed Local and retry.",
            true,
        )
    })?;
    worker
        .inspect_session_map(&response.run.session_id)
        .await
        .map_err(|_| {
            CommandErrorDto::local_sanitized(
                "session_map_unavailable",
                "The session map could not be loaded for this thread.",
                true,
            )
        })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_approval_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    approval_mode: DesktopApprovalModeDto,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let settings = settings_store()?.load()?;
    let space_id = settings.selected_space_id.as_deref().ok_or_else(|| {
        CommandErrorDto::invalid(
            "approvalMode",
            "Select a Workspace before changing permission mode.",
        )
    })?;
    if state.selected_target_id().await.as_deref() != Some(space_id)
        || !state.managed_lifecycle_ready_for(space_id).await
    {
        return Err(CommandErrorDto::invalid(
            "approvalMode",
            "Permission mode can be changed only while Managed Local is selected and ready.",
        ));
    }
    let _run_admission = state.approval_mode_change_guard_for(space_id).await;
    state.refresh_approval_mode_for(space_id).await;
    let current = state.approval_mode_for(space_id).await;
    if current == approval_mode {
        return desktop_status_from(&state, &settings).await;
    }
    reject_active_managed_runs(&state).await?;
    if approval_mode.requires_native_confirmation_from(current) {
        let _approval_guard = state.try_approval_guard().ok_or_else(|| {
            CommandErrorDto::busy("Another native approval confirmation is already open.")
        })?;
        if !confirm_approval_mode(&app, approval_mode).await? {
            return desktop_status_from(&state, &settings).await;
        }
    }
    let worker = state.managed_worker_for(space_id).await.ok_or_else(|| {
        CommandErrorDto::local_sanitized(
            "approval_mode_unavailable",
            "Managed Local permission mode is unavailable. Restart Managed Local and retry.",
            true,
        )
    })?;
    let confirmed = worker
        .set_approval_mode(approval_mode.worker_mode())
        .await
        .map_err(|_| {
            CommandErrorDto::local_sanitized(
                "approval_mode_unavailable",
                "Managed Local permission mode could not be changed. Restart Managed Local and retry.",
                true,
            )
        })?;
    if DesktopApprovalModeDto::from_worker_mode(confirmed) != approval_mode {
        state.refresh_approval_mode_for(space_id).await;
        return Err(CommandErrorDto::local_sanitized(
            "approval_mode_invalid",
            "Managed Local returned an invalid permission mode response.",
            false,
        ));
    }
    state.set_approval_mode_for(space_id, approval_mode).await;
    desktop_status_from(&state, &settings).await
}

async fn confirm_approval_mode(
    app: &AppHandle,
    approval_mode: DesktopApprovalModeDto,
) -> Result<bool, CommandErrorDto> {
    let (title, message, allow_label) = match approval_mode {
        DesktopApprovalModeDto::RiskAuto => (
            "Enable automatic low-risk approvals?",
            "Risk auto lets the configured risk evaluator satisfy eligible low-risk approval obligations without asking. Other approval obligations still pause for confirmation. Policy denials, tool authority, and sandbox boundaries do not change.",
            "Enable risk auto",
        ),
        DesktopApprovalModeDto::FullAccess => (
            "Enable full approval access?",
            "Full access satisfies approval obligations without asking. Allowed tools may change or delete workspace data and perform configured network actions. Policy denials, tool authority, and sandbox boundaries do not change.",
            "Enable full access",
        ),
        DesktopApprovalModeDto::Deny | DesktopApprovalModeDto::Ask => return Ok(true),
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                allow_label.into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "approval_mode_confirmation",
            "The native permission-mode confirmation could not be opened.",
            true,
        )
    })
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
    if settings.spaces.iter().any(|space| space.id == target_id) {
        settings.activate_space(&target_id)?;
        if !state.connected(&target_id).await && settings.managed_configured() {
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
    state.select_target(Some(target_id.clone())).await;
    if settings.selected_space_id.as_deref() == Some(target_id.as_str()) {
        state.touch_managed_space(&target_id).await;
        state.activate_managed_terminal_for(&target_id).await;
    }
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
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let _guard = connect_guard(&state)?;
    let store = settings_store()?;
    let mut settings = store.load()?;
    if enabled
        && settings.local_terminal_consent_version != LOCAL_TERMINAL_CONSENT_VERSION
        && !confirm_local_terminal_access(&app).await?
    {
        return desktop_status_from(&state, &settings).await;
    }
    settings.terminal_enabled = enabled;
    if enabled {
        settings.local_terminal_consent_version = LOCAL_TERMINAL_CONSENT_VERSION;
    }
    store.save(&settings)?;
    state.set_terminal_enabled(enabled).await;
    desktop_status_from(&state, &settings).await
}

async fn confirm_local_terminal_access(app: &AppHandle) -> Result<bool, CommandErrorDto> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(
                "The embedded shell runs as your signed-in OS user and can read, change, or delete anything that user can access. Shell commands are outside Colossus policy and audit, and deliberately detached processes may outlive the app.\n\nOnly enable this on a trusted Colossus Desktop installation.",
            )
            .title("Enable local terminal access?")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Enable terminal".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| {
        CommandErrorDto::local_sanitized(
            "terminal_confirmation",
            "The native terminal confirmation could not be opened.",
            true,
        )
    })
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
        include_archived: false,
    };
    let mut probe = tauri::async_runtime::spawn(async move {
        // The permit lives with the actual request task. If the health deadline
        // expires, a non-cancellable platform-keychain read remains globally bounded.
        let _permit = permit;
        run_list::list_runs(&existing.client, request).await
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
    let Some(space_id) = settings.selected_space_id.as_deref() else {
        return;
    };
    if state.connected(space_id).await {
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
    state.set_managed_health_for(space_id, health).await;
}

async fn space_summaries(state: &AppState, settings: &DesktopSettings) -> Vec<SpaceSummaryDto> {
    let selected = settings.selected_space_id.as_deref();
    let attention = space_search::attention_counts(settings).unwrap_or_default();
    let last_activity = space_search::last_activity(settings).unwrap_or_default();
    let mut summaries = Vec::with_capacity(settings.spaces.len());
    for profile in &settings.spaces {
        let mut summary = SpaceSummaryDto::sleeping(profile, selected == Some(&profile.id));
        summary.attention_count = attention.get(&profile.id).copied().unwrap_or(0);
        summary.last_activity_at = last_activity.get(&profile.id).cloned();
        if selected == Some(profile.id.as_str()) {
            summary.provider_configured = settings.managed_configured();
        }
        if !profile.archived && state.connected(&profile.id).await {
            state.sync_managed_lifecycle_health_for(&profile.id).await;
            let health = state.managed_health_for(&profile.id).await;
            summary.state = managed_state_name(health.state).into();
            summary.message = health.message;
        } else if !profile.archived && !summary.provider_configured {
            summary.state = "needs_provider".into();
        }
        summaries.push(summary);
    }
    summaries.sort_by(|left, right| {
        left.archived
            .cmp(&right.archived)
            .then_with(|| right.last_opened_at_ms.cmp(&left.last_opened_at_ms))
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    summaries
}

async fn external_target_ready(state: &AppState, target_id: &str) -> bool {
    let (connected, health) = state.external_target_snapshot(target_id).await;
    connected && health.is_none_or(|health| health.state != "unreachable")
}

fn desktop_capabilities(
    state: &AppState,
    settings: &DesktopSettings,
    advertised: &ServerCapabilities,
    selected_managed: bool,
    workspace_available: bool,
) -> DesktopCapabilitiesDto {
    DesktopCapabilitiesDto {
        research: selected_managed && advertised.contains("research.create"),
        delegation: advertised.contains("agent_runs.delegation"),
        plugins: advertised.contains("plugins.discovery"),
        plugin_skill_selection: advertised.contains("plugins.skill_selection"),
        tui: selected_managed && cfg!(any(target_os = "macos", target_os = "windows")),
        shell_terminal: managed_workspace_is_selected(settings)
            && workspace_available
            && cfg!(target_os = "macos"),
        files: selected_managed
            && workspace_available
            && settings.access_profile != AccessProfileSetting::Minimal,
        artifacts: advertised.contains("artifacts.read"),
        plan_continuation: advertised.contains(PLAN_CONTINUATION_CAPABILITY),
        session_activity: advertised.contains(SESSION_ACTIVITY_CAPABILITY),
        update_available: state.update_available(),
        agent_workflows: advertised.contains("automation.workflows"),
        attachments: advertised.contains("attachments.run_input"),
    }
}

async fn selected_managed_health(
    state: &AppState,
    settings: &DesktopSettings,
    selected: Option<&str>,
) -> (Option<String>, bool, ManagedHealth) {
    let selected_managed_id = selected
        .filter(|target_id| settings.spaces.iter().any(|space| space.id == *target_id))
        .map(str::to_owned);
    if let Some(space_id) = selected_managed_id.as_deref() {
        state.sync_managed_lifecycle_health_for(space_id).await;
    }
    let managed_connected = match selected_managed_id.as_deref() {
        Some(space_id) => state.connected(space_id).await,
        None => false,
    };
    let managed_closed = match selected_managed_id.as_deref() {
        Some(space_id) => state.target_is_closed(space_id).await,
        None => false,
    };
    let managed_health = match selected_managed_id.as_deref() {
        Some(space_id) if managed_connected => state.managed_health_for(space_id).await,
        Some(_) if !settings.managed_configured() => ManagedHealth {
            state: ManagedRuntimeStateDto::NeedsProvider,
            message: "Configure a provider to start this Workspace.".into(),
            failure_code: None,
        },
        Some(_) => ManagedHealth {
            state: ManagedRuntimeStateDto::Starting,
            message: "This Workspace starts when selected.".into(),
            failure_code: None,
        },
        None => ManagedHealth::default(),
    };
    let health_was_ready = managed_health.state == ManagedRuntimeStateDto::Ready;
    let managed_health = managed_health_for_status(managed_health, managed_closed);
    if managed_closed
        && health_was_ready
        && let Some(space_id) = selected_managed_id.as_deref()
    {
        state
            .set_managed_health_for(space_id, managed_health.clone())
            .await;
    }
    (selected_managed_id, managed_connected, managed_health)
}

fn desktop_connection_status(
    selected: Option<&str>,
    settings: &DesktopSettings,
    managed_ready: bool,
    managed_health: &ManagedHealth,
    selected_external_ready: bool,
) -> ConnectionStatusDto {
    match selected {
        Some(target_id)
            if settings.spaces.iter().any(|space| space.id == target_id) && managed_ready =>
        {
            ConnectionStatusDto::connected(target_id)
        }
        Some(target_id) if settings.spaces.iter().any(|space| space.id == target_id) => {
            managed_connection_status(target_id, managed_health)
        }
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
    }
}

async fn desktop_status_from(
    state: &AppState,
    settings: &DesktopSettings,
) -> Result<DesktopStatusDto, CommandErrorDto> {
    let selected = state.selected_target_id().await;
    let selected_space_id = settings.selected_space_id.clone();
    let (selected_managed_id, managed_connected, managed_health) =
        selected_managed_health(state, settings, selected.as_deref()).await;
    let workspace = settings.workspace.as_ref().map(WorkspaceSummaryDto::from);
    let managed_state = managed_health.state;
    let managed_ready = managed_connected && managed_state == ManagedRuntimeStateDto::Ready;
    let spaces = space_summaries(state, settings).await;
    let mut targets = spaces
        .iter()
        .filter(|space| !space.archived)
        .filter_map(|space| {
            let profile = settings.space(&space.space_id)?;
            Some(RuntimeTargetDto {
                target_id: space.target_id.clone(),
                kind: RuntimeTargetKindDto::ManagedLocal,
                label: space.display_name.clone(),
                state: space.state.clone(),
                message: space.message.clone(),
                selected: selected.as_deref() == Some(space.target_id.as_str()),
                terminal_available: space.state == "ready"
                    && cfg!(any(target_os = "macos", target_os = "windows")),
                workspace: Some(WorkspaceSummaryDto::from(&profile.workspace)),
                failure_code: if selected.as_deref() == Some(space.target_id.as_str()) {
                    managed_health.failure_code
                } else {
                    None
                },
            })
        })
        .collect::<Vec<_>>();
    let mut selected_external_ready = false;
    for target in &settings.external_targets {
        let (status, ready) = external_target_status(state, target, selected.as_deref()).await;
        if status.selected {
            selected_external_ready = ready;
        }
        targets.push(status);
    }
    let connection = desktop_connection_status(
        selected.as_deref(),
        settings,
        managed_ready,
        &managed_health,
        selected_external_ready,
    );
    let selected_managed = selected_managed_id.is_some() && managed_ready;
    if managed_ready && let Some(space_id) = selected_managed_id.as_deref() {
        state.ensure_approval_mode_synchronized_for(space_id).await;
    }
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
    let capabilities = desktop_capabilities(
        state,
        settings,
        &advertised,
        selected_managed,
        workspace.is_some(),
    );
    Ok(DesktopStatusDto {
        release_channel: DesktopReleaseChannelDto::current(),
        connection,
        targets,
        selected_target_id: selected,
        spaces,
        selected_space_id,
        managed_state,
        workspace,
        provider: ProviderSummaryDto::from_settings(settings),
        codex_auth: crate::codex_auth::current_status(),
        managed_model_configuration: ManagedModelConfigurationDto::from_settings(settings),
        access_profile: settings.access_profile,
        execution_boundary: settings.execution_boundary,
        approval_mode: match selected_managed_id.as_deref() {
            Some(space_id) => state.approval_mode_for(space_id).await,
            None => DesktopApprovalModeDto::Ask,
        },
        terminal_enabled: settings.local_terminal_enabled(),
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

fn managed_connection_status(target_id: &str, health: &ManagedHealth) -> ConnectionStatusDto {
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
    ConnectionStatusDto::managed_for(target_id, state, &health.message)
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

pub(crate) fn settings_store() -> Result<SettingsStore, CommandErrorDto> {
    SettingsStore::open_application()
}

pub(crate) fn connect_guard(
    state: &AppState,
) -> Result<tokio::sync::MutexGuard<'_, ()>, CommandErrorDto> {
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
    settings
        .spaces
        .iter()
        .any(|space| space.id == target_id && !space.archived)
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

fn managed_workspace_is_selected(settings: &DesktopSettings) -> bool {
    settings.selected_target_id == settings.selected_space_id
        && settings.selected_space_id.is_some()
}

fn spawn_managed_start_on_initialize(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let Ok(_guard) = connect_guard(&state) else {
            return;
        };
        let Ok(store) = settings_store() else {
            return;
        };
        let Ok(settings) = store.load() else {
            return;
        };
        if should_start_managed_on_initialize(
            has_managed_configuration(&settings),
            match settings.selected_space_id.as_deref() {
                Some(space_id) => state.connected(space_id).await,
                None => false,
            },
        ) && managed_runtime::start(&state, &store, &settings, false)
            .await
            .is_ok()
            && let Some(space_id) = settings.selected_space_id.as_deref()
        {
            state.activate_managed_terminal_for(space_id).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_search_pagination_follows_unique_tokens_with_a_hard_run_bound() {
        let mut seen = BTreeSet::new();
        let page = PageResponse {
            next_page_token: "page-2".into(),
        };
        assert_eq!(
            next_space_search_page_token(Some(&page), &mut seen, 100).as_deref(),
            Some("page-2")
        );
        assert_eq!(
            next_space_search_page_token(Some(&page), &mut seen, 200),
            None,
            "a repeated server token must not loop forever"
        );
        assert_eq!(
            next_space_search_page_token(
                Some(&PageResponse {
                    next_page_token: String::new(),
                }),
                &mut seen,
                200,
            ),
            None
        );
        assert_eq!(
            next_space_search_page_token(
                Some(&PageResponse {
                    next_page_token: "page-3".into(),
                }),
                &mut seen,
                MAX_SPACE_SEARCH_INDEX_RUNS,
            ),
            None,
            "the live refresh must stay bounded"
        );
    }

    fn test_workspace_identity() -> colossus_sdk::WorkspaceIdentity {
        colossus_sdk::WorkspaceIdentity::from_macos_parts(1, 2, 1_700_000_000, 0)
            .expect("current workspace identity")
    }

    fn provider_request() -> ConfigureManagedRuntimeInput {
        ConfigureManagedRuntimeInput {
            workspace_id: Uuid::now_v7().to_string(),
            provider_kind: crate::desktop_settings::ProviderKindSetting::Compatible,
            model: "new-model".into(),
            access_profile: crate::desktop_settings::AccessProfileSetting::Development,
            execution_boundary:
                crate::desktop_settings::ExecutionBoundarySetting::WorkspaceIsolated,
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
                kind: crate::desktop_settings::ProviderKindSetting::Compatible,
                base_url: crate::desktop_settings::OPENROUTER_BASE_URL.into(),
                credential_id: Some(credential_id.into()),
                timeout_ms: Some(120_000),
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
                    image_inputs: false,
                },
                reasoning_effort: None,
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
        let mut settings = DesktopSettings {
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
        let managed_id = Uuid::now_v7().to_string();
        settings
            .add_space(WorkspaceSetting {
                id: managed_id.clone(),
                path: "/tmp/managed-workspace".into(),
                identity: Some(test_workspace_identity()),
                display_name: "managed-workspace".into(),
                display_path: "/tmp/managed-workspace".into(),
            })
            .expect("managed Space");
        assert!(validate_target(&settings, &managed_id).is_ok());
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
                Some(workspace.id.as_str())
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
    fn embedded_shell_selection_survives_transient_runtime_disconnects() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        let workspace = WorkspaceSetting {
            id: Uuid::now_v7().to_string(),
            path: "/tmp/selected-workspace".into(),
            identity: Some(test_workspace_identity()),
            display_name: "selected-workspace".into(),
            display_path: "/tmp/selected-workspace".into(),
        };
        settings.add_space(workspace).expect("selected Space");

        assert!(managed_workspace_is_selected(&settings));

        settings.selected_target_id = Some("external-target".into());
        assert!(!managed_workspace_is_selected(&settings));
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
    fn space_rebind_rollback_reactivates_the_previous_runtime_configuration() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        let selected_space_id = Uuid::now_v7().to_string();
        settings
            .add_space(WorkspaceSetting {
                id: selected_space_id.clone(),
                path: "/tmp/selected-workspace".into(),
                identity: Some(test_workspace_identity()),
                display_name: "selected-workspace".into(),
                display_path: "/tmp/selected-workspace".into(),
            })
            .expect("selected Space");
        let rebound_space_id = Uuid::now_v7().to_string();
        let rebound_workspace = WorkspaceSetting {
            id: rebound_space_id.clone(),
            path: "/tmp/rebound-workspace".into(),
            identity: Some(
                colossus_sdk::WorkspaceIdentity::from_macos_parts(3, 4, 1_700_000_001, 0)
                    .expect("rebound workspace identity"),
            ),
            display_name: "rebound-workspace".into(),
            display_path: "/tmp/rebound-workspace".into(),
        };
        settings
            .add_space(rebound_workspace.clone())
            .expect("rebound Space");
        settings
            .activate_space(&selected_space_id)
            .expect("restore original selection");

        let rollback = rebound_runtime_rollback_settings(&settings, &rebound_space_id)
            .expect("rollback settings")
            .expect("active rebound Space");

        assert_eq!(
            rollback.selected_space_id.as_deref(),
            Some(rebound_space_id.as_str())
        );
        assert_eq!(
            rollback.selected_target_id.as_deref(),
            Some(rebound_space_id.as_str())
        );
        assert_eq!(rollback.workspace.as_ref(), Some(&rebound_workspace));
        assert!(has_managed_configuration(&rollback));
        assert_eq!(
            settings.selected_space_id.as_deref(),
            Some(selected_space_id.as_str()),
            "building rollback settings must not disturb the durable selection"
        );
    }

    #[test]
    fn space_rebind_rollback_does_not_restart_an_archived_space() {
        let mut settings = settings_with_provider(&Uuid::now_v7().to_string());
        let rebound_space_id = Uuid::now_v7().to_string();
        settings
            .add_space(WorkspaceSetting {
                id: rebound_space_id.clone(),
                path: "/tmp/archived-workspace".into(),
                identity: Some(test_workspace_identity()),
                display_name: "archived-workspace".into(),
                display_path: "/tmp/archived-workspace".into(),
            })
            .expect("archived Space");
        settings
            .spaces
            .iter_mut()
            .find(|space| space.id == rebound_space_id)
            .expect("Space profile")
            .archived = true;

        assert!(
            rebound_runtime_rollback_settings(&settings, &rebound_space_id)
                .expect("rollback decision")
                .is_none()
        );
    }

    #[test]
    fn provider_reuse_is_native_and_never_crosses_provider_kinds() {
        let settings = settings_with_provider(&Uuid::now_v7().to_string());
        let mut request = provider_request();
        assert!(reusable_provider_credential(&settings, &request));

        request.replace_credential = true;
        assert!(!reusable_provider_credential(&settings, &request));
        request.replace_credential = false;
        request.provider_kind = crate::desktop_settings::ProviderKindSetting::Responses;
        assert!(!reusable_provider_credential(&settings, &request));
        assert!(!reusable_provider_credential(
            &DesktopSettings::default(),
            &provider_request(),
        ));
    }

    #[test]
    fn access_and_execution_authority_confirm_only_on_independent_elevation() {
        let mut minimal = settings_with_provider(&Uuid::now_v7().to_string());
        minimal.access_profile = AccessProfileSetting::Minimal;
        minimal.execution_boundary = ExecutionBoundarySetting::OfflineIsolated;
        assert!(access_profile_elevation(
            &minimal,
            AccessProfileSetting::Development
        ));
        assert!(access_profile_elevation(
            &minimal,
            AccessProfileSetting::AllowAll
        ));
        assert!(execution_boundary_elevation(
            &minimal,
            ExecutionBoundarySetting::WorkspaceIsolated
        ));
        assert!(execution_boundary_elevation(
            &minimal,
            ExecutionBoundarySetting::FullAccess
        ));

        let full = settings_with_provider(&Uuid::now_v7().to_string());
        assert!(!access_profile_elevation(
            &full,
            AccessProfileSetting::Development
        ));
        assert!(!execution_boundary_elevation(
            &full,
            ExecutionBoundarySetting::WorkspaceIsolated
        ));
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
