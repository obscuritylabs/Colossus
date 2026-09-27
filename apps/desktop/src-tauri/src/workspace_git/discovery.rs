use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::error;
use crate::{
    desktop_settings::{WorkspaceSetting, revalidate_workspace, validate_workspace},
    dto::CommandErrorDto,
};

/// Native-only binding. Git paths and filesystem identities never cross IPC.
#[derive(Clone)]
pub(super) struct RepositoryBinding {
    pub root: WorkspaceSetting,
    pub git: WorkspaceSetting,
    pub common: WorkspaceSetting,
    pub workspace: PathBuf,
}

impl RepositoryBinding {
    pub fn needs_approval(&self) -> bool {
        self.root.path != self.workspace
            || !self.common.path.starts_with(&self.workspace)
            || !self.git.path.starts_with(&self.workspace)
    }

    pub fn same_repository(&self, other: &Self) -> bool {
        self.workspace == other.workspace
            && [&self.root, &self.git, &self.common]
                .into_iter()
                .zip([&other.root, &other.git, &other.common])
                .all(|(a, b)| a.path == b.path && a.identity == b.identity)
    }

    pub fn revalidate(&self) -> Result<(), CommandErrorDto> {
        for directory in [&self.root, &self.git, &self.common] {
            revalidate_workspace(directory)?;
        }
        let current = discover(&self.workspace)?
            .ok_or_else(|| error("The repository was removed. Refresh Git."))?;
        if !self.same_repository(&current) {
            return Err(error("The repository changed. Refresh Git to reconnect."));
        }
        Ok(())
    }

    pub fn relative(&self, path: &Path) -> Option<String> {
        let scoped = self.workspace.strip_prefix(&self.root.path).ok()?;
        let relative = path.strip_prefix(scoped).ok()?;
        let value = relative.to_str()?.replace('\\', "/");
        (!value.is_empty()
            && value.len() <= 2048
            && !value.chars().any(char::is_control)
            && relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))))
        .then_some(value)
    }
}

/// Inspect only directory markers and bounded worktree pointer files during discovery.
/// Reading repository metadata outside the workspace requires a later native approval.
pub(super) fn discover(workspace: &Path) -> Result<Option<RepositoryBinding>, CommandErrorDto> {
    for root in workspace.ancestors().take(32) {
        let marker = root.join(".git");
        let metadata = match fs::symlink_metadata(&marker) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(error("Git metadata is not readable.")),
        };
        reject_link(&marker)?;
        let (git, common) = if metadata.is_dir() {
            (marker.clone(), marker)
        } else if metadata.is_file() {
            let pointer = read_small(&marker, 4096)?;
            let target = pointer
                .trim()
                .strip_prefix("gitdir: ")
                .ok_or_else(|| error("This Git directory marker is invalid."))?;
            let git = resolve_pointer(root, target)?;
            let common = resolve_pointer(&git, read_small(&git.join("commondir"), 4096)?.trim())?;
            // Linked worktrees must have both the conventional common-dir relationship
            // and an exact backlink. Separate-git-dir and submodule roots are not guessed.
            if git.parent() != Some(common.join("worktrees").as_path())
                || resolve_pointer(&git, read_small(&git.join("gitdir"), 4096)?.trim())?
                    != fs::canonicalize(&marker)
                        .map_err(|_| error("The worktree marker is unavailable."))?
            {
                return Err(error(
                    "This Git directory layout is not supported. Open a repository root or a linked worktree.",
                ));
            }
            (git, common)
        } else {
            return Err(error("This Git directory marker is invalid."));
        };
        return Ok(Some(RepositoryBinding {
            root: validate_workspace(
                &fs::canonicalize(root)
                    .map_err(|_| error("The repository folder is unavailable."))?,
            )?,
            git: validate_workspace(
                &fs::canonicalize(git).map_err(|_| error("The Git directory is unavailable."))?,
            )?,
            common: validate_workspace(
                &fs::canonicalize(common)
                    .map_err(|_| error("The shared Git directory is unavailable."))?,
            )?,
            workspace: workspace.into(),
        }));
    }
    Ok(None)
}

fn resolve_pointer(base: &Path, value: &str) -> Result<PathBuf, CommandErrorDto> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(error("The worktree metadata link is invalid."));
    }
    let path = base.join(value);
    reject_link(&path)?;
    fs::canonicalize(path).map_err(|_| error("The linked Git metadata is unavailable."))
}

pub(super) fn reject_link(path: &Path) -> Result<(), CommandErrorDto> {
    for component in path.ancestors() {
        let meta =
            fs::symlink_metadata(component).map_err(|_| error("Git metadata is unavailable."))?;
        if meta.file_type().is_symlink() || is_reparse(&meta) {
            return Err(error(
                "Linked filesystem paths are not supported for Git inspection.",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    meta.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool {
    false
}

pub(super) fn read_small(path: &Path, maximum: u64) -> Result<String, CommandErrorDto> {
    reject_link(path)?;
    let metadata = fs::metadata(path).map_err(|_| error("Git metadata is not readable."))?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(error("Git metadata exceeds the inspection limit."));
    }
    let mut value = String::new();
    fs::File::open(path)
        .map_err(|_| error("Git metadata is not readable."))?
        .take(maximum + 1)
        .read_to_string(&mut value)
        .map_err(|_| error("Git metadata is not valid text."))?;
    if value.len() as u64 > maximum {
        return Err(error("Git metadata exceeds the inspection limit."));
    }
    Ok(value)
}

/// Bound metadata traversal and reject alternate object stores and filesystem links
/// before libgit2 opens refs, index or objects. Worktree sibling indexes are not read.
pub(super) fn inspect_metadata(binding: &RepositoryBinding) -> Result<(), CommandErrorDto> {
    for path in [
        binding.common.path.join("config"),
        binding.git.path.join("config.worktree"),
    ] {
        if path
            .try_exists()
            .map_err(|_| error("Git configuration is unavailable."))?
        {
            let configuration = read_small(&path, 256 * 1024)?;
            let normalized: String = configuration
                .chars()
                .filter(|c| !c.is_ascii_whitespace())
                .flat_map(char::to_lowercase)
                .collect();
            if normalized.contains("[include") {
                return Err(error(
                    "Repository config includes are not supported for Desktop Git inspection. Use a self-contained repository configuration.",
                ));
            }
        }
    }
    let started = Instant::now();
    let mut pending = vec![binding.common.path.clone()];
    if binding.git.path != binding.common.path {
        pending.push(binding.git.path.clone());
    }
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|_| error("Git metadata is not readable."))? {
            count += 1;
            if count > 100_000 || started.elapsed() > Duration::from_secs(5) {
                return Err(error(
                    "This repository is too large for Desktop Git inspection.",
                ));
            }
            let entry = entry.map_err(|_| error("Git metadata is not readable."))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| error("Git metadata is not readable."))?;
            if metadata.file_type().is_symlink() || is_reparse(&metadata) {
                return Err(error(
                    "Git metadata contains an unsupported filesystem link.",
                ));
            }
            if path == binding.common.path.join("objects/info/alternates")
                || path == binding.common.path.join("objects/info/http-alternates")
            {
                return Err(error(
                    "Repositories with alternate object stores are not supported yet.",
                ));
            }
            if metadata.is_dir() {
                if path != binding.common.path.join("worktrees")
                    && path != binding.common.path.join("modules")
                    && path != binding.common.path.join("hooks")
                    && path != binding.common.path.join("logs")
                {
                    pending.push(path);
                }
            } else if !metadata.is_file()
                || metadata.len() > 512 * 1024 * 1024
                || (entry.file_name() == "index" && metadata.len() > 16 * 1024 * 1024)
            {
                return Err(error("Git metadata exceeds the inspection limit."));
            }
        }
    }
    Ok(())
}
