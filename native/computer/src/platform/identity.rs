//! Native identity, interactive desktop, integrity and selected-window geometry.
use super::Error;
use crate::{NativeProcessIdentity, TrustedComputerContext, geometry::bounded};
use colossus_contracts::ComputerRect;
use serde::{Deserialize, Serialize};
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HWND, RECT},
    Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        RemoteDesktop::ProcessIdToSessionId,
        StationsAndDesktops::{
            CloseDesktop, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW,
            OpenInputDesktop, UOI_NAME,
        },
        Threading::{
            GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            GA_ROOT, GW_OWNER, GetAncestor, GetForegroundWindow, GetSystemMetrics, GetWindow,
            GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
            SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        },
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Window {
    pub handle: u64,
    pub process: NativeProcessIdentity,
    pub rect: ComputerRect,
    pub dpi: u32,
    pub desktop: ComputerRect,
}
impl Window {
    pub fn hwnd(&self) -> HWND {
        self.handle as usize as HWND
    }
    pub fn revalidate(&self, context: &TrustedComputerContext) -> Result<(), Error> {
        if &inspect(self.hwnd(), context)? != self {
            return Err(Error::Stale);
        }
        Ok(())
    }
}

pub(crate) fn revalidate_admission(
    root: &Window,
    target: &Window,
    context: &TrustedComputerContext,
) -> Result<(), Error> {
    root.revalidate(context)?;
    if target != root {
        target.revalidate(context)?;
        if !owned_modal(root, target) {
            return Err(Error::Denied);
        }
    }
    Ok(())
}

pub(crate) fn process(pid: u32) -> Result<NativeProcessIdentity, Error> {
    // SAFETY: query-only handle; owned until process times are copied.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(Error::Denied);
    }
    // SAFETY: successful OpenProcess transfers ownership.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
    let mut birth = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: valid retained process handle and stack outputs.
    if unsafe {
        GetProcessTimes(
            owned.as_raw_handle().cast(),
            &mut birth,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(Error::Denied);
    }
    Ok(NativeProcessIdentity {
        pid,
        birth: (u64::from(birth.dwHighDateTime) << 32) | u64::from(birth.dwLowDateTime),
    })
}

pub(crate) fn desktop() -> Result<(), Error> {
    // SAFETY: query current interactive desktop only, never switch to secure desktop.
    let input = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if input.is_null() {
        return Err(Error::Unavailable);
    }
    let mut name = [0u16; 256];
    let mut needed = 0;
    // SAFETY: bounded writable UTF16 name buffer, input is retained until close.
    let ok = unsafe {
        GetUserObjectInformationW(input, UOI_NAME, name.as_mut_ptr().cast(), 512, &mut needed)
    };
    // SAFETY: own the opened desktop.
    unsafe { CloseDesktop(input) };
    if ok == 0
        || String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(256)])
            != "Default"
    {
        return Err(Error::Unavailable);
    }
    // A helper must itself remain on the same ordinary input desktop.
    let mut thread_name = [0u16; 256];
    // SAFETY: borrowed current desktop and bounded output.
    let current =
        unsafe { GetThreadDesktop(windows_sys::Win32::System::Threading::GetCurrentThreadId()) };
    // SAFETY: borrowed thread desktop, stack name buffer.
    if unsafe {
        GetUserObjectInformationW(
            current,
            UOI_NAME,
            thread_name.as_mut_ptr().cast(),
            512,
            &mut needed,
        )
    } == 0
        || thread_name != name
    {
        return Err(Error::Unavailable);
    }
    Ok(())
}

fn allowed_process(
    identity: &NativeProcessIdentity,
    context: &TrustedComputerContext,
) -> Result<(), Error> {
    if identity.pid == std::process::id() || context.protected.contains(identity) {
        return Err(Error::Denied);
    }
    let mut target_session = 0;
    let mut own_session = 0;
    // SAFETY: query-only process session IDs and stack outputs.
    let target_ok = unsafe { ProcessIdToSessionId(identity.pid, &mut target_session) };
    // SAFETY: own process identifier and valid stack output.
    let own_ok = unsafe { ProcessIdToSessionId(std::process::id(), &mut own_session) };
    if target_ok == 0 || own_ok == 0 || target_session == 0 || target_session != own_session {
        return Err(Error::Denied);
    }
    ordinary_process(identity.pid)?;
    for parent in ancestors(identity.pid)? {
        if context
            .protected
            .iter()
            .any(|p| p.pid == parent && process(parent).ok().as_ref() == Some(p))
        {
            return Err(Error::Denied);
        }
    }
    Ok(())
}

pub(crate) fn ordinary_host() -> Result<(), Error> {
    desktop()?;
    ordinary_process(std::process::id())
}

fn ordinary_process(pid: u32) -> Result<(), Error> {
    // SAFETY: query process token to reject elevated targets and hosts.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(Error::Denied);
    }
    let mut token = null_mut();
    // SAFETY: valid query process and token output.
    let opened = unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut token) };
    // SAFETY: own process handle.
    unsafe { CloseHandle(handle) };
    if opened == 0 {
        return Err(Error::Denied);
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut bytes = 0;
    // SAFETY: token retained and fixed elevation output fits the given size.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut bytes,
        )
    };
    // SAFETY: own token.
    unsafe { CloseHandle(token) };
    if ok == 0 || elevation.TokenIsElevated != 0 {
        return Err(Error::Denied);
    }
    Ok(())
}

pub(crate) fn ancestors(pid: u32) -> Result<Vec<u32>, Error> {
    // SAFETY: read-only process snapshot; no window or effect authority is granted.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(Error::Failed);
    }
    // SAFETY: successful snapshot transfers ownership.
    let owned = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = std::collections::HashMap::new();
    // SAFETY: initialized correctly sized entry and retained snapshot.
    let mut more = unsafe { Process32FirstW(raw, &mut entry) };
    while more != 0 && parents.len() < 65536 {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        // SAFETY: retained snapshot and initialized fixed-size entry remain valid.
        more = unsafe { Process32NextW(raw, &mut entry) };
    }
    drop(owned);
    let mut result = Vec::new();
    let mut next = pid;
    for _ in 0..32 {
        let Some(parent) = parents.get(&next).copied() else {
            break;
        };
        if parent == 0 || parent == next || result.contains(&parent) {
            break;
        }
        result.push(parent);
        next = parent;
    }
    Ok(result)
}

pub(crate) fn inspect(hwnd: HWND, context: &TrustedComputerContext) -> Result<Window, Error> {
    desktop()?;
    // SAFETY: only queries a borrowed candidate window; identity is rechecked by each operation.
    let unavailable = unsafe {
        IsWindow(hwnd) == 0
            || IsWindowVisible(hwnd) == 0
            || IsIconic(hwnd) != 0
            || GetAncestor(hwnd, GA_ROOT) != hwnd
    };
    if unavailable {
        return Err(Error::Denied);
    }
    let mut cloaked = 0u32;
    // SAFETY: fixed DWM attribute output, no fallback to occluding desktop pixels.
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            4,
        )
    } < 0
        || cloaked != 0
    {
        return Err(Error::Denied);
    }
    let mut pid = 0;
    // SAFETY: borrowed HWND query and valid pid output.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let process = process(pid)?;
    allowed_process(&process, context)?;
    let mut bounds = RECT::default();
    // SAFETY: DWM physical visible frame bounds exclude invisible resize borders.
    if unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut bounds as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    } < 0
    {
        return Err(Error::Unavailable);
    }
    let width = u32::try_from(i64::from(bounds.right) - i64::from(bounds.left))
        .map_err(|_| Error::Denied)?;
    let height = u32::try_from(i64::from(bounds.bottom) - i64::from(bounds.top))
        .map_err(|_| Error::Denied)?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        return Err(Error::Denied);
    }
    // SAFETY: window DPI and virtual desktop are native-produced placement facts.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    // SAFETY: fixed system metrics contain native virtual-desktop placement only.
    let desktop = unsafe {
        ComputerRect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32,
        }
    };
    Ok(Window {
        handle: hwnd as usize as u64,
        process,
        rect: ComputerRect {
            x: bounds.left,
            y: bounds.top,
            width,
            height,
        },
        dpi,
        desktop,
    })
}

pub(crate) fn foreground() -> HWND {
    // SAFETY: read-only query, returned handle is borrowed and revalidated later.
    unsafe { GetForegroundWindow() }
}
pub(crate) fn title(hwnd: HWND) -> String {
    let mut text = [0u16; 513];
    // SAFETY: bounded caption buffer; this is not an in-process control-value read.
    let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), 513) };
    bounded(&String::from_utf16_lossy(&text[..len.max(0) as usize]), 512)
}
pub(crate) fn owned_modal(root: &Window, candidate: &Window) -> bool {
    if root.process != candidate.process {
        return false;
    }
    let mut next = candidate.hwnd();
    for _ in 0..16 {
        if next == root.hwnd() {
            return true;
        }
        // SAFETY: borrowed owner-chain query; no HWND is accepted from a caller.
        next = unsafe { GetWindow(next, GW_OWNER) };
        if next.is_null() {
            break;
        }
    }
    false
}
