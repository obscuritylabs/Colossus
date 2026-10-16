use super::*;
use windows_sys::Win32::Security::{
    WinHighLabelSid, WinMediumLabelSid, WinNullSid, WinSystemLabelSid,
};

#[test]
fn descendant_integrity_does_not_relax_the_exact_low_principal_policy() {
    for kind in [WinLowLabelSid, WinUntrustedLabelSid] {
        let sid = well_known_sid(kind).unwrap();
        validate_descendant_integrity(sid.as_ptr().cast_mut().cast()).unwrap();
        let principal = AppContainerPrincipal {
            package: well_known_sid(WinNullSid).unwrap(),
            user: well_known_sid(WinNullSid).unwrap(),
            integrity: sid,
        };
        assert_eq!(
            principal.validate_low_integrity().is_ok(),
            kind == WinLowLabelSid
        );
    }
    for kind in [
        WinNullSid,
        WinMediumLabelSid,
        WinHighLabelSid,
        WinSystemLabelSid,
    ] {
        let sid = well_known_sid(kind).unwrap();
        assert!(matches!(
            validate_descendant_integrity(sid.as_ptr().cast_mut().cast()),
            Err(WindowsNativeError::UnsafePermissions)
        ));
    }
}

#[test]
fn descendant_capabilities_require_a_complete_bounded_empty_array() {
    let header = offset_of!(TOKEN_GROUPS, Groups);
    for length in 0..header {
        assert!(validate_no_capability_groups(&vec![0; length]).is_err());
    }
    assert!(validate_no_capability_groups(&vec![0; 64 * 1024 + 1]).is_err());
    let mut bytes = vec![0; header];
    validate_no_capability_groups(&bytes).unwrap();
    bytes[..4].copy_from_slice(&1_u32.to_ne_bytes());
    assert!(validate_no_capability_groups(&bytes).is_err());
    bytes.resize(header + size_of::<SID_AND_ATTRIBUTES>(), 0);
    assert!(validate_no_capability_groups(&bytes).is_err());
    bytes[..4].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert!(validate_no_capability_groups(&bytes).is_err());
}
