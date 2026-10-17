//! Auxiliary-only fixed module-presence observations of one retained browser child.
use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
            TH32CS_SNAPMODULE,
        },
        Threading::{GetProcessId, WaitForSingleObject},
    },
};

/// Closed diagnostic outcome; no status changes process or cleanup authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupModuleObservationStatus {
    /// Enumeration ended normally while the same retained child stayed live.
    Complete,
    /// The retained process identifier could not be queried.
    ProcessQueryFailed,
    /// The retained child was already signaled or exited during observation.
    ProcessNotLive,
    /// A zero-duration retained-process wait failed.
    ProcessWaitFailed,
    /// A zero-duration wait returned an unexpected value.
    UnexpectedWait,
    /// The one snapshot call failed; there is no retry loop.
    SnapshotFailed,
    /// No module entry was available; loader initialization may be incomplete.
    NoModules,
    /// Enumeration ended with an error other than normal completion.
    EnumerationFailed,
    /// An entry did not have the exact process binding or a bounded module name.
    InvalidModuleEntry,
    /// The finite entry cap was reached; the partial list is not accepted.
    EntryLimit,
}

/// Fixed observations only; no names, paths, addresses, PIDs or process handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupModuleObservation {
    /// Closed final observation status, never an acceptance or cleanup result.
    pub status: StartupModuleObservationStatus,
    /// Whether enumeration reached its normal end before the entry cap.
    pub completed: bool,
    /// Whether the exact retained process was unsignaled before and after.
    pub live_bound: bool,
    /// Bits 0..4: bootstrap EXE, client DLL, chrome_elf, libcef and wintrust.
    /// Valid only when completed and live_bound; otherwise zero means unknown.
    /// Mapping does not establish DllMain, signature-check or entry completion.
    pub mask: u8,
    /// Immediate Win32 error only, or zero for a non-API closed status.
    pub os_code: u32,
    /// Number of module entries examined, at most 512.
    pub examined: u32,
}

impl StartupModuleObservation {
    fn empty(status: StartupModuleObservationStatus, os_code: u32) -> Self {
        Self {
            status,
            completed: false,
            live_bound: false,
            mask: 0,
            os_code,
            examined: 0,
        }
    }
}

/// Observe only fixed module leaves in the caller's exact retained browser child.
///
/// The caller retains that child through the complete call. No process is opened,
/// suspended or given new rights, and only the snapshot becomes an owned handle.
/// The retained child handle must permit limited queries and synchronization.
/// ToolHelp may internally inspect loader state and can fail on access or races.
///
/// This auxiliary diagnostic makes one synchronous Win32 snapshot call, without
/// retries. It has no internal wall-clock deadline: the 512-entry cap bounds
/// enumeration only, not the duration of the native call. It never prints, and
/// any failure remains observational; the caller must still perform its unchanged
/// owned process/context/I/O cleanup. Module absence is only a completed snapshot
/// observation, not evidence that a module or function was never entered.
#[must_use]
pub fn observe_browser_startup_modules(process: BorrowedHandle<'_>) -> StartupModuleObservation {
    let raw = process.as_raw_handle().cast();
    // SAFETY: the caller retains this exact borrowed process object for the call.
    let pid = unsafe { GetProcessId(raw) };
    if pid == 0 {
        // SAFETY: capture the failed query's thread-local error immediately.
        let error = unsafe { GetLastError() };
        return StartupModuleObservation::empty(
            StartupModuleObservationStatus::ProcessQueryFailed,
            error,
        );
    }
    if let Err((status, code)) = live(raw) {
        return StartupModuleObservation::empty(status, code);
    }
    // SAFETY: one read-only module snapshot for the exact live derived PID. No
    // process/thread/heap enumeration or inheritable snapshot flag is requested.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) };
    if snapshot == INVALID_HANDLE_VALUE {
        // SAFETY: capture snapshot failure before another Win32 call.
        let error = unsafe { GetLastError() };
        return finish(
            raw,
            StartupModuleObservation::empty(StartupModuleObservationStatus::SnapshotFailed, error),
        );
    }
    if snapshot.is_null() {
        return finish(
            raw,
            StartupModuleObservation::empty(StartupModuleObservationStatus::SnapshotFailed, 0),
        );
    }
    // SAFETY: the one successful snapshot is owned exactly once and closed by
    // OwnedHandle. MODULEENTRY32W.hModule observations are never adopted or closed.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.cast()) };
    // SAFETY: zeroed output plus the documented byte size precede enumeration.
    let mut entry: MODULEENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut observed =
        StartupModuleObservation::empty(StartupModuleObservationStatus::NoModules, 0);
    // SAFETY: snapshot and initialized output remain live for this native call.
    if unsafe { Module32FirstW(snapshot.as_raw_handle().cast(), &mut entry) } == 0 {
        // SAFETY: immediate error from the failed enumeration call.
        let error = unsafe { GetLastError() };
        if error != ERROR_NO_MORE_FILES {
            observed.status = StartupModuleObservationStatus::EnumerationFailed;
            observed.os_code = error;
        }
        return finish(raw, observed);
    }
    loop {
        observed.examined += 1;
        let Some(end) = entry.szModule.iter().position(|unit| *unit == 0) else {
            observed.status = StartupModuleObservationStatus::InvalidModuleEntry;
            break;
        };
        if entry.th32ProcessID != pid || end == 0 {
            observed.status = StartupModuleObservationStatus::InvalidModuleEntry;
            break;
        }
        observed.mask |= known_module(&entry.szModule[..end]);
        if observed.examined == 512 {
            observed.status = StartupModuleObservationStatus::EntryLimit;
            break;
        }
        entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
        // SAFETY: output and the sole owned snapshot remain valid as above.
        if unsafe { Module32NextW(snapshot.as_raw_handle().cast(), &mut entry) } == 0 {
            // SAFETY: capture the enumeration result's error immediately.
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                observed.status = StartupModuleObservationStatus::Complete;
                observed.completed = true;
            } else {
                observed.status = StartupModuleObservationStatus::EnumerationFailed;
                observed.os_code = error;
            }
            break;
        }
    }
    finish(raw, observed)
}

fn live(process: HANDLE) -> Result<(), (StartupModuleObservationStatus, u32)> {
    // SAFETY: query-only zero-duration wait on the exact retained process object.
    match unsafe { WaitForSingleObject(process, 0) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err((StartupModuleObservationStatus::ProcessNotLive, 0)),
        WAIT_FAILED => {
            // SAFETY: capture the failed wait's immediate thread-local error.
            let error = unsafe { GetLastError() };
            Err((StartupModuleObservationStatus::ProcessWaitFailed, error))
        }
        _ => Err((StartupModuleObservationStatus::UnexpectedWait, 0)),
    }
}

fn finish(process: HANDLE, mut observed: StartupModuleObservation) -> StartupModuleObservation {
    match live(process) {
        Ok(()) => observed.live_bound = true,
        Err((status, code)) => {
            observed.status = status;
            observed.os_code = code;
        }
    }
    if !observed.completed || !observed.live_bound {
        observed.mask = 0;
    }
    observed
}

fn known_module(name: &[u16]) -> u8 {
    const MODULES: [&[u8]; 5] = [
        b"colossus-native-browser-host.exe",
        b"colossus-native-browser-host.dll",
        b"chrome_elf.dll",
        b"libcef.dll",
        b"wintrust.dll",
    ];
    let mut mask = 0;
    for (index, expected) in MODULES.iter().enumerate() {
        if name.len() == expected.len()
            && name.iter().zip(*expected).all(|(actual, expected)| {
                *actual <= u16::from(u8::MAX) && (*actual as u8).eq_ignore_ascii_case(expected)
            })
        {
            mask |= 1 << index;
        }
    }
    mask
}
