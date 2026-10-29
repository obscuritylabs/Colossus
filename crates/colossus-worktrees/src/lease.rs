use crate::{
    WorktreeError, WorktreeState,
    error::Result,
    filesystem::read_small,
    model::{Entry, Origin},
    repository::Binding,
    storage::Storage,
    validation,
};
use colossus_home::{ConfinedFile, ConfinedRoot};
use fs4::fs_std::FileExt;
use std::{fs, path::Path};

pub(crate) const ORIGIN_FILE: &str = "colossus-managed-worktree.json";

/// Existing-only shared use lease. Keep it alive through worker/run/terminal cleanup.
/// A competing archive/removal requires the same file's exclusive native lock.
pub struct WorkspaceLease {
    _file: ConfinedFile,
    _root: ConfinedRoot,
}

/// Acquire a shared lock for a Colossus-managed checkout, or `None` for an unmanaged
/// repository. This never creates a home, catalog, lock, runtime or provider.
pub fn acquire_workspace_lease(workspace: &Path) -> Result<Option<WorkspaceLease>> {
    let canonical = fs::canonicalize(workspace).map_err(|_| WorktreeError::ownership())?;
    let managed_shape = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| validation::id(value).is_ok())
        && canonical
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "trees")
        && canonical
            .parent()
            .and_then(Path::parent)
            .is_some_and(|root| root.join("catalog.json").exists());
    let marker = canonical.join(".git");
    let Ok(metadata) = fs::symlink_metadata(&marker) else {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    };
    if metadata.is_dir() {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    }
    // Unmanaged repositories (including submodules and separate gitdirs) keep
    // their existing startup behavior. Strict managed validation begins only
    // when the private ownership marker is actually present.
    let Ok(bytes) = read_small(&marker, 4096) else {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    };
    let Some(pointer) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|value| value.trim().strip_prefix("gitdir: "))
    else {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    };
    let Ok(git) = crate::repository::pointer_path(&canonical, pointer) else {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    };
    let origin_path = git.join(ORIGIN_FILE);
    if !origin_path.try_exists().map_err(|_| WorktreeError::io())? {
        return if managed_shape {
            Err(WorktreeError::ownership())
        } else {
            Ok(None)
        };
    }
    let binding = Binding::discover(&canonical)?;
    let origin: Origin = serde_json::from_slice(&read_small(&origin_path, 8192)?)
        .map_err(|_| WorktreeError::ownership())?;
    validate_origin(&origin, &binding)?;
    let storage = Storage::open(&origin.storage_root, false)?;
    let _mutation = storage.lock_existing()?;
    let catalog = storage.load()?;
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == origin.id)
        .ok_or_else(WorktreeError::ownership)?;
    validate_entry(entry, &binding, storage.root.path())?;
    if entry.state != WorktreeState::Active {
        return Err(WorktreeError::new(
            crate::WorktreeErrorCode::Conflict,
            "Restore the archived managed worktree before starting a Colossus session.",
        ));
    }
    let file = storage
        .root
        .open_existing_file_read_write(
            Path::new("leases")
                .join(format!("{}.lock", entry.id))
                .as_path(),
        )
        .map_err(|_| WorktreeError::ownership())?;
    if !FileExt::try_lock_shared(file.file()).map_err(|_| WorktreeError::busy())? {
        return Err(WorktreeError::busy());
    }
    file.revalidate(&storage.root)
        .map_err(|_| WorktreeError::ownership())?;
    binding.revalidate()?;
    Ok(Some(WorkspaceLease {
        _file: file,
        _root: storage.root.clone(),
    }))
}

pub(crate) fn validate_origin(origin: &Origin, binding: &Binding) -> Result<()> {
    validation::id(&origin.id)?;
    if origin.schema_version != 1
        || origin.purpose != "ColossusManagedWorktree"
        || origin.repository_id != binding.id
        || !origin.storage_root.is_absolute()
    {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}

pub(crate) fn validate_entry(entry: &Entry, binding: &Binding, root: &Path) -> Result<()> {
    let identity = binding.workspace.as_ref();
    if entry.path != binding.workspace.canonical_path()
        || entry.path != root.join("trees").join(&entry.id)
        || entry.common_path != binding.common.canonical_path()
        || entry.repository_id != binding.id
        || entry.workspace_identity != Some((identity.version, identity.sha256.into()))
        || binding.git.canonical_path() != entry.common_path.join("worktrees").join(&entry.git_name)
    {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}

pub(crate) fn exclusive(storage: &Storage, id: &str) -> Result<ConfinedFile> {
    validation::id(id)?;
    let file = storage
        .root
        .open_existing_file_read_write(Path::new("leases").join(format!("{id}.lock")).as_path())
        .map_err(|_| WorktreeError::ownership())?;
    if !FileExt::try_lock_exclusive(file.file()).map_err(|_| WorktreeError::busy())? {
        return Err(WorktreeError::busy());
    }
    Ok(file)
}
