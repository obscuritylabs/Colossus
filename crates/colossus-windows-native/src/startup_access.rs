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
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, DuplicateTokenEx, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION,
        GetSecurityDescriptorLength, GetTokenInformation, LookupPrivilegeValueW,
        OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, PSECURITY_DESCRIPTOR, PrivilegeCheck,
        SE_PRIVILEGE_ENABLED, SecurityIdentification, TOKEN_DUPLICATE, TOKEN_GROUPS, TOKEN_QUERY,
        TOKEN_USER, TokenImpersonation, TokenRestrictedSids, TokenUser,
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
    /// Actual descriptor, hypothetical in-memory grant, or categorical token fact.
    pub policy: &'static str,
    /// Closed ACL categories only; never raw descriptor text or identities.
    pub detail: Option<String>,
    /// Completed or a closed native-operation category.
    pub stage: &'static str,
    /// Immediate native error, or zero when no API error occurred.
    pub os_code: u32,
    /// AccessCheck mask or categorical token value; None means observation unavailable.
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
/// Hypothetical ACL grants modify only disposable descriptors in memory.
/// DACL decisions do not prove actual traversal, mandatory-label or loader-policy
/// access; this observer never loads a DLL or impersonates the browser.
#[must_use]
pub fn observe_browser_dependency_access(
    process: BorrowedHandle<'_>,
    component: &BoundPath,
    package_sid: &str,
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
    let mut records = Vec::with_capacity(32);
    if let Ok(token) = &token {
        for (object, result) in token_facts(token) {
            records.push(record(object, "token_fact", result));
        }
    }
    let valid_package = package_sid.starts_with("S-1-15-2-")
        && package_sid.len() <= 184
        && package_sid
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-' || b == b'S');
    for (object, path, directory) in paths {
        let result = match &token {
            Ok(token) => {
                let bound = if directory {
                    BoundPath::open_directory(&path)
                } else {
                    BoundPath::open_file(&path)
                };
                bound.map_err(|_| ("bind_object", 0)).and_then(|bound| {
                    let descriptor = descriptor(&bound)?;
                    if let Ok(user) = token_user_text(token) {
                        for (policy, addition) in [
                            (
                                "virtual_user_aap_rx",
                                format!("(A;;0x1200a9;;;{user})(A;;0x1200a9;;;AC)"),
                            ),
                            (
                                "virtual_user_package_rx",
                                format!("(A;;0x1200a9;;;{user})(A;;0x1200a9;;;{package_sid})"),
                            ),
                        ] {
                            if valid_package {
                                records.push(record(
                                    object,
                                    policy,
                                    virtual_access(token, &descriptor, &addition),
                                ));
                            }
                        }
                    }

                    if let Ok(sddl) = descriptor_text(&descriptor) {
                        let mut summary = record(object, "descriptor_categories", Ok(0));
                        summary.detail = Some(descriptor_categories(&sddl));
                        records.push(summary);
                    }
                    records.push(record(
                        object,
                        "explicit_rx_actual",
                        check_desired(token, &descriptor, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE),
                    ));
                    records.push(record(
                        object,
                        "explicit_rx_aap",
                        virtual_access_desired(
                            token,
                            &descriptor,
                            "(A;;0x1200a9;;;AC)",
                            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                        ),
                    ));
                    if valid_package {
                        for (policy, addition) in [
                            ("virtual_aap_rx", "(A;;0x1200a9;;;AC)".to_owned()),
                            (
                                "virtual_package_rx",
                                format!("(A;;0x1200a9;;;{package_sid})"),
                            ),
                            (
                                "virtual_world_aap_rx",
                                "(A;;0x1200a9;;;WD)(A;;0x1200a9;;;AC)".to_owned(),
                            ),
                        ] {
                            records.push(record(
                                object,
                                policy,
                                virtual_access(token, &descriptor, &addition),
                            ));
                        }
                    }
                    check(token, &descriptor)
                })
            }
            Err(error) => Err(*error),
        };
        records.push(record(object, "actual", result));
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

fn descriptor(bound: &BoundPath) -> Result<Descriptor, (&'static str, u32)> {
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
    Ok(descriptor)
}

fn check(token: &OwnedHandle, descriptor: &Descriptor) -> Result<u32, (&'static str, u32)> {
    check_desired(token, descriptor, MAXIMUM_ALLOWED)
}

fn check_desired(token: &OwnedHandle, descriptor: &Descriptor, desired: u32) -> ObservationResult {
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
            desired,
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
    let granted = check(&token, &descriptor(&bound).unwrap()).unwrap();
    assert_eq!(granted & FILE_GENERIC_READ, FILE_GENERIC_READ);
}

type ObservationResult = Result<u32, (&'static str, u32)>;

fn record(
    object: &'static str,
    policy: &'static str,
    result: ObservationResult,
) -> StartupAccessObservation {
    let (stage, os_code, granted) = match result {
        Ok(granted) => ("complete", 0, Some(granted)),
        Err((stage, code)) => (stage, code, None),
    };
    StartupAccessObservation {
        object,
        policy,
        detail: None,
        stage,
        os_code,
        granted,
        live_bound: false,
    }
}

// Both descriptors are disposable memory only. There is deliberately no SetSecurityInfo,
// SetFileSecurity, token adjustment, impersonation or object write in this observer.
fn virtual_access(
    token: &OwnedHandle,
    descriptor: &Descriptor,
    addition: &str,
) -> ObservationResult {
    virtual_access_desired(token, descriptor, addition, MAXIMUM_ALLOWED)
}

fn descriptor_text(descriptor: &Descriptor) -> Result<String, (&'static str, u32)> {
    let mut text = null_mut();
    let mut chars = 0;
    // SAFETY: retained descriptor; returned string is LocalAlloc-owned.
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            1,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut text,
            &mut chars,
        )
    } == 0
    {
        return Err(("descriptor_encode", unsafe { GetLastError() }));
    }
    let allocation = Descriptor(text.cast());
    if text.is_null() || chars == 0 || chars > 65_536 {
        return Err(("text_bound", 0));
    }
    // SAFETY: the API returned this many UTF-16 code units including the terminator.
    let units = unsafe { std::slice::from_raw_parts(text, chars as usize) };
    if units.last() != Some(&0) {
        return Err(("text_terminator", 0));
    }
    let text = String::from_utf16(&units[..units.len() - 1]).map_err(|_| ("text_encoding", 0))?;
    drop(allocation);
    Ok(text)
}

fn virtual_access_desired(
    token: &OwnedHandle,
    descriptor: &Descriptor,
    addition: &str,
    desired: u32,
) -> ObservationResult {
    let text = descriptor_text(descriptor)?;
    if !text.contains("D:") || text.contains("S:") {
        return Err(("descriptor_shape", 0));
    }
    let amended = format!("{text}{addition}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut raw = null_mut();
    let mut bytes = 0;
    // SAFETY: bounded terminated in-memory descriptor; no object handle or write API.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            amended.as_ptr(),
            1,
            &mut raw,
            &mut bytes,
        )
    } == 0
    {
        return Err(("descriptor_decode", unsafe { GetLastError() }));
    }
    let amended = Descriptor(raw);
    if raw.is_null() || bytes == 0 || bytes > 65_536 {
        return Err(("virtual_descriptor_bound", 0));
    }
    check_desired(token, &amended, desired)
}

fn token_facts(token: &OwnedHandle) -> Vec<(&'static str, ObservationResult)> {
    let mut facts = Vec::with_capacity(3);
    for (name, class, minimum) in [
        ("user_attributes", TokenUser, size_of::<TOKEN_USER>()),
        ("restricted_count", TokenRestrictedSids, size_of::<u32>()),
    ] {
        let mut data = vec![0_usize; 65_536 / size_of::<usize>()];
        let mut returned = 0;
        // SAFETY: retained query-only token, aligned bounded output; no SID pointer is dereferenced.
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                class,
                data.as_mut_ptr().cast(),
                65_536,
                &mut returned,
            )
        };
        let result = if ok == 0 {
            Err(("query_token_fact", unsafe { GetLastError() }))
        } else if (returned as usize) < minimum || returned > 65_536 {
            Err(("token_fact_bound", 0))
        } else {
            // SAFETY: the fixed leading structure is within the validated aligned output.
            Ok(unsafe {
                if class == TokenUser {
                    (*data.as_ptr().cast::<TOKEN_USER>()).User.Attributes
                } else {
                    (*data.as_ptr().cast::<TOKEN_GROUPS>()).GroupCount
                }
            })
        };
        facts.push((name, result));
    }
    let mut required = PRIVILEGE_SET {
        PrivilegeCount: 1,
        Control: 1,
        ..Default::default()
    };
    required.Privilege[0].Attributes = SE_PRIVILEGE_ENABLED;
    let name = "SeChangeNotifyPrivilege"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut enabled = 0;
    // SAFETY: fixed privilege name and writable LUID; reads system privilege identity only.
    let result =
        if unsafe { LookupPrivilegeValueW(null(), name.as_ptr(), &mut required.Privilege[0].Luid) }
            == 0
        {
            Err(("lookup_traverse", unsafe { GetLastError() }))
        // SAFETY: one-entry privilege set and retained query-only identification token.
        } else if unsafe {
            PrivilegeCheck(token.as_raw_handle().cast(), &mut required, &mut enabled)
        } == 0
        {
            Err(("query_traverse", unsafe { GetLastError() }))
        } else {
            Ok(u32::from(enabled != 0))
        };
    facts.push(("traverse_enabled", result));
    facts
}

#[test]
fn virtual_acl_comparison_preserves_real_object_and_observes_kernel_decision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dependency.fixture");
    std::fs::write(&path, b"never executed").unwrap();
    let bound = BoundPath::open_file(&path).unwrap();
    // SAFETY: observe only this process's pseudo handle.
    let token =
        identification(unsafe { windows_sys::Win32::System::Threading::GetCurrentProcess() })
            .unwrap();
    let before = descriptor(&bound).unwrap();
    let actual = check(&token, &before).unwrap();
    let hypothetical = virtual_access(&token, &before, "(A;;0x1200a9;;;AC)").unwrap();
    assert_eq!(hypothetical & FILE_GENERIC_READ, FILE_GENERIC_READ);
    assert_eq!(check(&token, &descriptor(&bound).unwrap()).unwrap(), actual);
    assert!(token_user_text(&token).unwrap().starts_with("S-1-"));
    assert!(
        token_facts(&token)
            .into_iter()
            .all(|(_, result)| result.is_ok())
    );
}

fn descriptor_categories(text: &str) -> String {
    let mut entries = Vec::new();
    for entry in text.split('(').skip(1).take(16) {
        let Some(end) = entry.find(')') else {
            entries.push("unsupported".into());
            break;
        };
        let fields = entry[..end].split(';').collect::<Vec<_>>();
        if fields.len() != 6 {
            entries.push("unsupported".into());
            continue;
        }
        let kind = match fields[0] {
            "A" => "allow",
            "D" => "deny",
            _ => "other",
        };
        let flags =
            if fields[1].len() <= 16 && fields[1].bytes().all(|b| b"CIOPNHIDSFA".contains(&b)) {
                fields[1]
            } else {
                "other"
            };
        let rights = match fields[2] {
            "FA" => FILE_ALL_ACCESS,
            "FR" => FILE_GENERIC_READ,
            "FX" => FILE_GENERIC_EXECUTE,
            "FRFX" => FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            value => value
                .strip_prefix("0x")
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .unwrap_or(0),
        };
        let principal = match fields[5] {
            "SY" => "system",
            "BA" => "administrators",
            "BU" => "users",
            "WD" => "world",
            "AC" | "S-1-15-2-1" => "all_packages",
            "S-1-15-2-2" => "restricted_packages",
            "OW" => "owner_rights",
            "CO" => "creator_owner",
            "AN" => "anonymous",
            "RC" => "restricted_code",
            "AU" => "authenticated_users",
            _ => "other_identity",
        };
        entries.push(format!(
            "{kind}:{principal}:flags={flags}:mask={rights:08x}"
        ));
    }
    entries.join(",")
}

#[test]
fn descriptor_categories_never_include_raw_account_or_package_identity() {
    let text = "O:S-1-5-21-111-222-333-1001G:BAD:AI(A;ID;FA;;;S-1-5-21-111-222-333-1001)(D;;FR;;;AC)(A;;0x1200a9;;;S-1-15-2-2)";
    assert_eq!(
        descriptor_categories(text),
        "allow:other_identity:flags=ID:mask=001f01ff,deny:all_packages:flags=:mask=00120089,allow:restricted_packages:flags=:mask=001200a9"
    );
}

fn token_user_text(token: &OwnedHandle) -> Result<String, (&'static str, u32)> {
    let mut data = vec![0_usize; 65_536 / size_of::<usize>()];
    let mut returned = 0;
    // SAFETY: retained query-only token and aligned bounded initialized output.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            data.as_mut_ptr().cast(),
            65_536,
            &mut returned,
        )
    } == 0
    {
        return Err(("query_user", unsafe { GetLastError() }));
    }
    if (returned as usize) < size_of::<TOKEN_USER>() || returned > 65_536 {
        return Err(("user_buffer", 0));
    }
    // SAFETY: fixed TOKEN_USER lies within the validated aligned allocation.
    let sid = unsafe { (*data.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let start = data.as_ptr() as usize;
    let at = sid as usize;
    if at < start
        || at
            .checked_add(8)
            .is_none_or(|end| end > start + returned as usize)
    {
        return Err(("user_sid_header", 0));
    }
    // SAFETY: the SID header is within the validated token output.
    let count = unsafe { *sid.cast::<u8>().add(1) } as usize;
    let bytes = 8 + count * 4;
    if bytes > 68 || at + bytes > start + returned as usize {
        return Err(("user_sid_bound", 0));
    }
    // SAFETY: the complete SID is retained in the validated token output.
    if unsafe { windows_sys::Win32::Security::IsValidSid(sid) } == 0 {
        return Err(("invalid_user_sid", 0));
    }
    let mut text = null_mut();
    // SAFETY: validated retained SID; output is allocated by Windows.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(("encode_user_sid", unsafe { GetLastError() }));
    }
    let allocation = Descriptor(text.cast());
    let mut length = 0;
    // SAFETY: this API supplies a terminated SID string; numeric SID text is bounded.
    while length < 184 && unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    if length == 184 {
        return Err(("user_text_bound", 0));
    }
    // SAFETY: measured characters are within the retained allocation.
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|_| ("user_text_encoding", 0));
    drop(allocation);
    result
}
