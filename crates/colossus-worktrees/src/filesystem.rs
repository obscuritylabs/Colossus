use crate::{
    WorktreeError, WorktreeErrorCode,
    error::Result,
    tree::{TrackedLink, TrackedLinks},
};
use colossus_home::ConfinedRoot;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MAX_ENTRIES: usize = 100_000;

pub(crate) fn reject_links(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(WorktreeError::ownership());
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| WorktreeError::io())?;
        if metadata.file_type().is_symlink() || reparse(&metadata) {
            return Err(WorktreeError::ownership());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
const fn reparse(_: &fs::Metadata) -> bool {
    false
}

pub(crate) fn read_small(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    use std::io::Read as _;
    reject_links(path)?;
    let file = fs::File::open(path).map_err(|_| WorktreeError::io())?;
    let metadata = file.metadata().map_err(|_| WorktreeError::io())?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(WorktreeError::ownership());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| WorktreeError::io())?;
    if bytes.len() as u64 > maximum {
        return Err(WorktreeError::ownership());
    }
    Ok(bytes)
}

/// No recursive walker follows a link. Only exact tracked contained file links
/// are retained for a separately validated, nonrecursive leaf unlink.
pub(crate) fn inspect_checkout(root: &Path, approved: &TrackedLinks) -> Result<Vec<LinkLeaf>> {
    reject_links(root)?;
    let started = Instant::now();
    let mut pending = vec![root.to_owned()];
    let mut count = 0;
    let mut leaves = Vec::new();
    while let Some(directory) = pending.pop() {
        reject_links(&directory)?;
        for entry in fs::read_dir(directory).map_err(|_| WorktreeError::io())? {
            count += 1;
            if count > MAX_ENTRIES || started.elapsed() > Duration::from_secs(10) {
                return Err(WorktreeError::new(
                    WorktreeErrorCode::Limit,
                    "The worktree is too large for a bounded removal check.",
                ));
            }
            let entry = entry.map_err(|_| WorktreeError::io())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| WorktreeError::io())?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| WorktreeError::ownership())?;
            if let Some(link) = approved.get(relative) {
                validate_link(root, &path, &metadata, link)?;
                if metadata.file_type().is_symlink() {
                    let parent =
                        ConfinedRoot::bind(path.parent().ok_or_else(WorktreeError::ownership)?)
                            .map_err(|_| WorktreeError::ownership())?;
                    leaves.push(LinkLeaf {
                        parent,
                        path,
                        root: root.to_owned(),
                        literal: link.literal.clone(),
                        target: link.target.clone(),
                    });
                    continue;
                }
            }
            if metadata.file_type().is_symlink()
                || reparse(&metadata)
                || (!metadata.is_file() && !metadata.is_dir())
            {
                return Err(WorktreeError::new(
                    WorktreeErrorCode::Ownership,
                    "The checkout contains filesystem links or special files. Retain it and remove those explicitly before retrying.",
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if link_count(&path, &metadata)? != 1 {
                return Err(WorktreeError::new(
                    WorktreeErrorCode::Ownership,
                    "The checkout contains hard-linked files. Retain it and remove those links explicitly.",
                ));
            }
        }
    }
    Ok(leaves)
}

fn validate_link(
    root: &Path,
    path: &Path,
    metadata: &fs::Metadata,
    link: &TrackedLink,
) -> Result<()> {
    let target = root.join(&link.target);
    reject_links(&target)?;
    if !fs::symlink_metadata(&target)
        .map_err(|_| WorktreeError::ownership())?
        .is_file()
    {
        return Err(WorktreeError::ownership());
    }
    if metadata.file_type().is_symlink() {
        if metadata.is_dir()
            || fs::read_link(path)
                .map_err(|_| WorktreeError::ownership())?
                .as_os_str()
                != Path::new(&link.literal).as_os_str()
        {
            return Err(WorktreeError::ownership());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt as _;
            // File symbolic links are the only admitted reparse leaf. Junctions,
            // mount points and directory symbolic links remain rejected.
            if metadata.file_attributes() & 0x10 != 0 {
                return Err(WorktreeError::ownership());
            }
        }
    } else if !cfg!(windows)
        || !metadata.is_file()
        || reparse(metadata)
        || read_small(path, 2048)? != link.literal.as_bytes()
    {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}

pub(crate) struct LinkLeaf {
    parent: ConfinedRoot,
    path: PathBuf,
    root: PathBuf,
    literal: String,
    target: PathBuf,
}
impl LinkLeaf {
    pub fn unlink(self) -> Result<()> {
        self.parent
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
        if self.path.parent() != Some(self.parent.path()) {
            return Err(WorktreeError::ownership());
        }
        let metadata = fs::symlink_metadata(&self.path).map_err(|_| WorktreeError::ownership())?;
        if !metadata.file_type().is_symlink() {
            return Err(WorktreeError::ownership());
        }
        validate_link(
            &self.root,
            &self.path,
            &metadata,
            &TrackedLink {
                literal: self.literal,
                target: self.target,
            },
        )?;
        self.parent
            .revalidate()
            .map_err(|_| WorktreeError::ownership())?;
        // remove_file unlinks this exact leaf without following its destination.
        // This is pathname-based under checked private parent custody, not an
        // atomic exclusion of same-user writers that ignore Colossus leases.
        fs::remove_file(&self.path).map_err(|_| WorktreeError::uncertain())?;
        match fs::symlink_metadata(&self.path) {
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(WorktreeError::uncertain()),
        }
        self.parent
            .revalidate()
            .map_err(|_| WorktreeError::uncertain())?;
        Ok(())
    }
}

#[cfg(unix)]
fn link_count(_: &Path, metadata: &fs::Metadata) -> Result<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(metadata.nlink())
}
#[cfg(windows)]
fn link_count(path: &Path, _: &fs::Metadata) -> Result<u64> {
    colossus_windows_native::BoundPath::open_file(path)
        .and_then(|bound| bound.link_count())
        .map_err(|_| WorktreeError::ownership())
}
#[cfg(not(any(unix, windows)))]
fn link_count(_: &Path, _: &fs::Metadata) -> Result<u64> {
    Err(WorktreeError::ownership())
}
