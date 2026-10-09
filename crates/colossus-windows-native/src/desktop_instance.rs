//! Serialize Desktop startup and hand foreground permission to its existing owner.
use crate::WindowsNativeError;
use std::{
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId},
};

/// Holds the Windows startup gate until Tauri has installed its single-instance listener.
/// This closes the plugin's mutex-before-window race during simultaneous launches.
/// Drop on the acquiring thread, after building the app and before its event loop.
pub struct DesktopLaunchGuard {
    mutex: OwnedHandle,
    _thread: PhantomData<Rc<()>>,
}

impl DesktopLaunchGuard {
    /// Serialize startup for a native-owned application identifier, then permit an
    /// existing Tauri instance to bring its window forward. No arguments are executed.
    pub fn acquire(application_id: &str) -> Result<Self, WindowsNativeError> {
        let guard = Self::acquire_with_timeout(application_id, 15_000)?;
        let class = wide(&format!("{application_id}-sic"));
        let title = wide(&format!("{application_id}-siw"));
        // SAFETY: strings are bounded and NUL-terminated; the returned HWND is borrowed.
        let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !window.is_null() {
            let mut pid = 0;
            // SAFETY: HWND is used only for querying its owner, and pid is writable.
            unsafe { GetWindowThreadProcessId(window, &mut pid) };
            if pid != 0 {
                // SAFETY: grant only to the existing instance, never ASFW_ANY. Windows
                // may deny this if the caller itself has no foreground permission.
                unsafe { AllowSetForegroundWindow(pid) };
            }
        }
        Ok(guard)
    }

    fn acquire_with_timeout(id: &str, timeout_ms: u32) -> Result<Self, WindowsNativeError> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        {
            return Err(WindowsNativeError::InvalidInput);
        }
        let name = wide(&format!("Local\\{id}-startup"));
        // SAFETY: no inherited handle, default caller DACL, bounded terminated name.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(native_error(std::io::Error::last_os_error()));
        }
        // SAFETY: CreateMutexW transferred one owned handle to this function.
        let mutex = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
        // SAFETY: the mutex remains owned for the bounded wait.
        match unsafe { WaitForSingleObject(handle, timeout_ms) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self {
                mutex,
                _thread: PhantomData,
            }),
            WAIT_TIMEOUT => Err(native_error(std::io::Error::from(
                std::io::ErrorKind::TimedOut,
            ))),
            _ => Err(native_error(std::io::Error::last_os_error())),
        }
    }
}

impl Drop for DesktopLaunchGuard {
    fn drop(&mut self) {
        // SAFETY: this guard owns the acquired mutex and cannot cross threads.
        unsafe { ReleaseMutex(self.mutex.as_raw_handle().cast()) };
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn native_error(source: std::io::Error) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation: "Desktop startup",
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_gate_serializes_launches_and_releases_ownership() {
        let id = format!("colossus.startup.test.{}", std::process::id());
        let guard = DesktopLaunchGuard::acquire(&id).unwrap();
        let other = id.clone();
        assert!(
            std::thread::spawn(
                move || DesktopLaunchGuard::acquire_with_timeout(&other, 10).is_err()
            )
            .join()
            .unwrap()
        );
        drop(guard);
        assert!(
            std::thread::spawn(move || DesktopLaunchGuard::acquire_with_timeout(&id, 10).is_ok())
                .join()
                .unwrap()
        );
    }

    #[test]
    fn startup_identifiers_cannot_inject_native_names() {
        for id in ["", "Global\\another", "app\0name", "app/name"] {
            assert!(matches!(
                DesktopLaunchGuard::acquire(id),
                Err(WindowsNativeError::InvalidInput)
            ));
        }
        assert!(DesktopLaunchGuard::acquire(&"a".repeat(129)).is_err());
    }
}
