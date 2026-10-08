//! Account for every name of a plugin cache blob before allowing its removal.

use super::CleanupError;
use colossus_windows_native::{BoundPath, FileIdentity};
use std::{
    collections::BTreeMap,
    fs,
    os::windows::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct CleanupFiles(Vec<CleanupFile>);

struct CleanupFile {
    path: PathBuf,
    identity: FileIdentity,
    links: u64,
}

impl CleanupFiles {
    pub fn push(
        &mut self,
        path: &Path,
        relative: &Path,
        binding: &BoundPath,
    ) -> Result<(), CleanupError> {
        let links = binding
            .link_count()
            .map_err(|error| CleanupError::from_native(&error))?;
        if links == 0 || (links > 1 && !super::ownership::plugin_blob(relative)) {
            return Err(CleanupError::UnsafeData);
        }
        self.0.push(CleanupFile {
            path: path.to_owned(),
            identity: binding.identity(),
            links,
        });
        Ok(())
    }

    pub fn validate_links(&self) -> Result<(), CleanupError> {
        let mut names = BTreeMap::new();
        for file in &self.0 {
            let identity = (file.identity.volume_serial_number, file.identity.file_id);
            let (observed, expected) = names.entry(identity).or_insert((0_u64, file.links));
            if *expected != file.links {
                return Err(CleanupError::UnsafeData);
            }
            *observed += 1;
        }
        // Plugin installation intentionally links cache blobs into retained OCI
        // layouts. Permit them only when every link is in the inspected owned
        // cache. A link outside it must preserve both the file and its attributes.
        if names
            .values()
            .any(|(observed, expected)| observed != expected)
        {
            return Err(CleanupError::UnsafeData);
        }
        Ok(())
    }

    pub fn check_idle(&self, writers: &super::writers::CleanupWriters) -> Result<(), CleanupError> {
        for file in &self.0 {
            let binding = BoundPath::open_file(&file.path)
                .map_err(|error| CleanupError::from_native(&error))?;
            if binding.identity() != file.identity
                || binding.link_count().map_err(|_| CleanupError::UnsafeData)? != file.links
            {
                return Err(CleanupError::UnsafeData);
            }
            drop(binding);
            if writers.contains(&file.path) {
                continue; // The cleanup plan itself retains this writer handle.
            }
            // Do not mistake our own inspection handle for an active runtime.
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&file.path)
                .map_err(|error| CleanupError::from_io(&error))?;
        }
        Ok(())
    }
}
