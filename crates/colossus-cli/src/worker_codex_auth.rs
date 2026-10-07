//! Trusted worker-only account selection; no credential bytes are read at startup.

use colossus_codex_auth::CodexAuthStore;
use colossus_home::ConfinedRoot;
use std::{
    fs, io,
    path::{Component, Path},
};

pub(super) fn runtime_options(
    worker: &super::WorkerCommand,
    options: colossus_runtime::RuntimeOpenOptions,
) -> colossus_runtime::RuntimeOpenOptions {
    if worker.no_model_network_tools {
        options.without_model_network_tools()
    } else {
        options
    }
}

const MAX_AUTH_FILE_BYTES: u64 = 256 * 1024;

pub(super) fn select(path: Option<&Path>, workspace: &Path) -> io::Result<Option<CodexAuthStore>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|value| value.len() > 4096 || value.chars().any(char::is_control))
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid());
    }
    let parent = path.parent().ok_or_else(invalid)?;
    // ConfinedRoot::bind may prepare a missing directory for other callers. This
    // selection only observes an existing private store and never creates a path.
    let parent_metadata = fs::symlink_metadata(parent).map_err(|_| invalid())?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    let root = ConfinedRoot::bind(parent).map_err(|_| invalid())?;
    let file = root
        .open_existing_file(Path::new(path.file_name().ok_or_else(invalid)?))
        .map_err(|_| invalid())?;
    let metadata = file.file().metadata().map_err(|_| invalid())?;
    if metadata.len() > MAX_AUTH_FILE_BYTES {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid());
        }
    }
    file.revalidate(&root).map_err(|_| invalid())?;
    let workspace = fs::canonicalize(workspace).map_err(|_| invalid())?;
    let canonical_file = fs::canonicalize(file.path()).map_err(|_| invalid())?;
    if canonical_file.starts_with(&workspace) {
        return Err(invalid());
    }
    file.revalidate(&root).map_err(|_| invalid())?;
    // CodexAuthStore rechecks file ownership/privacy at each permit-bearing load
    // and refresh. No environment selector or default store replaces this choice.
    Ok(Some(
        CodexAuthStore::confined(root, Path::new(path.file_name().ok_or_else(invalid)?))
            .map_err(|_| invalid())?,
    ))
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "--codex-auth-path requires an existing absolute private owner-held regular file outside the workspace and without links; selection failed without account fallback",
    )
}
