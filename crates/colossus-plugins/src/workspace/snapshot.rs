//! Local snapshot identity is a file digest manifest, independent of OCI distribution.

use super::*;

pub(crate) const SNAPSHOT_MANIFEST: &str = "workspace-snapshot.json";
pub(crate) const MAX_SNAPSHOT_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const SNAPSHOT_FORMAT: &str = "colossus.workspace-plugin.snapshot.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotFile {
    pub path: String,
    pub size: u64,
    pub executable: bool,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotManifest {
    format: String,
    pub files: Vec<SnapshotFile>,
}

impl SnapshotManifest {
    pub(crate) fn parse(bytes: &[u8], digest: &str) -> Result<Self, StoreError> {
        if bytes.len() as u64 > MAX_SNAPSHOT_MANIFEST_BYTES || sha256_digest(bytes) != digest {
            return Err(StoreError::Verification(
                "local snapshot manifest digest or size mismatch".into(),
            ));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(adapter)?;
        if manifest.format != SNAPSHOT_FORMAT || manifest.files.len() > MAX_FILES {
            return Err(adapter("invalid local snapshot manifest"));
        }
        let mut previous = None;
        let mut total = 0_u64;
        for file in &manifest.files {
            if file.path.is_empty()
                || file.path.contains(['\\', ':'])
                || posix_path(Path::new(&file.path))? != file.path
                || previous.is_some_and(|path| path >= file.path.as_str())
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                || file.size > MAX_FILE_BYTES
            {
                return Err(adapter("invalid local snapshot file"));
            }
            previous = Some(file.path.as_str());
            total = total
                .checked_add(file.size)
                .filter(|size| *size <= MAX_WORKSPACE_PLUGIN_BYTES)
                .ok_or_else(|| adapter("local snapshot exceeds 256 MiB"))?;
        }
        if !manifest.files.iter().any(|file| file.path == "plugin.json") {
            return Err(adapter("local snapshot requires plugin.json"));
        }
        Ok(manifest)
    }
}

/// Frozen local files and their exact identity; no archive or compression is involved.
pub struct WorkspacePluginSnapshot {
    captured: tempfile::TempDir,
    pub(crate) manifest: Vec<u8>,
    pub(crate) files: Vec<SnapshotFile>,
    digest: String,
}

impl WorkspacePluginSnapshot {
    pub(super) fn new(
        captured: tempfile::TempDir,
        mut files: Vec<SnapshotFile>,
    ) -> Result<Self, StoreError> {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = serde_json::to_vec(&SnapshotManifest {
            format: SNAPSHOT_FORMAT.into(),
            files: files.clone(),
        })
        .map_err(adapter)?;
        let digest = sha256_digest(&manifest);
        SnapshotManifest::parse(&manifest, &digest)?;
        Ok(Self {
            captured,
            manifest,
            files,
            digest,
        })
    }

    /// SHA-256 of the versioned, sorted file digest manifest, including executable modes.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub(crate) fn root(&self) -> &Path {
        self.captured.path()
    }

    pub(crate) fn copy_to(&self, destination: &Path) -> Result<(), StoreError> {
        // The private capture is independent of the mutable source. Never hard-link it.
        fs::create_dir(destination).map_err(adapter)?;
        let reader = ReadRoot::bind(self.root())?;
        for entry in &self.files {
            let relative = Path::new(&entry.path);
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(adapter)?;
            }
            let source = reader.open_file(relative, entry.size)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(adapter)?;
            let mut source = source.take(entry.size.saturating_add(1));
            let mut size = 0_u64;
            let mut hash = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = source.read(&mut buffer).map_err(adapter)?;
                if read == 0 {
                    break;
                }
                size = size
                    .checked_add(read as u64)
                    .filter(|size| *size <= entry.size)
                    .ok_or_else(|| adapter("local snapshot changed during publication"))?;
                output.write_all(&buffer[..read]).map_err(adapter)?;
                hash.update(&buffer[..read]);
            }
            if size != entry.size || hex::encode(hash.finalize()) != entry.sha256 {
                return Err(adapter("local snapshot changed during publication"));
            }
            set_snapshot_mode(&output, entry.executable)?;
            output.sync_all().map_err(adapter)?;
        }
        reader.revalidate()
    }
}

pub(crate) fn set_snapshot_mode(file: &File, executable: bool) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(if executable {
            0o755
        } else {
            0o644
        }))
        .map_err(adapter)?;
    }
    #[cfg(not(unix))]
    let _ = (file, executable);
    Ok(())
}
