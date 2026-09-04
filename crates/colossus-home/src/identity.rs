use crate::HomeError;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use sha2::{Digest as _, Sha256};
#[cfg(unix)]
use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(any(target_os = "linux", test))]
use std::io;
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", test))]
const LINUX_IDENTITY_DOMAIN: &[u8] = b"colossus-home-workspace-linux-device-inode-birthtime-v4\0";
#[cfg(any(target_os = "linux", test))]
const LINUX_NFS_IDENTITY_DOMAIN: &[u8] =
    b"colossus-home-workspace-linux-nfs-server-file-handle-v5\0";
#[cfg(target_os = "macos")]
const MACOS_IDENTITY_DOMAIN: &[u8] =
    b"colossus-sidecar-workspace-macos-device-inode-birthtime-v2\0";
#[cfg(windows)]
const WINDOWS_IDENTITY_DOMAIN: &[u8] = b"colossus-sidecar-workspace-windows-volume-file-id-v3\0";

/// Opaque, versioned identity of one canonical workspace directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceIdentity {
    canonical_path: PathBuf,
    version: u16,
    sha256: String,
}

/// Opaque identity captured from one already-open Linux workspace directory.
///
/// This type contains only a version and digest. Raw kernel file-handle material
/// is discarded immediately after the digest is derived.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxWorkspaceIdentity {
    version: u16,
    digest: [u8; 32],
}

#[cfg(any(target_os = "linux", test))]
impl LinuxWorkspaceIdentity {
    /// Exact Linux identity derivation version.
    pub const fn version(self) -> u16 {
        self.version
    }

    /// Domain-separated SHA-256 digest of the object identity.
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

impl WorkspaceIdentity {
    /// Canonical path captured with the object identity.
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    /// Versioned opaque identity fields suitable for partition derivation.
    pub fn as_ref(&self) -> WorkspaceIdentityRef<'_> {
        WorkspaceIdentityRef {
            version: self.version,
            sha256: &self.sha256,
        }
    }

    /// Exact identity derivation version.
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Lowercase SHA-256 identity digest.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Prove that the selected pathname still names the same workspace object.
    pub fn revalidate(&self) -> Result<(), HomeError> {
        let current = detect_workspace_identity(&self.canonical_path)?;
        if current == *self {
            Ok(())
        } else {
            Err(HomeError::InvalidWorkspace(self.canonical_path.clone()))
        }
    }
}

/// Borrowed workspace identity accepted from another trusted native component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceIdentityRef<'a> {
    /// Exact platform identity derivation version.
    pub version: u16,
    /// Lowercase SHA-256 of the domain-separated object identity.
    pub sha256: &'a str,
}

impl WorkspaceIdentityRef<'_> {
    pub(crate) fn validate(self) -> Result<(), HomeError> {
        if matches!(self.version, 2..=5)
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(())
        } else {
            Err(HomeError::InvalidWorkspaceIdentity)
        }
    }
}

/// Capture the canonical path and stable object identity of a workspace directory.
pub fn detect_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, HomeError> {
    detect_platform_workspace_identity(workspace)
}

/// Capture the identity of an already-open Linux workspace directory.
///
/// Linux filesystems that report `statx` birth time retain the v4 identity.
/// When birth time is absent, a v5 identity is accepted only for NFS and is
/// derived from the remote volume identity plus the kernel's opaque file handle.
#[cfg(target_os = "linux")]
pub fn capture_linux_workspace_identity(directory: &File) -> io::Result<LinuxWorkspaceIdentity> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Err(invalid_linux_identity());
    }
    let statx = rustix::fs::statx(
        directory,
        "",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::BASIC_STATS | rustix::fs::StatxFlags::BTIME,
    )
    .map_err(io::Error::from)?;
    select_linux_workspace_identity(
        metadata.dev(),
        metadata.ino(),
        LinuxStatxIdentityEvidence {
            metadata_matches: statx.stx_mask & rustix::fs::StatxFlags::INO.bits() != 0
                && statx.stx_ino == metadata.ino()
                && statx.stx_dev_major == rustix::fs::major(metadata.dev())
                && statx.stx_dev_minor == rustix::fs::minor(metadata.dev()),
            birthtime: (statx.stx_mask & rustix::fs::StatxFlags::BTIME.bits() != 0)
                .then_some((statx.stx_btime.tv_sec, statx.stx_btime.tv_nsec)),
        },
        || {
            let identity = colossus_linux_native::capture_nfs_file_identity(directory)?;
            Ok(linux_nfs_identity_digest(
                identity.nfs_version(),
                identity.server_address(),
                identity.server_port(),
                identity.fsid_major(),
                identity.fsid_minor(),
                identity.handle_type(),
                identity.handle(),
            ))
        },
    )
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy)]
struct LinuxStatxIdentityEvidence {
    metadata_matches: bool,
    birthtime: Option<(i64, u32)>,
}

#[cfg(any(target_os = "linux", test))]
fn select_linux_workspace_identity(
    device: u64,
    inode: u64,
    evidence: LinuxStatxIdentityEvidence,
    nfs_fallback: impl FnOnce() -> io::Result<[u8; 32]>,
) -> io::Result<LinuxWorkspaceIdentity> {
    if !evidence.metadata_matches {
        return Err(invalid_linux_identity());
    }
    if let Some((birth_seconds, birth_nanoseconds)) = evidence.birthtime {
        if birth_seconds <= 0 || birth_nanoseconds >= 1_000_000_000 {
            return Err(invalid_linux_identity());
        }
        return Ok(LinuxWorkspaceIdentity {
            version: 4,
            digest: linux_birthtime_identity_digest(
                device,
                inode,
                birth_seconds,
                birth_nanoseconds,
            ),
        });
    }
    Ok(LinuxWorkspaceIdentity {
        version: 5,
        digest: nfs_fallback()?,
    })
}

#[cfg(any(target_os = "linux", test))]
fn invalid_linux_identity() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Linux workspace identity evidence is invalid",
    )
}

#[cfg(any(target_os = "linux", test))]
fn linux_birthtime_identity_digest(
    device: u64,
    inode: u64,
    birth_seconds: i64,
    birth_nanoseconds: u32,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(LINUX_IDENTITY_DOMAIN);
    digest.update(device.to_le_bytes());
    digest.update(inode.to_le_bytes());
    digest.update(birth_seconds.to_le_bytes());
    digest.update(birth_nanoseconds.to_le_bytes());
    digest.finalize().into()
}

#[cfg(any(target_os = "linux", test))]
#[allow(clippy::too_many_arguments)]
fn linux_nfs_identity_digest(
    nfs_version: u8,
    server_address: &[u8],
    server_port: u16,
    fsid_major: u64,
    fsid_minor: u64,
    handle_type: i32,
    handle: &[u8],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(LINUX_NFS_IDENTITY_DOMAIN);
    digest.update([nfs_version]);
    digest.update((server_address.len() as u32).to_le_bytes());
    digest.update(server_address);
    digest.update(server_port.to_le_bytes());
    digest.update(fsid_major.to_le_bytes());
    digest.update(fsid_minor.to_le_bytes());
    digest.update(handle_type.to_le_bytes());
    digest.update((handle.len() as u32).to_le_bytes());
    digest.update(handle);
    digest.finalize().into()
}

#[cfg(unix)]
fn detect_platform_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, HomeError> {
    use std::{fs::File, os::unix::fs::MetadataExt as _};

    let canonical_path =
        fs::canonicalize(workspace).map_err(|error| HomeError::io(workspace, error))?;
    let before = fs::symlink_metadata(&canonical_path)
        .map_err(|error| HomeError::io(&canonical_path, error))?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(HomeError::InvalidWorkspace(canonical_path));
    }
    let directory = rustix::fs::open(
        &canonical_path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| HomeError::io(&canonical_path, error.into()))?;
    let opened = directory
        .metadata()
        .map_err(|error| HomeError::io(&canonical_path, error))?;
    let after = fs::symlink_metadata(&canonical_path)
        .map_err(|error| HomeError::io(&canonical_path, error))?;
    if !opened.is_dir()
        || after.file_type().is_symlink()
        || !after.is_dir()
        || before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || after.dev() != opened.dev()
        || after.ino() != opened.ino()
    {
        return Err(HomeError::InvalidWorkspace(canonical_path));
    }

    #[cfg(target_os = "macos")]
    let (version, sha256) = {
        use std::os::macos::fs::MetadataExt as _;

        let birth_seconds = opened.st_birthtime();
        let birth_nanoseconds = opened.st_birthtime_nsec();
        if birth_seconds <= 0
            || !(0..1_000_000_000).contains(&birth_nanoseconds)
            || before.st_birthtime() != birth_seconds
            || before.st_birthtime_nsec() != birth_nanoseconds
            || after.st_birthtime() != birth_seconds
            || after.st_birthtime_nsec() != birth_nanoseconds
        {
            return Err(HomeError::InvalidWorkspace(canonical_path));
        }
        let mut digest = Sha256::new();
        digest.update(MACOS_IDENTITY_DOMAIN);
        digest.update(opened.dev().to_le_bytes());
        digest.update(opened.ino().to_le_bytes());
        digest.update(birth_seconds.to_le_bytes());
        digest.update(birth_nanoseconds.to_le_bytes());
        (2, hex::encode(digest.finalize()))
    };

    #[cfg(target_os = "linux")]
    let (version, sha256) = {
        let identity = capture_linux_workspace_identity(&directory)
            .map_err(|_| HomeError::InvalidWorkspace(canonical_path.clone()))?;
        (identity.version(), hex::encode(identity.digest()))
    };

    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    let (version, sha256) = return Err(HomeError::InvalidWorkspace(canonical_path));

    Ok(WorkspaceIdentity {
        canonical_path,
        version,
        sha256,
    })
}

#[cfg(windows)]
fn detect_platform_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, HomeError> {
    let binding = colossus_windows_native::BoundPath::open_directory(workspace)
        .map_err(|_| HomeError::InvalidWorkspace(workspace.to_owned()))?;
    binding
        .revalidate()
        .map_err(|_| HomeError::InvalidWorkspace(workspace.to_owned()))?;
    let identity = binding.identity();
    if identity.volume_serial_number == 0 || identity.file_id == [0; 16] {
        return Err(HomeError::InvalidWorkspace(workspace.to_owned()));
    }
    let mut digest = Sha256::new();
    digest.update(WINDOWS_IDENTITY_DOMAIN);
    digest.update(identity.volume_serial_number.to_le_bytes());
    digest.update(identity.file_id);
    Ok(WorkspaceIdentity {
        canonical_path: binding.canonical_path().to_owned(),
        version: 3,
        sha256: hex::encode(digest.finalize()),
    })
}

#[cfg(not(any(unix, windows)))]
fn detect_platform_workspace_identity(workspace: &Path) -> Result<WorkspaceIdentity, HomeError> {
    Err(HomeError::InvalidWorkspace(workspace.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::private_tempdir;

    #[test]
    fn identity_is_stable_for_one_workspace_and_differs_for_another() {
        let first = private_tempdir();
        let second = private_tempdir();
        let first_identity = detect_workspace_identity(first.path()).expect("first identity");
        assert_eq!(
            first_identity,
            detect_workspace_identity(first.path()).expect("stable identity")
        );
        assert_ne!(
            first_identity.sha256(),
            detect_workspace_identity(second.path())
                .expect("second identity")
                .sha256()
        );
    }

    #[test]
    fn legacy_path_reusable_identity_is_not_valid_for_home_partitioning() {
        let digest = "0".repeat(64);
        assert!(
            WorkspaceIdentityRef {
                version: 1,
                sha256: &digest,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn linux_birthtime_v4_digest_is_unchanged() {
        assert_eq!(
            hex::encode(linux_birthtime_identity_digest(
                42,
                84,
                1_700_000_000,
                123_456_789,
            )),
            "af11b5d9a35e09cce55b020dfb8656f6ed3a1cb035119bf2d5f9f36013734c62"
        );
    }

    #[test]
    fn linux_nfs_v5_digest_binds_remote_volume_and_opaque_handle() {
        let first =
            linux_nfs_identity_digest(4, &[192, 0, 2, 10], 2049, 0x1234, 0x5678, -7, &[1, 2, 3, 4]);
        assert_eq!(
            first,
            linux_nfs_identity_digest(4, &[192, 0, 2, 10], 2049, 0x1234, 0x5678, -7, &[1, 2, 3, 4],)
        );
        assert_ne!(
            first,
            linux_nfs_identity_digest(4, &[192, 0, 2, 11], 2049, 0x1234, 0x5678, -7, &[1, 2, 3, 4],)
        );
        assert_ne!(
            first,
            linux_nfs_identity_digest(4, &[192, 0, 2, 10], 2049, 0x1234, 0x5678, -7, &[1, 2, 3, 5],)
        );
    }

    #[test]
    fn nfs_file_handle_identity_is_valid_for_home_partitioning() {
        let digest = "0".repeat(64);
        assert!(
            WorkspaceIdentityRef {
                version: 5,
                sha256: &digest,
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn missing_linux_birthtime_selects_nfs_v5() {
        let identity = select_linux_workspace_identity(
            42,
            84,
            LinuxStatxIdentityEvidence {
                metadata_matches: true,
                birthtime: None,
            },
            || Ok([0x5a; 32]),
        )
        .expect("NFS fallback identity");

        assert_eq!(identity.version(), 5);
        assert_eq!(identity.digest(), [0x5a; 32]);
    }

    #[test]
    fn linux_nfs_v5_digest_ignores_transient_device_and_inode() {
        let capture = |device, inode| {
            select_linux_workspace_identity(
                device,
                inode,
                LinuxStatxIdentityEvidence {
                    metadata_matches: true,
                    birthtime: None,
                },
                || Ok([0xa5; 32]),
            )
            .expect("NFS fallback identity")
        };

        assert_eq!(capture(1, 2), capture(9, 10));
    }

    #[test]
    fn claimed_invalid_linux_birthtime_never_downgrades_to_nfs() {
        for birthtime in [(0, 0), (1, 1_000_000_000)] {
            assert!(
                select_linux_workspace_identity(
                    42,
                    84,
                    LinuxStatxIdentityEvidence {
                        metadata_matches: true,
                        birthtime: Some(birthtime),
                    },
                    || panic!("invalid claimed birthtime must not invoke NFS fallback"),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn conflicting_linux_metadata_never_downgrades_to_nfs() {
        assert!(
            select_linux_workspace_identity(
                42,
                84,
                LinuxStatxIdentityEvidence {
                    metadata_matches: false,
                    birthtime: None,
                },
                || panic!("conflicting metadata must not invoke NFS fallback"),
            )
            .is_err()
        );
    }

    #[test]
    fn missing_linux_birthtime_fails_closed_when_nfs_capture_fails() {
        assert!(
            select_linux_workspace_identity(
                42,
                84,
                LinuxStatxIdentityEvidence {
                    metadata_matches: true,
                    birthtime: None,
                },
                || Err(io::Error::other("unsupported NFS identity")),
            )
            .is_err()
        );
    }
}
