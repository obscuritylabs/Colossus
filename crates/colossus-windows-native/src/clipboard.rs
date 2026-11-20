//! Explicit, user-initiated copy into the local Windows clipboard.

use crate::WindowsNativeError;
use std::{
    io,
    ptr::{self, null, null_mut},
};
use windows_sys::{
    Win32::{
        Foundation::{GlobalFree, HGLOBAL, HWND},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock},
            RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId},
            StationsAndDesktops::{
                GetProcessWindowStation, GetUserObjectInformationW, UOI_FLAGS, USEROBJECTFLAGS,
            },
            Threading::GetCurrentProcessId,
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WSF_VISIBLE},
    },
    core::w,
};

const CF_UNICODETEXT: u32 = 13;
const MAX_TEXT_BYTES: usize = 64 * 1024;
pub(super) const TERMINAL_MANAGED_CLIPBOARD: &str = "COLOSSUS_TERMINAL_MANAGED_CLIPBOARD";

/// Store bounded Unicode text in the current local interactive clipboard.
///
/// The caller must have a fresh user-initiated copy action. Returns `Ok(false)`
/// outside the active Windows console session or inside an owned ConPTY so the
/// caller can use its terminal protocol.
pub fn try_copy_local_text(text: &str) -> Result<bool, WindowsNativeError> {
    if std::env::var_os(TERMINAL_MANAGED_CLIPBOARD).is_some() || !local_interactive_station() {
        return Ok(false);
    }
    copy_text(text)?;
    Ok(true)
}

fn local_interactive_station() -> bool {
    // SAFETY: These queries inspect only the calling process/session. The window
    // station handle is borrowed and remains valid for the call.
    unsafe {
        let mut process_session = 0;
        let console_session = WTSGetActiveConsoleSessionId();
        if console_session == u32::MAX
            || ProcessIdToSessionId(GetCurrentProcessId(), &mut process_session) == 0
            || process_session != console_session
        {
            return false;
        }
        let station = GetProcessWindowStation();
        let mut flags = USEROBJECTFLAGS::default();
        let mut bytes = 0;
        !station.is_null()
            && GetUserObjectInformationW(
                station,
                UOI_FLAGS,
                (&raw mut flags).cast(),
                size_of::<USEROBJECTFLAGS>() as u32,
                &mut bytes,
            ) != 0
            && bytes == size_of::<USEROBJECTFLAGS>() as u32
            && flags.dwFlags & WSF_VISIBLE as u32 != 0
    }
}

fn copy_text(text: &str) -> Result<(), WindowsNativeError> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Err(WindowsNativeError::InvalidInput);
    }
    // CF_UNICODETEXT uses CRLF line endings and a NUL terminator.
    let units: Vec<u16> = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .chain([0])
        .collect();
    let allocation = ClipboardAllocation::new(&units)?;
    let owner = ClipboardOwner::new()?;
    // SAFETY: The owner is a live window that belongs to the calling thread. The
    // guard is dropped before the owner, including on the error paths below.
    if unsafe { OpenClipboard(owner.0) } == 0 {
        return Err(last_error("open clipboard"));
    }
    let clipboard = OpenedClipboard(true);
    // SAFETY: The current thread has the clipboard open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(last_error("empty clipboard"));
    }
    // SAFETY: The allocation is unlocked, movable, and contains terminated
    // UTF-16. Windows takes ownership only when SetClipboardData succeeds.
    if unsafe { SetClipboardData(CF_UNICODETEXT, allocation.0) }.is_null() {
        return Err(last_error("set clipboard text"));
    }
    std::mem::forget(allocation);
    clipboard.close()
}

struct ClipboardAllocation(HGLOBAL);

impl ClipboardAllocation {
    fn new(units: &[u16]) -> Result<Self, WindowsNativeError> {
        // SAFETY: The input length is bounded by copy_text and the allocation is
        // owned by the guard until the clipboard accepts it.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size_of_val(units)) };
        if handle.is_null() {
            return Err(last_error("allocate clipboard text"));
        }
        let allocation = Self(handle);
        // SAFETY: The guard owns the live movable allocation.
        let target = unsafe { GlobalLock(handle) }.cast::<u16>();
        if target.is_null() {
            return Err(last_error("lock clipboard text"));
        }
        // SAFETY: The two allocations are disjoint and the target is locked
        // and sized for every supplied UTF-16 unit.
        unsafe {
            ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
            GlobalUnlock(handle);
        }
        Ok(allocation)
    }
}

impl Drop for ClipboardAllocation {
    fn drop(&mut self) {
        // SAFETY: Ownership has not been transferred to the OS.
        unsafe { GlobalFree(self.0) };
    }
}

struct ClipboardOwner(HWND);

impl ClipboardOwner {
    fn new() -> Result<Self, WindowsNativeError> {
        // SAFETY: The predefined window class name is NUL-terminated. This
        // creates a private message-only window on the calling thread.
        let window = unsafe {
            CreateWindowExW(
                0,
                w!("STATIC"),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if window.is_null() {
            return Err(last_error("create clipboard owner"));
        }
        Ok(Self(window))
    }
}

impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        // SAFETY: This thread owns the temporary window and has closed the clipboard.
        unsafe { DestroyWindow(self.0) };
    }
}

struct OpenedClipboard(bool);

impl OpenedClipboard {
    fn close(mut self) -> Result<(), WindowsNativeError> {
        // SAFETY: The current thread opened the clipboard and still owns the guard.
        if unsafe { CloseClipboard() } == 0 {
            return Err(last_error("close clipboard"));
        }
        self.0 = false;
        Ok(())
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: This thread opened the clipboard and has not closed it.
            unsafe { CloseClipboard() };
        }
    }
}

fn last_error(operation: &'static str) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: io::Error::last_os_error(),
    }
}

#[cfg(test)]
mod tests;
