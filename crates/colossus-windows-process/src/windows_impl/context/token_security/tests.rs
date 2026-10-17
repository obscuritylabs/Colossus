//! Real kernel access decisions on unassigned disposable token objects only.
use super::*;
use windows_sys::Win32::{
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, GetAce,
        IsWellKnownSid, SECURITY_ATTRIBUTES, TokenPrimary, WinLocalSystemSid,
    },
    System::Threading::GetCurrentProcess,
};

fn own() -> OwnedHandle {
    let mut raw = null_mut();
    // SAFETY: current test process only; no modification is requested or performed.
    assert_ne!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | READ_CONTROL | WRITE_DAC,
                &mut raw,
            )
        },
        0
    );
    OwnedHandle::new(raw, "own test primary").unwrap()
}
fn acl_bytes(token: &OwnedHandle) -> Vec<u8> {
    let descriptor = descriptor(token).unwrap();
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = null_mut();
    // SAFETY: retained descriptor from GetSecurityInfo and writable metadata outputs.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    assert!(!acl.is_null());
    // SAFETY: the valid kernel ACL supplies its exact byte size.
    unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), usize::from((*acl).AclSize)) }.to_vec()
}
fn fixture_token(source: &OwnedHandle, user: &[u8]) -> OwnedHandle {
    // Deliberately give this new, unassigned token only user read/duplicate access.
    // Existing handles are retained; no process ever runs with this fixture token.
    let user = identity::sid_text(user).unwrap();
    let text = format!("D:P(A;;0x2000a;;;{user})(A;;0xf01ff;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = null_mut();
    // SAFETY: static descriptor syntax plus the kernel-derived numeric own SID.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        },
        0
    );
    let descriptor = Allocation(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut raw = null_mut();
    // SAFETY: creates an unassigned copy of the test's own primary, noninheritable.
    assert_ne!(
        unsafe {
            DuplicateTokenEx(
                source.raw(),
                TOKEN_QUERY | TOKEN_DUPLICATE | READ_CONTROL | WRITE_DAC,
                &attributes,
                SecurityIdentification,
                TokenPrimary,
                &mut raw,
            )
        },
        0
    );
    OwnedHandle::new(raw, "disposable token ACL fixture").unwrap()
}

#[test]
fn new_token_user_grant_allows_self_access_and_preserves_other_aces() {
    let source = own();
    // SAFETY: observes only this test process's current primary identity.
    let facts = identity::token(unsafe { GetCurrentProcess() }).unwrap();
    let source_before = acl_bytes(&source);
    let token = fixture_token(&source, &facts.user);
    assert!(
        !all_access(&token).unwrap(),
        "fixture must reproduce missing self access"
    );
    grant_user(&token, &facts.user).unwrap();
    assert!(
        all_access(&token).unwrap(),
        "real kernel self access must now succeed"
    );
    assert_eq!(
        acl_bytes(&source),
        source_before,
        "source primary ACL changed"
    );
    let descriptor = descriptor(&token).unwrap();
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = null_mut();
    // SAFETY: retained actual descriptor and initialized outputs.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) },
        0
    );
    // SAFETY: valid kernel ACL returned above.
    assert_eq!(unsafe { (*acl).AceCount }, 2);
    let mut users = 0;
    let mut systems = 0;
    for index in 0..2 {
        let mut raw = null_mut();
        // SAFETY: exact valid ACE index in the retained ACL.
        assert_ne!(unsafe { GetAce(acl, index, &mut raw) }, 0);
        // SAFETY: API returned a valid ACE header.
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        assert_eq!(header.AceType, 0);
        assert_eq!(header.AceFlags, 0);
        assert!(usize::from(header.AceSize) >= size_of::<ACCESS_ALLOWED_ACE>());
        // SAFETY: header proves a complete standard allow ACE with a kernel-valid SID.
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Mask, TOKEN_ALL_ACCESS);
        let sid = (&raw const ace.SidStart).cast_mut().cast();
        // SAFETY: compares two retained valid SIDs.
        if unsafe {
            windows_sys::Win32::Security::EqualSid(sid, facts.user.as_ptr().cast_mut().cast())
        } != 0
        {
            users += 1;
        }
        // SAFETY: valid SID from the retained kernel ACE.
        else if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0 {
            systems += 1;
        } else {
            panic!("grant changed another principal");
        }
    }
    assert_eq!((users, systems), (1, 1));
}

#[test]
fn ordinary_current_process_is_rejected_without_acl_mutation() {
    let token = own();
    let before = acl_bytes(&token);
    // SAFETY: current process pseudo handle is duplicated as a real owned query handle.
    let own_process = unsafe { GetCurrentProcess() };
    let raw = identity::duplicate(
        own_process,
        own_process,
        windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
        false,
    )
    .unwrap();
    let process = OwnedHandle::new(raw as usize as _, "own query test process").unwrap();
    let facts = identity::token(process.raw()).unwrap();
    assert!(
        !facts.appcontainer,
        "this fixture requires an ordinary test caller"
    );
    assert!(retain_browser_user_access(&process, &facts.user).is_err());
    assert_eq!(acl_bytes(&token), before);
}
