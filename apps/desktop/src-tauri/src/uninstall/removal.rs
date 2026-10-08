//! Never recurse into containers that can receive state from a concurrent CLI.

use super::CleanupError;
use colossus_windows_native::{BoundPath, FileIdentity};
use std::{
    cmp::Reverse,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct CleanupRemoval(Vec<Entry>);

struct Entry {
    path: PathBuf,
    identity: FileIdentity,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    EmptyDirectory,
    DesktopTree,
    File,
}

impl CleanupRemoval {
    // Called only for entries already admitted by the ownership inspection.
    pub fn record(
        &mut self,
        home: &Path,
        path: &Path,
        binding: &BoundPath,
        directory: bool,
    ) -> Result<(), CleanupError> {
        let relative = path
            .strip_prefix(home)
            .map_err(|_| CleanupError::UnsafeData)?;
        let parts = relative
            .iter()
            .map(|part| part.to_str())
            .collect::<Option<Vec<_>>>()
            .ok_or(CleanupError::UnsafeData)?;
        let kind = match parts.as_slice() {
            [] | ["workspaces"] | ["workspaces", _] | ["workspaces", _, "cli"] => {
                Kind::EmptyDirectory
            }
            ["desktop" | "plugins"] | ["workspaces", _, "desktop" | "workspace-plugins"] => {
                Kind::DesktopTree
            }
            [_] if !directory => Kind::File,
            _ => return Ok(()), // Removed through its exclusively Desktop-owned tree.
        };
        self.0.push(Entry {
            path: path.to_owned(),
            identity: binding.identity(),
            kind,
        });
        Ok(())
    }

    pub fn remove(&self) -> Result<(), CleanupError> {
        let mut entries = self.0.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| Reverse(entry.path.components().count()));
        for entry in entries {
            let binding = match entry.kind {
                Kind::File => BoundPath::open_file(&entry.path),
                Kind::EmptyDirectory | Kind::DesktopTree => BoundPath::open_directory(&entry.path),
            }
            .map_err(|error| CleanupError::from_native(&error))?;
            if binding.identity() != entry.identity {
                return Err(CleanupError::UnsafeData);
            }
            binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
            let result = match entry.kind {
                // RemoveDirectory is atomic with respect to child creation. A late
                // write either prevents removal or fails because its parent is gone.
                // Parents also use this operation, preserving recreated CLI folders
                // and entirely new workspace partitions after the final idle check.
                Kind::EmptyDirectory => fs::remove_dir(&entry.path),
                Kind::DesktopTree => fs::remove_dir_all(&entry.path),
                Kind::File => {
                    let mut permissions = fs::metadata(&entry.path)
                        .map_err(|error| CleanupError::from_io(&error))?
                        .permissions();
                    if permissions.readonly() {
                        // This module is Windows-only: clear FILE_ATTRIBUTE_READONLY,
                        // without changing the owner-private DACL.
                        #[allow(clippy::permissions_set_readonly_false)]
                        permissions.set_readonly(false);
                        fs::set_permissions(&entry.path, permissions)
                            .map_err(|error| CleanupError::from_io(&error))?;
                    }
                    fs::remove_file(&entry.path)
                }
            };
            result.map_err(|error| {
                if error.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    CleanupError::UnsafeData
                } else {
                    CleanupError::from_io(&error)
                }
            })?;
        }
        Ok(())
    }
}
