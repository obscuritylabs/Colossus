use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, State, Webview};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::{
    discovery::{RepositoryBinding, discover},
    dto::{CommitDetails, CommitPage, CommitRequest, GitStatus, HistoryRequest, StatusRequest},
    error,
    reader::{HISTORY_LIMIT, PAGE_SIZE, Reader},
};
use crate::{
    desktop_settings::{
        AccessProfileSetting, DesktopSettings, SettingsStore, WorkspaceSetting,
        revalidate_workspace,
    },
    dto::CommandErrorDto,
    state::AppState,
};

pub(crate) struct GitState {
    gate: Arc<Semaphore>,
    approval_gate: Arc<Semaphore>,
    session: Arc<Mutex<Option<Session>>>,
}
impl Default for GitState {
    fn default() -> Self {
        Self {
            gate: Arc::new(Semaphore::new(1)),
            approval_gate: Arc::new(Semaphore::new(1)),
            session: Arc::new(Mutex::new(None)),
        }
    }
}

struct Session {
    context: Context,
    binding: RepositoryBinding,
    id: String,
    head: Option<String>,
    listed: HashSet<String>,
    cursor: Option<String>,
    offset: usize,
}

#[derive(Clone)]
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
            return Err(error("The selected workspace changed. Refresh Git."));
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
            .ok_or_else(|| error("Select the current workspace to inspect Git."))?;
        let target = settings
            .selected_space_id
            .ok_or_else(|| error("Git inspection needs a local workspace."))?;
        if settings.selected_target_id.as_deref() != Some(&target)
            || settings.access_profile == AccessProfileSetting::Minimal
        {
            return Err(error(
                "Git inspection is available for the selected local workspace with file access enabled.",
            ));
        }
        revalidate_workspace(&workspace)?;
        Ok(Self {
            workspace,
            target,
            epoch,
        })
    }
    fn same(&self, other: &Self) -> bool {
        self.workspace == other.workspace
            && self.target == other.target
            && self.epoch == other.epoch
    }
}

async fn run<T: Send + 'static>(
    state: &AppState,
    git: &GitState,
    context: Context,
    action: impl FnOnce(&mut Option<Session>) -> Result<T, CommandErrorDto> + Send + 'static,
) -> Result<T, CommandErrorDto> {
    let permit = Arc::clone(&git.gate)
        .try_acquire_owned()
        .map_err(|_| error("Git is still refreshing. Try again shortly."))?;
    let session = Arc::clone(&git.session);
    // The slot stays with the blocking task after a UI timeout. Repeated requests
    // therefore cannot create an unbounded queue of non-interruptible library calls.
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let mut session = session
            .lock()
            .map_err(|_| error("Git inspection is unavailable."))?;
        action(&mut session)
    });
    let result = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .map_err(|_| {
            error("Git inspection is taking too long. You can keep working and retry shortly.")
        })?
        .map_err(|_| error("Git inspection could not finish."))??;
    let current = Context::capture(state, &context.workspace.id)?;
    if !context.same(&current) {
        return Err(error("The selected workspace changed. Refresh Git."));
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn get_workspace_git_status(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    git: State<'_, GitState>,
    request: StatusRequest,
) -> Result<GitStatus, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let context = Context::capture(&state, &request.workspace_id)?;
    let approved = if request.approve_repository {
        let binding = approve_binding(app, &git, context.clone()).await?;
        if !context.same(&Context::capture(&state, &request.workspace_id)?) {
            return Err(error("The selected workspace changed. Refresh Git."));
        }
        binding
    } else {
        None
    };
    let operation_context = context.clone();
    run(&state, &git, context, move |session| {
        let root = revalidate_workspace(&operation_context.workspace)?;
        let Some(binding) = discover(&root)? else {
            *session = None;
            return Ok(GitStatus {
                state: "not_repository",
                repository: None,
            });
        };
        let existing = session.as_ref().is_some_and(|current| {
            current.context.same(&operation_context) && current.binding.same_repository(&binding)
        });
        if !existing {
            *session = None;
            if binding.needs_approval()
                && !approved
                    .as_ref()
                    .is_some_and(|approved| approved.same_repository(&binding))
            {
                return Ok(GitStatus {
                    state: "permission_required",
                    repository: None,
                });
            }
            *session = Some(Session {
                context: operation_context.clone(),
                binding: binding.clone(),
                id: Uuid::new_v4().to_string(),
                head: None,
                listed: HashSet::new(),
                cursor: None,
                offset: 0,
            });
        }
        let current = session
            .as_mut()
            .ok_or_else(|| error("Refresh Git to connect to this repository."))?;
        let reader = Reader::open(&binding)?;
        let repository = reader.status(&binding, current.id.clone())?;
        binding.revalidate()?;
        revalidate_workspace(&operation_context.workspace)?;
        if current.head != repository.head {
            current.listed.clear();
            current.cursor = None;
            current.offset = 0;
        }
        current.head.clone_from(&repository.head);
        Ok(GitStatus {
            state: "ready",
            repository: Some(repository),
        })
    })
    .await
}

// Human confirmation has no query deadline. The read timeout starts only after
// confirmation, and an independent slot prevents concurrent native prompts.
async fn approve_binding(
    app: AppHandle,
    git: &GitState,
    context: Context,
) -> Result<Option<RepositoryBinding>, CommandErrorDto> {
    let permit = Arc::clone(&git.approval_gate)
        .try_acquire_owned()
        .map_err(|_| error("A repository confirmation is already open."))?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let root = revalidate_workspace(&context.workspace)?;
        let Some(binding) = discover(&root)? else { return Ok(None); };
        if !binding.needs_approval() { return Ok(Some(binding)); }
        let approved = app.dialog().message(format!(
            "Inspect Git for {}?\n\nRepository: {}\nShared Git metadata: {}\n\nAllow read-only branch and commit metadata outside this folder while this workspace is selected? File paths stay limited to the workspace. No Git commands, hooks, or network requests will run.",
            context.workspace.display_name, binding.root.display_path, binding.common.display_path))
            .title("Inspect repository metadata")
            .buttons(MessageDialogButtons::OkCancelCustom("Allow Git inspection".into(), "Cancel".into())).blocking_show();
        binding.revalidate()?;
        Ok(approved.then_some(binding))
    }).await.map_err(|_| error("The repository confirmation could not be opened."))?
}

fn current_session<'a>(
    session: &'a mut Option<Session>,
    context: &Context,
    id: &str,
) -> Result<&'a mut Session, CommandErrorDto> {
    let current = session
        .as_mut()
        .filter(|s| s.context.same(context) && s.id == id)
        .ok_or_else(|| error("The repository selection is stale. Refresh Git."))?;
    current.binding.revalidate()?;
    Ok(current)
}

#[tauri::command]
pub(crate) async fn list_workspace_git_commits(
    caller: Webview,
    state: State<'_, AppState>,
    git: State<'_, GitState>,
    request: HistoryRequest,
) -> Result<CommitPage, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let context = Context::capture(&state, &request.workspace_id)?;
    let operation_context = context.clone();
    run(&state, &git, context, move |session| {
        let current = current_session(session, &operation_context, &request.repository_id)?;
        let offset = history_offset(current, request.cursor.as_deref())?;
        let reader = Reader::open(&current.binding)?;
        let head = reader.head()?.1;
        if head != current.head {
            return Err(error(
                "HEAD changed. Refresh Git before loading more history.",
            ));
        }
        let (commits, more) = match &head {
            Some(head) => reader.history(head, offset)?,
            None => (Vec::new(), false),
        };
        current.binding.revalidate()?;
        if reader.head()?.1 != head {
            return Err(error("HEAD changed. Refresh Git."));
        }
        if offset == 0 {
            current.listed.clear();
        }
        current
            .listed
            .extend(commits.iter().map(|commit| commit.id.clone()));
        current.offset = offset + commits.len();
        let limited = more && current.offset >= HISTORY_LIMIT;
        current.cursor = (more && !limited).then(|| Uuid::new_v4().to_string());
        Ok(CommitPage {
            head,
            commits,
            next_cursor: current.cursor.clone(),
            limited,
        })
    })
    .await
}

fn history_offset(session: &Session, cursor: Option<&str>) -> Result<usize, CommandErrorDto> {
    match cursor {
        None => Ok(0),
        Some(cursor)
            if Some(cursor) == session.cursor.as_deref()
                && session.offset < HISTORY_LIMIT
                && session.offset.is_multiple_of(PAGE_SIZE) =>
        {
            Ok(session.offset)
        }
        _ => Err(error("This history page expired. Reload history.")),
    }
}

#[tauri::command]
pub(crate) async fn get_workspace_git_commit(
    caller: Webview,
    state: State<'_, AppState>,
    git: State<'_, GitState>,
    request: CommitRequest,
) -> Result<CommitDetails, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    let context = Context::capture(&state, &request.workspace_id)?;
    let operation_context = context.clone();
    run(&state, &git, context, move |session| {
        let current = current_session(session, &operation_context, &request.repository_id)?;
        if !current.listed.contains(&request.commit_id) {
            return Err(error("Choose a commit from the current history list."));
        }
        let reader = Reader::open(&current.binding)?;
        if reader.head()?.1 != current.head {
            return Err(error("HEAD changed. Refresh Git."));
        }
        let details = reader.details(&current.binding, &request.commit_id)?;
        current.binding.revalidate()?;
        if reader.head()?.1 != current.head {
            return Err(error("HEAD changed. Refresh Git."));
        }
        Ok(details)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_settings::validate_workspace;

    #[test]
    fn renderer_cannot_choose_another_workspace_or_an_external_target() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let workspace = validate_workspace(&root).unwrap();
        let settings = DesktopSettings {
            workspace: Some(workspace.clone()),
            selected_space_id: Some("local-space".into()),
            selected_target_id: Some("local-space".into()),
            access_profile: AccessProfileSetting::Development,
            ..DesktopSettings::default()
        };
        assert!(Context::from_settings(settings.clone(), &workspace.id, 1).is_ok());
        assert!(Context::from_settings(settings.clone(), "foreign-workspace", 1).is_err());
        let mut minimal = settings.clone();
        minimal.access_profile = AccessProfileSetting::Minimal;
        assert!(Context::from_settings(minimal, &workspace.id, 1).is_err());
        let mut external = settings;
        external.selected_target_id = Some("external".into());
        assert!(Context::from_settings(external, &workspace.id, 1).is_err());
    }

    #[test]
    fn repository_handles_and_cursors_are_bound_to_the_native_selection() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        git2::Repository::init(&root).unwrap();
        let context = Context {
            workspace: validate_workspace(&root).unwrap(),
            target: "local-space".into(),
            epoch: 7,
        };
        let mut session = Some(Session {
            context: context.clone(),
            binding: discover(&root).unwrap().unwrap(),
            id: "native-repository".into(),
            head: None,
            listed: HashSet::new(),
            cursor: Some("native-cursor".into()),
            offset: PAGE_SIZE,
        });
        assert!(current_session(&mut session, &context, "foreign-repository").is_err());
        let mut stale = context.clone();
        stale.epoch += 1;
        assert!(current_session(&mut session, &stale, "native-repository").is_err());
        let current = current_session(&mut session, &context, "native-repository").unwrap();
        assert_eq!(
            history_offset(current, Some("native-cursor")).unwrap(),
            PAGE_SIZE
        );
        assert!(history_offset(current, Some("HEAD~40")).is_err());
        assert!(history_offset(current, Some("../other-repository")).is_err());
        current.cursor = None;
        assert!(history_offset(current, Some("native-cursor")).is_err());
    }
}
