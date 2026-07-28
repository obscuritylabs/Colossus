use fs4::fs_std::FileExt as _;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
};

use colossus_ports::StoreError;

const LEASE_DIRECTORY: &str = "colossus-worker-leases";
#[cfg(unix)]
const LEASE_INODE_DOMAIN: &[u8] = b"colossus-workspace-owner-unix-inode-v1\0";
#[cfg(target_os = "macos")]
const MACOS_IDENTITY_DOMAIN: &[u8] = b"colossus-workspace-owner-macos-inode-birthtime-v2\0";
#[cfg(windows)]
const WINDOWS_IDENTITY_DOMAIN: &[u8] = b"colossus-workspace-owner-windows-volume-file-id-v3\0";
#[cfg(not(any(unix, windows)))]
const LEASE_PATH_DOMAIN: &[u8] = b"colossus-workspace-owner-canonical-path-v1\0";
#[cfg(not(any(unix, windows)))]
const MAX_FALLBACK_WORKSPACE_IDENTITY_UNITS: usize = 32_768;

/// Opaque identity of one opened workspace directory.
///
/// Managed hosts can capture platform metadata on their private bootstrap boundary
/// and require runtime lease acquisition to match that exact object. The digest
/// domain remains runtime-owned so callers cannot mistake it for a path hash.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceIdentityTokenKind {
    #[cfg(all(unix, not(target_os = "macos")))]
    UnixV1,
    #[cfg(target_os = "macos")]
    MacosBirthtimeV2,
    #[cfg(windows)]
    WindowsFileIdV3,
}

/// Opaque expected identity supplied by a host that securely opened the workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceIdentityToken {
    kind: WorkspaceIdentityTokenKind,
    digest: [u8; 32],
}

impl WorkspaceIdentityToken {
    /// Construct a non-macOS Unix workspace token from metadata obtained from an
    /// opened, no-follow directory descriptor.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn from_unix_parts(device_id: u64, inode: u64) -> Self {
        let mut digest = Sha256::new();
        digest.update(LEASE_INODE_DOMAIN);
        digest.update(device_id.to_le_bytes());
        digest.update(inode.to_le_bytes());
        Self {
            kind: WorkspaceIdentityTokenKind::UnixV1,
            digest: digest.finalize().into(),
        }
    }

    /// Construct the persisted Managed Desktop token from descriptor-derived macOS
    /// metadata. Invalid or unavailable directory birthtime fails closed.
    #[cfg(target_os = "macos")]
    pub fn from_macos_parts(
        device_id: u64,
        inode: u64,
        birth_seconds: i64,
        birth_nanoseconds: i64,
    ) -> Option<Self> {
        if birth_seconds <= 0 || !(0..1_000_000_000).contains(&birth_nanoseconds) {
            return None;
        }
        let mut digest = Sha256::new();
        digest.update(MACOS_IDENTITY_DOMAIN);
        digest.update(device_id.to_le_bytes());
        digest.update(inode.to_le_bytes());
        digest.update(birth_seconds.to_le_bytes());
        digest.update(birth_nanoseconds.to_le_bytes());
        Some(Self {
            kind: WorkspaceIdentityTokenKind::MacosBirthtimeV2,
            digest: digest.finalize().into(),
        })
    }

    /// Construct the persisted Managed Desktop token from Windows `FileIdInfo`.
    #[cfg(windows)]
    pub fn from_windows_parts(volume_serial_number: u64, file_id: [u8; 16]) -> Option<Self> {
        if volume_serial_number == 0 || file_id == [0; 16] {
            return None;
        }
        let mut digest = Sha256::new();
        digest.update(WINDOWS_IDENTITY_DOMAIN);
        digest.update(volume_serial_number.to_le_bytes());
        digest.update(file_id);
        Some(Self {
            kind: WorkspaceIdentityTokenKind::WindowsFileIdV3,
            digest: digest.finalize().into(),
        })
    }
}

/// Process-held ownership of one canonical workspace across all runtime state backends.
pub(super) struct WorkspaceOwnershipLease {
    file: File,
    // Retaining the identity also retains the opened directory descriptor. This
    // prevents inode reuse and lets every effect revalidate the pathname immediately
    // before it reaches a path-based adapter.
    identity: WorkspaceIdentity,
}

impl WorkspaceOwnershipLease {
    #[cfg(test)]
    pub(super) fn acquire(workspace: &Path) -> Result<Self, StoreError> {
        Self::acquire_expected(workspace, None)
    }

    pub(super) fn acquire_expected(
        workspace: &Path,
        expected: Option<&WorkspaceIdentityToken>,
    ) -> Result<Self, StoreError> {
        let root = worker_coordination_root();
        Self::acquire_at_expected(workspace, &root, expected)
    }

    #[cfg(test)]
    pub(super) fn acquire_at(workspace: &Path, root: &Path) -> Result<Self, StoreError> {
        Self::acquire_at_expected(workspace, root, None)
    }

    fn acquire_at_expected(
        workspace: &Path,
        root: &Path,
        expected: Option<&WorkspaceIdentityToken>,
    ) -> Result<Self, StoreError> {
        let workspace_identity = open_workspace_identity(workspace)?;
        if let Some(expected) = expected
            && !workspace_identity.matches_expected(expected)?
        {
            return Err(identity_changed());
        }
        let root = prepare_private_directory(root)?;

        let mut digest = Sha256::new();
        update_workspace_identity(&mut digest, &workspace_identity)?;
        let lock_name = format!("{}.lock", hex::encode(digest.finalize()));
        let path = root.join(lock_name);
        let file = open_private_lock_file(&path)?;
        if !file.try_lock_exclusive().map_err(|_| lease_unavailable())? {
            return Err(StoreError::WriterLeaseHeld);
        }
        Ok(Self {
            file,
            identity: workspace_identity,
        })
    }

    pub(super) fn identity(&self) -> WorkspaceIdentity {
        self.identity.clone()
    }
}

#[cfg(unix)]
pub(super) fn worker_coordination_root() -> PathBuf {
    // A lease is live process coordination, not durable application state. A common
    // host namespace lets independently configured workers contend even when HOME is
    // absent or their state databases live elsewhere. The UID suffix plus the strict
    // owner/mode/link checks below fail closed against a pre-created shared entry.
    PathBuf::from("/tmp").join(format!(
        "{LEASE_DIRECTORY}-{}",
        rustix::process::geteuid().as_raw()
    ))
}

#[cfg(not(unix))]
pub(super) fn worker_coordination_root() -> PathBuf {
    std::env::temp_dir().join(LEASE_DIRECTORY)
}

impl Drop for WorkspaceOwnershipLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone)]
pub(super) struct WorkspaceIdentity(Arc<WorkspaceIdentityInner>);

#[cfg(unix)]
struct WorkspaceIdentityInner {
    directory: File,
    canonical_path: PathBuf,
    device: u64,
    inode: u64,
    #[cfg(target_os = "macos")]
    birth_seconds: i64,
    #[cfg(target_os = "macos")]
    birth_nanoseconds: i64,
}

#[cfg(windows)]
struct WorkspaceIdentityInner {
    binding: colossus_windows_native::BoundPath,
    canonical_path: PathBuf,
}

#[cfg(not(any(unix, windows)))]
struct WorkspaceIdentityInner {
    canonical_path: PathBuf,
}

impl WorkspaceIdentity {
    /// Fail closed when the selected pathname no longer names the directory opened
    /// at runtime composition. The check opens the current leaf without following a
    /// symlink and compares both path and descriptor metadata.
    pub(super) fn revalidate(&self) -> Result<(), StoreError> {
        let current = open_workspace_identity(self.canonical_path())?;
        if !self.same_object(&current) {
            return Err(identity_changed());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;

            let retained = self
                .0
                .directory
                .metadata()
                .map_err(|_| identity_changed())?;
            if !retained.is_dir()
                || retained.dev() != self.0.device
                || retained.ino() != self.0.inode
            {
                return Err(identity_changed());
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::macos::fs::MetadataExt as _;

                if retained.st_birthtime() != self.0.birth_seconds
                    || retained.st_birthtime_nsec() != self.0.birth_nanoseconds
                {
                    return Err(identity_changed());
                }
            }
        }
        #[cfg(windows)]
        self.0
            .binding
            .revalidate()
            .map_err(|_| identity_changed())?;
        Ok(())
    }

    pub(super) fn canonical_path(&self) -> &Path {
        &self.0.canonical_path
    }

    #[cfg(unix)]
    pub(super) fn directory(&self) -> Result<File, StoreError> {
        self.0.directory.try_clone().map_err(|_| identity_changed())
    }

    fn matches_expected(&self, expected: &WorkspaceIdentityToken) -> Result<bool, StoreError> {
        match expected.kind {
            #[cfg(all(unix, not(target_os = "macos")))]
            WorkspaceIdentityTokenKind::UnixV1 => {
                let mut digest = Sha256::new();
                update_workspace_identity(&mut digest, self)?;
                Ok(expected.digest == <[u8; 32]>::from(digest.finalize()))
            }
            #[cfg(target_os = "macos")]
            WorkspaceIdentityTokenKind::MacosBirthtimeV2 => {
                Ok(WorkspaceIdentityToken::from_macos_parts(
                    self.0.device,
                    self.0.inode,
                    self.0.birth_seconds,
                    self.0.birth_nanoseconds,
                )
                .is_some_and(|actual| actual.digest == expected.digest))
            }
            #[cfg(windows)]
            WorkspaceIdentityTokenKind::WindowsFileIdV3 => {
                let identity = self.0.binding.identity();
                Ok(WorkspaceIdentityToken::from_windows_parts(
                    identity.volume_serial_number,
                    identity.file_id,
                )
                .is_some_and(|actual| actual.digest == expected.digest))
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn same_object(&self, other: &Self) -> bool {
        self.0.device == other.0.device
            && self.0.inode == other.0.inode
            && self.0.birth_seconds == other.0.birth_seconds
            && self.0.birth_nanoseconds == other.0.birth_nanoseconds
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn same_object(&self, other: &Self) -> bool {
        self.0.device == other.0.device && self.0.inode == other.0.inode
    }

    #[cfg(windows)]
    fn same_object(&self, other: &Self) -> bool {
        self.0.binding.identity() == other.0.binding.identity()
    }

    #[cfg(not(any(unix, windows)))]
    fn same_object(&self, other: &Self) -> bool {
        self.0.canonical_path == other.0.canonical_path
    }
}

#[cfg(unix)]
fn open_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, StoreError> {
    use std::os::unix::fs::MetadataExt as _;

    let canonical = fs::canonicalize(workspace).map_err(|_| lease_unavailable())?;
    let before = fs::symlink_metadata(&canonical).map_err(|_| lease_unavailable())?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(lease_unavailable());
    }

    let directory = rustix::fs::open(
        &canonical,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| lease_unavailable())?;
    let opened = directory.metadata().map_err(|_| lease_unavailable())?;
    let after = fs::symlink_metadata(&canonical).map_err(|_| lease_unavailable())?;
    if !opened.is_dir()
        || after.file_type().is_symlink()
        || !after.is_dir()
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || after.dev() != opened.dev()
        || after.ino() != opened.ino()
    {
        return Err(lease_unavailable());
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;

        if opened.st_birthtime() <= 0
            || !(0..1_000_000_000).contains(&opened.st_birthtime_nsec())
            || before.st_birthtime() != opened.st_birthtime()
            || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
            || after.st_birthtime() != opened.st_birthtime()
            || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
        {
            return Err(identity_changed());
        }
    }

    Ok(WorkspaceIdentity(Arc::new(WorkspaceIdentityInner {
        directory,
        canonical_path: canonical,
        device: opened.dev(),
        inode: opened.ino(),
        #[cfg(target_os = "macos")]
        birth_seconds: {
            use std::os::macos::fs::MetadataExt as _;
            opened.st_birthtime()
        },
        #[cfg(target_os = "macos")]
        birth_nanoseconds: {
            use std::os::macos::fs::MetadataExt as _;
            opened.st_birthtime_nsec()
        },
    })))
}

#[cfg(windows)]
fn open_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, StoreError> {
    let binding = colossus_windows_native::BoundPath::open_directory(workspace)
        .map_err(|_| lease_unavailable())?;
    let canonical_path = binding.canonical_path().to_owned();
    Ok(WorkspaceIdentity(Arc::new(WorkspaceIdentityInner {
        binding,
        canonical_path,
    })))
}

#[cfg(not(any(unix, windows)))]
fn open_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, StoreError> {
    // Platforms without the Unix device/inode contract retain the canonical-path
    // fallback. Its encoded input is explicitly bounded below before it reaches the
    // lease digest.
    let canonical_path = fs::canonicalize(workspace).map_err(|_| lease_unavailable())?;
    if !canonical_path.is_dir() {
        return Err(lease_unavailable());
    }
    Ok(WorkspaceIdentity(Arc::new(WorkspaceIdentityInner {
        canonical_path,
    })))
}

#[cfg(unix)]
fn update_workspace_identity(
    digest: &mut Sha256,
    workspace: &WorkspaceIdentity,
) -> Result<(), StoreError> {
    digest.update(LEASE_INODE_DOMAIN);
    digest.update(workspace.0.device.to_le_bytes());
    digest.update(workspace.0.inode.to_le_bytes());
    Ok(())
}

#[cfg(windows)]
fn update_workspace_identity(
    digest: &mut Sha256,
    workspace: &WorkspaceIdentity,
) -> Result<(), StoreError> {
    let identity = workspace.0.binding.identity();
    digest.update(WINDOWS_IDENTITY_DOMAIN);
    digest.update(identity.volume_serial_number.to_le_bytes());
    digest.update(identity.file_id);
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn update_workspace_identity(
    digest: &mut Sha256,
    workspace: &WorkspaceIdentity,
) -> Result<(), StoreError> {
    let encoded = workspace.0.canonical_path.to_string_lossy();
    if encoded.len() > MAX_FALLBACK_WORKSPACE_IDENTITY_UNITS {
        return Err(lease_unavailable());
    }
    digest.update(LEASE_PATH_DOMAIN);
    digest.update(encoded.as_bytes());
    Ok(())
}

fn prepare_private_directory(path: &Path) -> Result<PathBuf, StoreError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        builder.mode(0o700);
    }
    builder.create(path).map_err(|_| lease_unavailable())?;
    let canonical = fs::canonicalize(path).map_err(|_| lease_unavailable())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| lease_unavailable())?;
    if !metadata.file_type().is_dir() {
        return Err(lease_unavailable());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(lease_unavailable());
        }
    }
    Ok(canonical)
}

#[cfg(unix)]
fn open_private_lock_file(path: &Path) -> Result<File, StoreError> {
    use std::os::unix::fs::MetadataExt as _;

    let before = fs::symlink_metadata(path).ok();
    if before.as_ref().is_some_and(|metadata| {
        metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
    }) {
        return Err(lease_unavailable());
    }
    let file = rustix::fs::open(
        path,
        rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .map(File::from)
    .map_err(|_| lease_unavailable())?;
    let metadata = file.metadata().map_err(|_| lease_unavailable())?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || before
            .as_ref()
            .is_some_and(|before| before.dev() != metadata.dev() || before.ino() != metadata.ino())
    {
        return Err(lease_unavailable());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_private_lock_file(path: &Path) -> Result<File, StoreError> {
    use std::fs::OpenOptions;

    if fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(lease_unavailable());
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| lease_unavailable())?;
    if !file.metadata().map_err(|_| lease_unavailable())?.is_file() {
        return Err(lease_unavailable());
    }
    Ok(file)
}

fn lease_unavailable() -> StoreError {
    StoreError::Adapter("workspace ownership lease is unavailable".into())
}

fn identity_changed() -> StoreError {
    StoreError::WorkspaceIdentityChanged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Runtime, RuntimeConfig, RuntimeError, RuntimeOpenOptions};
    use colossus_policy::DenyApproval;
    use std::sync::Arc;

    #[test]
    fn same_workspace_conflicts_independently_of_runtime_state_path() {
        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let workspace = tempfile::tempdir().expect("workspace");
        let first = WorkspaceOwnershipLease::acquire_at(workspace.path(), &lease_root)
            .expect("first owner");

        assert!(matches!(
            WorkspaceOwnershipLease::acquire_at(workspace.path(), &lease_root),
            Err(StoreError::WriterLeaseHeld)
        ));

        drop(first);
        WorkspaceOwnershipLease::acquire_at(workspace.path(), &lease_root).expect("released owner");
    }

    #[test]
    fn distinct_workspaces_have_independent_ownership() {
        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let first_workspace = tempfile::tempdir().expect("first workspace");
        let second_workspace = tempfile::tempdir().expect("second workspace");
        let _first = WorkspaceOwnershipLease::acquire_at(first_workspace.path(), &lease_root)
            .expect("first owner");
        let _second = WorkspaceOwnershipLease::acquire_at(second_workspace.path(), &lease_root)
            .expect("second owner");
    }

    #[cfg(unix)]
    #[test]
    fn renamed_workspace_inode_remains_owned() {
        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let parent = tempfile::tempdir().expect("workspace parent");
        let original = parent.path().join("original");
        let renamed = parent.path().join("renamed");
        fs::create_dir(&original).expect("workspace");
        let first =
            WorkspaceOwnershipLease::acquire_at(&original, &lease_root).expect("first owner");

        fs::rename(&original, &renamed).expect("rename workspace");
        assert!(matches!(
            WorkspaceOwnershipLease::acquire_at(&renamed, &lease_root),
            Err(StoreError::WriterLeaseHeld)
        ));

        drop(first);
        WorkspaceOwnershipLease::acquire_at(&renamed, &lease_root).expect("released owner");
    }

    #[cfg(unix)]
    #[test]
    fn renamed_workspace_with_replacement_fails_identity_revalidation() {
        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let parent = tempfile::tempdir().expect("workspace parent");
        let original = parent.path().join("workspace");
        let renamed = parent.path().join("workspace-old");
        fs::create_dir(&original).expect("workspace");
        let owner =
            WorkspaceOwnershipLease::acquire_at(&original, &lease_root).expect("first owner");
        let identity = owner.identity();

        fs::rename(&original, &renamed).expect("rename workspace");
        fs::create_dir(&original).expect("replacement workspace");

        assert!(matches!(
            identity.revalidate(),
            Err(StoreError::WorkspaceIdentityChanged)
        ));
        // The replacement is a distinct workspace and therefore has a distinct lease;
        // the old runtime still cannot use it because its retained identity fails.
        WorkspaceOwnershipLease::acquire_at(&original, &lease_root)
            .expect("replacement has an independent owner");
    }

    #[test]
    fn runtime_composition_rejects_an_existing_workspace_owner_before_state_open() {
        let workspace = tempfile::tempdir().expect("workspace");
        let _owner = WorkspaceOwnershipLease::acquire(workspace.path()).expect("workspace owner");
        let config = RuntimeConfig::offline_template(workspace.path().join("independent.redb"));
        let result = Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path()).expect("workspace options"),
        );

        assert!(matches!(
            result,
            Err(RuntimeError::Store(StoreError::WriterLeaseHeld))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn expected_identity_is_compared_to_the_retained_open_directory() {
        use std::os::unix::fs::MetadataExt as _;

        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let parent = tempfile::tempdir().expect("workspace parent");
        let workspace = parent.path().join("workspace");
        let moved = parent.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let metadata = fs::metadata(&workspace).expect("workspace metadata");
        #[cfg(target_os = "macos")]
        let expected = {
            use std::os::macos::fs::MetadataExt as _;

            WorkspaceIdentityToken::from_macos_parts(
                metadata.dev(),
                metadata.ino(),
                metadata.st_birthtime(),
                metadata.st_birthtime_nsec(),
            )
            .expect("current workspace identity")
        };
        #[cfg(not(target_os = "macos"))]
        let expected = WorkspaceIdentityToken::from_unix_parts(metadata.dev(), metadata.ino());
        fs::rename(&workspace, &moved).expect("rename original");
        fs::create_dir(&workspace).expect("replacement");

        assert!(matches!(
            WorkspaceOwnershipLease::acquire_at_expected(&workspace, &lease_root, Some(&expected),),
            Err(StoreError::WorkspaceIdentityChanged)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_identity_distinguishes_birthtime_for_reused_inode_fields() {
        let first = WorkspaceIdentityToken::from_macos_parts(42, 84, 1_700_000_000, 1)
            .expect("first identity");
        let replacement = WorkspaceIdentityToken::from_macos_parts(42, 84, 1_700_000_000, 2)
            .expect("replacement identity");

        assert_ne!(first, replacement);
        assert!(WorkspaceIdentityToken::from_macos_parts(42, 84, 0, 0).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_shared_lease_directory_fails_closed() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().expect("lease root");
        let lease_root = root.path().join("leases");
        let workspace = tempfile::tempdir().expect("workspace");
        fs::create_dir(&lease_root).expect("lease directory");
        fs::set_permissions(&lease_root, fs::Permissions::from_mode(0o770))
            .expect("shared permissions");

        assert!(matches!(
            WorkspaceOwnershipLease::acquire_at(workspace.path(), &lease_root),
            Err(StoreError::Adapter(_))
        ));
    }
}
