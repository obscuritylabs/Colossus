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
        ["workspaces", partition] => directory && is_partition(partition),
        ["workspaces", partition, "desktop", ..] | ["desktop", "managed-local", partition, ..] => {
            is_partition(partition)
        }
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
            "settings.json" | "thread-search.redb" | "credentials-v1.redb" | "credentials-v1.lock",
        ] => !directory,
        ["desktop", temporary]
            if temporary
                .strip_prefix(".settings.json.")
                .and_then(|name| name.strip_suffix(".tmp"))
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()) =>
        {
            !directory
        }
        _ => false,
    }
}

fn is_partition(part: &str) -> bool {
    part.len() == 64
        && part
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
