//! Bounded observation of the calling process's attached console only.
use crate::WindowsNativeError;
use std::{
    io,
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, COORD, GetConsoleMode, GetConsoleScreenBufferInfo,
        ReadConsoleOutputCharacterW,
    },
};

const MAX_VISIBLE_CELLS: usize = 65_536;

/// One bounded native view of a console screen buffer's visible window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleScreenSnapshot {
    /// Cursor column relative to the visible window; it may lie outside that window.
    pub cursor_column: i32,
    /// Cursor row relative to the visible window; it may lie outside that window.
    pub cursor_row: i32,
    /// Exact visible rows, including trailing spaces, decoded from UTF-16 cells.
    pub rows: Vec<String>,
}

/// Native input flags and visible contents from the calling process's own console.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleSnapshot {
    /// Native input-mode bits returned by GetConsoleMode.
    pub input_mode: u32,
    /// Currently displayed console screen buffer.
    pub active: ConsoleScreenSnapshot,
}

/// Observes the calling process's own console for native terminal checks.
///
/// This never attaches to another process or accepts a window, process, or handle
/// selector. Each snapshot opens only CONOUT$ in the current console and reads at
/// most 65,536 visible cells. It does not mutate console state. VT alternate
/// screens share the console handle; retain snapshot values to verify restoration.
/// A process without an attached console receives an OS error.
pub struct ConsoleObserver {
    input: OwnedHandle,
}

impl ConsoleObserver {
    /// Retain this process's console input handle.
    pub fn current() -> Result<Self, WindowsNativeError> {
        Ok(Self {
            input: open_console("CONIN$")?,
        })
    }

    /// Observe native modes and the current screen. This is not an atomic frame capture;
    /// callers should query at an idle checkpoint when comparing buffer contents.
    pub fn snapshot(&self) -> Result<ConsoleSnapshot, WindowsNativeError> {
        let mut input_mode = 0;
        // SAFETY: the retained input handle is live and the output pointer is valid.
        check(
            unsafe { GetConsoleMode(self.input.as_raw_handle().cast(), &mut input_mode) },
            "console input mode",
        )?;
        let active = open_console("CONOUT$")?;
        Ok(ConsoleSnapshot {
            input_mode,
            active: screen(&active)?,
        })
    }
}

fn open_console(device: &str) -> Result<OwnedHandle, WindowsNativeError> {
    let name = device.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // SAFETY: device is one of the fixed console names above. name is terminated
    // and live for the call; no security attributes or template handle is supplied.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(failure("open current console"));
    }
    // SAFETY: CreateFileW returned one newly owned, valid handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

fn screen(handle: &OwnedHandle) -> Result<ConsoleScreenSnapshot, WindowsNativeError> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: handle is retained and info points to writable storage of the exact type.
    check(
        unsafe { GetConsoleScreenBufferInfo(handle.as_raw_handle().cast(), &mut info) },
        "console screen information",
    )?;
    let width = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
    let height = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
    let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else {
        return Err(WindowsNativeError::InvalidInput);
    };
    if width == 0
        || height == 0
        || width.saturating_mul(height) > MAX_VISIBLE_CELLS
        || info.srWindow.Left < 0
        || info.srWindow.Top < 0
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut rows = Vec::with_capacity(height);
    let mut cells = vec![0; width];
    for row in 0..height {
        let y = i32::from(info.srWindow.Top)
            + i32::try_from(row).map_err(|_| WindowsNativeError::InvalidInput)?;
        let position = COORD {
            X: info.srWindow.Left,
            Y: i16::try_from(y).map_err(|_| WindowsNativeError::InvalidInput)?,
        };
        let mut read = 0;
        // SAFETY: cells contains width writable UTF-16 units, bounded above; the
        // coordinate lies in the validated visible window and handle remains live.
        check(
            unsafe {
                ReadConsoleOutputCharacterW(
                    handle.as_raw_handle().cast(),
                    cells.as_mut_ptr(),
                    u32::try_from(width).map_err(|_| WindowsNativeError::InvalidInput)?,
                    position,
                    &mut read,
                )
            },
            "read current console cells",
        )?;
        if read as usize != width {
            return Err(WindowsNativeError::Io {
                operation: "read complete console row",
                source: io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete console row"),
            });
        }
        rows.push(String::from_utf16_lossy(&cells));
    }
    Ok(ConsoleScreenSnapshot {
        cursor_column: i32::from(info.dwCursorPosition.X) - i32::from(info.srWindow.Left),
        cursor_row: i32::from(info.dwCursorPosition.Y) - i32::from(info.srWindow.Top),
        rows,
    })
}

fn check(value: i32, operation: &'static str) -> Result<(), WindowsNativeError> {
    if value == 0 {
        Err(failure(operation))
    } else {
        Ok(())
    }
}

fn failure(operation: &'static str) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: io::Error::last_os_error(),
    }
}
