//! Product workflow resources bound to the selected target and native generation.
use crate::{
    commands::{target, unary_slot},
    desktop_settings::SettingsStore,
    dto::CommandErrorDto,
    state::{AppState, SelectedTargetLease, TargetConsentContext},
};
use colossus_sdk::{
    CreateWorkflowScheduleRequest, RegisteredWorkflow, SetWorkflowScheduleEnabledRequest,
    WorkflowClient, WorkflowPage, WorkflowRunSnapshot, WorkflowScheduleSnapshot,
};
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
    workflow_runs_read: bool,
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
        workflow_runs_read: capabilities.contains("workflow_runs.read"),
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
pub(crate) async fn get_scheduled_workflow_run(
    state: State<'_, AppState>,
    target_id: String,
    selection_epoch: u64,
    run_id: String,
) -> Result<WorkflowRunSnapshot, CommandErrorDto> {
    let lease = selected(&state, &target_id, selection_epoch).await?;
    let _slot = unary_slot(&lease.target)?;
    client(&lease)?
        .get_run(run_id)
        .await
        .map_err(CommandErrorDto::from_api)
}
