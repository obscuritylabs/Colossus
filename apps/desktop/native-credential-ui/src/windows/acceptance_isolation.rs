//! Test-only station isolation. The interactive clipboard is never opened.

use super::wide;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{GENERIC_ALL, GlobalFree, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        StationsAndDesktops::{
            CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW,
            GetProcessWindowStation, GetThreadDesktop, HDESK, HWINSTA, SetProcessWindowStation,
            SetThreadDesktop,
        },
        Threading::GetCurrentThreadId,
    },
};

pub(super) struct InputIsolation {
    original_station: HWINSTA,
    original_desktop: HDESK,
    station: HWINSTA,
    desktop: HDESK,
}

impl InputIsolation {
    pub(super) fn create() -> Self {
        // SAFETY: This dedicated test process has no windows or hooks yet. A
        // fresh unnamed noninteractive station has a separate empty clipboard.
        let mut isolation = unsafe {
            Self {
                original_station: GetProcessWindowStation(),
                original_desktop: GetThreadDesktop(GetCurrentThreadId()),
                station: CreateWindowStationW(null(), 0, GENERIC_ALL, null()),
                desktop: null_mut(),
            }
        };
        assert!(
            !isolation.station.is_null(),
            "create isolated window station"
        );
        assert_ne!(
            unsafe { SetProcessWindowStation(isolation.station) },
            0,
            "select isolated window station"
        );
        isolation.desktop = unsafe {
            CreateDesktopW(
                wide("CredentialInputTest").as_ptr(),
                null(),
                null(),
                0,
                GENERIC_ALL,
                null(),
            )
        };
        assert!(!isolation.desktop.is_null(), "create isolated desktop");
        assert_ne!(
            unsafe { SetThreadDesktop(isolation.desktop) },
            0,
            "select isolated desktop"
        );
        isolation
    }

    pub(super) fn close(mut self) {
        assert_ne!(unsafe { SetProcessWindowStation(self.original_station) }, 0);
        assert_ne!(unsafe { SetThreadDesktop(self.original_desktop) }, 0);
        assert_ne!(
            unsafe { CloseDesktop(self.desktop) },
            0,
            "release test desktop"
        );
        self.desktop = null_mut();
        assert_ne!(
            unsafe { CloseWindowStation(self.station) },
            0,
            "release test station"
        );
        self.station = null_mut();
    }
}

impl Drop for InputIsolation {
    fn drop(&mut self) {
        // All test windows are destroyed before this guard. No SwitchDesktop
        // call is made: the user's visible desktop and input are untouched.
        unsafe {
            if self.station.is_null() {
                return;
            }
            SetProcessWindowStation(self.original_station);
            if !self.desktop.is_null() {
                SetThreadDesktop(self.original_desktop);
                CloseDesktop(self.desktop);
            }
            CloseWindowStation(self.station);
        }
    }
}

struct Clipboard;

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

pub(super) fn set_clipboard(owner: HWND, value: &str) {
    let units = zeroize::Zeroizing::new(wide(value));
    // SAFETY: This function runs only after InputIsolation::create, so all clipboard
    // operations target the private test station, never the interactive one.
    assert_ne!(unsafe { OpenClipboard(owner) }, 0);
    let _clipboard = Clipboard;
    assert_ne!(unsafe { EmptyClipboard() }, 0);
    let allocation = unsafe { GlobalAlloc(GMEM_MOVEABLE, units.len() * size_of::<u16>()) };
    assert!(!allocation.is_null());
    let target = unsafe { GlobalLock(allocation) }.cast::<u16>();
    if target.is_null() {
        unsafe {
            GlobalFree(allocation);
        }
        panic!("lock synthetic clipboard allocation");
    }
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
        GlobalUnlock(allocation);
    }
    let stored = unsafe { SetClipboardData(13, allocation) }; // CF_UNICODETEXT
    if stored.is_null() {
        unsafe {
            GlobalFree(allocation);
        }
    }
    assert!(!stored.is_null(), "transfer synthetic clipboard ownership");
}
