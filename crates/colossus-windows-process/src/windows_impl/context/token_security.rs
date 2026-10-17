//! User access to the exact newly created, still-suspended browser's primary token.
use super::{
    super::{OwnedHandle, WindowsProcessError, last_error},
    identity,
};
use std::{mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        AccessCheck,
        Authorization::{
            EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, SE_KERNEL_OBJECT, SetEntriesInAclW,
            SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, DuplicateTokenEx, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
        GetSecurityDescriptorDacl, GetSecurityDescriptorLength, OWNER_SECURITY_INFORMATION,
        PRIVILEGE_SET, SecurityIdentification, TOKEN_ALL_ACCESS, TOKEN_DUPLICATE, TOKEN_EXECUTE,
        TOKEN_QUERY, TOKEN_READ, TOKEN_WRITE, TokenImpersonation,
    },
    Storage::FileSystem::{READ_CONTROL, WRITE_DAC},
    System::Threading::OpenProcessToken,
};

struct Allocation(*mut core::ffi::c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: owns exactly one descriptor/ACL allocated by the security APIs.
        unsafe { LocalFree(self.0) };
    }
}

/// Called only after the helper attests its exact returned child and atomic Job.
/// The existing ReturnedChild guard retains both suspended outputs on every error.
pub(super) fn retain_browser_user_access(
    process: &OwnedHandle,
    user: &[u8],
) -> Result<(), WindowsProcessError> {
    let mut raw = null_mut();
    // SAFETY: retained exact newly created process; no PID lookup or foreign token.
    // DUPLICATE supports a query-only identification copy for final AccessCheck.
    if unsafe {
        OpenProcessToken(
            process.raw(),
            TOKEN_QUERY | TOKEN_DUPLICATE | READ_CONTROL | WRITE_DAC,
            &mut raw,
        )
    } == 0
    {
        return Err(last_error("open suspended browser token security"));
    }
    let token = OwnedHandle::new(raw, "suspended browser token security")?;
    let facts = identity::token(process.raw())?;
    identity::require(
        facts.appcontainer
            && !facts.elevated
            && facts.integrity == 4096
            && facts.capabilities == 0
            && identity::same_sid(&facts.user, user),
        "browser token security identity changed",
    )?;
    grant_user(&token, user)?;
    identity::require(
        all_access(&token)?,
        "browser cannot attenuate its own primary",
    )
}

fn descriptor(token: &OwnedHandle) -> Result<Allocation, WindowsProcessError> {
    let mut raw = null_mut();
    // SAFETY: retained token with READ_CONTROL; only its DACL is read.
    let status = unsafe {
        GetSecurityInfo(
            token.raw(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut raw,
        )
    };
    identity::require(
        status == 0 && !raw.is_null(),
        "browser token DACL unavailable",
    )?;
    let descriptor = Allocation(raw);
    // SAFETY: successful API returned this allocated security descriptor.
    let length = unsafe { GetSecurityDescriptorLength(raw) };
    identity::require(length > 0 && length <= 65_536, "browser token DACL bound")?;
    Ok(descriptor)
}

fn grant_user(token: &OwnedHandle, user: &[u8]) -> Result<(), WindowsProcessError> {
    let descriptor = descriptor(token)?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut old_acl = null_mut();
    // SAFETY: retained kernel descriptor and writable DACL metadata outputs.
    identity::require(
        unsafe {
            GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut old_acl, &mut defaulted)
        } != 0
            && present != 0
            && !old_acl.is_null(),
        "browser token has no bounded DACL",
    )?;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: TOKEN_ALL_ACCESS,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: user.as_ptr().cast_mut().cast(),
            ..Default::default()
        },
    };
    let mut new_acl = null_mut();
    // SAFETY: one exact kernel-derived own user SID; preserves every other ACE.
    let status = unsafe { SetEntriesInAclW(1, &entry, old_acl, &mut new_acl) };
    identity::require(
        status == 0 && !new_acl.is_null(),
        "browser user token ACL allocation failed",
    )?;
    let acl = Allocation(new_acl.cast());
    // SAFETY: only the new owned browser token's DACL is changed. No default DACL,
    // owner, label, groups, privileges, capabilities, session or logon ID is changed.
    // The process remains suspended under exact creator custody throughout.
    let status = unsafe {
        SetSecurityInfo(
            token.raw(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.0.cast(),
            null_mut(),
        )
    };
    identity::require(status == 0, "browser user token ACL update failed")
}

fn all_access(token: &OwnedHandle) -> Result<bool, WindowsProcessError> {
    let mut raw = null_mut();
    // SAFETY: query-only identification copy of this exact browser token. It is
    // never assigned to a process or thread, and is not used for impersonation.
    if unsafe {
        DuplicateTokenEx(
            token.raw(),
            TOKEN_QUERY,
            std::ptr::null(),
            SecurityIdentification,
            TokenImpersonation,
            &mut raw,
        )
    } == 0
    {
        return Err(last_error("duplicate browser token for access readback"));
    }
    let identification = OwnedHandle::new(raw, "browser identification readback")?;
    let descriptor = descriptor(token)?;
    let mapping = GENERIC_MAPPING {
        GenericRead: TOKEN_READ,
        GenericWrite: TOKEN_WRITE,
        GenericExecute: TOKEN_EXECUTE,
        GenericAll: TOKEN_ALL_ACCESS,
    };
    let mut privileges = vec![0_usize; 65_536 / size_of::<usize>()];
    let mut length = 65_536;
    let mut granted = 0;
    let mut allowed = 0;
    // SAFETY: retained descriptor and query-only identification token, initialized
    // aligned bounded privilege buffer and writable fixed outputs. No mutation.
    if unsafe {
        AccessCheck(
            descriptor.0,
            identification.raw(),
            TOKEN_ALL_ACCESS,
            &mapping,
            privileges.as_mut_ptr().cast::<PRIVILEGE_SET>(),
            &mut length,
            &mut granted,
            &mut allowed,
        )
    } == 0
    {
        return Err(last_error("browser primary self-access readback"));
    }
    Ok(allowed != 0 && granted & TOKEN_ALL_ACCESS == TOKEN_ALL_ACCESS)
}

#[cfg(test)]
mod tests;
