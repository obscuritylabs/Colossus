use crate::WindowsNativeError;
use std::{fs::File, path::Path};

/// Create one directory with an owner-private DACL and no inherited broad access.
pub fn create_private_directory(path: &Path) -> Result<(), WindowsNativeError> {
    #[cfg(windows)]
    {
        crate::windows::create_private_directory(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(WindowsNativeError::UnsupportedPlatform)
    }
}

/// Atomically replace one private file with another file in the same private directory.
pub fn replace_private_file(source: &Path, destination: &Path) -> Result<(), WindowsNativeError> {
    #[cfg(windows)]
    {
        crate::windows::replace_private_file(source, destination)
    }
    #[cfg(not(windows))]
    {
        let _ = (source, destination);
        Err(WindowsNativeError::UnsupportedPlatform)
    }
}

/// Stable kernel identity returned by `FileIdInfo`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    /// Volume serial number containing the file.
    pub volume_serial_number: u64,
    /// Filesystem-provided 128-bit object identifier.
    pub file_id: [u8; 16],
}

/// A retained exact Windows filesystem object and all opened path ancestors.
pub struct BoundPath {
    #[cfg(windows)]
    inner: crate::windows::BoundPathInner,
    #[cfg(not(windows))]
    _unsupported: (),
}

impl std::fmt::Debug for BoundPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundPath")
            .field("canonical_path", &self.canonical_path())
            .field("identity", &self.identity())
            .finish_non_exhaustive()
    }
}

impl BoundPath {
    /// Open and retain one directory while rejecting every reparse-point component.
    pub fn open_directory(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound(path, crate::windows::BoundKind::Directory)
                .map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Open and retain one regular file while rejecting every reparse-point component.
    pub fn open_file(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound(path, crate::windows::BoundKind::File)
                .map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Canonical path captured after the exact object was opened.
    pub fn canonical_path(&self) -> &Path {
        #[cfg(windows)]
        {
            &self.inner.canonical_path
        }
        #[cfg(not(windows))]
        {
            Path::new("")
        }
    }

    /// Stable identity of the retained object.
    pub fn identity(&self) -> FileIdentity {
        #[cfg(windows)]
        {
            self.inner.identity
        }
        #[cfg(not(windows))]
        {
            FileIdentity {
                volume_serial_number: 0,
                file_id: [0; 16],
            }
        }
    }

    /// Clone the retained object handle as a standard file.
    pub fn try_clone_file(&self) -> Result<File, WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner
                .file
                .try_clone()
                .map_err(|source| WindowsNativeError::Io {
                    operation: "clone retained handle",
                    source,
                })
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Reopen the canonical name and prove it still names the retained object.
    pub fn revalidate(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner.revalidate()
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Require ownership by the current user or a trusted local system principal and a
    /// DACL whose allow entries grant access only to those same trusted principals.
    pub fn validate_private_owner_dacl(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner.validate_private_owner_dacl()
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }
}
