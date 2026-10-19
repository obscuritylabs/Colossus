//! Kernel-logon scope and exact private mutex security. No ambient SID strings.
use super::{WindowsNativeError, failed, last_error, wide};
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, LocalFree, NO_ERROR},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        },
        CopySid, CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid,
        GetSecurityDescriptorControl, GetTokenInformation, IsValidAcl, IsValidSid,
        OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        SECURITY_MAX_SID_SIZE, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER, TokenStatistics,
        TokenUser, WinLocalSystemSid,
    },
    System::{
        SystemServices::ACCESS_ALLOWED_ACE_TYPE,
        Threading::{CreateMutexW, GetCurrentProcess, MUTEX_ALL_ACCESS, OpenProcessToken},
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Principal {
    pub logon: u64,
    user: Vec<u8>,
}
impl Principal {
    pub fn current() -> Result<Self, WindowsNativeError> {
        // SAFETY: GetCurrentProcess returns a non-owned pseudo handle.
        Self::of(unsafe { GetCurrentProcess() })
    }
    pub fn of(process: HANDLE) -> Result<Self, WindowsNativeError> {
        let mut token = std::ptr::null_mut();
        // SAFETY: query-only process token, returned handle is owned below.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(last_error("instance token"));
        }
        // SAFETY: successful API returned one owned real token handle.
        let token = unsafe { OwnedHandle::from_raw_handle(token.cast()) };
        let mut required = 0;
        // SAFETY: query the bounded TOKEN_USER allocation size.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut required,
            )
        };
        if required < size_of::<TOKEN_USER>() as u32 || required > 8192 {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let mut storage = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
        // SAFETY: word-aligned buffer covers exactly the bounded requested size.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenUser,
                storage.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(last_error("instance token user"));
        }
        // SAFETY: native TOKEN_USER is aligned within the retained result buffer.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let user = copy_sid(user.User.Sid)?;
        let mut statistics = TOKEN_STATISTICS::default();
        // SAFETY: fixed writable TOKEN_STATISTICS and retained query-only token.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenStatistics,
                (&mut statistics as *mut TOKEN_STATISTICS).cast(),
                size_of::<TOKEN_STATISTICS>() as u32,
                &mut required,
            )
        } == 0
            || required != size_of::<TOKEN_STATISTICS>() as u32
        {
            return Err(last_error("instance token logon"));
        }
        Ok(Self {
            user,
            logon: (u64::from(statistics.AuthenticationId.HighPart as u32) << 32)
                | u64::from(statistics.AuthenticationId.LowPart),
        })
    }
    fn sid(&self) -> PSID {
        self.user.as_ptr().cast_mut().cast()
    }
}

struct LocalMemory(*mut core::ffi::c_void);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: only LocalAlloc-backed Win32 security/string results enter this owner.
            unsafe { LocalFree(self.0) };
        }
    }
}
fn copy_sid(sid: PSID) -> Result<Vec<u8>, WindowsNativeError> {
    // SAFETY: callers provide a SID in a live kernel token/security result.
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    // SAFETY: previous API proved a valid SID.
    let length = unsafe { GetLengthSid(sid) };
    if length == 0 || length > SECURITY_MAX_SID_SIZE {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    let mut copied = vec![0u8; length as usize];
    // SAFETY: bounded self-contained destination remains live through the call.
    if unsafe { CopySid(length, copied.as_mut_ptr().cast(), sid) } == 0 {
        return Err(last_error("instance SID copy"));
    }
    Ok(copied)
}
fn system_sid() -> Result<Vec<u8>, WindowsNativeError> {
    let mut storage = [0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut length = SECURITY_MAX_SID_SIZE;
    // SAFETY: fixed maximum-size destination and built-in SYSTEM SID, no domain input.
    if unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            std::ptr::null_mut(),
            storage.as_mut_ptr().cast(),
            &mut length,
        )
    } == 0
    {
        return Err(last_error("instance SYSTEM SID"));
    }
    Ok(storage[..length as usize].to_vec())
}

pub(super) struct MutexHandle(OwnedHandle);
impl MutexHandle {
    pub fn raw(&self) -> HANDLE {
        self.0.as_raw_handle().cast()
    }
    pub fn create(name: &str, principal: &Principal) -> Result<Self, WindowsNativeError> {
        let mut sid_text = std::ptr::null_mut();
        // SAFETY: retained copied kernel SID; successful output is LocalAlloc-backed.
        if unsafe { ConvertSidToStringSidW(principal.sid(), &mut sid_text) } == 0 {
            return Err(last_error("instance SID text"));
        }
        let _sid_text = LocalMemory(sid_text.cast());
        let mut len = 0;
        // SAFETY: ConvertSidToStringSid always terminates its bounded SID result.
        while len < 256 && unsafe { *sid_text.add(len) } != 0 {
            len += 1;
        }
        if len == 256 {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        // SAFETY: length was found inside the retained native UTF-16 string.
        let sid_text = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, len) })
            .map_err(|_| WindowsNativeError::UnsafePermissions)?;
        // Explicit owner, protected DACL, exact mutex rights for user and SYSTEM only.
        let sddl = wide(&format!(
            "O:{sid_text}D:P(A;;0x001f0001;;;SY)(A;;0x001f0001;;;{sid_text})"
        ));
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: native-produced SID text and fixed SDDL structure, no external strings.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error("instance security descriptor"));
        }
        let _descriptor = LocalMemory(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name = wide(name);
        // SAFETY: descriptor/name retained through creation; handle is explicitly non-inherited.
        let mutex = unsafe { CreateMutexW(&attributes, 0, name.as_ptr()) };
        if mutex.is_null() {
            return Err(last_error("instance mutex"));
        }
        // SAFETY: CreateMutex returned one owned handle, whether new or existing.
        let mutex = Self(unsafe { OwnedHandle::from_raw_handle(mutex.cast()) });
        // CreateMutex ignores a supplied descriptor on reuse. Check actual kernel
        // owner/DACL before relying on either a fresh or a preexisting named object.
        mutex.validate(principal)?;
        Ok(mutex)
    }
    fn validate(&self, principal: &Principal) -> Result<(), WindowsNativeError> {
        let mut owner = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: owned mutex has READ_CONTROL; returned descriptor owns all borrowed pointers.
        let status = unsafe {
            GetSecurityInfo(
                self.raw(),
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != NO_ERROR {
            return Err(WindowsNativeError::Io {
                operation: "instance mutex security",
                source: std::io::Error::from_raw_os_error(status as i32),
            });
        }
        let _descriptor = LocalMemory(descriptor);
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: queried descriptor and DACL remain retained; exact owner is kernel current user.
        if owner.is_null()
            || dacl.is_null()
            || unsafe { EqualSid(owner, principal.sid()) } == 0
            || unsafe { IsValidAcl(dacl) } == 0
            || unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
            || control & SE_DACL_PROTECTED == 0
            || unsafe { (*dacl).AceCount } != 2
        {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        let system = system_sid()?;
        let mut seen = [false; 2];
        for index in 0..2 {
            let mut raw = std::ptr::null_mut();
            // SAFETY: IsValidAcl and exact ACE count bound this index.
            if unsafe { GetAce(dacl, index, &mut raw) } == 0 || raw.is_null() {
                return Err(last_error("instance mutex ACE"));
            }
            // SAFETY: successful GetAce always returns at least ACE_HEADER.
            let header = unsafe { &*raw.cast::<ACE_HEADER>() };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
                || header.AceFlags != 0
                || header.AceSize < 16
            {
                return Err(WindowsNativeError::UnsafePermissions);
            }
            // SAFETY: fixed access-allowed header, mask and minimum SID header fit AceSize.
            let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
            let sid: PSID = (&ace.SidStart as *const u32).cast_mut().cast();
            // SAFETY: the first two SID bytes fit the minimum ACE size above.
            let sid_length = 8 + 4 * usize::from(unsafe { *sid.cast::<u8>().add(1) });
            if sid_length + 8 > usize::from(header.AceSize)
                || ace.Mask != MUTEX_ALL_ACCESS
                || unsafe { IsValidSid(sid) } == 0
            {
                return Err(WindowsNativeError::UnsafePermissions);
            }
            // SAFETY: both candidate principals and the bounded ACE SID remain live.
            let slot = if unsafe { EqualSid(sid, principal.sid()) } != 0 {
                0
            } else if unsafe { EqualSid(sid, system.as_ptr().cast_mut().cast()) } != 0 {
                1
            } else {
                return Err(WindowsNativeError::UnsafePermissions);
            };
            if seen[slot] {
                return Err(WindowsNativeError::UnsafePermissions);
            }
            seen[slot] = true;
        }
        if !seen.into_iter().all(|seen| seen) {
            return Err(failed(
                "instance private ACL",
                std::io::ErrorKind::PermissionDenied,
            ));
        }
        Ok(())
    }
}
