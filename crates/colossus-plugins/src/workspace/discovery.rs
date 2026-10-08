use super::*;

fn issue(result: &mut WorkspacePluginDiscovery, path: &str, detail: &str) {
    result.issues.push(WorkspacePluginIssue {
        path: path.into(),
        detail: detail.into(),
    });
}

/// Discover only fixed `.agents` locations and previously registered local paths.
pub fn discover_workspace_plugins(
    workspace: &Path,
    registered: &[String],
) -> WorkspacePluginDiscovery {
    discover_workspace_plugins_with_icon_budget(
        workspace,
        registered,
        &mut crate::PluginIconBudget::default(),
    )
}

/// Discover local sources within the caller's cumulative display budget.
pub fn discover_workspace_plugins_with_icon_budget(
    workspace: &Path,
    registered: &[String],
    icons: &mut crate::PluginIconBudget,
) -> WorkspacePluginDiscovery {
    let mut result = WorkspacePluginDiscovery::default();
    let mut paths = registered.iter().cloned().collect::<BTreeSet<_>>();
    if fs::symlink_metadata(workspace.join(".agents")).is_ok() {
        match workspace_plugin_root(workspace, Path::new(".agents")) {
            Ok(root) => {
                let direct = fs::symlink_metadata(root.join("plugin.json")).is_ok();
                let collection = collection_paths(workspace, &mut result);
                if direct
                    && (!collection.is_empty()
                        || result
                            .issues
                            .iter()
                            .any(|issue| issue.path == ".agents/plugins"))
                {
                    issue(
                        &mut result,
                        ".agents",
                        "Choose one layout: .agents/plugin.json or .agents/plugins/<name>/plugin.json",
                    );
                    paths.retain(|path| path != ".agents" && !path.starts_with(".agents/plugins/"));
                } else if direct {
                    paths.insert(".agents".into());
                } else {
                    paths.extend(collection);
                }
            }
            Err(_) => issue(
                &mut result,
                ".agents",
                "Workspace plugin container must be a real contained directory",
            ),
        }
    }
    if paths.len() > MAX_WORKSPACE_PLUGINS {
        issue(
            &mut result,
            ".agents/plugins",
            "Workspace discovery exceeds 128 plugin sources",
        );
        return result;
    }
    let mut remaining = MAX_WORKSPACE_PLUGIN_BYTES;
    for path in paths {
        match capture_with_budget(
            workspace,
            Path::new(&path),
            &mut remaining,
            icons.for_origin(PluginOrigin::Workspace),
        ) {
            Ok(candidate) => result.candidates.push(candidate),
            Err(_) => issue(
                &mut result,
                &path,
                "Invalid workspace plugin: check plugin.json, contained regular files, and the 256 MiB discovery limit",
            ),
        }
    }
    result
}

fn collection_paths(workspace: &Path, result: &mut WorkspacePluginDiscovery) -> BTreeSet<String> {
    let relative = Path::new(".agents/plugins");
    let mut paths = BTreeSet::new();
    if fs::symlink_metadata(workspace.join(relative)).is_err() {
        return paths;
    }
    let entries = match workspace_plugin_root(workspace, relative)
        .and_then(|root| fs::read_dir(root).map_err(adapter))
    {
        Ok(entries) => entries,
        Err(_) => {
            issue(
                result,
                ".agents/plugins",
                "Workspace plugin collection must be a real contained directory",
            );
            return paths;
        }
    };
    // Reject overflow instead of selecting an arbitrary filesystem iteration prefix.
    for (index, entry) in entries.enumerate() {
        if index == MAX_WORKSPACE_PLUGINS {
            issue(
                result,
                ".agents/plugins",
                "Workspace discovery exceeds 128 collection entries",
            );
            return BTreeSet::new();
        }
        let Ok(entry) = entry else {
            issue(
                result,
                ".agents/plugins",
                "Unable to enumerate workspace plugin sources",
            );
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            issue(
                result,
                ".agents/plugins",
                "Workspace source names must be valid UTF-8",
            );
            continue;
        };
        let path = format!(".agents/plugins/{name}");
        match entry.file_type() {
            Ok(kind) if kind.is_symlink() => issue(
                result,
                &path,
                "Linked workspace plugin sources are not allowed",
            ),
            Ok(kind) if kind.is_dir() => {
                paths.insert(path);
            }
            Ok(_) => {}
            Err(_) => issue(result, &path, "Unable to inspect workspace plugin source"),
        }
    }
    paths
}
