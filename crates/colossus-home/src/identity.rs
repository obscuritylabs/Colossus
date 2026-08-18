use crate::HomeError;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use sha2::{Digest as _, Sha256};
#[cfg(unix)]
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
const LINUX_IDENTITY_DOMAIN: &[u8] = b"colossus-home-workspace-linux-device-inode-birthtime-v4\0";
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
        if matches!(self.version, 2..=4)
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
        let statx = rustix::fs::statx(
            &directory,
            "",
            rustix::fs::AtFlags::EMPTY_PATH,
            rustix::fs::StatxFlags::BASIC_STATS | rustix::fs::StatxFlags::BTIME,
        )
        .map_err(|error| HomeError::io(&canonical_path, error.into()))?;
        if statx.stx_mask & rustix::fs::StatxFlags::BTIME.bits() == 0
            || statx.stx_ino != opened.ino()
            || statx.stx_dev_major != rustix::fs::major(opened.dev())
            || statx.stx_dev_minor != rustix::fs::minor(opened.dev())
            || statx.stx_btime.tv_sec <= 0
            || statx.stx_btime.tv_nsec >= 1_000_000_000
        {
            return Err(HomeError::InvalidWorkspace(canonical_path));
        }
        let mut digest = Sha256::new();
        digest.update(LINUX_IDENTITY_DOMAIN);
        digest.update(opened.dev().to_le_bytes());
        digest.update(opened.ino().to_le_bytes());
        digest.update(statx.stx_btime.tv_sec.to_le_bytes());
        digest.update(statx.stx_btime.tv_nsec.to_le_bytes());
        (4, hex::encode(digest.finalize()))
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
}
