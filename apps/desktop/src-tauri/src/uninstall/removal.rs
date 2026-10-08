//! Never recurse into containers that can receive state from a concurrent CLI.

use super::CleanupError;
use colossus_windows_native::{BoundPath, FileIdentity};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct CleanupRemoval(Vec<Entry>);

struct Entry {
    path: PathBuf,
    identity: FileIdentity,
    kind: Kind,
    links: Option<u64>,
}

#[derive(Clone, Copy)]
enum Kind {
    EmptyDirectory,
    InspectedDirectory,
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
            ["plugins", ..] | ["workspaces", _, "workspace-plugins", ..] => {
                if directory {
                    Kind::InspectedDirectory
                } else {
                    Kind::File
                }
            }
            [] | ["workspaces"] | ["workspaces", _] | ["workspaces", _, "cli"] => {
                Kind::EmptyDirectory
            }
            ["desktop"] | ["workspaces", _, "desktop"] => Kind::DesktopTree,
            [_] if !directory => Kind::File,
            _ => return Ok(()), // Removed through its exclusively Desktop-owned tree.
        };
        self.0.push(Entry {
            path: path.to_owned(),
            identity: binding.identity(),
            kind,
            links: if directory {
                None
            } else {
                Some(binding.link_count().map_err(|_| CleanupError::UnsafeData)?)
            },
        });
        Ok(())
    }

    pub fn remove(&self) -> Result<(), CleanupError> {
        self.check_plugin_directories()?;
        let mut entries = self.0.iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| Reverse(entry.path.components().count()));
        let mut links = BTreeMap::new();
        for entry in &entries {
            if let Some(count) = entry.links {
                links.insert(file_key(entry.identity), count);
            }
        }
        for entry in entries {
            let binding = match entry.kind {
                Kind::File => BoundPath::open_file(&entry.path),
                Kind::EmptyDirectory | Kind::InspectedDirectory | Kind::DesktopTree => {
                    BoundPath::open_directory(&entry.path)
                }
            }
            .map_err(|error| CleanupError::from_native(&error))?;
            if binding.identity() != entry.identity {
                return Err(CleanupError::UnsafeData);
            }
            binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
            if let Some(expected) = links.get(&file_key(entry.identity)) {
                if binding.link_count().map_err(|_| CleanupError::UnsafeData)? != *expected {
                    return Err(CleanupError::UnsafeData);
                }
            }
            if matches!(entry.kind, Kind::File | Kind::InspectedDirectory) {
                let mut permissions = fs::metadata(&entry.path)
                    .map_err(|error| CleanupError::from_io(&error))?
                    .permissions();
                if permissions.readonly() {
                    // Windows-only: clear FILE_ATTRIBUTE_READONLY on the inspected
                    // entry, without changing its owner-private DACL.
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    fs::set_permissions(&entry.path, permissions)
                        .map_err(|error| CleanupError::from_io(&error))?;
                }
            }
            let result = match entry.kind {
                // RemoveDirectory is atomic with respect to child creation. A late
                // write either prevents removal or fails because its parent is gone.
                // Parents also use this operation, preserving recreated CLI folders
                // and entirely new workspace partitions after the final idle check.
                Kind::EmptyDirectory | Kind::InspectedDirectory => fs::remove_dir(&entry.path),
                Kind::DesktopTree => fs::remove_dir_all(&entry.path),
                Kind::File => fs::remove_file(&entry.path),
            };
            result.map_err(|error| {
                if error.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    CleanupError::UnsafeData
                } else {
                    CleanupError::from_io(&error)
                }
            })?;
            if let Some(count) = links.get_mut(&file_key(entry.identity)) {
                *count -= 1;
            }
        }
        Ok(())
    }

    fn check_plugin_directories(&self) -> Result<(), CleanupError> {
        let mut children = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
        for entry in &self.0 {
            if let Some(parent) = entry.path.parent() {
                children
                    .entry(parent.to_owned())
                    .or_default()
                    .insert(entry.path.clone());
            }
        }
        let empty = BTreeSet::new();
        for directory in self
            .0
            .iter()
            .filter(|entry| matches!(entry.kind, Kind::InspectedDirectory))
        {
            let expected = children.get(&directory.path).unwrap_or(&empty);
            let binding = BoundPath::open_directory(&directory.path)
                .map_err(|error| CleanupError::from_native(&error))?;
            if binding.identity() != directory.identity {
                return Err(CleanupError::UnsafeData);
            }
            let mut observed = BTreeSet::new();
            for (index, child) in fs::read_dir(&directory.path)
                .map_err(|error| CleanupError::from_io(&error))?
                .enumerate()
            {
                if index >= expected.len() {
                    return Err(CleanupError::UnsafeData);
                }
                observed.insert(child.map_err(|error| CleanupError::from_io(&error))?.path());
            }
            if &observed != expected {
                return Err(CleanupError::UnsafeData);
            }
            binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
        }
        Ok(())
    }
}

fn file_key(identity: FileIdentity) -> (u64, [u8; 16]) {
    (identity.volume_serial_number, identity.file_id)
}
