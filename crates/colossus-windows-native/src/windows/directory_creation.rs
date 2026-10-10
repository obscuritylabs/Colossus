//! A positive allocation owner exists before any fallible path verification.
use super::*;
use windows_sys::{
    Wdk::{
        Foundation::OBJECT_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION_EX,
            FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, FileDispositionInformationEx,
            NtCreateFile,
        },
    },
    Win32::Foundation::{OBJ_CASE_INSENSITIVE, UNICODE_STRING},
};

/// Exact newly allocated private directory, retained before metadata/path binding.
/// Register this owner before calling `bind` or performing any asynchronous effect.
/// Failed binding does not lose allocation ownership. Explicit removal is bounded
/// to the empty exact object; Drop is best effort and never a cleanup receipt.
pub struct PrivateDirectoryCreation {
    file: Option<File>,
    parent: BoundPathInner,
    path: PathBuf,
    identity: Option<FileIdentity>,
    removal_started: bool,
    closed: bool,
    package: Option<AppContainerPrincipal>,
}
impl PrivateDirectoryCreation {
    /// Atomically create one fresh leaf relative to a retained owner-private parent.
    /// NT `FILE_CREATE` returns the allocation handle in the same operation and
    /// cannot adopt an existing directory, junction or caller-selected replacement.
    pub fn create(path: &Path) -> Result<Self, WindowsNativeError> {
        let parent = open_bound(
            path.parent().ok_or(WindowsNativeError::InvalidInput)?,
            BoundKind::Directory,
        )?;
        parent.validate_private_owner_dacl()?;
        parent.validate_ancestor_namespace_authority()?;
        parent.revalidate()?;
        let leaf = path.file_name().ok_or(WindowsNativeError::InvalidInput)?;
        Self::allocate(parent, leaf, None)
    }

    /// Create a fresh transfer directory beneath this process's exact enrolled profile.
    /// The current kernel token supplies the package SID; names are single components
    /// and allocation ownership is returned before fallible path binding.
    pub fn create_in_appcontainer_profile(
        parent: &crate::BoundPath,
        basename: &str,
    ) -> Result<Self, WindowsNativeError> {
        parent.revalidate()?;
        parent.validate_private_appcontainer_dacl()?;
        let package = AppContainerPrincipal::of_current()?;
        let bound = open_bound_with_mode(
            parent.canonical_path(),
            BoundKind::Directory,
            appcontainer::PACKAGE_PROFILE_RIGHTS | SYNCHRONIZE,
            false,
            true,
        )?;
        if bound.identity != parent.identity() {
            return Err(WindowsNativeError::IdentityChanged);
        }
        Self::allocate(bound, std::ffi::OsStr::new(basename), Some(package))
    }

    fn allocate(
        parent: BoundPathInner,
        leaf: &std::ffi::OsStr,
        package: Option<AppContainerPrincipal>,
    ) -> Result<Self, WindowsNativeError> {
        let mut encoded: Vec<u16> = leaf.encode_wide().collect();
        if encoded.is_empty()
            || encoded.len() > 255
            || encoded.iter().any(|c| matches!(*c, 0 | 47 | 92 | 58))
        {
            return Err(WindowsNativeError::InvalidInput);
        }
        let mut name = UNICODE_STRING {
            Length: (encoded.len() * 2) as u16,
            MaximumLength: (encoded.len() * 2) as u16,
            Buffer: encoded.as_mut_ptr(),
        };
        let file = with_security_attributes(
            PRIVATE_DIRECTORY_LABELS,
            SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            package.as_ref().map(AppContainerPrincipal::package_sid),
            |security| {
                let attributes = OBJECT_ATTRIBUTES {
                    Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
                    RootDirectory: parent.file.as_raw_handle().cast(),
                    ObjectName: &mut name,
                    Attributes: OBJ_CASE_INSENSITIVE,
                    SecurityDescriptor: security.lpSecurityDescriptor.cast(),
                    SecurityQualityOfService: null(),
                };
                let mut output = null_mut();
                let mut status = IO_STATUS_BLOCK::default();
                // SAFETY: exact relative leaf and protected descriptor remain live;
                // parent is retained, outputs are valid, and FILE_CREATE is exclusive.
                let result = unsafe {
                    NtCreateFile(
                        &mut output,
                        if package.is_some() {
                            appcontainer::PACKAGE_PROFILE_RIGHTS | SYNCHRONIZE
                        } else {
                            FILE_ALL_ACCESS
                        },
                        &attributes,
                        &mut status,
                        null(),
                        FILE_ATTRIBUTE_DIRECTORY,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        FILE_CREATE,
                        FILE_DIRECTORY_FILE
                            | FILE_OPEN_REPARSE_POINT
                            | FILE_SYNCHRONOUS_IO_NONALERT,
                        null(),
                        0,
                    )
                };
                if result < 0 {
                    // SAFETY: NTSTATUS translation has no borrowed memory.
                    let error = unsafe { RtlNtStatusToDosError(result) };
                    return Err(WindowsNativeError::Io {
                        operation: "allocate private directory",
                        source: std::io::Error::from_raw_os_error(error as i32),
                    });
                }
                if output.is_null() || output == INVALID_HANDLE_VALUE {
                    return Err(WindowsNativeError::IdentityChanged);
                }
                // SAFETY: successful NtCreateFile transferred this exact allocation.
                Ok(unsafe { File::from_raw_handle(output.cast()) })
            },
        )?;
        // No fallible operation follows successful allocation until the owner is
        // returned. In particular, path canonicalization and file IDs happen in bind.
        let path = parent.canonical_path.join(leaf);
        Ok(Self {
            file: Some(file),
            parent,
            path,
            identity: None,
            removal_started: false,
            closed: false,
            package,
        })
    }

    /// Verify and bind the same created object without consuming its owner.
    pub fn bind(&self) -> Result<crate::BoundPath, WindowsNativeError> {
        if self.closed || self.removal_started {
            return Err(WindowsNativeError::IdentityChanged);
        }
        self.parent.revalidate()?;
        let file = self
            .file
            .as_ref()
            .ok_or(WindowsNativeError::IdentityChanged)?;
        let bound = if let Some(package) = &self.package {
            validate_owner_dacl(
                file,
                DaclValidation::PrivateAppContainer(package.package_sid()),
            )?;
            open_bound_appcontainer_directory(&self.path)?
        } else {
            validate_private_owner_dacl(file)?;
            open_bound(&self.path, BoundKind::Directory)?
        };
        if bound.identity != file_identity(file)? {
            return Err(WindowsNativeError::IdentityChanged);
        }
        if self.package.is_none() {
            bound.validate_private_owner_dacl()?;
        }
        bound.revalidate()?;
        Ok(crate::BoundPath::from_inner(bound))
    }

    /// Original positively allocated path; never an authority to erase a replacement.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete only the exact empty allocation and prove its name is absent or now
    /// names a different object. Release other bindings first; pending deletion
    /// beneath retained handles remains unknown and retryable through this owner.
    pub fn remove_empty(&mut self) -> Result<(), WindowsNativeError> {
        if self.closed {
            return Ok(());
        }
        self.parent.revalidate()?;
        if !self.removal_started {
            let file = self
                .file
                .as_ref()
                .ok_or(WindowsNativeError::IdentityChanged)?;
            self.identity = Some(file_identity(file)?);
            let mut information =
                windows_sys::Win32::Storage::FileSystem::FILE_STANDARD_INFO::default();
            // SAFETY: exact retained allocation handle and correctly sized output.
            if unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle().cast(),
                    windows_sys::Win32::Storage::FileSystem::FileStandardInfo,
                    (&mut information
                        as *mut windows_sys::Win32::Storage::FileSystem::FILE_STANDARD_INFO)
                        .cast(),
                    size_of::<windows_sys::Win32::Storage::FileSystem::FILE_STANDARD_INFO>() as u32,
                )
            } == 0
            {
                return Err(last_error("query exact private directory retirement"));
            }
            let disposition = FILE_DISPOSITION_INFORMATION_EX { Flags: 0x1 | 0x2 }; // DELETE | POSIX_SEMANTICS
            let mut status = IO_STATUS_BLOCK::default();
            // SAFETY: exact allocation handle includes DELETE; empty directory
            // disposition never recursively deletes children or resolves a path.
            let result = if information.DeletePending {
                // A separately retained quarantine cleanup already marked this
                // exact kernel object for deletion. No path/name inference is used.
                0
            } else {
                unsafe {
                    NtSetInformationFile(
                        file.as_raw_handle().cast(),
                        &mut status,
                        (&disposition as *const FILE_DISPOSITION_INFORMATION_EX).cast(),
                        size_of::<FILE_DISPOSITION_INFORMATION_EX>() as u32,
                        FileDispositionInformationEx,
                    )
                }
            };
            if result < 0 {
                // SAFETY: NTSTATUS translation does not borrow memory.
                let error = unsafe { RtlNtStatusToDosError(result) };
                return Err(WindowsNativeError::Io {
                    operation: "retire exact private directory",
                    source: std::io::Error::from_raw_os_error(error as i32),
                });
            }
            self.removal_started = true;
            self.file.take();
        }
        let current = if self.package.is_some() {
            open_bound_appcontainer_directory(&self.path)
        } else {
            open_bound(&self.path, BoundKind::Directory)
        };
        match current {
            Ok(current) if Some(current.identity) != self.identity => {}
            Ok(_) => return Err(WindowsNativeError::IdentityChanged),
            Err(WindowsNativeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.closed = true;
        Ok(())
    }
}
impl Drop for PrivateDirectoryCreation {
    fn drop(&mut self) {
        let _ = self.remove_empty();
    }
}
