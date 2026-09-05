//! Native-owned target binding, file dialogs, and policy approval for plugin management.

use colossus_worker_protocol::PluginManagementRequest;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::plugin_adapter::{self, PluginInventoryDto, PluginPreviewInput, PluginPreviewKind};
use crate::{
    commands::{target, unary_slot},
    dto::CommandErrorDto,
    managed_diagnostics::worker_for,
    state::{AppState, TargetConsentContext},
};

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn get_plugin_inventory(
    state: State<'_, AppState>,
    target_id: String,
) -> Result<PluginInventoryDto, CommandErrorDto> {
    let selected = target(&state, &target_id).await?;
    let _slot = unary_slot(&selected.target)?;
    let managed = matches!(selected.target.consent, TargetConsentContext::ManagedLocal);
    if managed {
        let worker = worker_for(&state, &target_id).await?;
        return plugin_adapter::inventory(&worker)
            .await
            .map_err(operation_error);
    }
    let plugins = selected
        .target
        .client
        .plugins()
        .ok_or_else(unavailable)?
        .list()
        .await
        .map_err(CommandErrorDto::from_api)?;
    Ok(PluginInventoryDto {
        plugins,
        management_available: managed,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn read_plugin_preview(
    state: State<'_, AppState>,
    target_id: String,
    request: PluginPreviewInput,
) -> Result<Value, CommandErrorDto> {
    let selected = target(&state, &target_id).await?;
    let _slot = unary_slot(&selected.target)?;
    if matches!(selected.target.consent, TargetConsentContext::ManagedLocal) {
        return plugin_adapter::preview(&worker_for(&state, &target_id).await?, request)
            .await
            .map_err(operation_error);
    }
    let client = selected.target.client.plugins().ok_or_else(unavailable)?;
    match request.kind {
        PluginPreviewKind::Skill => {
            let skill = client
                .skill(&request.skill_id, &request.digest)
                .await
                .map_err(CommandErrorDto::from_api)?;
            Ok(json!({"instructions": skill.instructions, "digest": skill.digest}))
        }
        PluginPreviewKind::Resources => serde_json::to_value(
            client
                .resources(&request.skill_id, &request.digest)
                .await
                .map_err(CommandErrorDto::from_api)?,
        )
        .map_err(|_| invalid_response()),
        PluginPreviewKind::Resource => serde_json::to_value(
            client
                .resource(
                    &request.skill_id,
                    &request.digest,
                    request.path.as_deref().unwrap_or_default(),
                )
                .await
                .map_err(CommandErrorDto::from_api)?,
        )
        .map_err(|_| invalid_response()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PluginManageInput {
    operation_id: String,
    request: PluginManagementRequest,
    #[serde(default)]
    verify_archive: bool,
}

struct Registration<'a> {
    state: &'a AppState,
    id: String,
}
impl Drop for Registration<'_> {
    fn drop(&mut self) {
        if let Some((_, cancellation)) = self
            .state
            .plugin_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id)
        {
            cancellation.send_replace(true);
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn manage_plugin(
    app: AppHandle,
    state: State<'_, AppState>,
    target_id: String,
    input: PluginManageInput,
) -> Result<Value, CommandErrorDto> {
    let selected = target(&state, &target_id).await?;
    let _slot = unary_slot(&selected.target)?;
    if !matches!(selected.target.consent, TargetConsentContext::ManagedLocal) {
        return Err(unavailable());
    }
    uuid::Uuid::parse_str(&input.operation_id).map_err(|_| {
        CommandErrorDto::invalid("operationId", "Use a fresh operation identifier.")
    })?;
    let (cancel, cancellation) = tokio::sync::watch::channel(false);
    {
        let mut operations = state
            .plugin_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if operations.len() >= 4 || operations.contains_key(&input.operation_id) {
            return Err(CommandErrorDto::busy(
                "A plugin operation is already running. Wait or cancel it first.",
            ));
        }
        operations.insert(input.operation_id.clone(), (target_id.clone(), cancel));
    }
    let _registration = Registration {
        state: &state,
        id: input.operation_id,
    };
    let worker = worker_for(&state, &target_id).await?;
    // A renderer-supplied path never authorizes native filesystem access. Every external
    // import/export path is selected here while the selected-target lease remains held.
    let dialog_app = app.clone();
    let Some(request) = tauri::async_runtime::spawn_blocking(move || {
        choose_paths(&dialog_app, input.request, input.verify_archive)
    })
    .await
    .map_err(|_| invalid_response())??
    else {
        return Ok(json!({"cancelled": true}));
    };
    plugin_adapter::manage(&worker, request, cancellation, |prompt| {
        let app = app.clone();
        let target_id = target_id.clone();
        async move {
            let approve = prompt.choices.first()?.clone();
            let deny = prompt.choices.get(1).cloned().unwrap_or_else(|| "Deny".into());
            let detail = format!("Workspace: {target_id}\n\n{}\n\n{}\n\nThis operation may affect every Workspace sharing this Colossus home.", prompt.question, prompt.details);
            let accepted = tauri::async_runtime::spawn_blocking(move || app.dialog().message(detail)
                .title(prompt.title).kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(approve.clone(), deny)).blocking_show()).await.ok()?;
            accepted.then_some(prompt.choices[0].clone())
        }
    }).await.map_err(operation_error)
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn cancel_plugin_operation(
    state: State<'_, AppState>,
    target_id: String,
    operation_id: String,
) -> Result<(), CommandErrorDto> {
    let _selected = target(&state, &target_id).await?;
    let operations = state
        .plugin_operations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (owner, cancellation) = operations.get(&operation_id).ok_or_else(|| {
        CommandErrorDto::invalid("operationId", "This operation is no longer active.")
    })?;
    if owner != &target_id {
        return Err(unavailable());
    }
    cancellation.send_replace(true);
    Ok(())
}

fn choose_paths(
    app: &AppHandle,
    request: PluginManagementRequest,
    verify_archive: bool,
) -> Result<Option<PluginManagementRequest>, CommandErrorDto> {
    plugin_adapter::select_paths(request, verify_archive, |file, save| {
        let dialog = app.dialog().file();
        let selected = if save {
            dialog
                .set_title("Choose a new plugin output path")
                .blocking_save_file()
        } else if file {
            dialog
                .set_title("Select an OCI layout archive")
                .blocking_pick_file()
        } else {
            dialog
                .set_title("Select a plugin or OCI layout directory")
                .blocking_pick_folder()
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        Ok(Some(
            selected
                .into_path()
                .map_err(|_| {
                    CommandErrorDto::invalid("path", "The selected local path is unavailable.")
                })?
                .display()
                .to_string(),
        ))
    })
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "plugins_unavailable",
        "This target does not support the requested plugin operation. Management requires Managed Local.",
        false,
    )
}
fn invalid_response() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "plugin_response_invalid",
        "The runtime returned an invalid plugin response. Restart the target and retry.",
        true,
    )
}
fn operation_error(error: colossus_worker_protocol::WorkerControlError) -> CommandErrorDto {
    // Worker errors are already released by the policy/audit boundary. Never include the
    // local endpoint, authentication material, or transport internals in renderer output.
    match error {
        colossus_worker_protocol::WorkerControlError::Remote(message) => CommandErrorDto::local_sanitized("plugin_operation_failed", &message.chars().filter(|c| !c.is_control() || *c == '\n').take(4096).collect::<String>(), false),
        _ => CommandErrorDto { code: "plugin_operation_unknown".into(), message: "The plugin operation lost contact with the runtime. Refresh the inventory before retrying; a change may have committed.".into(), retryable: false, outcome_unknown: true, violations: Vec::new() },
    }
}
