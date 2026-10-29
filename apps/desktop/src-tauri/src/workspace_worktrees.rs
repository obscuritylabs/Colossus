//! Human-only managed worktree controls. No SDK/agent tool exposes mutations.
use colossus_worktrees::{
    CreateWorktree, WorkspaceLease, WorktreeCatalog, WorktreeError, WorktreeErrorCode,
    WorktreeManager, WorktreeSettings, acquire_workspace_lease,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, State, Webview};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};
use tokio::sync::Semaphore;

use crate::{
    desktop_dto::DesktopStatusDto,
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, SettingsStore, WorkspaceSetting,
        revalidate_workspace,
    },
    dto::CommandErrorDto,
    state::AppState,
};

pub(crate) struct WorkspaceWorktreeState {
    gate: Arc<Semaphore>,
    prompt: Arc<Semaphore>,
    approved: Mutex<Option<Context>>,
    selected: Mutex<Option<(PathBuf, WorkspaceLease)>>,
}
impl Default for WorkspaceWorktreeState {
    fn default() -> Self {
        Self {
            gate: Arc::new(Semaphore::new(1)),
            prompt: Arc::new(Semaphore::new(1)),
            approved: Mutex::new(None),
            selected: Mutex::new(None),
        }
    }
}
impl WorkspaceWorktreeState {
    pub(crate) fn retain_selected_target(
        &self,
        settings: &DesktopSettings,
        target: Option<&str>,
    ) -> Result<(), CommandErrorDto> {
        let workspace = target
            .and_then(|id| settings.space(id))
            .map(|space| space.workspace.path.as_path());
        self.set_selected_workspace(workspace)
    }

    /// Call from native selection transitions, including initial restoration.
    /// Acquire the replacement before dropping the previous selection's lease.
    pub(crate) fn set_selected_workspace(
        &self,
        path: Option<&Path>,
    ) -> Result<(), CommandErrorDto> {
        if self
            .selected
            .lock()
            .map_err(|_| error("Worktree selection is unavailable."))?
            .as_ref()
            .is_some_and(|(selected, _)| Some(selected.as_path()) == path)
        {
            return Ok(());
        }
        let next = path
            .map(|path| {
                acquire_workspace_lease(path)
                    .map(|lease| lease.map(|lease| (path.to_owned(), lease)))
            })
            .transpose()
            .map_err(|failure| service_error(&failure))?
            .flatten();
        *self
            .selected
            .lock()
            .map_err(|_| error("Worktree selection is unavailable."))? = next;
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
struct Context {
    workspace: WorkspaceSetting,
    target: String,
    epoch: u64,
}
impl Context {
    fn capture(state: &AppState, workspace_id: &str) -> Result<Self, CommandErrorDto> {
        let epoch = *state.subscribe_selection().borrow();
        let context = Self::from_settings(
            SettingsStore::open_application()?.load()?,
            workspace_id,
            epoch,
        )?;
        if !state.selection_is_current(&context.target, epoch) {
            return Err(error("The selected workspace changed. Refresh worktrees."));
        }
        Ok(context)
    }
    fn from_settings(
        settings: DesktopSettings,
        workspace_id: &str,
        epoch: u64,
    ) -> Result<Self, CommandErrorDto> {
        let workspace = settings
            .workspace
            .filter(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| error("Choose the current local workspace to manage worktrees."))?;
        let target = settings
            .selected_space_id
            .ok_or_else(|| error("Worktrees require a local workspace."))?;
        if settings.selected_target_id.as_deref() != Some(&target)
            || settings.access_profile == AccessProfileSetting::Minimal
        {
            return Err(error(
                "Worktrees require the selected local workspace with file access enabled.",
            ));
        }
        revalidate_workspace(&workspace)?;
        Ok(Self {
            workspace,
            target,
            epoch,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListRequest {
    workspace_id: String,
    #[serde(default)]
    approve_repository: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorktreeSnapshot {
    state: &'static str,
    catalog: Option<WorktreeCatalog>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Create { request: CreateWorktree },
    Open { id: String },
    Archive { id: String },
    Remove { id: String },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionRequest {
    workspace_id: String,
    action: Action,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionResult {
    catalog: WorktreeCatalog,
    opened: Option<DesktopStatusDto>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preferences {
    directory: PathBuf,
    settings: WorktreeSettings,
}

fn error(message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized("worktree_unavailable", message, false)
}
fn unknown() -> CommandErrorDto {
    CommandErrorDto { code: "worktree_outcome_unknown".into(), message: "The worktree operation may have completed. Refresh the inventory before another action; do not repeat it automatically.".into(), retryable: false, outcome_unknown: true, violations: Vec::new() }
}
fn service_error(value: &WorktreeError) -> CommandErrorDto {
    let code = match value.code {
        WorktreeErrorCode::InvalidInput => "invalid_input",
        WorktreeErrorCode::UnsupportedRepository => "unsupported_repository",
        WorktreeErrorCode::Ownership => "ownership",
        WorktreeErrorCode::Busy => "busy",
        WorktreeErrorCode::Dirty => "dirty",
        WorktreeErrorCode::Conflict => "conflict",
        WorktreeErrorCode::NotFound => "not_found",
        WorktreeErrorCode::Limit => "limit",
        WorktreeErrorCode::Io => "io",
        WorktreeErrorCode::OutcomeUnknown => "outcome_unknown",
    };
    CommandErrorDto {
        code: format!("worktree_{code}"),
        message: value.message.into(),
        retryable: false,
        outcome_unknown: value.code == WorktreeErrorCode::OutcomeUnknown,
        violations: Vec::new(),
    }
}
pub(crate) fn workspace_guard(path: &Path) -> Result<Option<WorkspaceLease>, CommandErrorDto> {
    acquire_workspace_lease(path).map_err(|failure| service_error(&failure))
}
async fn blocking<T: Send + 'static>(
    lane: &WorkspaceWorktreeState,
    mutation: bool,
    action: impl FnOnce() -> Result<T, CommandErrorDto> + Send + 'static,
) -> Result<T, CommandErrorDto> {
    let permit = Arc::clone(&lane.gate)
        .try_acquire_owned()
        .map_err(|_| error("A worktree operation is still running. Refresh after it finishes."))?;
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        action()
    });
    tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .map_err(|_| {
            if mutation {
                unknown()
            } else {
                error("Worktree inspection is taking too long. Refresh shortly.")
            }
        })?
        .map_err(|_| {
            if mutation {
                unknown()
            } else {
                error("Worktree inspection could not finish.")
            }
        })?
}
async fn confirm(
    app: AppHandle,
    lane: &WorkspaceWorktreeState,
    title: &'static str,
    message: String,
    button: &'static str,
) -> Result<(), CommandErrorDto> {
    let permit = Arc::clone(&lane.prompt)
        .try_acquire_owned()
        .map_err(|_| error("A worktree confirmation is already open."))?;
    let approved = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        app.dialog()
            .message(message)
            .title(title)
            .buttons(MessageDialogButtons::OkCancelCustom(
                button.into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|_| error("The worktree confirmation could not open."))?;
    if approved {
        Ok(())
    } else {
        Err(CommandErrorDto::local_sanitized(
            "cancelled",
            "Worktree operation cancelled.",
            false,
        ))
    }
}

#[tauri::command]
pub(crate) async fn get_workspace_worktrees(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    lane: State<'_, WorkspaceWorktreeState>,
    request: ListRequest,
) -> Result<WorktreeSnapshot, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let context = Context::capture(&state, &request.workspace_id)?;
    let approved = lane
        .approved
        .lock()
        .map_err(|_| error("Worktree approval is unavailable."))?
        .as_ref()
        == Some(&context);
    if !approved {
        if !request.approve_repository {
            return Ok(WorktreeSnapshot {
                state: "permission_required",
                catalog: None,
            });
        }
        confirm(app, &lane, "Manage repository worktrees", "Allow Colossus to inspect this selected repository and its shared Git metadata, and keep managed checkouts in your private worktree directory?\n\nCreate, open, archive and remove each require a separate confirmation. Git hooks, commands and network requests do not run.".into(), "Allow worktrees").await?;
        if context != Context::capture(&state, &request.workspace_id)? {
            return Err(error(
                "The workspace changed during confirmation. Refresh worktrees.",
            ));
        }
        *lane
            .approved
            .lock()
            .map_err(|_| error("Worktree approval is unavailable."))? = Some(context.clone());
    }
    let operation = context.clone();
    let catalog = blocking(&lane, false, move || {
        WorktreeManager::open(&revalidate_workspace(&operation.workspace)?, None)
            .and_then(|manager| manager.list())
            .map_err(|failure| service_error(&failure))
    })
    .await?;
    if context != Context::capture(&state, &request.workspace_id)? {
        return Err(error("The selected workspace changed. Refresh worktrees."));
    }
    Ok(WorktreeSnapshot {
        state: "ready",
        catalog: Some(catalog),
    })
}

#[tauri::command]
pub(crate) async fn workspace_worktree_action(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    lane: State<'_, WorkspaceWorktreeState>,
    request: ActionRequest,
) -> Result<ActionResult, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let context = Context::capture(&state, &request.workspace_id)?;
    if lane
        .approved
        .lock()
        .map_err(|_| error("Worktree approval is unavailable."))?
        .as_ref()
        != Some(&context)
    {
        return Err(error(
            "Allow worktree access for the current workspace first.",
        ));
    }
    let (message, button) = match &request.action {
        Action::Create { request } => {
            request
                .validate()
                .map_err(|failure| service_error(&failure))?;
            (
                format!(
                    "Create managed worktree '{}' for the selected repository?\n\nA separate checkout and branch will be created. Existing branches are never overwritten. If Open after creation is enabled, Desktop will switch to the new workspace.",
                    request.name
                ),
                "Create worktree",
            )
        }
        Action::Open { id } => {
            validate_id(id)?;
            ("Open this managed worktree as the current Desktop workspace? Its existing local space is reused when available.".into(), "Open worktree")
        }
        Action::Archive { id } => {
            validate_id(id)?;
            ("Archive this managed worktree? Files and its Git branch stay in place. Every Colossus session using it must be closed first.".into(), "Archive worktree")
        }
        Action::Remove { id } => {
            validate_id(id)?;
            ("Remove this archived managed checkout? It must be clean, unlocked and unused. Untracked and ignored files block removal. The Git branch is retained.".into(), "Remove checkout")
        }
    };
    confirm(app.clone(), &lane, "Managed worktree", message, button).await?;
    if context != Context::capture(&state, &request.workspace_id)? {
        return Err(error(
            "The workspace changed during confirmation. Refresh worktrees.",
        ));
    }
    let operation = context.clone();
    let (catalog, open) = blocking(&lane, true, move || {
        let manager = WorktreeManager::open(&revalidate_workspace(&operation.workspace)?, None)
            .map_err(|failure| service_error(&failure))?;
        let path = match request.action {
            Action::Create { request } => {
                let created = manager
                    .create(&request)
                    .map_err(|failure| service_error(&failure))?;
                manager
                    .list()
                    .map_err(|_| unknown())?
                    .settings
                    .open_after_create
                    .then_some(created.path)
            }
            Action::Open { id } => Some(
                manager
                    .open_path(&id)
                    .map_err(|failure| service_error(&failure))?,
            ),
            Action::Archive { id } => {
                manager
                    .archive(&id)
                    .map_err(|failure| service_error(&failure))?;
                None
            }
            Action::Remove { id } => {
                manager
                    .remove(&id)
                    .map_err(|failure| service_error(&failure))?;
                None
            }
        };
        Ok((manager.list().map_err(|_| unknown())?, path))
    })
    .await?;
    if context != Context::capture(&state, &context.workspace.id).map_err(|_| unknown())? {
        return Err(unknown());
    }
    let opened = if let Some(path) = open {
        Some(crate::desktop_commands::open_worktree_workspace(&app, &state, &path).await.map_err(|mut failure| { failure.outcome_unknown = true; failure.retryable = false; failure.message = format!("Worktree updated, but Desktop could not open it: {} Refresh worktrees before another change.", failure.message); failure })?)
    } else {
        None
    };
    Ok(ActionResult { catalog, opened })
}
fn validate_id(id: &str) -> Result<(), CommandErrorDto> {
    if id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(error(
            "Choose a managed worktree from the current inventory.",
        ))
    }
}

#[tauri::command]
pub(crate) async fn get_worktree_preferences(
    caller: Webview,
    lane: State<'_, WorkspaceWorktreeState>,
) -> Result<Preferences, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    blocking(&lane, false, || {
        colossus_worktrees::preferences()
            .map(|(directory, settings)| Preferences {
                directory,
                settings,
            })
            .map_err(|failure| service_error(&failure))
    })
    .await
}
#[tauri::command]
pub(crate) async fn save_worktree_preferences(
    caller: Webview,
    lane: State<'_, WorkspaceWorktreeState>,
    settings: WorktreeSettings,
) -> Result<Preferences, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    settings
        .validate()
        .map_err(|failure| service_error(&failure))?;
    blocking(&lane, true, move || {
        colossus_worktrees::update_preferences(settings)
            .map_err(|failure| service_error(&failure))?;
        colossus_worktrees::preferences()
            .map(|(directory, settings)| Preferences {
                directory,
                settings,
            })
            .map_err(|failure| service_error(&failure))
    })
    .await
}
#[tauri::command]
pub(crate) async fn choose_worktree_directory(
    app: AppHandle,
    caller: Webview,
    lane: State<'_, WorkspaceWorktreeState>,
    reset: bool,
) -> Result<Preferences, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let directory = if reset {
        confirm(app, &lane, "Use default worktree directory", "Use the default private worktree directory for future work? Existing directories and checkouts remain in place.".into(), "Use default").await?;
        None
    } else {
        let permit = Arc::clone(&lane.prompt)
            .try_acquire_owned()
            .map_err(|_| error("A worktree confirmation is already open."))?;
        let path = tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            app.dialog().file().blocking_pick_folder()
        })
        .await
        .map_err(|_| error("The folder picker could not open."))?
        .ok_or_else(|| {
            CommandErrorDto::local_sanitized(
                "cancelled",
                "Worktree directory selection cancelled.",
                false,
            )
        })?
        .into_path()
        .map_err(|_| error("The selected directory is unavailable."))?;
        Some(path.join("ColossusWorktrees"))
    };
    blocking(&lane, true, move || {
        colossus_worktrees::select_storage_directory(directory.as_deref())
            .map_err(|failure| service_error(&failure))?;
        colossus_worktrees::preferences()
            .map(|(directory, settings)| Preferences {
                directory,
                settings,
            })
            .map_err(|failure| service_error(&failure))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_select_another_workspace_or_remote_target() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = crate::desktop_settings::validate_workspace(temporary.path()).unwrap();
        let settings = DesktopSettings {
            workspace: Some(workspace.clone()),
            selected_space_id: Some("local".into()),
            selected_target_id: Some("local".into()),
            access_profile: AccessProfileSetting::Development,
            ..DesktopSettings::default()
        };
        assert!(Context::from_settings(settings.clone(), &workspace.id, 1).is_ok());
        assert!(Context::from_settings(settings.clone(), "another-workspace", 1).is_err());
        let mut remote = settings.clone();
        remote.selected_target_id = Some("remote".into());
        assert!(Context::from_settings(remote, &workspace.id, 1).is_err());
        let mut minimal = settings;
        minimal.access_profile = AccessProfileSetting::Minimal;
        assert!(Context::from_settings(minimal, &workspace.id, 1).is_err());
        assert!(validate_id("../../unowned").is_err());
    }
}
