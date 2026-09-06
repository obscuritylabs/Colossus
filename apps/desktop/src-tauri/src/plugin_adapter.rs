//! Typed native-to-worker plugin translation, shared with the opt-in acceptance bridge.
//! Target ownership, dialogs and fresh consent stay with the native command caller.

use colossus_sdk::{PluginInventoryEntry, PluginResourceEntry, PluginResourceRead};
use colossus_worker_protocol::{
    PluginInstallSource, PluginManagementPrompt, PluginManagementRequest, WorkerControlClient,
    WorkerControlError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::future::Future;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginInventoryDto {
    pub(crate) plugins: Vec<PluginInventoryEntry>,
    pub(crate) management_available: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PluginPreviewInput {
    pub(crate) kind: PluginPreviewKind,
    pub(crate) skill_id: String,
    pub(crate) digest: String,
    pub(crate) path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PluginPreviewKind {
    Skill,
    Resources,
    Resource,
}

pub(crate) async fn inventory(
    worker: &WorkerControlClient,
) -> Result<PluginInventoryDto, WorkerControlError> {
    Ok(PluginInventoryDto {
        plugins: serde_json::from_value(worker.plugins().await?).map_err(|_| invalid_response())?,
        management_available: true,
    })
}

pub(crate) async fn preview(
    worker: &WorkerControlClient,
    request: PluginPreviewInput,
) -> Result<Value, WorkerControlError> {
    let operation = match request.kind {
        PluginPreviewKind::Skill => PluginManagementRequest::SkillRead {
            skill_id: request.skill_id,
            digest: request.digest.clone(),
        },
        PluginPreviewKind::Resources => PluginManagementRequest::ResourceList {
            skill_id: request.skill_id,
            digest: request.digest.clone(),
        },
        PluginPreviewKind::Resource => PluginManagementRequest::ResourceRead {
            skill_id: request.skill_id,
            digest: request.digest.clone(),
            path: request.path.unwrap_or_default(),
        },
    };
    let value = worker.manage_plugin(operation).await?;
    match request.kind {
        PluginPreviewKind::Skill => {
            let instructions = value
                .get("instructions")
                .and_then(Value::as_str)
                .ok_or_else(invalid_response)?;
            Ok(json!({"instructions": instructions, "digest": request.digest}))
        }
        PluginPreviewKind::Resources => serde_json::from_value::<Vec<PluginResourceEntry>>(value)
            .and_then(serde_json::to_value)
            .map_err(|_| invalid_response()),
        PluginPreviewKind::Resource => serde_json::from_value::<PluginResourceRead>(value)
            .and_then(serde_json::to_value)
            .map_err(|_| invalid_response()),
    }
}

pub(crate) async fn manage<F, Fut>(
    worker: &WorkerControlClient,
    request: PluginManagementRequest,
    cancellation: tokio::sync::watch::Receiver<bool>,
    on_prompt: F,
) -> Result<Value, WorkerControlError>
where
    F: FnMut(PluginManagementPrompt) -> Fut + Send,
    Fut: Future<Output = Option<String>> + Send,
{
    worker
        .manage_plugin_interactive(request, cancellation, on_prompt)
        .await
}

/// Replace every native filesystem input with a host-selected path. `None` cancels
/// the entire request, including when the operator cancels the second dialog.
pub(crate) fn select_paths<E>(
    mut request: PluginManagementRequest,
    verify_archive: bool,
    mut select: impl FnMut(bool, bool) -> Result<Option<String>, E>,
) -> Result<Option<PluginManagementRequest>, E> {
    use PluginManagementRequest as Op;
    let mut paths = Vec::new();
    match &mut request {
        Op::Install {
            source:
                PluginInstallSource::Directory { path } | PluginInstallSource::Layout { path, .. },
            ..
        }
        | Op::Validate { path } => paths.push((path, false, false)),
        Op::Install {
            source: PluginInstallSource::Archive { path, .. },
            ..
        } => paths.push((path, true, false)),
        Op::Verify { path, .. } => paths.push((path, verify_archive, false)),
        Op::Package { directory, output } => {
            paths.push((directory, false, false));
            paths.push((output, false, true));
        }
        Op::Pull { output, .. } | Op::Export { output, .. } => paths.push((output, true, true)),
        Op::Push { layout, .. } => paths.push((layout, false, false)),
        _ => {}
    }
    for (path, file, save) in paths {
        let Some(selected) = select(file, save)? else {
            return Ok(None);
        };
        *path = selected;
    }
    Ok(Some(request))
}

fn invalid_response() -> WorkerControlError {
    WorkerControlError::Protocol("invalid plugin response".into())
}

#[cfg(test)]
#[path = "plugin_adapter_tests.rs"]
mod tests;
