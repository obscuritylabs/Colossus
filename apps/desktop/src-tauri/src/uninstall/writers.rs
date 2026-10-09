//! Keep the existing plugin writer locks from inspection through store removal.

use super::CleanupError;
use colossus_windows_native::{BoundPath, create_private_file};
use fs4::fs_std::FileExt as _;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct CleanupWriters(RefCell<BTreeMap<PathBuf, fs::File>>);

impl CleanupWriters {
    pub fn acquire(&self, store: &Path) -> Result<(), CleanupError> {
        let path = store.join("state.redb.writer.lock");
        if let Err(error) = create_private_file(&path, b"")
            && !matches!(&error, colossus_windows_native::WindowsNativeError::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists)
        {
            return Err(CleanupError::from_native(&error));
        }
        let binding = BoundPath::open_file_read_write(&path)
            .map_err(|error| CleanupError::from_native(&error))?;
        binding
            .validate_private_owner_dacl()
            .map_err(|_| CleanupError::UnsafeData)?;
        if binding.link_count().map_err(|_| CleanupError::UnsafeData)? != 1 {
            return Err(CleanupError::UnsafeData);
        }
        let file = binding
            .try_clone_file()
            .map_err(|_| CleanupError::UnsafeData)?;
        if !file
            .try_lock_exclusive()
            .map_err(|error| CleanupError::from_io(&error))?
        {
            return Err(CleanupError::Busy);
        }
        binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
        self.0.borrow_mut().insert(path, file);
        Ok(())
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.0.borrow().contains_key(path)
    }

    pub fn release_removed(&self, path: &Path) {
        // This is the last inspected child removed from this store. Its state
        // and snapshots are already gone. A writer creating new entries now
        // prevents the subsequent atomic empty-directory removal.
        self.0.borrow_mut().remove(path);
    }
}
