use crate::{
    WorktreeError, WorktreeErrorCode,
    error::Result,
    filesystem::{read_small, reject_links},
};
use colossus_home::{WorkspaceIdentity, detect_workspace_identity};
use git2::{Repository, RepositoryOpenFlags, StatusOptions};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// One exact repository root and its conventional Git metadata directories.
pub(crate) struct Binding {
    pub workspace: WorkspaceIdentity,
    pub git: WorkspaceIdentity,
    pub common: WorkspaceIdentity,
    pub id: String,
}
impl Binding {
    pub fn discover(workspace: &Path) -> Result<Self> {
        let workspace = fs::canonicalize(workspace).map_err(|_| {
            WorktreeError::new(
                WorktreeErrorCode::InvalidInput,
                "Select an existing repository root.",
            )
        })?;
        reject_links(&workspace)?;
        let marker = workspace.join(".git");
        let metadata = fs::symlink_metadata(&marker).map_err(|_| unsupported())?;
        reject_links(&marker)?;
        let (git, common) = if metadata.is_dir() {
            (marker.clone(), marker)
        } else if metadata.is_file() {
            let bytes = read_small(&marker, 4096)?;
            let pointer = std::str::from_utf8(&bytes)
                .map_err(|_| unsupported())?
                .trim()
                .strip_prefix("gitdir: ")
                .ok_or_else(unsupported)?;
            let git = pointer_path(&workspace, pointer)?;
            let common_bytes = read_small(&git.join("commondir"), 4096)?;
            let common = pointer_path(
                &git,
                std::str::from_utf8(&common_bytes)
                    .map_err(|_| unsupported())?
                    .trim(),
            )?;
            let back = read_small(&git.join("gitdir"), 4096)?;
            if git.parent() != Some(common.join("worktrees").as_path())
                || pointer_path(
                    &git,
                    std::str::from_utf8(&back)
                        .map_err(|_| unsupported())?
                        .trim(),
                )? != fs::canonicalize(&marker).map_err(|_| unsupported())?
            {
                return Err(unsupported());
            }
            (git, common)
        } else {
            return Err(unsupported());
        };
        let common = detect_workspace_identity(&common).map_err(|_| WorktreeError::ownership())?;
        let mut digest = Sha256::new();
        digest.update(b"colossus-managed-worktree-repository-v1\0");
        digest.update(common.canonical_path().as_os_str().as_encoded_bytes());
        digest.update(common.version().to_le_bytes());
        digest.update(common.sha256().as_bytes());
        Ok(Self {
            workspace: detect_workspace_identity(&workspace)
                .map_err(|_| WorktreeError::ownership())?,
            git: detect_workspace_identity(&git).map_err(|_| WorktreeError::ownership())?,
            common,
            id: hex::encode(digest.finalize()),
        })
    }
    pub fn revalidate(&self) -> Result<()> {
        for binding in [&self.workspace, &self.git, &self.common] {
            binding
                .revalidate()
                .map_err(|_| WorktreeError::ownership())?;
        }
        let current = Self::discover(self.workspace.canonical_path())?;
        if current.id != self.id || current.git != self.git {
            return Err(WorktreeError::ownership());
        }
        Ok(())
    }
    pub fn inspect_metadata(&self) -> Result<()> {
        for directory in [&self.common, &self.git] {
            let config = directory
                .canonical_path()
                .join(if directory == &self.common {
                    "config"
                } else {
                    "config.worktree"
                });
            if config.try_exists().map_err(|_| WorktreeError::io())? {
                let bytes = read_small(&config, 256 * 1024)?;
                let value: String = std::str::from_utf8(&bytes)
                    .map_err(|_| unsupported())?
                    .chars()
                    .filter(|c| !c.is_ascii_whitespace())
                    .flat_map(char::to_lowercase)
                    .collect();
                if value.contains("[include") {
                    return Err(WorktreeError::new(
                        WorktreeErrorCode::UnsupportedRepository,
                        "Repository configuration includes are unsupported for managed worktree operations.",
                    ));
                }
            }
        }
        let started = Instant::now();
        let mut pending = vec![self.common.canonical_path().to_owned()];
        let mut count = 0;
        while let Some(directory) = pending.pop() {
            reject_links(&directory)?;
            for entry in fs::read_dir(&directory).map_err(|_| WorktreeError::io())? {
                count += 1;
                if count > 100_000 || started.elapsed() > Duration::from_secs(10) {
                    return Err(WorktreeError::new(
                        WorktreeErrorCode::Limit,
                        "Repository metadata exceeds the bounded worktree inspection limit.",
                    ));
                }
                let entry = entry.map_err(|_| WorktreeError::io())?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).map_err(|_| WorktreeError::io())?;
                reject_links(&path)?;
                let relative = path
                    .strip_prefix(self.common.canonical_path())
                    .map_err(|_| WorktreeError::ownership())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative.eq_ignore_ascii_case("objects/info/alternates")
                    || relative.eq_ignore_ascii_case("objects/info/http-alternates")
                {
                    return Err(WorktreeError::new(
                        WorktreeErrorCode::UnsupportedRepository,
                        "Alternate Git object stores are unsupported for managed worktrees.",
                    ));
                }
                if metadata.is_dir() {
                    if !["worktrees", "modules", "hooks", "logs"]
                        .iter()
                        .any(|name| path == self.common.canonical_path().join(name))
                    {
                        pending.push(path);
                    }
                } else if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
                    return Err(unsupported());
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn pointer_path(base: &Path, value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(unsupported());
    }
    let raw = base.join(value);
    // Check the unresolved spelling too: canonicalization must not erase a
    // symlink/reparse point from a Git pointer containing conventional ../..
    // components.
    for ancestor in raw.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| unsupported())?;
        if metadata.file_type().is_symlink() {
            return Err(WorktreeError::ownership());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt as _;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(WorktreeError::ownership());
            }
        }
    }
    let path = fs::canonicalize(raw).map_err(|_| unsupported())?;
    reject_links(&path)?;
    Ok(path)
}
fn unsupported() -> WorktreeError {
    WorktreeError::new(
        WorktreeErrorCode::UnsupportedRepository,
        "Managed worktrees require a conventional repository root or linked worktree with a valid backlink.",
    )
}

pub(crate) struct SafeRepository {
    pub repo: Repository,
    _config: tempfile::NamedTempFile,
}
impl SafeRepository {
    pub fn open(binding: &Binding, private_root: &Path) -> Result<Self> {
        binding.revalidate()?;
        binding.inspect_metadata()?;
        let repo = Repository::open_ext(
            binding.workspace.canonical_path(),
            RepositoryOpenFlags::NO_SEARCH,
            std::iter::empty::<&Path>(),
        )
        .map_err(|_| unsupported())?;
        if repo.is_bare()
            || fs::canonicalize(repo.path()).map_err(|_| unsupported())?
                != binding.git.canonical_path()
            || fs::canonicalize(repo.commondir()).map_err(|_| unsupported())?
                != binding.common.canonical_path()
        {
            return Err(WorktreeError::ownership());
        }
        let original = repo.config().map_err(|_| unsupported())?;
        if ["core.attributesfile", "core.excludesfile"]
            .iter()
            .any(|name| {
                original
                    .get_string(name)
                    .is_ok_and(|value| !value.is_empty())
            })
        {
            return Err(WorktreeError::new(
                WorktreeErrorCode::UnsupportedRepository,
                "Managed worktrees currently require built-in Git file handling without external attribute or ignore files.",
            ));
        }
        // Only bounded scalar comparison settings survive; no executable/helper or
        // arbitrary external attribute/ignore configuration reaches libgit2 calls.
        let filemode = original.get_bool("core.filemode").unwrap_or(!cfg!(windows));
        let symlinks = original.get_bool("core.symlinks").unwrap_or(!cfg!(windows));
        let ignorecase = original
            .get_bool("core.ignorecase")
            .unwrap_or(cfg!(windows));
        let autocrlf = match original.get_string("core.autocrlf").ok().as_deref() {
            Some("true") => "true",
            Some("input") => "input",
            _ => "false",
        };
        let mut config =
            tempfile::NamedTempFile::new_in(private_root).map_err(|_| WorktreeError::io())?;
        writeln!(config, "[core]\n bare = false\n filemode = {filemode}\n symlinks = {symlinks}\n ignorecase = {ignorecase}\n autocrlf = {autocrlf}\n attributesfile =\n excludesfile =\n hooksPath =\n[diff]\n renames = false").map_err(|_| WorktreeError::io())?;
        config.flush().map_err(|_| WorktreeError::io())?;
        repo.set_config(&git2::Config::open(config.path()).map_err(|_| unsupported())?)
            .map_err(|_| unsupported())?;
        repo.set_workdir(binding.workspace.canonical_path(), false)
            .map_err(|_| unsupported())?;
        Ok(Self {
            repo,
            _config: config,
        })
    }
    pub fn dirty(&self) -> Result<bool> {
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(true)
            .recurse_ignored_dirs(true)
            .exclude_submodules(false)
            .update_index(false);
        let statuses = self.repo.statuses(Some(&mut options)).map_err(|_| {
            WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "Worktree status could not be established.",
            )
        })?;
        if statuses.len() > 100_000 {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Limit,
                "Worktree status exceeds the inspection limit.",
            ));
        }
        Ok(statuses
            .iter()
            .any(|status| status.status() != git2::Status::CURRENT))
    }
}
