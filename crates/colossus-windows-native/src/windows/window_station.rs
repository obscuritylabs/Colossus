//! One fresh logon-session station, with no authority over an existing station.
use super::{
    AppContainerPrincipal, ExclusiveAppContainerProfile, LocalAcl, LocalSecurityDescriptor,
    current_user_sid, last_error, sid_matches, well_known_sid,
};
use crate::WindowsNativeError;
use std::{
    mem::{offset_of, size_of},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::NO_ERROR,
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, EXPLICIT_ACCESS_W,
            GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_WINDOW_OBJECT, SET_ACCESS, SetEntriesInAclW,
            SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorSacl,
        InitializeSecurityDescriptor, IsValidAcl, IsValidSid, LABEL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        SECURITY_DESCRIPTOR, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
        SetSecurityDescriptorOwner, WinLocalSystemSid, WinLowLabelSid,
    },
    Storage::FileSystem::{READ_CONTROL, WRITE_OWNER},
    System::{
        StationsAndDesktops::{
            CloseWindowStation, CreateWindowStationW, GetProcessWindowStation,
            GetUserObjectInformationW, HWINSTA, UOI_FLAGS, USEROBJECTFLAGS,
        },
        SystemServices::{
            ACCESS_ALLOWED_ACE_TYPE, SECURITY_DESCRIPTOR_REVISION, SYSTEM_MANDATORY_LABEL_ACE_TYPE,
            SYSTEM_MANDATORY_LABEL_NO_WRITE_UP,
        },
    },
    UI::WindowsAndMessaging::{
        CWF_CREATE_ONLY, WINSTA_CREATEDESKTOP, WINSTA_READATTRIBUTES, WSF_VISIBLE,
    },
};

// Explicit noninteractive station rights avoid object-dependent GENERIC_ALL mapping.
const OWNER_RIGHTS: u32 = 0x000f_016f;
const PACKAGE_RIGHTS: u32 = 0x0002_010b;

/// A freshly created station granting only its owner, SYSTEM and one exact package.
/// There is no name input, raw-handle export, adoption or station-switching API.
/// This guard borrows its package authority and stays on its creating thread.
/// Retain it through acknowledged full Job drainage. Explicit `close` reports
/// retirement; Drop is best effort and never a cleanup acknowledgement.
pub struct OwnedAppContainerWindowStation<'authority> {
    station: HWINSTA,
    original_station: HWINSTA,
    _authority: StationAuthority<'authority>,
}

// Both variants retain their authority's lifetime. The profile variant also
// keeps the entire guard type non-Send/non-Sync, including process-backed guards.
enum StationAuthority<'authority> {
    Profile {
        _profile: &'authority ExclusiveAppContainerProfile,
    },
    Process {
        _principal: &'authority AppContainerPrincipal,
    },
}

impl<'authority> OwnedAppContainerWindowStation<'authority> {
    pub(super) fn create(
        profile: &'authority ExclusiveAppContainerProfile,
        package: PSID,
    ) -> Result<Self, WindowsNativeError> {
        Self::create_owned(StationAuthority::Profile { _profile: profile }, package)
    }

    pub(super) fn create_for_process(
        principal: &'authority AppContainerPrincipal,
    ) -> Result<Self, WindowsNativeError> {
        Self::create_owned(
            StationAuthority::Process {
                _principal: principal,
            },
            principal.package_sid(),
        )
    }

    fn create_owned(
        authority: StationAuthority<'authority>,
        package: PSID,
    ) -> Result<Self, WindowsNativeError> {
        let owner = current_user_sid()?;
        let system = well_known_sid(WinLocalSystemSid)?;
        let owner_sid = owner.as_ptr().cast_mut().cast();
        let system_sid = system.as_ptr().cast_mut().cast();
        // SAFETY: this is a borrowed current-process station, never closed by us.
        let original_station = unsafe { GetProcessWindowStation() };
        if original_station.is_null() {
            return Err(last_error("query current window station"));
        }
        let mut entries = [
            access_entry(owner_sid, OWNER_RIGHTS),
            access_entry(system_sid, OWNER_RIGHTS),
            access_entry(package, PACKAGE_RIGHTS),
        ];
        let mut acl = null_mut();
        // SAFETY: the three exact trustee SIDs remain live while Windows copies them.
        let status = unsafe { SetEntriesInAclW(3, entries.as_mut_ptr(), null(), &mut acl) };
        let _acl = LocalAcl(acl);
        if status != NO_ERROR {
            return Err(status_error("build fresh station DACL", status));
        }
        if acl.is_null() {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        // SAFETY: writable descriptor and retained ACL/SID storage outlive creation.
        if !unsafe {
            InitializeSecurityDescriptor((&raw mut descriptor).cast(), SECURITY_DESCRIPTOR_REVISION)
                != 0
                && SetSecurityDescriptorOwner((&raw mut descriptor).cast(), owner_sid, 0) != 0
                && SetSecurityDescriptorDacl((&raw mut descriptor).cast(), 1, acl, 0) != 0
                && SetSecurityDescriptorControl(
                    (&raw mut descriptor).cast(),
                    SE_DACL_PROTECTED,
                    SE_DACL_PROTECTED,
                ) != 0
        } {
            return Err(last_error("build fresh station security descriptor"));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&raw mut descriptor).cast(),
            bInheritHandle: 0,
        };
        // SAFETY: NULL asks Windows for the current logon-derived name; CREATE_ONLY
        // rejects an existing station. The descriptor is exact and noninheritable.
        let station = unsafe {
            CreateWindowStationW(
                null(),
                CWF_CREATE_ONLY,
                READ_CONTROL | WRITE_OWNER | (WINSTA_READATTRIBUTES | WINSTA_CREATEDESKTOP) as u32,
                &attributes,
            )
        };
        if station.is_null() {
            return Err(last_error("create exclusive logon window station"));
        }
        let owned = Self {
            station,
            original_station,
            _authority: authority,
        };
        owned.validate_noninteractive()?;
        owned.apply_low_label()?;
        owned.validate_security(owner_sid, system_sid, package)?;
        owned.validate_current_station()?;
        Ok(owned)
    }

    /// Close only this retained station after all enrolled children have drained.
    /// A failed Win32 close retains the handle for retry. The borrowed original
    /// station is never closed or restored; association changes are also reported.
    pub fn close(&mut self) -> Result<(), WindowsNativeError> {
        let unchanged = self.validate_current_station();
        if !self.station.is_null() {
            // SAFETY: this handle came only from successful CREATE_ONLY above.
            if unsafe { CloseWindowStation(self.station) } == 0 {
                return Err(last_error("close owned logon window station"));
            }
            self.station = null_mut();
        }
        unchanged?;
        self.validate_current_station()
    }

    fn validate_current_station(&self) -> Result<(), WindowsNativeError> {
        // SAFETY: queries the borrowed process association without mutating it.
        let current = unsafe { GetProcessWindowStation() };
        if current.is_null() {
            return Err(last_error("recheck current window station"));
        }
        if current != self.original_station {
            return Err(WindowsNativeError::IdentityChanged);
        }
        Ok(())
    }

    fn validate_noninteractive(&self) -> Result<(), WindowsNativeError> {
        let mut flags = USEROBJECTFLAGS::default();
        let mut bytes = 0;
        // SAFETY: the retained station is live and output is exactly sized.
        if unsafe {
            GetUserObjectInformationW(
                self.station,
                UOI_FLAGS,
                (&raw mut flags).cast(),
                size_of::<USEROBJECTFLAGS>() as u32,
                &mut bytes,
            )
        } == 0
        {
            return Err(last_error("query fresh station flags"));
        }
        if bytes != size_of::<USEROBJECTFLAGS>() as u32 || flags.dwFlags & WSF_VISIBLE as u32 != 0 {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        Ok(())
    }

    fn apply_low_label(&self) -> Result<(), WindowsNativeError> {
        let sddl: Vec<u16> = "S:(ML;;NW;;;LW)".encode_utf16().chain([0]).collect();
        let mut descriptor = null_mut();
        // SAFETY: fixed terminated noninheriting Low/NW SDDL; Windows owns allocation.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SECURITY_DESCRIPTOR_REVISION,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error("build fresh station Low label"));
        }
        let _descriptor = LocalSecurityDescriptor(descriptor);
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        // SAFETY: converted descriptor remains retained while its SACL is borrowed.
        if unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut acl, &mut defaulted) }
            == 0
            || present == 0
            || defaulted != 0
            || acl.is_null()
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        // SAFETY: LABEL alone changes only this newly owned station's label;
        // it does not request general SACL access or replace any existing DACL.
        let status = unsafe {
            SetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                LABEL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                acl,
            )
        };
        if status != NO_ERROR {
            return Err(status_error("set fresh station Low label", status));
        }
        Ok(())
    }

    fn validate_security(
        &self,
        owner: PSID,
        system: PSID,
        package: PSID,
    ) -> Result<(), WindowsNativeError> {
        let mut actual_owner = null_mut();
        let mut dacl = null_mut();
        let mut label = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: live station and valid output pointers; one LocalAlloc-owned SD.
        let status = unsafe {
            GetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
                &mut actual_owner,
                null_mut(),
                &mut dacl,
                &mut label,
                &mut descriptor,
            )
        };
        let _descriptor = LocalSecurityDescriptor(descriptor);
        if status != NO_ERROR {
            return Err(status_error("read back fresh station security", status));
        }
        // SAFETY: successful GetSecurityInfo returned the retained SD and its SIDs/ACLs.
        if descriptor.is_null()
            || actual_owner.is_null()
            || unsafe { IsValidSid(actual_owner) } == 0
            || !sid_matches(actual_owner, owner)
            || dacl.is_null()
            || label.is_null()
            || unsafe { IsValidAcl(dacl) } == 0
            || unsafe { IsValidAcl(label) } == 0
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptor is retained and valid outputs receive its control bits.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(last_error("inspect fresh station security control"));
        }
        // SAFETY: both ACL headers are valid and retained by the descriptor.
        if control & SE_DACL_PROTECTED == 0
            || unsafe { (*dacl).AceCount } != 3
            || unsafe { (*label).AceCount } != 1
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let mut seen = [false; 3];
        for index in 0..3 {
            let (mask, sid) = explicit_ace(dacl, index, ACCESS_ALLOWED_ACE_TYPE)?;
            let (position, rights) = if sid_matches(sid, owner) {
                (0, OWNER_RIGHTS)
            } else if sid_matches(sid, system) {
                (1, OWNER_RIGHTS)
            } else if sid_matches(sid, package) {
                (2, PACKAGE_RIGHTS)
            } else {
                return Err(WindowsNativeError::UnsafePermissions);
            };
            if seen[position] || mask != rights {
                return Err(WindowsNativeError::UnsafePermissions);
            }
            seen[position] = true;
        }
        let low = well_known_sid(WinLowLabelSid)?;
        let (mask, sid) = explicit_ace(label, 0, SYSTEM_MANDATORY_LABEL_ACE_TYPE)?;
        if !seen.into_iter().all(|value| value)
            || mask != SYSTEM_MANDATORY_LABEL_NO_WRITE_UP
            || !sid_matches(sid, low.as_ptr().cast_mut().cast())
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        Ok(())
    }
}

impl Drop for OwnedAppContainerWindowStation<'_> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn access_entry(sid: PSID, rights: u32) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: rights,
        grfAccessMode: SET_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    }
}

fn explicit_ace(acl: *mut ACL, index: u32, kind: u32) -> Result<(u32, PSID), WindowsNativeError> {
    let mut ace = null_mut();
    // SAFETY: callers validated the retained ACL; GetAce bounds the requested index.
    if unsafe { GetAce(acl, index, &mut ace) } == 0 || ace.is_null() {
        return Err(last_error("inspect fresh station ACE"));
    }
    // SAFETY: GetAce provides its header without any alignment assumption.
    let header = unsafe { std::ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
    let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
    // ACCESS_ALLOWED_ACE and SYSTEM_MANDATORY_LABEL_ACE share Header/Mask/SidStart.
    if u32::from(header.AceType) != kind
        || header.AceFlags != 0
        || usize::from(header.AceSize) < sid_offset + 8
    {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: the checked ACE contains its mask and complete fixed SID header.
    let mask = unsafe { std::ptr::read_unaligned(ace.cast::<ACCESS_ALLOWED_ACE>()) }.Mask;
    let sid_bytes = unsafe { ace.cast::<u8>().add(sid_offset) };
    let count = usize::from(unsafe { *sid_bytes.add(1) });
    if count > 15 || sid_offset + 8 + 4 * count > usize::from(header.AceSize) {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let sid = sid_bytes.cast();
    // SAFETY: the entire SID fits the retained ACE and is validated before comparison.
    if unsafe { IsValidSid(sid) } == 0 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    Ok((mask, sid))
}

fn status_error(operation: &'static str, status: u32) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::from_raw_os_error(status as i32),
    }
}
