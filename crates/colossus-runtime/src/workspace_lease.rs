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
#[cfg(target_os = "linux")]
const HOME_LINUX_IDENTITY_DOMAIN: &[u8] =
    b"colossus-home-workspace-linux-device-inode-birthtime-v4\0";
#[cfg(target_os = "macos")]
const MACOS_IDENTITY_DOMAIN: &[u8] = b"colossus-workspace-owner-macos-inode-birthtime-v2\0";
#[cfg(target_os = "macos")]
const HOME_MACOS_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-macos-device-inode-birthtime-v2\0";
#[cfg(windows)]
const WINDOWS_IDENTITY_DOMAIN: &[u8] = b"colossus-workspace-owner-windows-volume-file-id-v3\0";
#[cfg(windows)]
const HOME_WINDOWS_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-windows-volume-file-id-v3\0";
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
    #[cfg(target_os = "linux")]
    HomeLinuxBirthtimeV4,
    #[cfg(target_os = "macos")]
    HomeMacosBirthtimeV2,
    #[cfg(windows)]
    HomeWindowsFileIdV3,
}

/// Opaque expected identity supplied by a host that securely opened the workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceIdentityToken {
    kind: WorkspaceIdentityTokenKind,
    digest: [u8; 32],
}

impl WorkspaceIdentityToken {
    /// Bind runtime acquisition to the opaque identity used for a home partition.
    pub fn from_home_workspace_identity(
        identity: colossus_home::WorkspaceIdentityRef<'_>,
    ) -> Option<Self> {
        if identity.sha256.len() != 64
            || !identity
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        let digest: [u8; 32] = hex::decode(identity.sha256).ok()?.try_into().ok()?;
        let kind = match identity.version {
            #[cfg(target_os = "linux")]
            4 => WorkspaceIdentityTokenKind::HomeLinuxBirthtimeV4,
            #[cfg(target_os = "macos")]
            2 => WorkspaceIdentityTokenKind::HomeMacosBirthtimeV2,
            #[cfg(windows)]
            3 => WorkspaceIdentityTokenKind::HomeWindowsFileIdV3,
            _ => return None,
        };
        Some(Self { kind, digest })
    }

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
    #[cfg(target_os = "linux")]
    birth_seconds: i64,
    #[cfg(target_os = "linux")]
    birth_nanoseconds: u32,
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
            #[cfg(target_os = "linux")]
            {
                let (birth_seconds, birth_nanoseconds) =
                    linux_directory_birthtime(&self.0.directory, &retained)?;
                if birth_seconds != self.0.birth_seconds
                    || birth_nanoseconds != self.0.birth_nanoseconds
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
            #[cfg(target_os = "linux")]
            WorkspaceIdentityTokenKind::HomeLinuxBirthtimeV4 => {
                let mut digest = Sha256::new();
                digest.update(HOME_LINUX_IDENTITY_DOMAIN);
                digest.update(self.0.device.to_le_bytes());
                digest.update(self.0.inode.to_le_bytes());
                digest.update(self.0.birth_seconds.to_le_bytes());
                digest.update(self.0.birth_nanoseconds.to_le_bytes());
                Ok(expected.digest == <[u8; 32]>::from(digest.finalize()))
            }
            #[cfg(target_os = "macos")]
            WorkspaceIdentityTokenKind::HomeMacosBirthtimeV2 => {
                let mut digest = Sha256::new();
                digest.update(HOME_MACOS_IDENTITY_DOMAIN);
                digest.update(self.0.device.to_le_bytes());
                digest.update(self.0.inode.to_le_bytes());
                digest.update(self.0.birth_seconds.to_le_bytes());
                digest.update(self.0.birth_nanoseconds.to_le_bytes());
                Ok(expected.digest == <[u8; 32]>::from(digest.finalize()))
            }
            #[cfg(windows)]
            WorkspaceIdentityTokenKind::HomeWindowsFileIdV3 => {
                let identity = self.0.binding.identity();
                let mut digest = Sha256::new();
                digest.update(HOME_WINDOWS_IDENTITY_DOMAIN);
                digest.update(identity.volume_serial_number.to_le_bytes());
                digest.update(identity.file_id);
                Ok(expected.digest == <[u8; 32]>::from(digest.finalize()))
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

    #[cfg(target_os = "linux")]
    fn same_object(&self, other: &Self) -> bool {
        self.0.device == other.0.device
            && self.0.inode == other.0.inode
            && self.0.birth_seconds == other.0.birth_seconds
            && self.0.birth_nanoseconds == other.0.birth_nanoseconds
    }

    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
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
    #[cfg(target_os = "linux")]
    let (birth_seconds, birth_nanoseconds) = linux_directory_birthtime(&directory, &opened)?;

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
        #[cfg(target_os = "linux")]
        birth_seconds,
        #[cfg(target_os = "linux")]
        birth_nanoseconds,
    })))
}

#[cfg(target_os = "linux")]
fn linux_directory_birthtime(
    directory: &File,
    metadata: &fs::Metadata,
) -> Result<(i64, u32), StoreError> {
    use std::os::unix::fs::MetadataExt as _;

    let statx = rustix::fs::statx(
        directory,
        "",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::BASIC_STATS | rustix::fs::StatxFlags::BTIME,
    )
    .map_err(|_| identity_changed())?;
    if statx.stx_mask & rustix::fs::StatxFlags::BTIME.bits() == 0
        || statx.stx_ino != metadata.ino()
        || statx.stx_dev_major != rustix::fs::major(metadata.dev())
        || statx.stx_dev_minor != rustix::fs::minor(metadata.dev())
        || statx.stx_btime.tv_sec <= 0
        || statx.stx_btime.tv_nsec >= 1_000_000_000
    {
        return Err(identity_changed());
    }
    Ok((statx.stx_btime.tv_sec, statx.stx_btime.tv_nsec))
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
    use std::{process::Command, sync::Arc};

    fn private_tempdir() -> tempfile::TempDir {
        #[cfg(windows)]
        {
            let directory = tempfile::Builder::new()
                .prefix("colossus-runtime-lease-")
                .tempdir_in(current_user_profile())
                .expect("private temporary root");
            make_windows_private_directory(directory.path());
            directory
        }

        #[cfg(not(windows))]
        {
            let directory = tempfile::tempdir().expect("private temporary root");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;

                fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
                    .expect("private temporary root permissions");
            }
            directory
        }
    }

    #[cfg(windows)]
    fn current_user_profile() -> PathBuf {
        let (account, _) = current_windows_user();
        let user_name = account
            .rsplit('\\')
            .next()
            .filter(|value| !value.is_empty())
            .expect("current Windows account name");
        let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
        let users_root = PathBuf::from(format!("{system_drive}\\Users"));
        for entry in fs::read_dir(&users_root).expect("Windows users directory") {
            let entry = entry.expect("Windows user profile entry");
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(user_name)
            {
                return entry.path();
            }
        }
        panic!("Windows user profile not found for {account}");
    }

    #[cfg(windows)]
    fn make_windows_private_directory(path: &Path) {
        let (_, current_user_sid) = current_windows_user();
        let grant_current_user = format!("*{current_user_sid}:(OI)(CI)F");
        let output = Command::new("icacls.exe")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(grant_current_user)
            .arg("*S-1-5-18:(OI)(CI)F")
            .arg("*S-1-5-32-544:(OI)(CI)F")
            .output()
            .expect("make isolated Windows runtime lease directory private");
        assert!(
            output.status.success(),
            "failed to make isolated Windows runtime lease directory private\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(windows)]
    fn current_windows_user() -> (String, String) {
        let output = Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .expect("query current Windows user SID");
        assert!(
            output.status.success(),
            "failed to query current Windows user SID\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("Windows user SID output is UTF-8");
        let mut columns = stdout.split(',');
        let account = columns
            .next()
            .map(|value| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
            .expect("Windows account in whoami output");
        let sid_tail = columns
            .next()
            .and_then(|value| value.trim().trim_matches('"').strip_prefix("S-"))
            .expect("Windows user SID in whoami output");
        assert!(
            sid_tail
                .chars()
                .all(|character| character.is_ascii_digit() || character == '-'),
            "unexpected Windows user SID: S-{sid_tail}"
        );
        (account, format!("S-{sid_tail}"))
    }

    #[test]
    fn same_workspace_conflicts_independently_of_runtime_state_path() {
        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let workspace = private_tempdir();
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
        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let first_workspace = private_tempdir();
        let second_workspace = private_tempdir();
        let _first = WorkspaceOwnershipLease::acquire_at(first_workspace.path(), &lease_root)
            .expect("first owner");
        let _second = WorkspaceOwnershipLease::acquire_at(second_workspace.path(), &lease_root)
            .expect("second owner");
    }

    #[cfg(unix)]
    #[test]
    fn renamed_workspace_inode_remains_owned() {
        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let parent = private_tempdir();
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
        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let parent = private_tempdir();
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
        let workspace = private_tempdir();
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

        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let parent = private_tempdir();
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

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn home_partition_identity_binds_runtime_acquisition_to_the_same_object() {
        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let parent = private_tempdir();
        let workspace = parent.path().join("workspace");
        let moved = parent.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let identity =
            colossus_home::detect_workspace_identity(&workspace).expect("home workspace identity");
        let expected = WorkspaceIdentityToken::from_home_workspace_identity(identity.as_ref())
            .expect("runtime identity token");
        let owner =
            WorkspaceOwnershipLease::acquire_at_expected(&workspace, &lease_root, Some(&expected))
                .expect("matching workspace");
        drop(owner);

        fs::rename(&workspace, &moved).expect("move workspace");
        fs::create_dir(&workspace).expect("replacement workspace");
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

        let root = private_tempdir();
        let lease_root = root.path().join("leases");
        let workspace = private_tempdir();
        fs::create_dir(&lease_root).expect("lease directory");
        fs::set_permissions(&lease_root, fs::Permissions::from_mode(0o770))
            .expect("shared permissions");

        assert!(matches!(
            WorkspaceOwnershipLease::acquire_at(workspace.path(), &lease_root),
            Err(StoreError::Adapter(_))
        ));
    }
}
