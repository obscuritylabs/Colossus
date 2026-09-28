//! Name/path search only. No file content, hidden control state, or link traversal.
use crate::{
    desktop_settings::{SettingsStore, revalidate_workspace},
    dto::CommandErrorDto,
    state::AppState,
    workspace_files,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{State, Webview};
use tokio::sync::Semaphore;

const MAX_SCANNED: usize = 50_000;
const MAX_RESULTS: usize = 200;

pub(crate) struct SearchState(Arc<Semaphore>);
impl Default for SearchState {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(1)))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SearchRequest {
    workspace_id: String,
    query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResults {
    paths: Vec<String>,
    truncated: bool,
    scanned: usize,
}

fn error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "workspace_search",
        "File search is unavailable. Refresh and try again.",
        true,
    )
}

#[tauri::command]
pub(crate) async fn search_workspace_files(
    caller: Webview,
    state: State<'_, AppState>,
    search: State<'_, SearchState>,
    request: SearchRequest,
) -> Result<SearchResults, CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    if request.query.len() > 256 || request.query.chars().any(char::is_control) {
        return Err(error());
    }
    let epoch = *state.subscribe_selection().borrow();
    let settings = SettingsStore::open_application()?.load()?;
    let workspace = workspace_files::authorize_workspace(&settings, &request.workspace_id)?.clone();
    let target = settings.selected_space_id.ok_or_else(error)?;
    if !state.selection_is_current(&target, epoch) {
        return Err(error());
    }
    let root = revalidate_workspace(&workspace)?;
    let permit = Arc::clone(&search.0)
        .try_acquire_owned()
        .map_err(|_| error())?;
    let expected_workspace = workspace.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let result = search_paths(&root, &request.query);
        revalidate_workspace(&workspace)?;
        Ok::<_, CommandErrorDto>(result)
    });
    let result = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .map_err(|_| error())?
        .map_err(|_| error())??;
    let current = SettingsStore::open_application()?.load()?;
    if workspace_files::authorize_workspace(&current, &expected_workspace.id)?
        != &expected_workspace
        || current.selected_space_id.as_deref() != Some(&target)
    {
        return Err(error());
    }
    if !state.selection_is_current(&target, epoch) {
        return Err(error());
    }
    Ok(result)
}

fn search_paths(root: &Path, query: &str) -> SearchResults {
    let query = query.trim().to_lowercase();
    let mut result = SearchResults {
        paths: Vec::new(),
        truncated: false,
        scanned: 0,
    };
    if query.is_empty() {
        return result;
    }
    let started = Instant::now();
    let mut pending = vec![String::new()];
    while let Some(relative) = pending.pop() {
        let Ok(directory) = workspace_files::resolve_relative(root, &relative, true) else {
            result.truncated = true;
            continue;
        };
        let Ok(entries) = fs::read_dir(directory) else {
            result.truncated = true;
            continue;
        };
        for entry in entries {
            if result.scanned >= MAX_SCANNED
                || started.elapsed() >= Duration::from_secs(3)
                || result.paths.len() >= MAX_RESULTS
            {
                result.truncated = true;
                result.paths.sort();
                return result;
            }
            result.scanned += 1;
            let Ok(entry) = entry else {
                result.truncated = true;
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if workspace_files::hidden_entry(&name) || !workspace_files::renderer_safe_name(&name) {
                continue;
            }
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if path.len() > 2048 {
                result.truncated = true;
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                result.truncated = true;
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            // Revalidate every candidate, including Windows junction/reparse paths.
            if workspace_files::resolve_relative(root, &path, false).is_err() {
                continue;
            }
            if metadata.is_dir() {
                if path.split('/').count() < 48 {
                    pending.push(path);
                } else {
                    result.truncated = true;
                }
            } else if metadata.is_file() && path.to_lowercase().contains(&query) {
                result.paths.push(path);
            }
        }
    }
    result.paths.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_unopened_nested_paths_and_skips_protected_and_generated_folders() {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["src/deep", "node_modules", ".git", ".aws"] {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        for path in [
            "src/deep/Model.ts",
            "node_modules/Model.ts",
            ".git/Model.ts",
            ".aws/Model.ts",
            "src/.env.model",
        ] {
            fs::write(dir.path().join(path), "").unwrap();
        }
        let root = fs::canonicalize(dir.path()).unwrap();
        let result = search_paths(&root, "MODEL");
        assert_eq!(result.paths, ["src/deep/Model.ts"]);
        assert!(!result.truncated);
        assert!(search_paths(&root, " ").paths.is_empty());
    }
    #[test]
    fn large_flat_directories_are_searchable_beyond_tree_listing_limit_and_results_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..2200 {
            fs::write(dir.path().join(format!("file-{i:04}.txt")), "").unwrap();
        }
        let root = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(search_paths(&root, "file-2199").paths, ["file-2199.txt"]);
        let result = search_paths(&root, "file");
        assert_eq!(result.paths.len(), MAX_RESULTS);
        assert!(result.truncated);
    }
    #[cfg(unix)]
    #[test]
    fn does_not_follow_links_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
        assert!(
            search_paths(&fs::canonicalize(dir.path()).unwrap(), "secret")
                .paths
                .is_empty()
        );
    }
}
