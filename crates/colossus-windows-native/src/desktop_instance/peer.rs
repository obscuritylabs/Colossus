//! Exact existing receiver validation before donating foreground eligibility.
use super::{ACK, Names, WAKE, WindowsNativeError, failed, last_error, security::Principal, wide};
use crate::BoundPath;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::{HWND, WAIT_TIMEOUT},
    Storage::FileSystem::SYNCHRONIZE,
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        WaitForSingleObject,
    },
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, GetClassNameW, GetWindowThreadProcessId,
        SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT, SendMessageTimeoutW,
    },
};

pub(super) fn reject_legacy(names: &Names) -> Result<(), WindowsNativeError> {
    let class = wide(&names.legacy_class);
    let title = wide(&names.legacy_title);
    // SAFETY: exact old-plugin names only, checked while both startup gates are held.
    if !unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) }.is_null() {
        return Err(failed(
            "legacy instance must exit before restart",
            std::io::ErrorKind::AlreadyExists,
        ));
    }
    Ok(())
}

pub(super) fn wake(
    names: &Names,
    principal: &Principal,
    image: &BoundPath,
) -> Result<(), WindowsNativeError> {
    let class = wide(&names.class);
    // SAFETY: exact bounded class/title, no enumeration or renderer-provided handle.
    let hwnd = unsafe { FindWindowW(class.as_ptr(), class.as_ptr()) };
    if hwnd.is_null() {
        return Err(failed(
            "instance endpoint unavailable",
            std::io::ErrorKind::NotFound,
        ));
    }
    let pid = window_process(hwnd)?;
    // SAFETY: query-only retained process, including a bounded alive-state query.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        return Err(last_error("instance peer process"));
    }
    // SAFETY: OpenProcess returned one owned handle, pinning this kernel process identity.
    let process = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    validate(hwnd, pid, names, &process, principal, image)?;
    // SAFETY: donate only existing foreground permission to this exact retained peer.
    // Windows may refuse; no global grants, synthetic keys or input-queue attachment.
    unsafe { AllowSetForegroundWindow(pid) };
    let mut response = 0;
    // SAFETY: one fixed wake-only message, zero payload, no pointers. Bound foreign wait.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WAKE,
            0,
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
            1000,
            &mut response,
        )
    };
    if sent == 0 || response != ACK as usize {
        return Err(failed(
            "instance wake acknowledgement",
            std::io::ErrorKind::TimedOut,
        ));
    }
    validate(hwnd, pid, names, &process, principal, image)
}
fn window_process(hwnd: HWND) -> Result<u32, WindowsNativeError> {
    let mut pid = 0;
    // SAFETY: borrowed HWND queried only for its kernel process owner.
    if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 || pid == 0 {
        return Err(last_error("instance window process"));
    }
    Ok(pid)
}
fn validate(
    hwnd: HWND,
    pid: u32,
    names: &Names,
    process: &OwnedHandle,
    principal: &Principal,
    image: &BoundPath,
) -> Result<(), WindowsNativeError> {
    // SAFETY: zero-time process wait; retained handle prevents PID identity substitution.
    if unsafe { WaitForSingleObject(process.as_raw_handle().cast(), 0) } != WAIT_TIMEOUT
        || window_process(hwnd)? != pid
        || &Principal::of(process.as_raw_handle().cast())? != principal
    {
        return Err(WindowsNativeError::IdentityChanged);
    }
    let mut class = [0u16; 256];
    // SAFETY: bounded caller-owned buffer for this window's immutable registered class.
    let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    if len <= 0 || class[..len as usize] != names.class.encode_utf16().collect::<Vec<_>>() {
        return Err(WindowsNativeError::IdentityChanged);
    }
    let mut path = vec![0u16; 32_768];
    let mut length = path.len() as u32;
    // SAFETY: query-only retained process and bounded destination.
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle().cast(),
            0,
            path.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return Err(last_error("instance peer image"));
    }
    use std::os::windows::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&path[..length as usize]));
    let peer = BoundPath::open_immutable_file(&path)?;
    image.revalidate()?;
    peer.revalidate()?;
    if peer.identity() != image.identity() || peer.canonical_path() != image.canonical_path() {
        return Err(WindowsNativeError::IdentityChanged);
    }
    // Repeat ownership after filesystem/token queries. The primary independently
    // pins this same image before endpoint publication and through retirement.
    if window_process(hwnd)? != pid {
        return Err(WindowsNativeError::IdentityChanged);
    }
    Ok(())
}
