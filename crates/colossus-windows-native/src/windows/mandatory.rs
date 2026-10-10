//! Mandatory integrity applies only to the exact-package writable browser tree.
//! A Low label permits the enrolled Low-integrity token to write; its protected
//! package DACL still rejects every unrelated Low-integrity principal.
use super::*;
use windows_sys::Win32::{
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetSecurityDescriptorSacl, LABEL_SECURITY_INFORMATION, SE_SACL_PROTECTED,
        SYSTEM_MANDATORY_LABEL_ACE, SetSecurityDescriptorSacl, WinLowLabelSid,
    },
    System::SystemServices::{SYSTEM_MANDATORY_LABEL_ACE_TYPE, SYSTEM_MANDATORY_LABEL_NO_WRITE_UP},
};

pub(super) struct LowLabel {
    _descriptor: LocalSecurityDescriptor,
    acl: *mut windows_sys::Win32::Security::ACL,
}

impl LowLabel {
    pub(super) fn new() -> Result<Self, WindowsNativeError> {
        // This fixed descriptor has one inheritable Low mandatory label with
        // NO_WRITE_UP. It does not modify the separately constructed package DACL.
        let sddl: Vec<u16> = "S:(ML;OICI;NW;;;LW)".encode_utf16().chain([0]).collect();
        let mut descriptor = null_mut();
        // SAFETY: fixed terminated SDDL and valid output; Windows allocates the SD.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SECURITY_DESCRIPTOR_REVISION,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error("create browser Low integrity label"));
        }
        let owner = LocalSecurityDescriptor(descriptor);
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        // SAFETY: converted SD is retained; outputs borrow its valid SACL.
        if unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            == 0
            || present == 0
            || acl.is_null()
            || defaulted != 0
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        Ok(Self {
            _descriptor: owner,
            acl,
        })
    }

    pub(super) fn attach(
        &self,
        descriptor: &mut SECURITY_DESCRIPTOR,
    ) -> Result<(), WindowsNativeError> {
        // SAFETY: absolute SD is writable, and the borrowed SACL remains alive
        // through this owner's lifetime and the subsequent native creation call.
        if unsafe {
            SetSecurityDescriptorSacl(
                (descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                1,
                self.acl,
                0,
            ) != 0
                && SetSecurityDescriptorControl(
                    (descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                    SE_SACL_PROTECTED,
                    SE_SACL_PROTECTED,
                ) != 0
        } {
            Ok(())
        } else {
            Err(last_error("attach browser Low integrity label"))
        }
    }
}

pub(super) fn validate_low_label(file: &File) -> Result<(), WindowsNativeError> {
    let mut acl = null_mut();
    let mut descriptor = null_mut();
    // LABEL_SECURITY_INFORMATION requires READ_CONTROL, not general SACL access.
    // SAFETY: positively bound live file handle and valid output pointers.
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut acl,
            &mut descriptor,
        )
    };
    if result != NO_ERROR {
        return Err(WindowsNativeError::Io {
            operation: "query browser integrity label",
            source: std::io::Error::from_raw_os_error(i32::try_from(result).unwrap_or(i32::MAX)),
        });
    }
    let _descriptor = LocalSecurityDescriptor(descriptor);
    if acl.is_null() || descriptor.is_null() {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let low = well_known_sid(WinLowLabelSid)?;
    // SAFETY: GetSecurityInfo returns a valid retained security descriptor/ACL.
    if unsafe { (*acl).AceCount } != 1 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut ace = null_mut();
    // SAFETY: retained valid ACL; GetAce positively bounds the only requested ACE.
    if unsafe { GetAce(acl, 0, &mut ace) } == 0 || ace.is_null() {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: GetAce returns at least the ACE header, read without alignment assumptions.
    let header = unsafe { std::ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
    let sid_offset = offset_of!(SYSTEM_MANDATORY_LABEL_ACE, SidStart);
    if u32::from(header.AceType) != SYSTEM_MANDATORY_LABEL_ACE_TYPE
        || header.AceFlags & INHERIT_ONLY_ACE_FLAG != 0
        || usize::from(header.AceSize) < sid_offset + 8
    {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: the checked ACE contains its mask and the fixed SID header.
    let label = unsafe { std::ptr::read_unaligned(ace.cast::<SYSTEM_MANDATORY_LABEL_ACE>()) };
    let sid_bytes = unsafe { ace.cast::<u8>().add(sid_offset) };
    // SAFETY: second SID header byte is inside the positively bounded ACE.
    let sub_authorities = usize::from(unsafe { *sid_bytes.add(1) });
    if sub_authorities > 15 || sid_offset + 8 + 4 * sub_authorities > usize::from(header.AceSize) {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let sid = sid_bytes.cast();
    // SAFETY: the complete bounded SID is retained inside the kernel-returned ACE.
    if label.Mask != SYSTEM_MANDATORY_LABEL_NO_WRITE_UP
        || unsafe { IsValidSid(sid) } == 0
        || !sid_matches(sid, low.as_ptr().cast_mut().cast())
    {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    Ok(())
}
