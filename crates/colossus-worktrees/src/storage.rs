use crate::{
    WorktreeError, WorktreeErrorCode, error::Result, filesystem::reject_links, model::Catalog,
    validation,
};
use colossus_home::{ColossusHome, ConfinedFile, ConfinedRoot, detect_workspace_identity};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{Read as _, Seek as _, Write as _},
    path::{Path, PathBuf},
};

const CATALOG: &str = "catalog.json";
pub(crate) const MAX_WORKTREES: usize = 128;

/// Shared per-user location. Explicit `COLOSSUS_HOME` keeps tests/operator state isolated.
pub fn default_root() -> Result<PathBuf> {
    if let Some(home) = env::var_os("COLOSSUS_HOME") {
        let home = PathBuf::from(home);
        if !home.is_absolute() {
            return Err(WorktreeError::new(
                WorktreeErrorCode::InvalidInput,
                "COLOSSUS_HOME must be absolute.",
            ));
        }
        return Ok(home.join("worktrees"));
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_local_dir().join("ColossusWorktrees"))
        .ok_or_else(WorktreeError::io)
}

pub(crate) struct Storage {
    pub root: ConfinedRoot,
}
pub(crate) struct MutationLease {
    _file: ConfinedFile,
}
impl Storage {
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        if !path.is_absolute() {
            return Err(WorktreeError::new(
                WorktreeErrorCode::InvalidInput,
                "The managed worktree directory must be absolute.",
            ));
        }
        if path.exists() {
            reject_links(path)?;
            if !path.join(CATALOG).exists()
                && fs::read_dir(path)
                    .map_err(|_| WorktreeError::io())?
                    .next()
                    .is_some()
            {
                return Err(WorktreeError::new(
                    WorktreeErrorCode::Ownership,
                    "Choose a new or empty managed worktree directory; existing unrelated files are never adopted.",
                ));
            }
        } else if !create {
            return Err(WorktreeError::ownership());
        }
        let home =
            ColossusHome::ensure_at(path.to_owned()).map_err(|_| WorktreeError::ownership())?;
        let storage = Self {
            // Catalog paths and repository identities share canonical spelling,
            // including the verbatim namespace prefix on Windows.
            root: ConfinedRoot::bind(
                fs::canonicalize(home.root()).map_err(|_| WorktreeError::ownership())?,
            )
            .map_err(|_| WorktreeError::ownership())?,
        };
        if create {
            let _lease = storage.lock()?;
            let file = storage
                .root
                .open_file(Path::new(CATALOG))
                .map_err(|_| WorktreeError::ownership())?;
            if file.was_created() {
                let identity = detect_workspace_identity(storage.root.path())
                    .map_err(|_| WorktreeError::ownership())?;
                let value = Catalog {
                    storage_identity: Some((identity.version(), identity.sha256().into())),
                    ..Catalog::default()
                };
                write(file.file(), &value)?;
                storage
                    .root
                    .sync_directory()
                    .map_err(|_| WorktreeError::io())?;
            }
        }
        storage.load()?;
        Ok(storage)
    }
    pub fn lock(&self) -> Result<MutationLease> {
        let file = self
            .root
            .open_file(Path::new("mutation.lock"))
            .map_err(|_| WorktreeError::ownership())?;
        if !FileExt::try_lock_exclusive(file.file()).map_err(|_| WorktreeError::busy())? {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Busy,
                "Another managed worktree operation is still running.",
            ));
        }
        file.revalidate(&self.root)
            .map_err(|_| WorktreeError::ownership())?;
        Ok(MutationLease { _file: file })
    }
    pub fn lock_existing(&self) -> Result<MutationLease> {
        let file = self
            .root
            .open_existing_file_read_write(Path::new("mutation.lock"))
            .map_err(|_| WorktreeError::ownership())?;
        if !FileExt::try_lock_exclusive(file.file()).map_err(|_| WorktreeError::busy())? {
            return Err(WorktreeError::busy());
        }
        Ok(MutationLease { _file: file })
    }
    pub fn load(&self) -> Result<Catalog> {
        self.root
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
        let file = self
            .root
            .open_existing_file(Path::new(CATALOG))
            .map_err(|_| WorktreeError::ownership())?;
        let metadata = file.file().metadata().map_err(|_| WorktreeError::io())?;
        if metadata.len() > 4 * 1024 * 1024 {
            return Err(WorktreeError::ownership());
        }
        let mut bytes = Vec::new();
        file.file()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| WorktreeError::io())?;
        let catalog: Catalog =
            serde_json::from_slice(&bytes).map_err(|_| WorktreeError::ownership())?;
        let identity =
            detect_workspace_identity(self.root.path()).map_err(|_| WorktreeError::ownership())?;
        if catalog.schema_version != 1
            || catalog.purpose != "ColossusManagedWorktrees"
            || catalog.entries.len() > MAX_WORKTREES
            || catalog.storage_identity != Some((identity.version(), identity.sha256().into()))
        {
            return Err(WorktreeError::ownership());
        }
        validation::settings(&catalog.settings)?;
        let mut ids = std::collections::HashSet::new();
        for entry in &catalog.entries {
            validation::id(&entry.id)?;
            validation::name(&entry.name)?;
            validation::branch(&entry.branch)?;
            if !ids.insert(&entry.id)
                || entry.repository_id.len() != 64
                || entry.path != self.root.path().join("trees").join(&entry.id)
                || entry.git_name != format!("colossus-{}", entry.id)
            {
                return Err(WorktreeError::ownership());
            }
        }
        file.revalidate(&self.root)
            .map_err(|_| WorktreeError::ownership())?;
        Ok(catalog)
    }
    pub fn save(&self, catalog: &Catalog) -> Result<()> {
        let existing = self
            .root
            .open_existing_file_read_write(Path::new(CATALOG))
            .map_err(|_| WorktreeError::ownership())?;
        let name = format!(".catalog-{}", uuid::Uuid::now_v7().simple());
        let staged = self
            .root
            .open_file(Path::new(&name))
            .map_err(|_| WorktreeError::io())?;
        if !staged.was_created() {
            return Err(WorktreeError::ownership());
        }
        write(staged.file(), catalog)?;
        self.root
            .replace_existing_file(&staged, &existing)
            .map_err(|_| WorktreeError::uncertain())?;
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Location {
    schema_version: u16,
    purpose: String,
    directory: Option<PathBuf>,
    identity: Option<(u16, String)>,
}

pub(crate) fn configured_root() -> Result<PathBuf> {
    let default = default_root()?;
    let storage = Storage::open(&default, true)?;
    if !default
        .join("location.json")
        .try_exists()
        .map_err(|_| WorktreeError::io())?
    {
        return Ok(default);
    }
    let file = storage
        .root
        .open_existing_file(Path::new("location.json"))
        .map_err(|_| WorktreeError::ownership())?;
    let location: Location =
        serde_json::from_reader(file.file().take(8192)).map_err(|_| WorktreeError::ownership())?;
    if location.schema_version != 1
        || location.purpose != "ColossusWorktreeLocation"
        || location.directory.is_none() != location.identity.is_none()
    {
        return Err(WorktreeError::ownership());
    }
    if let Some(path) = location.directory {
        let configured = Storage::open(&path, false)?;
        let identity = detect_workspace_identity(configured.root.path())
            .map_err(|_| WorktreeError::ownership())?;
        if location.identity != Some((identity.version(), identity.sha256().into())) {
            return Err(WorktreeError::ownership());
        }
        return Ok(path);
    }
    Ok(default)
}

/// Select an explicit new/empty or already-owned central directory for future operations.
/// Existing worktrees and their source branches are retained in the previous directory.
pub fn select_storage_directory(directory: Option<&Path>) -> Result<PathBuf> {
    let default = default_root()?;
    let defaults = Storage::open(&default, true)?;
    let selected = directory.unwrap_or(&default);
    let configured = Storage::open(selected, true)?;
    let _lease = defaults.lock()?;
    let identity = detect_workspace_identity(configured.root.path())
        .map_err(|_| WorktreeError::ownership())?;
    let location = Location {
        schema_version: 1,
        purpose: "ColossusWorktreeLocation".into(),
        directory: directory.map(Path::to_owned),
        identity: directory.map(|_| (identity.version(), identity.sha256().into())),
    };
    let old = defaults
        .root
        .open_file(Path::new("location.json"))
        .map_err(|_| WorktreeError::ownership())?;
    let staged = defaults
        .root
        .open_file(Path::new(&format!(
            ".location-{}",
            uuid::Uuid::now_v7().simple()
        )))
        .map_err(|_| WorktreeError::ownership())?;
    let mut output = staged.file();
    serde_json::to_writer_pretty(&mut output, &location).map_err(|_| WorktreeError::io())?;
    output.write_all(b"\n").map_err(|_| WorktreeError::io())?;
    output.sync_all().map_err(|_| WorktreeError::io())?;
    defaults
        .root
        .replace_existing_file(&staged, &old)
        .map_err(|_| WorktreeError::uncertain())?;
    Ok(selected.to_owned())
}

/// Read global worktree defaults without selecting or starting a repository runtime.
pub fn preferences() -> Result<(PathBuf, crate::WorktreeSettings)> {
    let path = configured_root()?;
    let storage = Storage::open(&path, false)?;
    Ok((path, storage.load()?.settings))
}

/// Persist defaults for future checkouts without starting a repository runtime.
pub fn update_preferences(
    value: crate::WorktreeSettings,
) -> Result<(PathBuf, crate::WorktreeSettings)> {
    validation::settings(&value)?;
    let path = configured_root()?;
    let storage = Storage::open(&path, false)?;
    let _lease = storage.lock()?;
    let mut catalog = storage.load()?;
    catalog.settings = value.clone();
    storage.save(&catalog)?;
    Ok((path, value))
}
fn write(mut file: &fs::File, catalog: &Catalog) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(catalog).map_err(|_| WorktreeError::io())?;
    file.rewind().map_err(|_| WorktreeError::io())?;
    file.set_len(0).map_err(|_| WorktreeError::io())?;
    file.write_all(&bytes).map_err(|_| WorktreeError::io())?;
    file.write_all(b"\n").map_err(|_| WorktreeError::io())?;
    file.sync_all().map_err(|_| WorktreeError::io())
}
