use super::BrowserProfileError;
use colossus_contracts::BrowserProfileId;
use colossus_home::{ConfinedFile, ConfinedRoot};
use std::{
    fs::File,
    os::unix::fs::{DirBuilderExt as _, MetadataExt as _},
    path::Path,
};

pub(super) fn nonce() -> Result<String, BrowserProfileError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    Ok(hex::encode(bytes))
}

pub(super) struct Locked {
    file: ConfinedFile,
    unlocked: bool,
}
impl Locked {
    pub(super) fn acquire(
        file: ConfinedFile,
        root: &ConfinedRoot,
    ) -> Result<Self, BrowserProfileError> {
        file.revalidate(root)
            .map_err(|_| BrowserProfileError::Denied)?;
        rustix::fs::flock(
            file.file(),
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                BrowserProfileError::Busy
            } else {
                BrowserProfileError::Denied
            }
        })?;
        let lock = Self {
            file,
            unlocked: false,
        };
        lock.revalidate(root)?;
        Ok(lock)
    }
    pub(super) fn revalidate(&self, root: &ConfinedRoot) -> Result<(), BrowserProfileError> {
        if self.unlocked {
            return Err(BrowserProfileError::Denied);
        }
        self.file
            .revalidate(root)
            .map_err(|_| BrowserProfileError::OutcomeUnknown)
    }
    pub(super) fn unlock(&mut self) -> Result<(), BrowserProfileError> {
        rustix::fs::flock(self.file.file(), rustix::fs::FlockOperation::Unlock)
            .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
        self.unlocked = true;
        Ok(())
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        // Release fork-inherited duplicates of this exact open-file description too.
        let _ = rustix::fs::flock(self.file.file(), rustix::fs::FlockOperation::Unlock);
    }
}
pub(super) fn management(root: &ConfinedRoot) -> Result<Locked, BrowserProfileError> {
    let file = root
        .open_file(Path::new("management.lock"))
        .map_err(|_| BrowserProfileError::Denied)?;
    Locked::acquire(file, root)
}
pub(super) fn new_directory(
    root: &ConfinedRoot,
    name: &str,
) -> Result<ConfinedRoot, BrowserProfileError> {
    root.revalidate().map_err(|_| BrowserProfileError::Denied)?;
    let path = root.path().join(name);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|_| BrowserProfileError::Denied)?;
    root.revalidate_directory(&path)
        .map_err(|_| BrowserProfileError::Denied)?;
    ConfinedRoot::bind(path).map_err(|_| BrowserProfileError::Denied)
}
pub(super) fn existing_directory(
    root: &ConfinedRoot,
    name: &str,
) -> Result<ConfinedRoot, BrowserProfileError> {
    let path = root.path().join(name);
    root.revalidate_directory(&path)
        .map_err(|_| BrowserProfileError::Denied)?;
    ConfinedRoot::bind(path).map_err(|_| BrowserProfileError::Denied)
}
pub(super) fn same_directory(file: &File, path: &Path) -> Result<(), BrowserProfileError> {
    let expected = file
        .metadata()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    let actual = path
        .symlink_metadata()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    if !actual.is_dir()
        || actual.file_type().is_symlink()
        || actual.uid() != rustix::process::geteuid().as_raw()
        || actual.mode() & 0o077 != 0
        || actual.dev() != expected.dev()
        || actual.ino() != expected.ino()
    {
        return Err(BrowserProfileError::OutcomeUnknown);
    }
    Ok(())
}
pub(super) fn ids(root: &ConfinedRoot) -> Result<Vec<BrowserProfileId>, BrowserProfileError> {
    root.revalidate().map_err(|_| BrowserProfileError::Denied)?;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root.path()).map_err(|_| BrowserProfileError::Denied)? {
        let entry = entry.map_err(|_| BrowserProfileError::Denied)?;
        if entry.file_name() == "management.lock" {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrowserProfileError::Denied)?;
        let id = BrowserProfileId::parse(name).map_err(|_| BrowserProfileError::Denied)?;
        existing_directory(root, id.as_str())?;
        result.push(id);
        if result.len() > 32 {
            return Err(BrowserProfileError::LimitExceeded);
        }
    }
    root.revalidate().map_err(|_| BrowserProfileError::Denied)?;
    result.sort();
    Ok(result)
}
pub(super) fn clear_cache(
    root: &ConfinedRoot,
    cache: &ConfinedRoot,
    retained: &File,
) -> Result<(), BrowserProfileError> {
    root.revalidate()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    same_directory(retained, cache.path())?;
    let parent = root
        .directory_handle()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    let name = format!("retired-cache-{}", nonce()?);
    rustix::fs::renameat_with(
        &parent,
        "cache",
        &parent,
        &name,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    let moved = root.path().join(&name);
    same_directory(retained, &moved)?;
    // All browser processes have already been positively reaped before reset is allowed.
    // Rust's native recursive deletion does not follow a discovered symbolic link.
    std::fs::remove_dir_all(&moved).map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    root.revalidate()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)?;
    new_directory(root, "cache")?;
    root.sync_directory()
        .map_err(|_| BrowserProfileError::OutcomeUnknown)
}
