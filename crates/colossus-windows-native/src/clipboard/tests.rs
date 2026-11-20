use super::*;
use std::{process::Command, slice};
use windows_sys::Win32::{
    Foundation::GENERIC_ALL,
    System::{
        DataExchange::GetClipboardData,
        Memory::GlobalSize,
        StationsAndDesktops::{
            CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW,
            GetThreadDesktop, HDESK, HWINSTA, SetProcessWindowStation, SetThreadDesktop,
        },
        Threading::GetCurrentThreadId,
    },
};

const FIXTURE: &str = "COLOSSUS_PRIVATE_CLIPBOARD_TEST";
const VERIFIED: &str = "private clipboard readback verified";

#[test]
fn native_clipboard_stores_unicode_in_a_private_window_station() {
    if std::env::var_os(FIXTURE).is_none() {
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "clipboard::tests::native_clipboard_stores_unicode_in_a_private_window_station",
                "--nocapture",
            ])
            .env(FIXTURE, "1")
            .output()
            .expect("run isolated clipboard fixture");
        assert!(
            output.status.success(),
            "private clipboard fixture: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(VERIFIED),
            "private clipboard fixture did not confirm readback"
        );
        return;
    }

    let _private_desktop = PrivateDesktop::enter();
    // The production entry point rejects a noninteractive station. Exercise
    // its Win32 writer directly with this private clipboard.
    assert!(!try_copy_local_text("unavailable").expect("station check"));
    copy_text("界 e\u{301} 👩‍💻\nsecond  line").expect("native copy");
    assert_eq!(read_text(), "界 e\u{301} 👩‍💻\r\nsecond  line");
    assert!(copy_text("invalid\0text").is_err());
    assert_eq!(read_text(), "界 e\u{301} 👩‍💻\r\nsecond  line");
    println!("{VERIFIED}");
}

fn read_text() -> String {
    // SAFETY: The fixture switched to its private station before calling this
    // function. A null owner is valid for reading an existing clipboard item.
    unsafe {
        assert_ne!(OpenClipboard(null_mut()), 0);
        let _clipboard = OpenedClipboard(true);
        let handle = GetClipboardData(CF_UNICODETEXT);
        assert!(!handle.is_null(), "CF_UNICODETEXT is available");
        let bytes = GlobalSize(handle);
        assert!((2..=MAX_TEXT_BYTES * 4 + 2).contains(&bytes));
        let text = GlobalLock(handle).cast::<u16>();
        assert!(!text.is_null(), "lock fixture clipboard text");
        let units = slice::from_raw_parts(text, bytes / size_of::<u16>());
        let end = units.iter().position(|&unit| unit == 0);
        let value = end.map(|end| String::from_utf16(&units[..end]));
        GlobalUnlock(handle);
        value.expect("terminated text").expect("valid UTF-16")
    }
}

// The child process alone changes station. No window is ever shown or switched
// onto the visible desktop, and clipboard calls come after both checks succeed.
struct PrivateDesktop {
    original_station: HWINSTA,
    original_desktop: HDESK,
    station: HWINSTA,
    desktop: HDESK,
}

impl PrivateDesktop {
    fn enter() -> Self {
        // SAFETY: This fixture owns the new unnamed station and desktop. The
        // original station and desktop handles are borrowed and retained.
        unsafe {
            let mut isolation = Self {
                original_station: GetProcessWindowStation(),
                original_desktop: GetThreadDesktop(GetCurrentThreadId()),
                station: CreateWindowStationW(null(), 0, GENERIC_ALL, null()),
                desktop: null_mut(),
            };
            assert!(
                !isolation.station.is_null(),
                "create private window station"
            );
            assert_ne!(isolation.station, isolation.original_station);
            assert_ne!(SetProcessWindowStation(isolation.station), 0);
            assert_eq!(GetProcessWindowStation(), isolation.station);
            isolation.desktop = CreateDesktopW(
                w!("ColossusClipboardTest"),
                null(),
                null(),
                0,
                GENERIC_ALL,
                null(),
            );
            assert!(!isolation.desktop.is_null(), "create private desktop");
            assert_ne!(SetThreadDesktop(isolation.desktop), 0);
            assert_eq!(GetThreadDesktop(GetCurrentThreadId()), isolation.desktop);
            isolation
        }
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        // SAFETY: All temporary clipboard-owner windows have been destroyed.
        unsafe {
            SetProcessWindowStation(self.original_station);
            if !self.desktop.is_null() {
                SetThreadDesktop(self.original_desktop);
                CloseDesktop(self.desktop);
            }
            if !self.station.is_null() {
                CloseWindowStation(self.station);
            }
        }
    }
}
