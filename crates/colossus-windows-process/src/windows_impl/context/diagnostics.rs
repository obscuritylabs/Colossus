//! Observer-only catalog. Never treat these codes as a terminal creation receipt.
use super::{OwnedHandle, WindowsProcessError, launch::Role, wait_timeout};
use colossus_windows_native::WindowsNativeError;
use std::{cell::Cell, io::Write as _, time::Duration};
use windows_sys::Win32::Security;

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
thread_local! {
    static NATIVE: Cell<Option<[u32; 3]>> = const { Cell::new(None) };
    static BROWSER: Cell<Option<u16>> = const { Cell::new(None) };
    static SIZING: Cell<Option<[u32; 3]>> = const { Cell::new(None) };
    static JOB: Cell<Option<[u32; 3]>> = const { Cell::new(None) };
}
pub(super) fn clear_observations() {
    let _ = NATIVE.try_with(|slot| slot.set(None));
    let _ = BROWSER.try_with(|slot| slot.set(None));
    let _ = SIZING.try_with(|slot| slot.set(None));
    let _ = JOB.try_with(|slot| slot.set(None));
}
// These observations apply only to their exact returned Invalid label in Context/Create.
// They never replace the error, acknowledge creation, or establish cleanup.
pub(super) fn record_browser(mask: u16) {
    let _ = BROWSER.try_with(|slot| slot.set((1..=255).contains(&mask).then_some(mask)));
}
pub(super) fn record_sizing(class: i32, error: u32) {
    let bucket = [
        Security::TokenIsAppContainer,
        Security::TokenIntegrityLevel,
        Security::TokenStatistics,
        Security::TokenUser,
        Security::TokenSessionId,
        Security::TokenCapabilities,
        Security::TokenAppContainerSid,
    ]
    .iter()
    .position(|value| *value == class)
    .map(|index| index as u32 + 1);
    let parts = bucket.map(|bucket| match u16::try_from(error) {
        Ok(error) => [13, bucket, u32::from(error)],
        Err(_) => [14, bucket, 0],
    });
    let _ = SIZING.try_with(|slot| slot.set(parts));
}
pub(super) fn record_job(error: Option<u32>) {
    let parts = match error {
        Some(error) => match u16::try_from(error) {
            Ok(error) => [15, 1, u32::from(error)],
            Err(_) => [15, 2, 0],
        },
        None => [15, 3, 0], // Successful query positively reported nonmembership.
    };
    let _ = JOB.try_with(|slot| slot.set(Some(parts)));
}
fn invalid_parts(label: &str) -> [u32; 3] {
    let id = match label {
        "fixed local endpoint count" => 1,
        "browser executable parent absent" => 2,
        "context process values exceed Win32 bound" => 3,
        "created PROCESS_INFORMATION is incomplete" => 4,
        "missing suspended browser custody" => 5,
        "process image bound" => 6,
        "retained helper birth/image identity changed" => 7,
        "token sizing query rejected" => 8,
        "token information bound rejected" => 9,
        "token information overflow" => 10,
        "truncated token integer" => 11,
        "truncated token SID pointer" => 12,
        "token SID outside buffer" => 13,
        "token SID bound rejected" => 14,
        "invalid token SID" => 15,
        "invalid AppContainer token flag" => 16,
        "token integrity SID is not a mandatory label" => 17,
        "token statistics truncated" => 18,
        "integrity size" => 19,
        "token elevation output rejected" => 20,
        "truncated token capabilities" => 21,
        "capability array exceeds token buffer" => 22,
        "suspended browser token differs from exact fresh AppContainer context" => 23,
        "station child inner Job mismatch" => 24,
        "remote handle width" => 25,
        "invalid or pseudo remote handle" => 26,
        "native browser context identity rejected" => 27,
        _ => return [0, 0, 0],
    };
    [11, 0, id]
}
// Kinds0/1/2/3: Invalid/Win32/panic/Win32-code-outside-u16.
// Kinds4..10: LAST OBSERVED native Unsupported/Invalid/Reparse/Identity/Permissions/Io/Io-code-outside-u16.
// Context/Create kinds11..15: fixed Invalid label/browser mask/sizing OS/sizing overflow/Job query.
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
    let browser = BROWSER.try_with(Cell::take).ok().flatten();
    let sizing = SIZING.try_with(Cell::take).ok().flatten();
    let job = JOB.try_with(Cell::take).ok().flatten();
    let refined = matches!(role, Some(Role::Context)) && matches!(phase, Phase::Create);
    let [kind, operation, os] = match error {
        None => [2, 0, 0],
        Some(WindowsProcessError::Win32 { operation, source }) => os_parts(1, operation, source),
        Some(WindowsProcessError::Invalid(label)) => {
            let fallback = if refined {
                invalid_parts(label)
            } else {
                [0, 0, 0]
            };
            match label.as_str() {
                "native browser context identity rejected" => native.unwrap_or(fallback),
                "suspended browser token differs from exact fresh AppContainer context"
                    if refined =>
                {
                    browser
                        .map(|mask| [12, 0, u32::from(mask)])
                        .unwrap_or(fallback)
                }
                "token sizing query rejected" if refined => sizing.unwrap_or(fallback),
                "station child inner Job mismatch" if refined => job.unwrap_or(fallback),
                _ => fallback,
            }
        }
        Some(_) => [0, 0, 0],
    };
    let role = match role {
        None => 0,
        Some(Role::Launcher) => 1,
        Some(Role::Context) => 2,
    };
    (0x4000_0000 | (role << 28) | ((phase as u32) << 24) | (operation << 20) | (kind << 16) | os)
        as i32
}
// Closed decoder: role, phase, fixed kind, operation/bucket, OS/label/mask detail.
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
        && (role != 0 || phase == 0)
        && match kind {
            1 | 3 | 9 | 10 => operation > 0 && (matches!(kind, 1 | 9) || os == 0),
            0 | 2 | 4..=8 => operation == 0 && os == 0,
            11..=15 if role == 2 && phase == 10 => match kind {
                11 => operation == 0 && (1..=27).contains(&os),
                12 => operation == 0 && (1..=255).contains(&os),
                13 | 14 => (1..=7).contains(&operation) && (kind == 13 || os == 0),
                15 => operation == 1 || (matches!(operation, 2 | 3) && os == 0),
                _ => false,
            },
            _ => false,
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
        clear_observations();
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
        clear_observations();
        record_native(&WindowsNativeError::UnsafePermissions);
        let secret = "S-1-5-21-private C:\\private nonce=secret";
        let invalid = WindowsProcessError::Invalid(secret.into());
        let wrapper =
            WindowsProcessError::Invalid("native browser context identity rejected".into());
        let native = decode(exit_code(Some(Role::Launcher), Phase::Image, Some(&wrapper)) as u32);
        assert_eq!(native, Some([1, 1, 8, 0, 0]));
        assert!(!format!("{native:?}").contains(secret));
        assert_eq!(
            decode(exit_code(None, Phase::Dispatch, Some(&invalid)) as u32),
            Some([0, 0, 0, 0, 0])
        );
        record_native(&WindowsNativeError::UnsafePermissions);
        clear_observations();
        let error = WindowsProcessError::Win32 {
            operation: secret,
            source: std::io::Error::from_raw_os_error(70000),
        };
        assert_eq!(
            decode(exit_code(None, Phase::Dispatch, Some(&error)) as u32),
            Some([0, 0, 3, 15, 0])
        );
    }
    #[test]
    fn refined_catalog_has_literal_oracles_and_closed_ranges() {
        clear_observations();
        let error = |label: &str| WindowsProcessError::Invalid(label.into());
        let browser =
            error("suspended browser token differs from exact fresh AppContainer context");
        record_browser(0x55);
        assert_eq!(
            exit_code(Some(Role::Context), Phase::Create, Some(&browser)) as u32,
            0x6A0C_0055
        );
        record_sizing(Security::TokenAppContainerSid, 24);
        let sizing = error("token sizing query rejected");
        assert_eq!(
            exit_code(Some(Role::Context), Phase::Create, Some(&sizing)) as u32,
            0x6A7D_0018
        );
        record_job(Some(5));
        let job = error("station child inner Job mismatch");
        assert_eq!(
            exit_code(Some(Role::Context), Phase::Create, Some(&job)) as u32,
            0x6A1F_0005
        );
        assert_eq!(
            exit_code(Some(Role::Context), Phase::Create, Some(&job)) as u32,
            0x6A0B_0018
        );
        for code in [0x6A0C_0055, 0x6A7D_0018, 0x6A1F_0005, 0x6A0B_0018] {
            assert!(decode(code).is_some());
        }
        for code in [
            0x6A0B_0000,
            0x6A0B_001C,
            0x6A1B_0001,
            0x6A0C_0000,
            0x6A0C_0100,
            0x6A0D_0018,
            0x6A8D_0018,
            0x6A1E_0001,
            0x6A0F_0000,
            0x6A2F_0001,
            0x6A4F_0000,
            0x660B_0001,
            0x5A0B_0001,
        ] {
            assert_eq!(decode(code), None);
        }
    }
    #[test]
    fn refined_observations_match_the_error_clear_and_never_truncate() {
        clear_observations();
        let invalid = |label: &str| WindowsProcessError::Invalid(label.into());
        record_native(&WindowsNativeError::UnsafePermissions);
        record_browser(1);
        record_sizing(Security::TokenUser, 5);
        record_job(Some(5));
        let other = invalid("process image bound");
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&other)) as u32),
            Some([2, 10, 11, 0, 6])
        );
        let sizing = invalid("token sizing query rejected");
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&sizing)) as u32),
            Some([2, 10, 11, 0, 8])
        );
        record_sizing(Security::TokenUser, 70000);
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&sizing)) as u32),
            Some([2, 10, 14, 4, 0])
        );
        let job = invalid("station child inner Job mismatch");
        record_job(Some(70000));
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&job)) as u32),
            Some([2, 10, 15, 2, 0])
        );
        record_job(None);
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&job)) as u32),
            Some([2, 10, 15, 3, 0])
        );
        record_browser(255);
        record_sizing(Security::TokenUser, 5);
        record_job(None);
        clear_observations();
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&job)) as u32),
            Some([2, 10, 11, 0, 24])
        );
        for (role, phase) in [
            (Some(Role::Launcher), Phase::Create),
            (Some(Role::Context), Phase::Tokens),
            (None, Phase::Dispatch),
        ] {
            record_sizing(Security::TokenUser, 5);
            let code = exit_code(role, phase, Some(&sizing)) as u32;
            assert!(decode(code).is_some());
            assert_eq!((code >> 16) & 15, 0);
        }
        let secret = invalid("S-1-5-private C:\\private nonce=secret");
        record_native(&WindowsNativeError::UnsafePermissions);
        assert_eq!(
            decode(exit_code(Some(Role::Context), Phase::Create, Some(&secret)) as u32),
            Some([2, 10, 0, 0, 0])
        );
    }
}
