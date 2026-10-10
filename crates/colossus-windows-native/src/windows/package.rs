//! Exact new package ownership and explicitly acknowledged loopback retirement.
use super::*;
use std::{marker::PhantomData, rc::Rc};
use windows_sys::Win32::{
    Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
    NetworkManagement::WindowsFirewall::{
        NetworkIsolationGetAppContainerConfig, NetworkIsolationSetAppContainerConfig,
    },
    Security::{
        Authorization::ConvertSidToStringSidW,
        CopySid, FreeSid, GetLengthSid,
        Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile},
        SID_AND_ATTRIBUTES,
    },
    System::{
        SystemInformation::GetWindowsDirectoryW,
        Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};

/// Fresh native-owned package profile; an existing package is never adopted.
/// Keep this owner, the Job and WFP guard until the full process tree is drained.
/// Explicit `close` verifies loopback absence before deleting the exact package.
/// Drop is only best effort and is never a cleanup acknowledgement.
pub struct ExclusiveAppContainerProfile {
    name: Vec<u16>,
    package: PSID,
    loopback_attempted: bool,
    closed: bool,
    _thread: PhantomData<Rc<()>>,
}
impl ExclusiveAppContainerProfile {
    /// Derive a filesystem principal from the exact retained spawned process and
    /// require its kernel package SID to equal this exclusive enrollment package.
    /// Child PID numbers or child self-reports cannot replace the borrowed handle.
    pub fn verify_process(
        &self,
        process: std::os::windows::io::BorrowedHandle<'_>,
    ) -> Result<AppContainerPrincipal, WindowsNativeError> {
        if self.closed {
            return Err(WindowsNativeError::IdentityChanged);
        }
        let principal = AppContainerPrincipal::of_process(process)?;
        if self.package.is_null()
            || unsafe { IsValidSid(self.package) } == 0
            || !sid_matches(self.package, principal.package_sid())
        {
            return Err(WindowsNativeError::IdentityChanged);
        }
        Ok(principal)
    }
    /// Create a nonce-derived profile exclusively. Names have a fixed browser
    /// prefix and exactly 128 bits of lowercase hex, with no OS-selected fallback.
    pub fn create(name: &str) -> Result<Self, WindowsNativeError> {
        let nonce = name
            .strip_prefix("colossus.browser.")
            .ok_or(WindowsNativeError::InvalidInput)?;
        if nonce.len() != 32
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(WindowsNativeError::InvalidInput);
        }
        let name = wide(name);
        let display = wide("Colossus browser");
        let description = wide("Dedicated supervised browser host");
        let mut package = null_mut();
        // SAFETY: fixed bounded terminated strings and output live for the call;
        // no capabilities are granted. Successful allocation is retained directly.
        let status = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                display.as_ptr(),
                description.as_ptr(),
                null(),
                0,
                &mut package,
            )
        };
        if status < 0 {
            return Err(hresult_error("create exclusive browser package", status));
        }
        Ok(Self {
            name,
            package,
            loopback_attempted: false,
            closed: false,
            _thread: PhantomData,
        })
    }

    /// Exact package SID derived by Windows from this newly created profile.
    pub fn sid_string(&self) -> Result<String, WindowsNativeError> {
        if self.closed || self.package.is_null() || unsafe { IsValidSid(self.package) } == 0 {
            return Err(WindowsNativeError::IdentityChanged);
        }
        let mut text = null_mut();
        // SAFETY: the retained OS SID is valid; output is LocalAlloc-owned.
        if unsafe { ConvertSidToStringSidW(self.package, &mut text) } == 0 {
            return Err(last_error("encode enrolled package SID"));
        }
        let mut length = 0;
        // SAFETY: ConvertSidToStringSidW returns a terminated SID string; cap is
        // much larger than the documented maximum string SID representation.
        unsafe {
            while length < 256 && *text.add(length) != 0 {
                length += 1;
            }
        }
        let value = if length < 256 {
            // SAFETY: bounded characters lie in the live returned string.
            String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
                .map_err(|_| WindowsNativeError::InvalidInput)
        } else {
            Err(WindowsNativeError::InvalidInput)
        };
        // SAFETY: the conversion allocated this exact pointer with LocalAlloc.
        unsafe {
            LocalFree(text.cast());
        }
        value
    }

    /// Provision the sole exact-package loopback exception before spawning.
    /// NetworkIsolation replaces a global list and has no atomic per-SID API.
    /// This uses an OS-global mutex for cooperating Colossus supervisors, retains
    /// all unrelated entries and verifies the committed list. Foreign administrator
    /// writers do not honor that mutex; publication still requires a broker and
    /// crash/concurrency acceptance. A loopback exception alone is not containment.
    pub fn enable_loopback(&mut self) -> Result<(), WindowsNativeError> {
        if self.closed || self.loopback_attempted {
            return Err(WindowsNativeError::InvalidInput);
        }
        self.loopback_attempted = true;
        change_loopback(self.package, true)
    }

    /// Positively verify exact-package loopback absence, then delete the owned
    /// package. An uncertain OS mutation remains retryable through this same owner.
    pub fn close(&mut self) -> Result<(), WindowsNativeError> {
        if self.closed {
            return Ok(());
        }
        if self.loopback_attempted {
            change_loopback(self.package, false)?;
            self.loopback_attempted = false;
        }
        // SAFETY: this exact name was exclusively created and remains retained.
        let status = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
        if status < 0 {
            return Err(hresult_error("delete owned browser package", status));
        }
        self.closed = true;
        Ok(())
    }
}
impl Drop for ExclusiveAppContainerProfile {
    fn drop(&mut self) {
        let _ = self.close();
        if !self.package.is_null() {
            // SAFETY: CreateAppContainerProfile transfers a FreeSid-owned SID.
            unsafe {
                FreeSid(self.package);
            }
        }
    }
}

/// Obtain the Windows directory from the kernel API without ambient environment.
pub fn system_windows_directory() -> Result<PathBuf, WindowsNativeError> {
    let mut value = vec![0u16; 32768];
    // SAFETY: bounded writable UTF-16 buffer, documented directory API.
    let length = unsafe { GetWindowsDirectoryW(value.as_mut_ptr(), value.len() as u32) };
    if length == 0 || length as usize >= value.len() {
        return Err(last_error("query system Windows directory"));
    }
    let directory = PathBuf::from(std::ffi::OsString::from_wide(&value[..length as usize]));
    let bound = open_bound(&directory, BoundKind::Directory)?;
    bound.validate_namespace_authority()?;
    bound.revalidate()?;
    Ok(bound.canonical_path)
}

struct GlobalConfigurationGuard(OwnedHandle, PhantomData<Rc<()>>);
impl GlobalConfigurationGuard {
    fn acquire() -> Result<Self, WindowsNativeError> {
        let name = wide("Global\\ColossusBrowserNetworkIsolation-v1");
        let handle = with_private_security_attributes(
            PRIVATE_DIRECTORY_LABELS,
            NO_INHERITANCE,
            |attributes| {
                // SAFETY: private noninheritable descriptor and fixed terminated name.
                let handle = unsafe { CreateMutexW(attributes, 0, name.as_ptr()) };
                if handle.is_null() {
                    Err(last_error("create browser NetworkIsolation gate"))
                } else {
                    Ok(handle)
                }
            },
        )?;
        // SAFETY: successful CreateMutexW transferred one owned handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
        // SAFETY: the mutex stays retained for this bounded wait.
        if !matches!(
            unsafe { WaitForSingleObject(handle.as_raw_handle().cast(), 5000) },
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            return Err(last_error("wait for browser NetworkIsolation gate"));
        }
        Ok(Self(handle, PhantomData))
    }
}
impl Drop for GlobalConfigurationGuard {
    fn drop(&mut self) {
        // SAFETY: only the acquiring thread owns/drops this guard.
        unsafe {
            ReleaseMutex(self.0.as_raw_handle().cast());
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct ConfigSid {
    sid: Box<[u8; SECURITY_MAX_SID_SIZE as usize]>,
    attributes: u32,
}
fn configuration() -> Result<Vec<ConfigSid>, WindowsNativeError> {
    let mut count = 0;
    let mut array = null_mut();
    // SAFETY: outputs receive one LocalAlloc-owned SID_AND_ATTRIBUTES array.
    let status = unsafe { NetworkIsolationGetAppContainerConfig(&mut count, &mut array) };
    if status != 0 {
        return Err(win32_error(
            "read browser NetworkIsolation configuration",
            status,
        ));
    }
    struct Allocation(*mut SID_AND_ATTRIBUTES);
    impl Drop for Allocation {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: NetworkIsolationGetAppContainerConfig transfers this allocation.
                unsafe {
                    LocalFree(self.0.cast());
                }
            }
        }
    }
    let allocation = Allocation(array);
    if count > 4096 || (count != 0 && array.is_null()) {
        return Err(WindowsNativeError::InvalidInput);
    }
    let entries = if count == 0 {
        &[][..]
    } else {
        // SAFETY: successful API output has exactly count entries, bounded above.
        unsafe { std::slice::from_raw_parts(allocation.0, count as usize) }
    };
    let mut result = Vec::with_capacity(entries.len());
    for item in entries {
        if item.Sid.is_null() || unsafe { IsValidSid(item.Sid) } == 0 {
            return Err(WindowsNativeError::UnsafePermissions);
        }
        // SAFETY: the returned SID was validated and allocation stays retained.
        let length = unsafe { GetLengthSid(item.Sid) };
        if length == 0 || length > SECURITY_MAX_SID_SIZE {
            return Err(WindowsNativeError::InvalidInput);
        }
        let mut sid = Box::new([0; SECURITY_MAX_SID_SIZE as usize]);
        // SAFETY: destination holds maximum SID bytes; source stays valid.
        if unsafe { CopySid(SECURITY_MAX_SID_SIZE, sid.as_mut_ptr().cast(), item.Sid) } == 0 {
            return Err(last_error("retain browser NetworkIsolation entry"));
        }
        result.push(ConfigSid {
            sid,
            attributes: item.Attributes,
        });
    }
    Ok(result)
}
fn change_loopback(package: PSID, add: bool) -> Result<(), WindowsNativeError> {
    if package.is_null() || unsafe { IsValidSid(package) } == 0 {
        return Err(WindowsNativeError::InvalidInput);
    }
    let _guard = GlobalConfigurationGuard::acquire()?;
    let original = configuration()?;
    let matches = |item: &ConfigSid| sid_matches(item.sid.as_ptr().cast_mut().cast(), package);
    if add && original.iter().any(matches) {
        return Err(WindowsNativeError::UnsafePermissions);
    }
    if !add && !original.iter().any(matches) {
        return Ok(());
    }
    let mut expected: Vec<_> = original.into_iter().filter(|item| !matches(item)).collect();
    if add {
        let mut copied = Box::new([0; SECURITY_MAX_SID_SIZE as usize]);
        // SAFETY: valid retained package SID and maximum-size destination.
        if unsafe { CopySid(SECURITY_MAX_SID_SIZE, copied.as_mut_ptr().cast(), package) } == 0 {
            return Err(last_error("retain enrolled loopback SID"));
        }
        expected.push(ConfigSid {
            sid: copied,
            attributes: 0,
        });
    }
    let array: Vec<_> = expected
        .iter()
        .map(|item| SID_AND_ATTRIBUTES {
            Sid: item.sid.as_ptr().cast_mut().cast(),
            Attributes: item.attributes,
        })
        .collect();
    // SAFETY: entries and their copied SID storage remain live for the entire call.
    let status =
        unsafe { NetworkIsolationSetAppContainerConfig(array.len() as u32, array.as_ptr()) };
    if status != 0 {
        return Err(win32_error(
            "write browser NetworkIsolation configuration",
            status,
        ));
    }
    let mut actual = configuration()?;
    expected.sort_by(|a, b| {
        a.sid
            .as_slice()
            .cmp(b.sid.as_slice())
            .then(a.attributes.cmp(&b.attributes))
    });
    actual.sort_by(|a, b| {
        a.sid
            .as_slice()
            .cmp(b.sid.as_slice())
            .then(a.attributes.cmp(&b.attributes))
    });
    if actual != expected {
        return Err(WindowsNativeError::IdentityChanged);
    }
    Ok(())
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn win32_error(operation: &'static str, error: u32) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::from_raw_os_error(error as i32),
    }
}
fn hresult_error(operation: &'static str, status: i32) -> WindowsNativeError {
    win32_error(operation, status as u32)
}
