//! Auxiliary DACL observations of fixed engine objects under the retained child token.
use crate::BoundPath;
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, HANDLE, LocalFree, WAIT_TIMEOUT},
    Security::{
        AccessCheck,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, DuplicateTokenEx, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
        GetSecurityDescriptorLength, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SecurityIdentification, TOKEN_DUPLICATE, TOKEN_QUERY, TokenImpersonation,
    },
    Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    },
    System::{
        SystemServices::MAXIMUM_ALLOWED,
        Threading::{OpenProcessToken, WaitForSingleObject},
    },
};

/// A fixed object category and its observed DACL result; never contains a path or SID.
#[derive(Debug)]
pub struct StartupAccessObservation {
    /// Dependency or bounded ancestor index, never an input name.
    pub object: &'static str,
    /// Completed or a closed native-operation category.
    pub stage: &'static str,
    /// Immediate native error, or zero when no API error occurred.
    pub os_code: u32,
    /// Actual AccessCheck mask; None means observation unavailable.
    pub granted: Option<u32>,
    /// The retained process remained live through the observation.
    pub live_bound: bool,
}

/// Read DACL decisions for chrome_elf and at most five component ancestors.
///
/// The caller retains the verified component and exact child. A query/duplicate
/// token handle is attenuated to a query-only identification token for AccessCheck;
/// it is never assigned or impersonated. No process is opened by PID and no ACL,
/// token, process, file or cleanup authority is modified. Native calls are synchronous;
/// bounds limit object count and buffers, not native call duration.
/// DACL decisions do not prove actual traversal, mandatory-label or loader-policy
/// access; this observer never loads a DLL or impersonates the browser.
#[must_use]
pub fn observe_browser_dependency_access(
    process: BorrowedHandle<'_>,
    component: &BoundPath,
) -> Vec<StartupAccessObservation> {
    let raw = process.as_raw_handle().cast();
    // SAFETY: the caller retains this exact process throughout all observations.
    let live = unsafe { WaitForSingleObject(raw, 0) } == WAIT_TIMEOUT;
    let token = if live {
        identification(raw)
    } else {
        Err(("not_live", 0))
    };
    let mut paths = vec![(
        "chrome_elf",
        component.canonical_path().join("chrome_elf.dll"),
        false,
    )];
    let labels = [
        "component",
        "ancestor1",
        "ancestor2",
        "ancestor3",
        "ancestor4",
    ];
    for (label, path) in labels
        .into_iter()
        .zip(component.canonical_path().ancestors())
    {
        paths.push((label, path.to_owned(), true));
    }
    let mut records = Vec::with_capacity(paths.len());
    for (object, path, directory) in paths {
        let result = match &token {
            Ok(token) => {
                let bound = if directory {
                    BoundPath::open_directory(&path)
                } else {
                    BoundPath::open_file(&path)
                };
                bound
                    .map_err(|_| ("bind_object", 0))
                    .and_then(|bound| access(token, &bound))
            }
            Err(error) => Err(*error),
        };
        let (stage, os_code, granted) = match result {
            Ok(granted) => ("complete", 0, Some(granted)),
            Err((stage, code)) => (stage, code, None),
        };
        records.push(StartupAccessObservation {
            object,
            stage,
            os_code,
            granted,
            live_bound: false,
        });
    }
    // SAFETY: the exact borrowed process still lives through this zero-duration query.
    let live_bound = live && unsafe { WaitForSingleObject(raw, 0) } == WAIT_TIMEOUT;
    for record in &mut records {
        record.live_bound = live_bound;
    }
    records
}

fn identification(process: HANDLE) -> Result<OwnedHandle, (&'static str, u32)> {
    let mut raw = null_mut();
    // SAFETY: exact retained child, writable output, no adjustment/assignment rights.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &mut raw) } == 0 {
        // SAFETY: capture the failed call's thread-local error immediately.
        return Err(("open_token", unsafe { GetLastError() }));
    }
    // SAFETY: successful API returned one owned token handle.
    let source = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let mut raw = null_mut();
    // SAFETY: retain source; create only a query-capable identification token.
    if unsafe {
        DuplicateTokenEx(
            source.as_raw_handle().cast(),
            TOKEN_QUERY,
            null(),
            SecurityIdentification,
            TokenImpersonation,
            &mut raw,
        )
    } == 0
    {
        // SAFETY: capture the failed call's thread-local error immediately.
        return Err(("duplicate_identification", unsafe { GetLastError() }));
    }
    // SAFETY: the sole successful duplicate is closed by OwnedHandle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw.cast()) })
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: this exact descriptor allocation came from GetSecurityInfo.
        unsafe {
            LocalFree(self.0.cast());
        }
    }
}

fn access(token: &OwnedHandle, bound: &BoundPath) -> Result<u32, (&'static str, u32)> {
    let file = bound.try_clone_file().map_err(|_| ("clone_object", 0))?;
    let mut descriptor = null_mut();
    // SAFETY: retained bound file/directory handle, output allocation owned below;
    // only owner/group/DACL metadata is requested, never SACL mutation authority.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(("read_descriptor", status));
    }
    if descriptor.is_null() {
        return Err(("empty_descriptor", 0));
    }
    let descriptor = Descriptor(descriptor);
    // SAFETY: GetSecurityInfo returned a valid descriptor retained by the guard.
    let length = unsafe { GetSecurityDescriptorLength(descriptor.0) };
    if length == 0 || length > 65_536 {
        return Err(("descriptor_bound", 0));
    }
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    };
    let mut privileges = vec![0_usize; 65_536 / size_of::<usize>()];
    let mut bytes = 65_536;
    let mut granted = 0;
    let mut allowed = 0;
    // SAFETY: valid retained descriptor and identification token, aligned bounded
    // writable privilege buffer and fixed output values. AccessCheck is observational.
    if unsafe {
        AccessCheck(
            descriptor.0,
            token.as_raw_handle().cast(),
            MAXIMUM_ALLOWED,
            &mapping,
            privileges.as_mut_ptr().cast(),
            &mut bytes,
            &mut granted,
            &mut allowed,
        )
    } == 0
    {
        // SAFETY: capture the failed call's thread-local error immediately.
        return Err(("access_check", unsafe { GetLastError() }));
    }
    if bytes > 65_536 {
        return Err(("privilege_bound", 0));
    }
    Ok(if allowed != 0 { granted } else { 0 })
}

#[test]
fn access_check_reads_real_kernel_descriptor_with_identification_token() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dependency.fixture");
    std::fs::write(&path, b"never executed").unwrap();
    let bound = BoundPath::open_file(&path).unwrap();
    // SAFETY: current-process pseudo handle is only observed, never owned or closed.
    let token =
        identification(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
            .unwrap();
    let granted = access(&token, &bound).unwrap();
    assert_eq!(granted & FILE_GENERIC_READ, FILE_GENERIC_READ);
}
