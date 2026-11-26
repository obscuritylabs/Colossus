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
    verify_plugin_files(
        root,
        &manifest.files,
        "cached local plugin content is corrupt; restore the exact snapshot before use",
    )
    .map(|_| ())
}
