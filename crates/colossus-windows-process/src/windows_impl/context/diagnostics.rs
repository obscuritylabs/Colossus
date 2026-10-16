//! Observer-only catalog. Never treat these codes as a terminal creation receipt.
use super::{OwnedHandle, WindowsProcessError, launch::Role, wait_timeout};
use colossus_windows_native::WindowsNativeError;
use std::{cell::Cell, io::Write as _, time::Duration};

#[derive(Clone, Copy)]
#[repr(u32)]
pub(super) enum Phase {
    Dispatch,
    Image,
    Pipe,
    Peer,
    CoreOpen,
    CoreIdentity,
    Tokens,
    Policy,
    Hello,
    Request,
    Create,
    Return,
    Ack,
    Station,
    Drain,
}
thread_local! { static NATIVE: Cell<Option<[u32; 3]>> = const { Cell::new(None) }; }
pub(super) fn clear_native() {
    let _ = NATIVE.try_with(|slot| slot.set(None));
}
// Kinds0/1/2/3: Invalid/Win32/panic/Win32-code-outside-u16.
// Kinds4..10: LAST OBSERVED native Unsupported/Invalid/Reparse/Identity/Permissions/Io/Io-code-outside-u16.
// Native observations are separate evidence, never a replacement error or cleanup acknowledgment.
pub(super) fn record_native(error: &WindowsNativeError) {
    let parts = match error {
        WindowsNativeError::Io { operation, source } => os_parts(9, operation, source),
        WindowsNativeError::UnsupportedPlatform => [4, 0, 0],
        WindowsNativeError::InvalidInput => [5, 0, 0],
        WindowsNativeError::ReparsePoint => [6, 0, 0],
        WindowsNativeError::IdentityChanged => [7, 0, 0],
        WindowsNativeError::UnsafePermissions => [8, 0, 0],
    };
    let _ = NATIVE.try_with(|slot| slot.set(Some(parts)));
}
fn operation(value: &str) -> u32 {
    match value {
        "open private browser rendezvous" => 1,
        "set browser rendezvous message mode" => 2,
        "OpenProcess(observe exact Core)" => 3,
        "OpenProcessToken(browser context)" => 4,
        "GetTokenInformation(browser context)" => 5,
        "GetTokenInformation(context elevation)" => 6,
        "CreateProcessWithLogonW(browser context)" => 7,
        "CreateProcessW(browser context)" => 8,
        "DuplicateHandle(browser context target)" => 9,
        "GetProcessTimes(browser context)" => 10,
        "complete browser rendezvous I/O" => 11,
        "wait browser rendezvous" => 12,
        "open path without following reparse points" => 13,
        "close context endpoint copy" => 14,
        _ => 15, // Unknown static operation; never echo its text.
    }
}
fn os_parts(kind: u32, operation_name: &str, error: &std::io::Error) -> [u32; 3] {
    match error
        .raw_os_error()
        .and_then(|code| u16::try_from(code).ok())
    {
        Some(code) => [kind, operation(operation_name), u32::from(code)],
        None => [if kind == 9 { 10 } else { 3 }, operation(operation_name), 0],
    }
}
pub(super) fn exit_code(
    role: Option<Role>,
    phase: Phase,
    error: Option<&WindowsProcessError>,
) -> i32 {
    let native = NATIVE.try_with(Cell::take).ok().flatten();
    let [kind, operation, os] = match error {
        None => [2, 0, 0],
        Some(WindowsProcessError::Win32 { operation, source }) => os_parts(1, operation, source),
        Some(_) => native.unwrap_or([0, 0, 0]),
    };
    let role = match role {
        None => 0,
        Some(Role::Launcher) => 1,
        Some(Role::Context) => 2,
    };
    (0x4000_0000 | (role << 28) | ((phase as u32) << 24) | (operation << 20) | (kind << 16) | os)
        as i32
}
// Closed decoder: role, phase, returned-error/last-native kind, fixed operation, exact OS code.
fn decode(code: u32) -> Option<[u32; 5]> {
    let values = [
        (code >> 28) & 3,
        (code >> 24) & 15,
        (code >> 16) & 15,
        (code >> 20) & 15,
        code & 65535,
    ];
    let [role, phase, kind, operation, os] = values;
    (code & 0xC000_0000 == 0x4000_0000
        && role <= 2
        && phase <= 14
        && kind <= 10
        && (role != 0 || phase == 0)
        && if matches!(kind, 1 | 3 | 9 | 10) {
            operation > 0 && (matches!(kind, 1 | 9) || os == 0)
        } else {
            operation == 0 && os == 0
        })
    .then_some(values)
}
pub(super) fn observe(role: &'static str, process: Option<&OwnedHandle>) {
    // At most100ms per retained process. Observation never terminates/resumes it.
    let (status, code) = match process {
        None => ("absent", None),
        Some(process) => match wait_timeout(process, Duration::from_millis(100)) {
            Ok(Some(code)) => ("exited", Some(code)),
            Ok(None) => ("not-exited", None),
            Err(_) => ("query-failed", None),
        },
    };
    let _ = writeln!(
        std::io::stderr().lock(),
        "native Windows helper observation role={role} status={status} code={code:?} decoded_role_phase_kind_operation_os={:?}",
        code.and_then(decode)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_catalog_has_an_independent_literal_oracle() {
        clear_native();
        let error = WindowsProcessError::Win32 {
            operation: "GetTokenInformation(browser context)",
            source: std::io::Error::from_raw_os_error(24),
        };
        let code = exit_code(Some(Role::Context), Phase::Tokens, Some(&error));
        assert_eq!(code as u32, 0x6651_0018);
        assert_eq!(decode(code as u32), Some([2, 6, 1, 5, 24]));
        assert_eq!(
            decode(exit_code(Some(Role::Launcher), Phase::Create, None) as u32),
            Some([1, 10, 2, 0, 0])
        );
    }
    #[test]
    fn catalog_rejects_unrecognized_and_malformed_exit_values() {
        for code in [
            0,
            1,
            259,
            0xC000_0005,
            0x7000_0000,
            0x6F00_0000,
            0x600B_0000,
            0x6000_0001,
            0x6001_0000,
        ] {
            assert_eq!(decode(code), None);
        }
    }
    #[test]
    fn observation_redacts_text_clears_stale_values_and_never_truncates_os_codes() {
        clear_native();
        record_native(&WindowsNativeError::UnsafePermissions);
        let secret = "S-1-5-21-private C:\\private nonce=secret";
        let invalid = WindowsProcessError::Invalid(secret.into());
        let native = decode(exit_code(Some(Role::Launcher), Phase::Image, Some(&invalid)) as u32);
        assert_eq!(native, Some([1, 1, 8, 0, 0]));
        assert!(!format!("{native:?}").contains(secret));
        assert_eq!(
            decode(exit_code(None, Phase::Dispatch, Some(&invalid)) as u32),
            Some([0, 0, 0, 0, 0])
        );
        record_native(&WindowsNativeError::UnsafePermissions);
        clear_native();
        let error = WindowsProcessError::Win32 {
            operation: secret,
            source: std::io::Error::from_raw_os_error(70000),
        };
        assert_eq!(
            decode(exit_code(None, Phase::Dispatch, Some(&error)) as u32),
            Some([0, 0, 3, 15, 0])
        );
    }
}
