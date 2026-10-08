//! Identify generated namespaces before treating any subtree as application data.

use std::path::Path;

pub(super) fn owned_path(relative: &Path, directory: bool) -> bool {
    let Some(parts) = relative
        .iter()
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    match parts.as_slice() {
        ["desktop" | "workspaces" | "plugins"] | ["desktop", "self-test" | "managed-local"] => {
            directory
        }
        ["workspaces", partition] | ["workspaces", partition, "cli" | "workspace-plugins"] => {
            directory && is_partition(partition)
        }
        ["workspaces", partition, "workspace-plugins", _, ..]
        | ["workspaces", partition, "desktop", ..]
        | ["desktop", "managed-local", partition, ..] => is_partition(partition),
        [
            "desktop",
            "self-test",
            "runtime" | "runtime-v2" | "runtime-v3" | "workspace",
            ..,
        ]
        | ["desktop", "trust" | "codex-auth", ..]
        | ["plugins", _, ..] => true,
        ["desktop", session, ..]
            if session
                .strip_prefix("browser-session-")
                .is_some_and(|suffix| {
                    suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
                }) =>
        {
            true
        }
        ["config.yaml" | "AGENTS.md"]
        | [
            "desktop",
            "settings.json"
            | "remembered-commands.json"
            | "thread-search.redb"
            | "credentials-v1.redb"
            | "credentials-v1.lock",
        ] => !directory,
        ["desktop", temporary]
            if temporary
                .strip_prefix(".settings.json.")
                .or_else(|| temporary.strip_prefix(".remembered-commands.json."))
                .and_then(|name| name.strip_suffix(".tmp"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()) =>
        {
            !directory
        }
        _ => false,
    }
}

// CLI metadata commands create this directory even when no CLI state is stored.
// Only the directory itself is owned; every child remains rejected.
pub(super) fn empty_cli_surface(relative: &Path) -> bool {
    let Some(parts) = relative
        .iter()
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    matches!(parts.as_slice(), ["workspaces", partition, "cli"] if is_partition(partition))
}

fn is_partition(part: &str) -> bool {
    part.len() == 64
        && part
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn plugin_blob(relative: &Path) -> bool {
    let Some(parts) = relative
        .iter()
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let cache = match parts.as_slice() {
        ["plugins", cache @ ..] => cache,
        ["workspaces", partition, "workspace-plugins", cache @ ..] if is_partition(partition) => {
            cache
        }
        _ => return false,
    };
    match cache {
        ["blobs", "sha256", digest] => is_partition(digest),
        ["layouts", "sha256", layout, "blobs", "sha256", digest] => {
            is_partition(layout) && is_partition(digest)
        }
        ["staging", staging, "blobs", "sha256", digest] => {
            staging
                .strip_prefix("generated-layout-")
                .or_else(|| staging.strip_prefix("retained-layout-"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
                && is_partition(digest)
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
