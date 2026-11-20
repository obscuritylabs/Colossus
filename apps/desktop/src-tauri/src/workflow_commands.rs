//! Product workflow resources bound to the selected target and native generation.
use crate::{
    commands::{target, unary_slot},
    desktop_settings::SettingsStore,
    dto::CommandErrorDto,
    state::{AppState, MAX_LIVE_MANAGED_SPACES, SelectedTargetLease, TargetConsentContext},
};
use colossus_contracts::always_on::AlwaysOnProfile;
use colossus_sdk::{
    CreateWorkflowScheduleRequest, DeleteWorkflowScheduleRequest, DeletedWorkflowSchedule,
    RegisteredWorkflow, SetWorkflowScheduleEnabledRequest, StartWorkflowRunRequest, WorkflowClient,
    WorkflowPage, WorkflowRunSnapshot, WorkflowScheduleSnapshot,
};
use colossus_worker_protocol::AlwaysOnCommand;
use futures_util::{StreamExt as _, stream};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;
use tokio::sync::OwnedRwLockReadGuard;

#[derive(Serialize)]
// These independent advertised permissions form the renderer capability DTO.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct WorkflowContext {
    selection_epoch: u64,
    workflows_read: bool,
    workflows_register: bool,
    schedules_read: bool,
    schedules_create: bool,
    schedules_control: bool,
    schedules_delete: bool,
    workflow_runs_read: bool,
    workflow_runs_start: bool,
    calendar_schedules: bool,
    task_schedules: bool,
    workflow_run_history: bool,
    managed: bool,
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn workflow_context(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<WorkflowContext, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let capabilities = lease.target.client.capabilities();
    Ok(WorkflowContext {
        selection_epoch: lease.epoch(),
        workflows_read: capabilities.contains("workflows.read"),
        workflows_register: capabilities.contains("workflows.register"),
        schedules_read: capabilities.contains("schedules.read"),
        schedules_create: capabilities.contains("schedules.create"),
        schedules_control: capabilities.contains("schedules.control"),
        schedules_delete: capabilities.contains("schedules.delete"),
        workflow_runs_read: capabilities.contains("workflow_runs.read"),
        workflow_runs_start: capabilities.contains("workflow_runs.start"),
        calendar_schedules: capabilities.contains("schedules.calendar"),
        task_schedules: capabilities.contains("schedules.tasks"),
        workflow_run_history: capabilities.contains("workflow_runs.history"),
        managed: matches!(lease.target.consent, TargetConsentContext::ManagedLocal),
    })
}
async fn selected<'a>(
    state: &'a AppState,
    id: &str,
    epoch: u64,
) -> Result<SelectedTargetLease<'a>, CommandErrorDto> {
    let lease = target(state, id).await?;
    if lease.epoch() != epoch {
        return Err(CommandErrorDto::invalid(
            "selectionEpoch",
            "The Workspace changed. Refresh and review this operation again.",
        ));
    }
    Ok(lease)
}
fn client(lease: &SelectedTargetLease<'_>) -> Result<Arc<dyn WorkflowClient>, CommandErrorDto> {
    lease.target.client.workflows().ok_or_else(|| CommandErrorDto::invalid("targetId", "This runtime does not advertise workflow resources. Ask its administrator to enable and enroll the required scopes."))
}
async fn mutation_guard(
    state: &AppState,
    lease: &SelectedTargetLease<'_>,
    id: &str,
) -> Result<Option<OwnedRwLockReadGuard<()>>, CommandErrorDto> {
    if !matches!(lease.target.consent, TargetConsentContext::ManagedLocal) {
        return Ok(None);
    }
    let settings = SettingsStore::open_application()?.load()?;
    let workspace = settings
        .space(id)
        .ok_or_else(|| CommandErrorDto::invalid("targetId", "The managed Workspace is unknown."))?;
    if workspace.configuration.accepted_global_revision < settings.global_configuration.revision {
        return Err(CommandErrorDto::busy(
            "This Workspace has a saved settings update. Wait for it to apply, then refresh and review again.",
        ));
    }
    if state.configuration_draining_for(id).await {
        return Err(CommandErrorDto::busy(
            "This Workspace is applying configuration. Wait for the restart, then refresh and review again.",
        ));
    }
    let guard = state.run_creation_guard_for(id).await;
    if state.configuration_draining_for(id).await {
        return Err(CommandErrorDto::busy(
            "This Workspace is applying configuration. Wait for the restart, then refresh and review again.",
        ));
    }
    Ok(Some(guard))
}
fn bounded_yaml(yaml: &str) -> Result<(), CommandErrorDto> {
    if yaml.len() > 192 * 1024 {
        return Err(CommandErrorDto::invalid(
            "yaml",
            "Definition YAML must be no larger than 192 KiB.",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopAlwaysOnStatus {
    selection_epoch: u64,
    profile: Option<AlwaysOnProfile>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum AlwaysOnWorkspaceStatus {
    Enrolled { profile: AlwaysOnProfile },
    Unenrolled,
    Unavailable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesktopAlwaysOnWorkspace {
    space_id: String,
    display_name: String,
    #[serde(flatten)]
    status: AlwaysOnWorkspaceStatus,
}

/// Read existing owner-scoped profiles without changing selection or starting workers.
#[tauri::command]
pub(crate) async fn list_desktop_always_on(
    state: State<'_, AppState>,
) -> Result<Vec<DesktopAlwaysOnWorkspace>, CommandErrorDto> {
    let settings = SettingsStore::open_application()?.load()?;
    let live_targets = state.live_managed_target_ids().await;
    let state = state.inner();
    let rows = stream::iter(settings.spaces.into_iter().filter(|space| !space.archived))
        .map(|space| {
            let live = live_targets.contains(&space.id);
            async move {
                let status = if live {
                    read_always_on_workspace(state, &space.id).await
                } else {
                    AlwaysOnWorkspaceStatus::Unavailable
                };
                DesktopAlwaysOnWorkspace {
                    space_id: space.id,
                    display_name: space.display_name,
                    status,
                }
            }
        })
        .buffered(MAX_LIVE_MANAGED_SPACES)
        .collect()
        .await;
    Ok(rows)
}

async fn read_always_on_workspace(state: &AppState, space_id: &str) -> AlwaysOnWorkspaceStatus {
    let Some(target) = state
        .target(space_id)
        .await
        .filter(|target| matches!(target.consent, TargetConsentContext::ManagedLocal))
    else {
        return AlwaysOnWorkspaceStatus::Unavailable;
    };
    let Ok(_slot) = unary_slot(&target) else {
        return AlwaysOnWorkspaceStatus::Unavailable;
    };
    let Ok(worker) = crate::managed_diagnostics::worker_for(state, space_id).await else {
        return AlwaysOnWorkspaceStatus::Unavailable;
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        worker.always_on(AlwaysOnCommand::Status),
    )
    .await;
    if !state.managed_lifecycle_ready_for(space_id).await
        || !state
            .target(space_id)
            .await
            .is_some_and(|current| current.same_registration(&target))
    {
        return AlwaysOnWorkspaceStatus::Unavailable;
    }
    match result {
        Ok(Ok(Some(profile))) => AlwaysOnWorkspaceStatus::Enrolled { profile },
        Ok(Ok(None)) => AlwaysOnWorkspaceStatus::Unenrolled,
        Ok(Err(_)) | Err(_) => AlwaysOnWorkspaceStatus::Unavailable,
    }
}

async fn always_on_command(
    state: &AppState,
    lease: &SelectedTargetLease<'_>,
    target_id: &str,
    command: AlwaysOnCommand,
) -> Result<Option<AlwaysOnProfile>, CommandErrorDto> {
    if !matches!(lease.target.consent, TargetConsentContext::ManagedLocal) {
        return Err(CommandErrorDto::invalid(
            "targetId",
            "Always On is available in a managed local Workspace.",
        ));
    }
    let _slot = unary_slot(&lease.target)?;
    crate::managed_diagnostics::worker_for(state, target_id)
        .await?
        .always_on(command)
        .await
        .map_err(|_| {
            CommandErrorDto::local_sanitized(
                "always_on_unavailable",
                "Could not verify Always On state. Refresh this Workspace before trying again.",
                true,
            )
        })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn desktop_always_on_status(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<DesktopAlwaysOnStatus, CommandErrorDto> {
    let lease = target(&state, &target_id).await?;
    let profile = always_on_command(&state, &lease, &target_id, AlwaysOnCommand::Status).await?;
    Ok(DesktopAlwaysOnStatus {
        selection_epoch: lease.epoch(),
        profile,
    })
}

async fn mutate_always_on(
    state: &AppState,
    target_id: &str,
    selection_epoch: u64,
    command: AlwaysOnCommand,
) -> Result<AlwaysOnProfile, CommandErrorDto> {
    let lease = selected(state, target_id, selection_epoch).await?;
    let _guard = mutation_guard(state, &lease, target_id).await?;
    always_on_command(state, &lease, target_id, command)
        .await?
        .ok_or_else(|| {
            CommandErrorDto::local_sanitized(
                "always_on_unavailable",
                "The worker did not return an Always On profile.",
                true,
            )
        })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn desktop_always_on_enable(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    session_id: String,
    expected_revision: u64,
) -> Result<AlwaysOnProfile, CommandErrorDto> {
    mutate_always_on(
        &state,
        &target_id,
        selection_epoch,
        AlwaysOnCommand::Enable {
            session_id,
            expected_revision,
        },
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn desktop_always_on_pause(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    expected_revision: u64,
) -> Result<AlwaysOnProfile, CommandErrorDto> {
    mutate_always_on(
        &state,
        &target_id,
        selection_epoch,
        AlwaysOnCommand::Pause { expected_revision },
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn desktop_always_on_wake(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    expected_revision: u64,
) -> Result<AlwaysOnProfile, CommandErrorDto> {
    mutate_always_on(
        &state,
        &target_id,
        selection_epoch,
        AlwaysOnCommand::Wake { expected_revision },
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_registered_workflows(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    after: Option<String>,
) -> Result<WorkflowPage<RegisteredWorkflow>, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .list_workflows(after, 32)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_registered_workflow(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    workflow_id: String,
) -> Result<RegisteredWorkflow, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .get_workflow(workflow_id)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn validate_workflow_definition(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    yaml: String,
) -> Result<RegisteredWorkflow, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    bounded_yaml(&yaml)?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .validate_definition(yaml)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn register_workflow_definition(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    yaml: String,
    expected_hash: String,
    idempotency_key: String,
) -> Result<RegisteredWorkflow, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _guard = mutation_guard(&state, &lease, &target_id).await?;
    bounded_yaml(&yaml)?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .register_definition(yaml, expected_hash, idempotency_key)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_workflow_schedules(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    after: Option<String>,
) -> Result<WorkflowPage<WorkflowScheduleSnapshot>, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .list_schedules(after, 32)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_workflow_schedule(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    schedule_id: String,
) -> Result<WorkflowScheduleSnapshot, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .get_schedule(schedule_id)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn create_workflow_schedule(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    request: CreateWorkflowScheduleRequest,
) -> Result<WorkflowScheduleSnapshot, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _guard = mutation_guard(&state, &lease, &target_id).await?;
    if serde_json::to_vec(&request.inputs).map_or(true, |bytes| bytes.len() > 64 * 1024) {
        return Err(CommandErrorDto::invalid(
            "inputs",
            "Inputs must be no larger than 64 KiB.",
        ));
    }
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .create_schedule(request)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn set_workflow_schedule_enabled(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    request: SetWorkflowScheduleEnabledRequest,
) -> Result<WorkflowScheduleSnapshot, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _guard = mutation_guard(&state, &lease, &target_id).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .set_schedule_enabled(request)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn delete_workflow_schedule(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    request: DeleteWorkflowScheduleRequest,
) -> Result<DeletedWorkflowSchedule, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _guard = mutation_guard(&state, &lease, &target_id).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .delete_schedule(request)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_scheduled_workflow_run(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    run_id: String,
) -> Result<WorkflowRunDto, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .get_run(run_id)
        .await
        .map(WorkflowRunDto::from)
        .map_err(CommandErrorDto::from_api)
}

/// Preserve final JSON numbers across JavaScript's double-precision boundary.
#[derive(Serialize)]
pub(crate) struct WorkflowRunDto {
    #[serde(flatten)]
    state: WorkflowRunSnapshot,
    result_json: Option<String>,
}
impl From<WorkflowRunSnapshot> for WorkflowRunDto {
    fn from(mut state: WorkflowRunSnapshot) -> Self {
        let result_json = state.result.take().map(|result| {
            serde_json::to_string_pretty(&result)
                .ok()
                .filter(|json| json.len() <= 64 * 1024)
                .unwrap_or_else(|| result.to_string())
        });
        Self { state, result_json }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn list_workflow_runs(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    workflow_id: String,
    after: Option<String>,
) -> Result<WorkflowPage<WorkflowRunSnapshot>, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .list_runs(workflow_id, after, 16)
        .await
        .map_err(CommandErrorDto::from_api)
}
#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn start_workflow_run(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    request: StartWorkflowRunRequest,
) -> Result<WorkflowRunSnapshot, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _guard = mutation_guard(&state, &lease, &target_id).await?;
    if serde_json::to_vec(&request.inputs).map_or(true, |bytes| bytes.len() > 64 * 1024) {
        return Err(CommandErrorDto::invalid(
            "inputs",
            "Inputs must be no larger than 64 KiB.",
        ));
    }
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .start_run(request)
        .await
        .map_err(CommandErrorDto::from_api)
}
