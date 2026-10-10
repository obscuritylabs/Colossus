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

/// Create one file with an explicit owner-private DACL and write its exact contents.
///
/// The new file never inherits the parent directory's access entries, fails when the
/// name already exists, and is removed again when its committed owner or DACL is not
/// private. Use it for local secrets that must stay unreadable by other local accounts.
pub fn create_private_file(path: &Path, contents: &[u8]) -> Result<(), WindowsNativeError> {
    #[cfg(windows)]
    {
        crate::windows::create_private_file(path, contents)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, contents);
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

impl FileIdentity {
    /// Query the stable identity of an already-open file without resolving its path.
    pub fn of(file: &File) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::file_identity(file)
        }
        #[cfg(not(windows))]
        {
            let _ = file;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }
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
    #[cfg(windows)]
    pub(crate) fn from_inner(inner: crate::windows::BoundPathInner) -> Self {
        Self { inner }
    }
    /// Bind this host's protected profile using its actual AppContainer identity.
    /// Ancestors are retained with zero requested access and checked for reparse
    /// points/identity without opening the user's storage for read or mutation.
    /// The leaf requires an exact protected current-package DACL. Parent-side
    /// strict ownership validation remains a separate supervision obligation.
    pub fn open_appcontainer_directory(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound_appcontainer_directory(path).map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Read one native-produced single-link file under the current AppContainer.
    /// Zero-access retained ancestors reject reparse points; the file's inherited
    /// ACL may grant only current user/System/Admin and this exact kernel package.
    /// It accepts no supplied SID and does not replace parent profile supervision.
    pub fn open_appcontainer_file(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound_appcontainer_file(path).map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Unlink only this retained native-produced file in the current AppContainer.
    /// The kernel package, DACL, single-link policy and exact file identity are
    /// checked before a DELETE-enabled handle marks the object for POSIX deletion.
    /// Success proves no filename links remain; release retained bindings before
    /// retiring the containing directory. A replacement is never deleted.
    pub fn remove_appcontainer_file(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::remove_bound_appcontainer_file(&self.inner)
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }
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

    /// Retain a regular file while denying writes/deletion and ancestor replacement.
    /// Use for digest-verified executable/DLL bytes that must remain frozen until
    /// the entire supervised process tree exits. All components reject reparse
    /// points; ancestor handles deny deletion, and the leaf shares read access only.
    pub fn open_immutable_file(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound_immutable_file(path).map(|inner| Self { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Open and retain one regular file for read/write access while rejecting every
    /// reparse-point component.
    pub fn open_file_read_write(path: &Path) -> Result<Self, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::open_bound_file_read_write(path).map(|inner| Self { inner })
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

    /// Number of filesystem names linked to this exact retained object.
    pub fn link_count(&self) -> Result<u64, WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::file_link_count(&self.inner.file)
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

    /// Reject untrusted byte/namespace mutation on the directory and every ancestor.
    /// Read/execute access is permitted, but adding an unlisted DLL/file or child
    /// directory, changing metadata, deletion and DACL/owner mutation are denied.
    /// This supplements frozen executable handles; it does not grant trust to an
    /// inventory or prove a publisher signature.
    pub fn validate_immutable_directory_dacl(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner.validate_immutable_directory_dacl()
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Require a protected profile DACL for this process's actual AppContainer.
    /// Only its current user, System/Admin and kernel-derived package may access
    /// the profile. The package cannot change DACL/ownership. No SID input is used.
    /// An ordinary process cannot substitute this for owner-private validation.
    pub fn validate_private_appcontainer_dacl(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::validate_private_appcontainer_dacl(&self.inner.file)
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Positively query this retained object's effective Low integrity label and
    /// NO_WRITE_UP policy. This grants no filesystem access and accepts no caller
    /// supplied SID. Package-specific DACL validation also requires this check.
    pub fn validate_low_integrity_label(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            crate::windows::validate_low_integrity_label(&self.inner.file)
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Require every retained ancestor namespace to be owned by a trusted principal
    /// and deny untrusted principals authority to replace or replay entries beneath it.
    pub fn validate_ancestor_namespace_authority(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner.validate_ancestor_namespace_authority()
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }

    /// Validate namespace authority for this directory and every retained ancestor.
    pub fn validate_namespace_authority(&self) -> Result<(), WindowsNativeError> {
        #[cfg(windows)]
        {
            self.inner.validate_namespace_authority()
        }
        #[cfg(not(windows))]
        {
            Err(WindowsNativeError::UnsupportedPlatform)
        }
    }
}
