//! Native-owned target binding, file dialogs, and policy approval for plugin management.

use colossus_worker_protocol::PluginManagementRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

#[cfg(windows)]
use crate::managed_configuration::FieldOverrideSetting;
use crate::plugin_adapter::{self, PluginInventoryDto, PluginPreviewInput, PluginPreviewKind};
use crate::{
    commands::{target, unary_slot},
    desktop_commands::{connect_guard, settings_store},
    dto::CommandErrorDto,
    managed_configuration_commands::persist_and_restart,
    managed_diagnostics::worker_for,
    managed_runtime,
    state::{AppState, TargetConsentContext},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutlookCompanionStatusDto {
    supported: bool,
    enabled: bool,
    active_digest: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn outlook_companion_status(
    state: State<'_, AppState>,
    space_id: String,
) -> Result<OutlookCompanionStatusDto, CommandErrorDto> {
    let settings = settings_store()?.load()?;
    let space = settings
        .space(&space_id)
        .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Workspace is unknown."))?;
    #[cfg(windows)]
    let active_digest = state.outlook_companion_digest_for(&space_id).await;
    #[cfg(not(windows))]
    let active_digest = {
        let _ = state;
        None
    };
    Ok(OutlookCompanionStatusDto {
        supported: cfg!(windows),
        enabled: space.outlook_companion_enabled,
        active_digest,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub(crate) async fn configure_outlook_companion(
    state: State<'_, AppState>,
    space_id: String,
    enabled: bool,
) -> Result<OutlookCompanionStatusDto, CommandErrorDto> {
    #[cfg(not(windows))]
    {
        let _ = (state, space_id, enabled);
        return Err(CommandErrorDto::invalid(
            "spaceId",
            "Classic Outlook integration requires Windows Desktop.",
        ));
    }
    #[cfg(windows)]
    {
        let _guard = connect_guard(&state)?;
        let store = settings_store()?;
        let mut settings = store.load()?;
        if settings.selected_space_id.as_deref() != Some(&space_id) {
            return Err(CommandErrorDto::invalid(
                "spaceId",
                "Select this Workspace before changing its Outlook connection.",
            ));
        }
        let previous = settings.clone();
        let space = settings
            .space(&space_id)
            .ok_or_else(|| CommandErrorDto::invalid("spaceId", "The Workspace is unknown."))?;
        if space.archived {
            return Err(CommandErrorDto::invalid(
                "spaceId",
                "Restore this Workspace first.",
            ));
        }
        let space = settings
            .spaces
            .iter_mut()
            .find(|space| space.id == space_id)
            .expect("space checked above");
        let portable_changed =
            disable_portable_outlook_connection(&mut space.configuration.field_overrides)?;
        if space.outlook_companion_enabled != enabled || portable_changed {
            if enabled {
                // Verify both the active signed digest and its companion handshake
                // before persisting a setting that would fail every future startup.
                let (probe, _) =
                    crate::outlook_companion::OutlookCompanion::start(store.home_root()?).await?;
                probe.stop().await;
            }
            space.outlook_companion_enabled = enabled;
            let drain =
                managed_runtime::drain_active_runs_for_configuration(&state, &space_id).await?;
            persist_and_restart(&state, &store, &mut settings, previous, &space_id).await?;
            drop(drain);
        }
        let active_digest = state.outlook_companion_digest_for(&space_id).await;
        Ok(OutlookCompanionStatusDto {
            supported: true,
            enabled,
            active_digest,
        })
    }
}

#[cfg(windows)]
fn disable_portable_outlook_connection(
    overrides: &mut Vec<FieldOverrideSetting>,
) -> Result<bool, CommandErrorDto> {
    let mut servers = overrides
        .iter()
        .find(|field| field.field_id == "plugins.mcpServers")
        .map_or_else(
            || json!({ crate::managed_configuration::PLUGIN_SERVER_PATCH_MARKER: true }),
            |field| field.value.clone(),
        );
    let servers = servers.as_object_mut().ok_or_else(|| {
        CommandErrorDto::invalid("plugins.mcpServers", "The plugin connections are invalid.")
    })?;
    let server = servers
        .entry("outlook-classic/mail")
        .or_insert_with(|| json!({}));
    let server = server.as_object_mut().ok_or_else(|| {
        CommandErrorDto::invalid("plugins.mcpServers", "The Outlook connection is invalid.")
    })?;
    server.insert("enabled".into(), Value::Bool(false));
    let value = Value::Object(servers.clone());
    if let Some(existing) = overrides
        .iter_mut()
        .find(|field| field.field_id == "plugins.mcpServers")
    {
        if existing.value == value {
            return Ok(false);
        }
        existing.value = value;
    } else {
        overrides.push(FieldOverrideSetting {
            field_id: "plugins.mcpServers".into(),
            value,
        });
    }
    Ok(true)
}

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
    #[cfg(windows)]
    let revokes_outlook = matches!(
        &request,
        PluginManagementRequest::Enable { name, .. }
            | PluginManagementRequest::Disable { name }
            | PluginManagementRequest::Update { name, .. }
            | PluginManagementRequest::Uninstall { name, .. }
            if name == "outlook-classic"
    );
    #[cfg(windows)]
    let force_revoke_on_success = match &request {
        PluginManagementRequest::Disable { name } if name == "outlook-classic" => true,
        PluginManagementRequest::Uninstall { name, digest, .. } if name == "outlook-classic" => {
            state
                .outlook_companion_snapshots()
                .await
                .iter()
                .any(|(_, active_digest)| active_digest == digest)
        }
        _ => false,
    };
    let result = plugin_adapter::manage(&worker, request, cancellation, |prompt| {
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
    }).await;
    #[cfg(windows)]
    if revokes_outlook {
        match &result {
            Ok(value)
                if force_revoke_on_success
                    && value.get("cancelled").and_then(Value::as_bool) != Some(true) =>
            {
                // A disable, or removal of the running digest, invalidates its
                // session even if another process re-enables it immediately.
                state.stop_all_outlook_companions().await;
            }
            Ok(_) | Err(colossus_worker_protocol::WorkerControlError::Remote(_)) => {
                // Updates can leave the old digest active, and a failed operation
                // can still have changed it. Keep a healthy helper when it matches.
                crate::outlook_companion::reconcile_active_digest(&state).await;
            }
            Err(_) => {
                // A lost worker response leaves the operation outcome unknown.
                state.stop_all_outlook_companions().await;
            }
        }
    }
    result.map_err(operation_error)
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn disconnect_persists_portable_stdio_disabled_without_copying_other_servers() {
        let mut fields = vec![FieldOverrideSetting {
            field_id: "plugins.mcpServers".into(),
            value: json!({
                "outlook-classic/mail": {"enabled": true, "allowedTools": ["get_status"]},
                "local/docs": {"enabled": true}
            }),
        }];
        assert!(disable_portable_outlook_connection(&mut fields).expect("disable"));
        assert_eq!(fields[0].value["outlook-classic/mail"]["enabled"], false);
        assert_eq!(
            fields[0].value["outlook-classic/mail"]["allowedTools"],
            json!(["get_status"])
        );
        assert_eq!(fields[0].value["local/docs"]["enabled"], true);
        assert!(!disable_portable_outlook_connection(&mut fields).expect("idempotent"));
    }

    #[test]
    fn disconnect_without_local_override_uses_a_server_patch() {
        let mut fields = Vec::new();
        assert!(disable_portable_outlook_connection(&mut fields).expect("disable"));
        assert_eq!(fields[0].value["$colossusPatchV1"], true);
        assert_eq!(fields[0].value["outlook-classic/mail"]["enabled"], false);
    }
}
