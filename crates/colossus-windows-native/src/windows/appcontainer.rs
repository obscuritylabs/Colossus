//! Package identity is derived from a retained kernel token, never an input SID.
use super::*;
use std::os::windows::io::BorrowedHandle;
use windows_sys::Win32::{
    Security::{
        TOKEN_APPCONTAINER_INFORMATION, TokenAppContainerSid, TokenIntegrityLevel,
        TokenIsAppContainer, WinLowLabelSid,
    },
    Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
};

pub(super) const PACKAGE_PROFILE_RIGHTS: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE | DELETE | 0x40; // FILE_DELETE_CHILD

/// Exact AppContainer principal derived from a positively retained process token.
/// This opaque type accepts no SID strings and cannot be serialized or nominated
/// by a renderer. Native composition must use its exact spawned process handle.
pub struct AppContainerPrincipal {
    package: Box<[u8; SECURITY_MAX_SID_SIZE as usize]>,
    user: Box<[u8; SECURITY_MAX_SID_SIZE as usize]>,
    integrity: Box<[u8; SECURITY_MAX_SID_SIZE as usize]>,
}

impl AppContainerPrincipal {
    /// Bind the package identity and user of an already retained process handle.
    /// Reject ordinary processes and processes owned by another user.
    pub fn of_process(process: BorrowedHandle<'_>) -> Result<Self, WindowsNativeError> {
        Self::bind_process(process.as_raw_handle().cast())
    }

    fn bind_process(
        process: windows_sys::Win32::Foundation::HANDLE,
    ) -> Result<Self, WindowsNativeError> {
        let mut raw = null_mut();
        // SAFETY: borrowed process stays alive, and raw receives one token handle.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
            return Err(last_error("open enrolled AppContainer token"));
        }
        // SAFETY: OpenProcessToken transferred this uniquely owned token handle.
        let token = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
        let mut is_container: u32 = 0;
        let mut returned = 0;
        // SAFETY: exactly sized integer output, live retained token handle.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenIsAppContainer,
                (&raw mut is_container).cast(),
                size_of::<u32>() as u32,
                &mut returned,
            )
        } == 0
            || returned != size_of::<u32>() as u32
            || is_container != 1
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let package = token_sid(&token, TokenAppContainerSid)?;
        let user = token_sid(&token, TokenUser)?;
        let integrity = token_sid(&token, TokenIntegrityLevel)?;
        let current = current_user_sid()?;
        if !sid_matches(
            user.as_ptr().cast_mut().cast(),
            current.as_ptr().cast_mut().cast(),
        ) {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let principal = Self {
            package,
            user,
            integrity,
        };
        principal.validate_low_integrity()?;
        Ok(principal)
    }

    /// Independently bind this host's actual current AppContainer token.
    pub fn of_current() -> Result<Self, WindowsNativeError> {
        // SAFETY: the current-process pseudo handle is valid for OpenProcessToken.
        // It is deliberately not wrapped in BorrowedHandle, which rejects -1.
        Self::bind_process(unsafe { GetCurrentProcess() })
    }

    pub(super) fn package_sid(&self) -> PSID {
        self.package.as_ptr().cast_mut().cast()
    }

    /// Verify the integrity SID captured from the retained enrolled kernel token.
    /// Binding already requires this check; no caller can nominate an integrity SID.
    pub fn validate_low_integrity(&self) -> Result<(), WindowsNativeError> {
        let low = well_known_sid(WinLowLabelSid)?;
        if sid_matches(
            self.integrity.as_ptr().cast_mut().cast(),
            low.as_ptr().cast_mut().cast(),
        ) {
            Ok(())
        } else {
            Err(WindowsNativeError::UnsafePermissions)
        }
    }

    /// Create one fresh profile beneath a retained owner-private parent.
    /// Grants only current user/System/Admin and this kernel-derived package.
    /// Package cache access excludes WRITE_DAC and WRITE_OWNER; inherited broad
    /// access and every reparse component fail closed. Existing names are rejected.
    pub fn create_directory(&self, path: &Path) -> Result<(), WindowsNativeError> {
        let current = current_user_sid()?;
        if !sid_matches(
            current.as_ptr().cast_mut().cast(),
            self.user.as_ptr().cast_mut().cast(),
        ) {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let parent = open_bound(
            path.parent().ok_or(WindowsNativeError::InvalidInput)?,
            BoundKind::Directory,
        )?;
        parent.validate_private_owner_dacl()?;
        parent.validate_ancestor_namespace_authority()?;
        parent.revalidate()?;
        let encoded = nul_terminated_path(path)?;
        with_security_attributes(
            PRIVATE_DIRECTORY_LABELS,
            SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            Some(self.package_sid()),
            |attributes| {
                // SAFETY: terminated path and protected descriptor live for the call.
                if unsafe { CreateDirectoryW(encoded.as_ptr(), attributes) } == 0 {
                    return Err(last_error("create enrolled AppContainer profile"));
                }
                Ok(())
            },
        )?;
        let verify = || {
            let created = open_bound(path, BoundKind::Directory)?;
            validate_owner_dacl(
                &created.file,
                DaclValidation::PrivateAppContainer(self.package_sid()),
            )?;
            created.revalidate()?;
            parent.revalidate()
        };
        if let Err(error) = verify() {
            let _ = fs::remove_dir(path);
            return Err(error);
        }
        Ok(())
    }
}

fn token_sid(
    token: &OwnedHandle,
    kind: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Result<Box<[u8; SECURITY_MAX_SID_SIZE as usize]>, WindowsNativeError> {
    let mut required = 0;
    // SAFETY: sizing query has null buffer and valid retained token/output.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            kind,
            null_mut(),
            0,
            &mut required,
        );
    }
    if required < size_of::<TOKEN_APPCONTAINER_INFORMATION>() as u32 || required > 64 * 1024 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut bytes = vec![0_u8; required as usize];
    // SAFETY: exact bounded output allocation; token and buffer remain live.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            kind,
            bytes.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(last_error("query enrolled AppContainer SID"));
    }
    // TOKEN_USER, TOKEN_APPCONTAINER_INFORMATION and TOKEN_MANDATORY_LABEL all
    // begin with a SID pointer (the latter's SID_AND_ATTRIBUTES first field).
    // SAFETY: the returned structure has at least pointer size; read is unaligned.
    let sid = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<PSID>()) };
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: Windows returned a validated SID inside live token information.
    let length = unsafe { windows_sys::Win32::Security::GetLengthSid(sid) };
    if length == 0 || length > SECURITY_MAX_SID_SIZE {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut copied = Box::new([0_u8; SECURITY_MAX_SID_SIZE as usize]);
    // SAFETY: destination has fixed maximum SID size and source stays valid.
    if unsafe {
        windows_sys::Win32::Security::CopySid(
            SECURITY_MAX_SID_SIZE,
            copied.as_mut_ptr().cast(),
            sid,
        )
    } == 0
    {
        return Err(last_error("copy enrolled AppContainer SID"));
    }
    Ok(copied)
}
