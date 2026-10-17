//! Real kernel token attenuation; fresh NETONLY and full creation belong to live fixtures.
use super::*;
use windows_sys::Win32::{Foundation::LUID, Security::TOKEN_STATISTICS};

fn kernel_elevation(token: HANDLE) -> u32 {
    let mut value = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: retained token and the documented fixed TokenElevation output.
    assert!(
        unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&raw mut value).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        } != 0,
        "kernel TokenElevation query failed"
    );
    assert_eq!(returned as usize, size_of::<TOKEN_ELEVATION>());
    assert!(value.TokenIsElevated <= 1, "kernel elevation flag invalid");
    value.TokenIsElevated
}
fn kernel_authentication(token: HANDLE) -> LUID {
    let mut value = TOKEN_STATISTICS::default();
    let mut returned = 0;
    // SAFETY: retained token and the documented fixed TokenStatistics output.
    assert!(
        unsafe {
            GetTokenInformation(
                token,
                TokenStatistics,
                (&raw mut value).cast(),
                size_of::<TOKEN_STATISTICS>() as u32,
                &mut returned,
            )
        } != 0,
        "kernel TokenStatistics query failed"
    );
    assert_eq!(returned as usize, size_of::<TOKEN_STATISTICS>());
    value.AuthenticationId
}

#[test]
fn browser_primary_restriction_preserves_logon_identity_without_changing_caller() {
    let mut raw = null_mut();
    // SAFETY: opens only the libtest process's actual primary for independent observation.
    assert!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } != 0,
        "own primary query open failed"
    );
    let own = OwnedHandle::new(raw, "OpenProcessToken(test primary)").expect("own token custody");
    let before = token_handle(own.raw()).expect("own kernel token facts");
    let elevated = kernel_elevation(own.raw());
    let authentication = kernel_authentication(own.raw());
    let restricted = restricted_browser_primary().expect("own browser primary attenuation failed");
    if elevated == 0 {
        assert!(
            restricted.is_none(),
            "ordinary token must retain CreateProcessW path"
        );
    } else {
        let restricted = restricted.expect("elevated caller must produce a restricted primary");
        assert_eq!(
            kernel_elevation(restricted.raw()),
            0,
            "restricted token remains elevated"
        );
        let actual_authentication = kernel_authentication(restricted.raw());
        assert!(
            actual_authentication.LowPart == authentication.LowPart
                && actual_authentication.HighPart == authentication.HighPart,
            "restricted token changed kernel AuthenticationId"
        );
        let actual = token_handle(restricted.raw()).expect("restricted kernel token facts");
        assert_user_system_only(&object_dacl(restricted.raw()), &before.user);
        assert!(
            same_sid(&actual.user, &before.user),
            "restricted token changed user"
        );
        assert!(
            actual.session == before.session,
            "restricted token changed session"
        );
        assert!(!actual.appcontainer, "primary is already an AppContainer");
    }
    let mut raw = null_mut();
    // SAFETY: freshly reopens the CURRENT primary; the retained before-handle would
    // not detect replacement of the caller's primary token by the constructor.
    assert!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } != 0,
        "current primary query open failed"
    );
    let current = OwnedHandle::new(raw, "OpenProcessToken(test current primary)")
        .expect("current token custody");
    let current_facts = token_handle(current.raw()).expect("current kernel token facts");
    assert!(
        same_sid(&current_facts.user, &before.user),
        "constructor changed caller user"
    );
    assert!(
        current_facts.session == before.session,
        "constructor changed caller session"
    );
    assert_eq!(
        kernel_elevation(current.raw()),
        elevated,
        "constructor changed caller elevation"
    );
    let current_authentication = kernel_authentication(current.raw());
    assert!(
        current_authentication.LowPart == authentication.LowPart
            && current_authentication.HighPart == authentication.HighPart,
        "constructor changed caller AuthenticationId"
    );
}

fn object_dacl(handle: HANDLE) -> Vec<u32> {
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetKernelObjectSecurity};
    let mut needed = 0;
    // SAFETY: retained own token with READ_CONTROL and an output sizing value.
    unsafe {
        GetKernelObjectSecurity(
            handle,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            0,
            &mut needed,
        )
    };
    assert!(needed > 0 && needed <= 65_536);
    let mut bytes = vec![0_u32; (needed as usize).div_ceil(4)];
    // SAFETY: initialized aligned buffer sized by the kernel for this retained token.
    assert_ne!(
        unsafe {
            GetKernelObjectSecurity(
                handle,
                DACL_SECURITY_INFORMATION,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    bytes
}

fn assert_user_system_only(descriptor: &[u32], user: &[u8]) {
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, GetAce, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, IsWellKnownSid, SE_DACL_PROTECTED, TOKEN_ALL_ACCESS,
        WinLocalSystemSid,
    };
    let raw = descriptor.as_ptr().cast_mut().cast();
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor was returned by GetKernelObjectSecurity into retained storage.
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(raw, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = null_mut();
    // SAFETY: retained descriptor and writable DACL metadata outputs.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(raw, &mut present, &mut acl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    assert!(!acl.is_null());
    // SAFETY: the kernel supplied this valid ACL inside the retained descriptor.
    assert_eq!(unsafe { (*acl).AceCount }, 2);
    let mut user_count = 0;
    let mut system_count = 0;
    for index in 0..2 {
        let mut ace = null_mut();
        // SAFETY: exact valid ACE index and retained kernel ACL.
        assert_ne!(unsafe { GetAce(acl, index, &mut ace) }, 0);
        // SAFETY: GetAce returned a valid kernel ACE; inspect its header first.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        assert_eq!(header.AceType, 0);
        assert!(usize::from(header.AceSize) >= size_of::<ACCESS_ALLOWED_ACE>());
        // SAFETY: the checked allow ACE has its full fixed header and a kernel-valid SID.
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Header.AceFlags, 0);
        assert_eq!(ace.Mask, TOKEN_ALL_ACCESS);
        let sid = (&raw const ace.SidStart).cast_mut().cast();
        // SAFETY: valid SID in the kernel allow ACE, compared to the validated own SID.
        if unsafe { EqualSid(sid, user.as_ptr().cast_mut().cast()) } != 0 {
            user_count += 1;
        }
        // SAFETY: valid SID in the same retained kernel ACE.
        else if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0 {
            system_count += 1;
        } else {
            panic!("unexpected browser primary principal");
        }
    }
    assert_eq!((user_count, system_count), (1, 1));
}

#[test]
fn browser_primary_object_security_retains_actual_user_without_changing_source() {
    use windows_sys::Win32::Storage::FileSystem::READ_CONTROL;
    let mut raw = null_mut();
    // SAFETY: opens this test process's own token only for duplication and observation.
    assert_ne!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | READ_CONTROL,
                &mut raw,
            )
        },
        0
    );
    let own = OwnedHandle::new(raw, "own primary security test").unwrap();
    let before = token_handle(own.raw()).unwrap();
    let dacl_before = object_dacl(own.raw());
    let duplicate = token_security::with_user_access(&own, &before.user).unwrap();
    let actual = token_handle(duplicate.raw()).unwrap();
    assert!(same_sid(&actual.user, &before.user));
    assert_eq!(actual.authentication, before.authentication);
    assert_eq!(actual.session, before.session);
    assert_eq!(actual.integrity, before.integrity);
    assert_eq!(actual.elevated, before.elevated);
    assert_user_system_only(&object_dacl(duplicate.raw()), &before.user);
    assert_eq!(object_dacl(own.raw()), dacl_before);
}
