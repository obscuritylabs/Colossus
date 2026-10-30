//! Bounded commit admission, including contained tracked relative file links.
use crate::{
    WorktreeError, WorktreeErrorCode,
    error::Result,
    filesystem::read_small,
    repository::{Binding, SafeRepository},
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct TrackedLink {
    pub literal: String,
    pub target: PathBuf,
}
pub(crate) type TrackedLinks = BTreeMap<PathBuf, TrackedLink>;

fn unsupported() -> WorktreeError {
    WorktreeError::new(
        WorktreeErrorCode::UnsupportedRepository,
        "The commit contains unsupported links, submodules, oversized files or paths outside managed checkout bounds.",
    )
}

pub(crate) fn inspect_commit(
    repository: &git2::Repository,
    commit: &git2::Commit<'_>,
) -> Result<TrackedLinks> {
    let tree = commit.tree().map_err(|_| WorktreeError::ownership())?;
    let mut count = 0;
    let mut total_bytes = 0_usize;
    let mut rejected = false;
    let started = Instant::now();
    let mut links = BTreeMap::new();
    let result = tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        let inspect = (|| -> Result<()> {
            count += 1;
            if count > 100_000
                || started.elapsed() > Duration::from_secs(10)
                || root.len() + entry.name_bytes().len() > 2048
                || entry.filemode() == 0o160_000
                || entry.name_bytes().iter().any(u8::is_ascii_control)
            {
                return Err(unsupported());
            }
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let blob = repository
                    .find_blob(entry.id())
                    .map_err(|_| unsupported())?;
                total_bytes = total_bytes.saturating_add(blob.size());
                if blob.size() > 64 * 1024 * 1024 || total_bytes > 512 * 1024 * 1024 {
                    return Err(unsupported());
                }
                if entry.filemode() == 0o120_000 {
                    if blob.size() > 2048 {
                        return Err(unsupported());
                    }
                    let literal = std::str::from_utf8(blob.content()).map_err(|_| unsupported())?;
                    let name = entry.name().map_err(|_| unsupported())?;
                    let relative = Path::new(root).join(name);
                    let target = resolve_relative(&relative, literal)?;
                    let destination = tree.get_path(&target).map_err(|_| unsupported())?;
                    if destination.kind() != Some(git2::ObjectType::Blob)
                        || !matches!(destination.filemode(), 0o100_644 | 0o100_755)
                    {
                        return Err(unsupported());
                    }
                    links.insert(
                        relative,
                        TrackedLink {
                            literal: literal.into(),
                            target,
                        },
                    );
                }
            }
            Ok(())
        })();
        if inspect.is_err() {
            rejected = true;
            git2::TreeWalkResult::Abort
        } else {
            git2::TreeWalkResult::Ok
        }
    });
    if result.is_err() || rejected {
        return Err(unsupported());
    }
    Ok(links)
}

fn resolve_relative(link: &Path, literal: &str) -> Result<PathBuf> {
    if literal.is_empty()
        || literal.starts_with('/')
        || literal.contains(['\\', ':'])
        || literal.chars().any(char::is_control)
    {
        return Err(unsupported());
    }
    let mut parts = link
        .parent()
        .ok_or_else(unsupported)?
        .components()
        .map(|part| part.as_os_str().to_owned())
        .collect::<Vec<_>>();
    for part in literal.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(unsupported());
                }
            }
            value => parts.push(value.into()),
        }
    }
    if parts.is_empty() {
        return Err(unsupported());
    }
    Ok(parts.into_iter().collect())
}

/// HEAD/index comparison detects independent Git changes around the clean check.
pub(crate) struct CleanSnapshot {
    head: git2::Oid,
    index: [u8; 32],
}
impl CleanSnapshot {
    pub fn capture(repository: &SafeRepository, binding: &Binding) -> Result<(Self, TrackedLinks)> {
        let head = repository
            .repo
            .head()
            .and_then(|reference| reference.peel_to_commit())
            .map_err(|_| unsupported())?;
        let snapshot = Self {
            head: head.id(),
            index: index_digest(binding)?,
        };
        let links = inspect_commit(&repository.repo, &head)?;
        if repository.dirty()? {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Dirty,
                "The worktree contains changes, untracked files or ignored files. Commit or preserve them before removal.",
            ));
        }
        snapshot.revalidate(repository, binding)?;
        Ok((snapshot, links))
    }
    pub fn revalidate(&self, repository: &SafeRepository, binding: &Binding) -> Result<()> {
        binding.revalidate()?;
        let head = repository
            .repo
            .head()
            .and_then(|reference| reference.peel_to_commit())
            .map_err(|_| WorktreeError::ownership())?;
        if head.id() != self.head || index_digest(binding)? != self.index {
            return Err(WorktreeError::new(
                WorktreeErrorCode::Conflict,
                "Git HEAD or index changed during the removal check. Refresh and inspect the checkout.",
            ));
        }
        Ok(())
    }
}
fn index_digest(binding: &Binding) -> Result<[u8; 32]> {
    Ok(Sha256::digest(read_small(
        &binding.git.canonical_path().join("index"),
        16 * 1024 * 1024,
    )?)
    .into())
}
