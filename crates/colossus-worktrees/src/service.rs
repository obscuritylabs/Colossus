use crate::{
    CreateWorktree, ManagedWorktree, WorktreeCatalog, WorktreeError, WorktreeErrorCode,
    WorktreeSettings, WorktreeState,
    error::Result,
    filesystem::{inspect_checkout, read_small},
    lease::{ORIGIN_FILE, exclusive, validate_entry, validate_origin},
    model::{Catalog, Entry, Origin},
    repository::{Binding, SafeRepository},
    storage::{MAX_WORKTREES, Storage, configured_root},
    tree::{CleanSnapshot, inspect_commit},
    validation,
};
use colossus_home::ConfinedRoot;
use git2::{BranchType, Oid, WorktreeAddOptions, WorktreeLockStatus, WorktreePruneOptions};
use serde::Serialize;
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Human-operated worktree service. A manager binds one selected repository and
/// private catalog; IDs never authorize arbitrary directories or unowned Git trees.
pub struct WorktreeManager {
    binding: Binding,
    storage: Storage,
    _trees: ConfinedRoot,
}
impl WorktreeManager {
    /// Bind an exact repository root and selected managed directory. No runtime,
    /// provider, agent permission or repository executable is started.
    pub fn open(workspace: &Path, root_override: Option<&Path>) -> Result<Self> {
        let binding = Binding::discover(workspace)?;
        let root = root_override.map_or_else(configured_root, |root| Ok(root.to_owned()))?;
        let storage = Storage::open(&root, true)?;
        let trees = storage
            .root
            .prepare_directory(Path::new("trees"))
            .map_err(|_| WorktreeError::ownership())?;
        let _trees = ConfinedRoot::bind(&trees).map_err(|_| WorktreeError::ownership())?;
        Ok(Self {
            binding,
            storage,
            _trees,
        })
    }

    /// Current bounded inventory; interrupted operations remain visible and are not replayed.
    pub fn list(&self) -> Result<WorktreeCatalog> {
        let _lease = self.storage.lock()?;
        self.binding.revalidate()?;
        let catalog = self.storage.load()?;
        let mut worktrees = Vec::new();
        for entry in catalog
            .entries
            .iter()
            .filter(|entry| entry.repository_id == self.binding.id)
        {
            worktrees.push(self.describe(entry));
        }
        worktrees.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(WorktreeCatalog {
            repository_name: self
                .binding
                .workspace
                .canonical_path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            repository_id: self.binding.id.clone(),
            storage_root: self.storage.root.path().to_owned(),
            settings: catalog.settings,
            worktrees,
            max_worktrees: MAX_WORKTREES,
        })
    }

    /// Persist future creation preferences without changing any existing checkout.
    pub fn update_settings(&self, settings: WorktreeSettings) -> Result<WorktreeSettings> {
        validation::settings(&settings)?;
        let _lease = self.storage.lock()?;
        let mut catalog = self.storage.load()?;
        catalog.settings = settings.clone();
        self.storage.save(&catalog)?;
        Ok(settings)
    }

    /// Create a separate checkout at a pinned commit. A failed effect retains its
    /// intent and any branch/files for explicit reconciliation; nothing is reset.
    pub fn create(&self, request: &CreateWorktree) -> Result<ManagedWorktree> {
        validation::request(request)?;
        let _lease = self.storage.lock()?;
        let mut catalog = self.storage.load()?;
        // Removed records can leave the bounded inventory after their durable receipt.
        if catalog.entries.len() >= MAX_WORKTREES {
            catalog
                .entries
                .retain(|entry| entry.state != WorktreeState::Removed);
        }
        if catalog.entries.len() >= MAX_WORKTREES {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Limit,
                "Archive and remove an unused managed worktree before creating another.",
            ));
        }
        if catalog.entries.iter().any(|entry| {
            entry.repository_id == self.binding.id
                && entry.name == request.name
                && entry.state != WorktreeState::Removed
        }) {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "A managed worktree already uses that name.",
            ));
        }
        let repository = SafeRepository::open(&self.binding, self.storage.root.path())?;
        let branch = request
            .branch
            .clone()
            .unwrap_or_else(|| format!("{}{}", catalog.settings.branch_prefix, request.name));
        validation::branch(&branch)?;
        if request.existing_branch
            && branch_is_checked_out(self.binding.common.canonical_path(), &branch)?
        {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "The local branch is already checked out. Choose another branch or create a new one.",
            ));
        }
        let commit = if request.existing_branch {
            repository
                .repo
                .find_branch(&branch, BranchType::Local)
                .and_then(|branch| branch.into_reference().peel_to_commit())
                .map_err(|_| {
                    WorktreeError::new(
                        WorktreeErrorCode::NotFound,
                        "The requested local branch does not exist.",
                    )
                })?
        } else {
            if repository
                .repo
                .find_branch(&branch, BranchType::Local)
                .is_ok()
            {
                return Err(WorktreeError::new(
                    WorktreeErrorCode::Conflict,
                    "The branch already exists. Choose an existing-branch checkout or a new branch name.",
                ));
            }
            resolve_commit(&repository.repo, request.base.as_deref().unwrap_or("HEAD"))?
        };
        let approved_links = inspect_commit(&repository.repo, &commit)?;
        let id = uuid::Uuid::now_v7().simple().to_string();
        let path = self.storage.root.path().join("trees").join(&id);
        if path.try_exists().map_err(|_| WorktreeError::io())? {
            return Err(WorktreeError::ownership());
        }
        let mut entry = Entry {
            id: id.clone(),
            name: request.name.clone(),
            branch: branch.clone(),
            base_commit: commit.id().to_string(),
            repository_id: self.binding.id.clone(),
            common_path: self.binding.common.canonical_path().to_owned(),
            git_name: format!("colossus-{id}"),
            path,
            workspace_identity: None,
            state: WorktreeState::Creating,
            created_at_ms: now_ms()?,
        };
        catalog.entries.push(entry.clone());
        self.storage.save(&catalog)?;
        // Intent is durable before any Git ref, checkout or ownership marker effect.
        let result = (|| {
            self.binding.revalidate()?;
            self._trees
                .revalidate()
                .map_err(|_| WorktreeError::ownership())?;
            let reference = if request.existing_branch {
                repository.repo.find_branch(&branch, BranchType::Local)
            } else {
                repository.repo.branch(&branch, &commit, false)
            }
            .map_err(|_| WorktreeError::uncertain())?
            .into_reference();
            let mut options = WorktreeAddOptions::new();
            options.reference(Some(&reference));
            let worktree = repository
                .repo
                .worktree(&entry.git_name, &entry.path, Some(&options))
                .map_err(|_| WorktreeError::uncertain())?;
            worktree
                .validate()
                .map_err(|_| WorktreeError::uncertain())?;
            let _private =
                ConfinedRoot::bind(&entry.path).map_err(|_| WorktreeError::uncertain())?;
            let target = Binding::discover(&entry.path)?;
            let _validated_links = inspect_checkout(&entry.path, &approved_links)?;
            let identity = target.workspace.as_ref();
            entry.workspace_identity = Some((identity.version, identity.sha256.into()));
            validate_entry(&entry, &target, self.storage.root.path())?;
            let origin = Origin {
                schema_version: 1,
                purpose: "ColossusManagedWorktree".into(),
                id: entry.id.clone(),
                storage_root: self.storage.root.path().to_owned(),
                repository_id: entry.repository_id.clone(),
            };
            write_new(&target.git.canonical_path().join(ORIGIN_FILE), &origin)?;
            let use_lock = self
                .storage
                .root
                .open_file(&Path::new("leases").join(format!("{}.lock", entry.id)))
                .map_err(|_| WorktreeError::uncertain())?;
            if !use_lock.was_created() {
                return Err(WorktreeError::ownership());
            }
            use_lock
                .file()
                .sync_all()
                .map_err(|_| WorktreeError::uncertain())?;
            entry.state = WorktreeState::Active;
            replace_entry(&mut catalog, &entry)?;
            self.storage.save(&catalog)?;
            self.binding.revalidate()?;
            Ok(())
        })();
        if let Err(_failure) = result {
            entry.state = WorktreeState::NeedsAttention;
            let _retained =
                replace_entry(&mut catalog, &entry).and_then(|()| self.storage.save(&catalog));
            return Err(WorktreeError::uncertain());
        }
        Ok(self.describe(&entry))
    }

    /// Archive retains all files and the branch. Active Colossus users block this operation.
    pub fn archive(&self, id: &str) -> Result<ManagedWorktree> {
        let _lease = self.storage.lock()?;
        let mut catalog = self.storage.load()?;
        let mut entry = self.entry(&catalog, id)?.clone();
        self.reject_current(&entry)?;
        if entry.state != WorktreeState::Active {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "Only an active managed worktree can be archived.",
            ));
        }
        let _usage = exclusive(&self.storage, id)?;
        self.owned_binding(&entry)?;
        entry.state = WorktreeState::Archived;
        replace_entry(&mut catalog, &entry)?;
        self.storage.save(&catalog)?;
        drop(_usage);
        Ok(self.describe(&entry))
    }

    /// Restore an archived checkout and return its natively verified path. The caller
    /// must acquire a use lease before opening its runtime or retaining it as selected.
    pub fn open_path(&self, id: &str) -> Result<PathBuf> {
        let _lease = self.storage.lock()?;
        let mut catalog = self.storage.load()?;
        let mut entry = self.entry(&catalog, id)?.clone();
        if !matches!(entry.state, WorktreeState::Active | WorktreeState::Archived) {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "The worktree needs ownership reconciliation before it can be opened.",
            ));
        }
        // Existing active checkouts may have multiple shared users. Restoring
        // an archived checkout changes admission and requires exclusive use.
        let _usage = if entry.state == WorktreeState::Archived {
            Some(exclusive(&self.storage, id)?)
        } else {
            None
        };
        self.owned_binding(&entry)?;
        if entry.state == WorktreeState::Archived {
            entry.state = WorktreeState::Active;
            replace_entry(&mut catalog, &entry)?;
            self.storage.save(&catalog)?;
        }
        Ok(entry.path)
    }

    /// Remove only an archived, clean, unlocked and unused owned checkout. The Git
    /// branch remains available. Untracked and ignored files also block removal.
    pub fn remove(&self, id: &str) -> Result<ManagedWorktree> {
        let _lease = self.storage.lock()?;
        let mut catalog = self.storage.load()?;
        let mut entry = self.entry(&catalog, id)?.clone();
        self.reject_current(&entry)?;
        if entry.state != WorktreeState::Archived {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "Archive the managed worktree before removing it.",
            ));
        }
        let _usage = exclusive(&self.storage, id)?;
        let binding = self.owned_binding(&entry)?;
        let repository = SafeRepository::open(&self.binding, self.storage.root.path())?;
        let worktree = repository
            .repo
            .find_worktree(&entry.git_name)
            .map_err(|_| WorktreeError::ownership())?;
        worktree
            .validate()
            .map_err(|_| WorktreeError::ownership())?;
        if worktree
            .is_locked()
            .map_err(|_| WorktreeError::ownership())?
            != WorktreeLockStatus::Unlocked
        {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "Git has locked this worktree. Its lock is retained; unlock it explicitly before removal.",
            ));
        }
        let target = SafeRepository::open(&binding, self.storage.root.path())?;
        let (snapshot, approved_links) = CleanSnapshot::capture(&target, &binding)?;
        let link_leaves = inspect_checkout(&entry.path, &approved_links)?;
        snapshot.revalidate(&target, &binding)?;
        binding.revalidate()?;
        self._trees
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
        entry.state = WorktreeState::Removing;
        replace_entry(&mut catalog, &entry)?;
        self.storage.save(&catalog)?;
        // Intent precedes the first unlink. Partial failure retains Removing
        // state and never attempts replay, branch deletion or restoration.
        for leaf in link_leaves {
            snapshot
                .revalidate(&target, &binding)
                .map_err(|_| WorktreeError::uncertain())?;
            leaf.unlink().map_err(|_| WorktreeError::uncertain())?;
        }
        snapshot
            .revalidate(&target, &binding)
            .map_err(|_| WorktreeError::uncertain())?;
        drop(target);
        let mut options = WorktreePruneOptions::new();
        options.valid(true).locked(false).working_tree(true);
        if worktree.prune(Some(&mut options)).is_err() {
            return Err(WorktreeError::uncertain());
        }
        if entry
            .path
            .try_exists()
            .map_err(|_| WorktreeError::uncertain())?
            || repository.repo.find_worktree(&entry.git_name).is_ok()
        {
            return Err(WorktreeError::uncertain());
        }
        self.binding.revalidate()?;
        entry.state = WorktreeState::Removed;
        let receipt_directory = self
            .storage
            .root
            .prepare_directory(Path::new("receipts"))
            .map_err(|_| WorktreeError::uncertain())?;
        write_new(
            &receipt_directory.join(format!("{}.json", entry.id)),
            &entry,
        )?;
        replace_entry(&mut catalog, &entry)?;
        self.storage.save(&catalog)?;
        Ok(self.describe(&entry))
    }

    fn entry<'a>(&self, catalog: &'a Catalog, id: &str) -> Result<&'a Entry> {
        validation::id(id)?;
        catalog
            .entries
            .iter()
            .find(|entry| entry.id == id && entry.repository_id == self.binding.id)
            .ok_or_else(|| {
                WorktreeError::new(
                    WorktreeErrorCode::NotFound,
                    "That managed worktree does not belong to the selected repository.",
                )
            })
    }
    fn reject_current(&self, entry: &Entry) -> Result<()> {
        if entry.path == self.binding.workspace.canonical_path() {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Busy,
                "Switch away from this worktree before archiving or removing it.",
            ));
        }
        Ok(())
    }
    fn owned_binding(&self, entry: &Entry) -> Result<Binding> {
        self.binding.revalidate()?;
        let binding = Binding::discover(&entry.path)?;
        validate_entry(entry, &binding, self.storage.root.path())?;
        let origin: Origin = serde_json::from_slice(&read_small(
            &binding.git.canonical_path().join(ORIGIN_FILE),
            8192,
        )?)
        .map_err(|_| WorktreeError::ownership())?;
        validate_origin(&origin, &binding)?;
        if origin.id != entry.id || origin.storage_root != self.storage.root.path() {
            return Err(WorktreeError::ownership());
        }
        Ok(binding)
    }
    fn describe(&self, entry: &Entry) -> ManagedWorktree {
        let mut note = None;
        let mut in_use = false;
        let dirty = if matches!(entry.state, WorktreeState::Active | WorktreeState::Archived) {
            match self
                .owned_binding(entry)
                .and_then(|binding| SafeRepository::open(&binding, self.storage.root.path()))
                .and_then(|repository| repository.dirty())
            {
                Ok(value) => Some(value),
                Err(error) => {
                    note = Some(error.message.into());
                    None
                }
            }
        } else {
            None
        };
        if matches!(entry.state, WorktreeState::Active | WorktreeState::Archived) {
            match exclusive(&self.storage, &entry.id) {
                Ok(_lease) => {}
                Err(error) if error.code == WorktreeErrorCode::Busy => in_use = true,
                Err(error) => note = Some(error.message.into()),
            }
        }
        if matches!(
            entry.state,
            WorktreeState::Creating | WorktreeState::Removing | WorktreeState::NeedsAttention
        ) {
            note = Some("An interrupted operation needs inspection. Files and the branch are retained; no automatic replay or forced cleanup is performed.".into());
        }
        ManagedWorktree {
            id: entry.id.clone(),
            name: entry.name.clone(),
            branch: entry.branch.clone(),
            base_commit: entry.base_commit.clone(),
            path: entry.path.clone(),
            state: entry.state,
            created_at_ms: entry.created_at_ms,
            in_use,
            dirty,
            note,
        }
    }
}

fn resolve_commit<'a>(repository: &'a git2::Repository, base: &str) -> Result<git2::Commit<'a>> {
    let result = if base == "HEAD" {
        repository
            .head()
            .and_then(|reference| reference.peel_to_commit())
    } else if base.len() == 40 && base.bytes().all(|c| c.is_ascii_hexdigit()) {
        Oid::from_str(base).and_then(|id| repository.find_commit(id))
    } else {
        repository
            .find_branch(base, BranchType::Local)
            .and_then(|branch| branch.into_reference().peel_to_commit())
            .or_else(|_| {
                repository
                    .find_reference(&format!("refs/tags/{base}"))
                    .and_then(|reference| reference.peel_to_commit())
            })
    };
    result.map_err(|_| WorktreeError::new(WorktreeErrorCode::NotFound, "The base must resolve to an existing local branch, tag or commit. No fetch is performed."))
}
fn branch_is_checked_out(common: &Path, branch: &str) -> Result<bool> {
    let expected = format!("ref: refs/heads/{branch}");
    let head_matches = |path: &Path| -> Result<bool> {
        let bytes = read_small(path, 4096)?;
        Ok(std::str::from_utf8(&bytes)
            .map_err(|_| WorktreeError::ownership())?
            .trim()
            == expected)
    };
    if head_matches(&common.join("HEAD"))? {
        return Ok(true);
    }
    let directory = common.join("worktrees");
    if !directory.try_exists().map_err(|_| WorktreeError::io())? {
        return Ok(false);
    }
    crate::filesystem::reject_links(&directory)?;
    for (index, item) in fs::read_dir(directory)
        .map_err(|_| WorktreeError::io())?
        .enumerate()
    {
        if index >= 1024 {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Limit,
                "Too many Git worktree metadata entries to inspect safely.",
            ));
        }
        let path = item.map_err(|_| WorktreeError::io())?.path().join("HEAD");
        if head_matches(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}
fn replace_entry(catalog: &mut Catalog, entry: &Entry) -> Result<()> {
    let stored = catalog
        .entries
        .iter_mut()
        .find(|current| current.id == entry.id)
        .ok_or_else(WorktreeError::ownership)?;
    *stored = entry.clone();
    Ok(())
}
fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| WorktreeError::uncertain())?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|_| WorktreeError::uncertain())?;
    file.write_all(b"\n")
        .map_err(|_| WorktreeError::uncertain())?;
    file.sync_all().map_err(|_| WorktreeError::uncertain())
}
fn now_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .ok_or_else(WorktreeError::io)
}
