//! Offline human-interface translation for the shared managed-worktree service.
use super::{
    output::print_json,
    worktree_args::{WorktreeAction, WorktreeCommand},
};
use colossus_worktrees::{CreateWorktree, WorktreeManager};
use serde_json::json;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

/// `Some(path)` re-enters the ordinary TUI through the caller's startup path.
pub(super) fn run(
    command: &WorktreeCommand,
    workspace: &Path,
) -> Result<Option<PathBuf>, Box<dyn Error>> {
    if let WorktreeAction::Settings {
        branch_prefix,
        open_after_create,
        storage_directory,
        reset_directory,
    } = &command.command
    {
        if command.directory.is_some() {
            return Err("worktree settings uses the persisted directory; --directory is only a per-command override".into());
        }
        if storage_directory.is_some() || *reset_directory {
            colossus_worktrees::select_storage_directory(storage_directory.as_deref())?;
        }
        let (_, mut preferences) = colossus_worktrees::preferences()?;
        if let Some(prefix) = branch_prefix {
            preferences.branch_prefix.clone_from(prefix);
        }
        if let Some(open) = open_after_create {
            preferences.open_after_create = *open;
        }
        let (root, settings) = if branch_prefix.is_some() || open_after_create.is_some() {
            colossus_worktrees::update_preferences(preferences)?
        } else {
            colossus_worktrees::preferences()?
        };
        print_json(&json!({"storageRoot":root,"settings":settings}))?;
        return Ok(None);
    }
    let manager = WorktreeManager::open(workspace, command.directory.as_deref())?;
    match &command.command {
        WorktreeAction::List => print_json(&manager.list()?)?,
        WorktreeAction::Create {
            name,
            branch,
            base,
            existing_branch,
        } => {
            let created = manager.create(&CreateWorktree {
                name: name.clone(),
                branch: branch.clone(),
                base: base.clone(),
                existing_branch: *existing_branch,
            })?;
            print_json(&created)?;
        }
        WorktreeAction::Open { id, print_path } => {
            let path = manager.open_path(id)?;
            if *print_path {
                print_json(&json!({"id":id,"path":path}))?;
            } else {
                return Ok(Some(path));
            }
        }
        WorktreeAction::Archive { id } => print_json(&manager.archive(id)?)?,
        WorktreeAction::Remove { id, yes } => {
            if !yes {
                return Err("inspect `worktree list`, then repeat this exact removal with --yes; dirty, locked or active worktrees are always retained".into());
            }
            print_json(&manager.remove(id)?)?;
        }
        WorktreeAction::Settings { .. } => unreachable!("handled before repository acquisition"),
    }
    Ok(None)
}
