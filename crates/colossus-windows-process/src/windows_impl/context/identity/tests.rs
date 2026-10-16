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
