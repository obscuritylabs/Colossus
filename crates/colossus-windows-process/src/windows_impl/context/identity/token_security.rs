//! Creation-time security for the new browser primary; never changes the caller.
use super::{OwnedHandle, WindowsProcessError, last_error, sid_text};
use std::{mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, DuplicateTokenEx,
        SECURITY_ATTRIBUTES, SecurityIdentification, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE,
        TOKEN_QUERY, TokenPrimary,
    },
    Storage::FileSystem::READ_CONTROL,
};

struct Descriptor(*mut core::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: this exact allocation came from the descriptor conversion.
        unsafe { LocalFree(self.0) };
    }
}

pub(super) fn with_user_access(
    source: &OwnedHandle,
    user: &[u8],
) -> Result<OwnedHandle, WindowsProcessError> {
    // The SID is a validated copy from this context's own TokenUser, never input.
    let user = sid_text(user)?;
    let descriptor_text = format!("D:P(A;;GA;;;{user})(A;;GA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut raw_descriptor = null_mut();
    // SAFETY: fixed protected DACL with only the kernel-derived user and SYSTEM;
    // the new LocalAlloc descriptor remains live through DuplicateTokenEx.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor_text.as_ptr(),
            1,
            &mut raw_descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("build browser primary security"));
    }
    let descriptor = Descriptor(raw_descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut raw = null_mut();
    // SAFETY: duplicates only the retained newly restricted primary. Explicit
    // security applies at allocation to the new token object; it does not modify
    // the source, its default DACL, privileges, groups or authentication identity.
    // READ_CONTROL permits independent security readback; no token-adjustment right is held.
    if unsafe {
        DuplicateTokenEx(
            source.raw(),
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | READ_CONTROL,
            &attributes,
            SecurityIdentification,
            TokenPrimary,
            &mut raw,
        )
    } == 0
    {
        return Err(last_error("DuplicateTokenEx(browser primary security)"));
    }
    OwnedHandle::new(raw, "DuplicateTokenEx(browser primary security)")
}
