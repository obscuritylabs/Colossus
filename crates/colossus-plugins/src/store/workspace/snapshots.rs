//! Publish and verify local content directly. OCI layouts remain a legacy read path.

use super::*;
use crate::workspace::{MAX_SNAPSHOT_MANIFEST_BYTES, SNAPSHOT_MANIFEST, SnapshotManifest};

impl PluginStore {
    pub(in crate::store) fn publish_workspace_snapshot(
        &self,
        candidate: &WorkspacePluginCandidate,
    ) -> Result<PathBuf, StoreError> {
        let snapshot = &candidate.snapshot;
        let digest = snapshot.digest();
        validate_lease_digest(digest)?;
        let manifest = SnapshotManifest::parse(&snapshot.manifest, digest)?;
        let hex = &digest["sha256:".len()..];
        let destination = self.root.join("content/sha256").join(hex);
        let layout = self.root.join("layouts/sha256").join(hex);
        let exists = match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
            Ok(_) => {
                return Err(StoreError::Verification(
                    "cached local snapshot root is corrupt".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(adapter(error)),
        };
        if exists {
            let bytes = read_contained(
                &layout,
                Path::new(SNAPSHOT_MANIFEST),
                MAX_SNAPSHOT_MANIFEST_BYTES,
            )?;
            if bytes != snapshot.manifest {
                return Err(StoreError::Verification(
                    "local snapshot identity differs from retained content".into(),
                ));
            }
            verify_snapshot_content(&destination, &manifest)?;
            return Ok(destination);
        }
        let staging = self
            .root
            .join("staging")
            .join(format!("local-{}", uuid::Uuid::now_v7()));
        let staged_layout = self
            .root
            .join("staging")
            .join(format!("local-layout-{}", uuid::Uuid::now_v7()));
        let result = (|| {
            snapshot.copy_to(&staging)?;
            fs::create_dir_all(staged_layout.join("blobs/sha256")).map_err(adapter)?;
            write_new(&staged_layout.join(SNAPSHOT_MANIFEST), &snapshot.manifest)?;
            if layout.exists() {
                // An interrupted publication may have retained its manifest first.
                let bytes = read_contained(
                    &layout,
                    Path::new(SNAPSHOT_MANIFEST),
                    MAX_SNAPSHOT_MANIFEST_BYTES,
                )?;
                if bytes != snapshot.manifest {
                    return Err(StoreError::Verification(
                        "local snapshot manifest is corrupt".into(),
                    ));
                }
                remove_tree_if_present(&staged_layout)?;
            } else {
                fs::rename(&staged_layout, &layout).map_err(adapter)?;
                make_tree_read_only(&layout)?;
            }
            fs::rename(&staging, &destination).map_err(adapter)?;
            make_tree_read_only(&destination)?;
            Ok(destination)
        })();
        let cleanup = remove_tree_if_present(&staging).and(remove_tree_if_present(&staged_layout));
        result.and_then(|destination| cleanup.map(|()| destination))
    }

    pub(in crate::store) fn verify_workspace_snapshot(
        &self,
        root: &Path,
        layout: &Path,
        digest: &str,
    ) -> Result<bool, StoreError> {
        match fs::symlink_metadata(layout.join(SNAPSHOT_MANIFEST)) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(adapter(error)),
        }
        let bytes = read_contained(
            layout,
            Path::new(SNAPSHOT_MANIFEST),
            MAX_SNAPSHOT_MANIFEST_BYTES,
        )?;
        let manifest = SnapshotManifest::parse(&bytes, digest)?;
        verify_snapshot_content(root, &manifest)?;
        Ok(true)
    }
}

fn verify_snapshot_content(root: &Path, manifest: &SnapshotManifest) -> Result<(), StoreError> {
    let mismatch = || {
        StoreError::Verification(
            "cached local plugin content is corrupt; restore the exact snapshot before use".into(),
        )
    };
    let reader = ReadRoot::bind(root)?;
    let mut paths = Vec::new();
    collect_regular_files(root, root, 0, &mut paths)?;
    let mut paths = paths
        .into_iter()
        .map(|path| posix_path(&path).map(|key| (key, path)))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    if paths.len() != manifest.files.len() {
        return Err(mismatch());
    }
    let mut buffer = [0_u8; 64 * 1024];
    for ((key, path), expected) in paths.iter().zip(&manifest.files) {
        if *key != expected.path {
            return Err(mismatch());
        }
        let mut file = reader.open_file(path, expected.size)?;
        let metadata = file.metadata().map_err(adapter)?;
        if metadata.len() != expected.size {
            return Err(mismatch());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = if expected.executable { 0o500 } else { 0o400 };
            if metadata.permissions().mode() & 0o7777 != mode {
                return Err(mismatch());
            }
        }
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(adapter)?;
            if read == 0 {
                break;
            }
            size = size
                .checked_add(read as u64)
                .filter(|size| *size <= expected.size)
                .ok_or_else(mismatch)?;
            hash.update(&buffer[..read]);
        }
        if size != expected.size || hex::encode(hash.finalize()) != expected.sha256 {
            return Err(mismatch());
        }
    }
    reader.revalidate()
}
