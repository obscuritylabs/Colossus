//! A modeless, owned Win32 form driven by the existing Tauri event loop.

#[cfg(feature = "native-test-driver")]
pub(crate) mod acceptance;
mod controls;
mod input;
mod painting;
#[cfg(test)]
mod tests;
mod visuals;

use crate::{DialogAppearance, PromptError, lifecycle::Completion, validation};
use colossus_contracts::HostSecret;
use std::{
    cell::{Cell, RefCell},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        COLOR_WINDOW, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::{
            BN_CLICKED, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, EN_CHANGE,
            GWLP_USERDATA, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
            IDC_ARROW, IsWindow, KillTimer, LoadCursorW, RegisterClassW, SW_SHOW,
            SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, WM_CLOSE,
            WM_COMMAND, WM_CREATE, WM_DESTROY, WM_ENDSESSION, WM_NCCREATE, WM_NCDESTROY,
            WM_SYSCHAR, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_CLIPCHILDREN, WS_EX_DLGMODALFRAME,
            WS_SYSMENU,
        },
    },
};
use zeroize::Zeroizing;

const INPUT_ID: usize = 101;
const SAVE_ID: usize = 102;
const CANCEL_ID: usize = 103;
const POLL_TIMER: usize = 1;

struct Session {
    parent: HWND,
    heading: HWND,
    description: HWND,
    label: HWND,
    input: HWND,
    status: HWND,
    error: HWND,
    save: HWND,
    cancel: HWND,
    visuals: visuals::Visuals,
    has_error: Cell<bool>,
    hovered: Cell<HWND>,
    cancelled: Arc<AtomicBool>,
    created: Arc<AtomicBool>,
    completion: RefCell<Option<Completion>>,
    result: RefCell<Option<Result<HostSecret, PromptError>>>,
}

pub(crate) fn open(
    parent: &tauri::Window,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    appearance: DialogAppearance,
) {
    let Ok(handle) = parent.hwnd() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    let parent = handle.0.cast();
    // SAFETY: Called on Tauri's UI thread with a live native parent. The owned
    // window retains the boxed session until WM_NCDESTROY. No callback blocks.
    unsafe {
        create(parent, cancelled, completion, true, appearance);
    }
}

unsafe fn create(
    parent: HWND,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    visible: bool,
    appearance: DialogAppearance,
) -> Option<HWND> {
    if cancelled.load(Ordering::Acquire) || unsafe { IsWindow(parent) } == 0 {
        completion.finish(Err(PromptError::Cancelled));
        return None;
    }
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = wide("ColossusNativeCredentialEntryV1");
    let description = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as _,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // Re-registering the same process-local class is harmless.
    unsafe {
        RegisterClassW(&raw const description);
    }
    let created = Arc::new(AtomicBool::new(false));
    let visuals = unsafe { visuals::Visuals::new(parent, appearance) };
    if !visuals.available() {
        completion.finish(Err(PromptError::Unavailable));
        return None;
    }
    let (width, height) = visuals.outer_size();
    let session = Box::new(Session {
        parent,
        heading: null_mut(),
        description: null_mut(),
        label: null_mut(),
        input: null_mut(),
        status: null_mut(),
        error: null_mut(),
        save: null_mut(),
        cancel: null_mut(),
        visuals,
        has_error: Cell::new(false),
        hovered: Cell::new(null_mut()),
        cancelled,
        created: created.clone(),
        completion: RefCell::new(Some(completion)),
        result: RefCell::new(None),
    });
    let pointer = Box::into_raw(session);
    let (left, top) = unsafe { initial_position(parent, width, height) };
    let window = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            wide("Save a Colossus credential").as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            left,
            top,
            width,
            height,
            parent,
            null_mut(),
            instance,
            pointer.cast(),
        )
    };
    if window.is_null() {
        // WM_NCCREATE stores the pointer only after it accepts creation. Every
        // subsequent failure is handled by WM_NCDESTROY, which owns the box.
        if !created.load(Ordering::Acquire) {
            let session = unsafe { Box::from_raw(pointer) };
            if let Some(completion) = session.completion.borrow_mut().take() {
                completion.finish(Err(PromptError::Unavailable));
            }
        }
        return None;
    }
    unsafe {
        (*pointer).visuals.apply_titlebar(window);
        EnableWindow(parent, 0);
    }
    if visible {
        unsafe {
            ShowWindow(window, SW_SHOW);
            SetForegroundWindow(window);
            SetFocus((*pointer).input);
        }
    }
    Some(window)
}

unsafe fn initial_position(parent: HWND, width: i32, height: i32) -> (i32, i32) {
    let mut bounds = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).expect("monitor structure size"),
        ..Default::default()
    };
    if unsafe { GetWindowRect(parent, &raw mut bounds) } == 0
        || unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST),
                &raw mut monitor,
            )
        } == 0
    {
        return (0, 0);
    }
    let work = monitor.rcWork;
    let left = bounds.left + (bounds.right - bounds.left - width) / 2;
    let top = bounds.top + (bounds.bottom - bounds.top - height) / 2;
    (
        left.clamp(work.left, (work.right - width).max(work.left)),
        top.clamp(work.top, (work.bottom - height).max(work.top)),
    )
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: CREATESTRUCTW and its session pointer belong to create().
        let creation = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe {
            (*(creation.lpCreateParams.cast::<Session>()))
                .created
                .store(true, Ordering::Release);
        }
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, creation.lpCreateParams as isize);
        }
        // Preserve native non-client initialization, including the window title.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut Session;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    if let Some(result) = unsafe { painting::handle(window, message, wparam, lparam, pointer) } {
        return result;
    }
    match message {
        WM_SYSCHAR if unsafe { input::mnemonic(wparam, pointer) } => 0,
        WM_CREATE => {
            if !unsafe { controls::create(window, pointer) }
                || unsafe { SetTimer(window, POLL_TIMER, 100, None) } == 0
            {
                unsafe {
                    (*pointer)
                        .result
                        .replace(Some(Err(PromptError::Unavailable)));
                }
                return -1;
            }
            0
        }
        WM_COMMAND => {
            unsafe {
                command(window, wparam, pointer);
            }
            0
        }
        WM_TIMER => {
            if unsafe {
                (*pointer).cancelled.load(Ordering::Acquire) || IsWindow((*pointer).parent) == 0
            } {
                unsafe {
                    DestroyWindow(window);
                }
            }
            0
        }
        WM_CLOSE => {
            unsafe {
                DestroyWindow(window);
            }
            0
        }
        WM_ENDSESSION if wparam != 0 => {
            unsafe {
                DestroyWindow(window);
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                KillTimer(window, POLL_TIMER);
            }
            // Clear the native allocation before destroying children and releasing
            // a result to the awaiting caller. Empty text is allowed by our filter.
            unsafe {
                SetWindowTextW((*pointer).input, wide("").as_ptr());
            }
            if unsafe { IsWindow((*pointer).parent) } != 0 {
                unsafe {
                    EnableWindow((*pointer).parent, 1);
                }
            }
            0
        }
        WM_NCDESTROY => {
            unsafe {
                release_session(window, pointer);
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

unsafe fn command(window: HWND, wparam: WPARAM, pointer: *const Session) {
    let id = wparam & 0xffff;
    let notification = (wparam >> 16) & 0xffff;
    if id == SAVE_ID && notification == BN_CLICKED as usize {
        unsafe {
            save(window, pointer);
        }
    } else if id == CANCEL_ID && notification == BN_CLICKED as usize {
        unsafe {
            DestroyWindow(window);
        }
    } else if id == INPUT_ID && notification == EN_CHANGE as usize {
        unsafe {
            update_count(pointer);
        }
    }
}

unsafe fn release_session(window: HWND, pointer: *mut Session) {
    unsafe {
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
    }
    let session = unsafe { Box::from_raw(pointer) };
    let result = session
        .result
        .borrow_mut()
        .take()
        .unwrap_or(Err(PromptError::Cancelled));
    if let Some(completion) = session.completion.borrow_mut().take() {
        completion.finish(result);
    }
}

unsafe fn update_count(pointer: *const Session) {
    let length = unsafe { GetWindowTextLengthW((*pointer).input) }.max(0);
    let message = format!("{length} / 65,536 bytes");
    unsafe {
        (*pointer).has_error.set(false);
        SetWindowTextW((*pointer).status, wide(&message).as_ptr());
        SetWindowTextW((*pointer).error, wide("").as_ptr());
        EnableWindow((*pointer).save, i32::from(length > 0));
        painting::invalidate_input(pointer);
    }
}

unsafe fn save(window: HWND, pointer: *const Session) {
    let length = unsafe { GetWindowTextLengthW((*pointer).input) };
    let count = usize::try_from(length).unwrap_or(0);
    if count == 0 || count > colossus_contracts::MAX_HOST_SECRET_BYTES {
        unsafe {
            input::error(pointer, validation::InputError::Empty);
        }
        return;
    }
    let mut units = Zeroizing::new(vec![0_u16; count + 1]);
    let copied = unsafe { GetWindowTextW((*pointer).input, units.as_mut_ptr(), length + 1) };
    if copied != length {
        return;
    }
    let Ok(decoded) = String::from_utf16(&units[..count]) else {
        return;
    };
    let mut secret = Zeroizing::new(decoded);
    if let Err(error) = validation::validate(&secret) {
        unsafe {
            input::error(pointer, error);
        }
        return;
    }
    if let Ok(secret) = HostSecret::new(std::mem::take(&mut *secret)) {
        unsafe {
            (*pointer).result.replace(Some(Ok(secret)));
            DestroyWindow(window);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
