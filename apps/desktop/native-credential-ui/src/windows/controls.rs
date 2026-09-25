use super::{CANCEL_ID, INPUT_ID, SAVE_ID, Session, input, wide};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_SETCUEBANNER, EM_SETLIMITTEXT},
        Input::KeyboardAndMouse::EnableWindow,
        Shell::SetWindowSubclass,
        WindowsAndMessaging::{
            BS_DEFPUSHBUTTON, CreateWindowExW, ES_AUTOHSCROLL, ES_PASSWORD, SendMessageW,
            WM_SETFONT, WS_BORDER, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
        },
    },
};

pub(super) unsafe fn create(window: HWND, pointer: *mut Session) -> bool {
    let label = unsafe { control(window, "STATIC", "&Token", 0, 22, 18, 570, 24, 0) };
    let input = unsafe {
        control(
            window,
            "EDIT",
            "",
            WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_BORDER,
            22,
            48,
            570,
            30,
            INPUT_ID,
        )
    };
    let status = unsafe { control(window, "STATIC", "0 / 65,536 bytes", 0, 22, 88, 570, 46, 0) };
    let save = unsafe {
        control(
            window,
            "BUTTON",
            "&Save",
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            384,
            151,
            100,
            32,
            SAVE_ID,
        )
    };
    let cancel = unsafe {
        control(
            window, "BUTTON", "&Cancel", WS_TABSTOP, 492, 151, 100, 32, CANCEL_ID,
        )
    };
    if [label, input, status, save, cancel]
        .iter()
        .any(|handle| handle.is_null())
    {
        return false;
    }
    unsafe {
        (*pointer).input = input;
        (*pointer).status = status;
        (*pointer).save = save;
        (*pointer).cancel = cancel;
    }
    // The explicit filter rejects overflows before insertion. This large native
    // limit prevents the standard edit control's default 32K silent truncation.
    unsafe {
        SendMessageW(input, EM_SETLIMITTEXT, 0, 0);
    }
    unsafe {
        SendMessageW(
            input,
            EM_SETCUEBANNER,
            0,
            wide("Paste your credential").as_ptr() as isize,
        );
    }
    for handle in [input, save, cancel] {
        if unsafe { SetWindowSubclass(handle, Some(input::control_proc), 1, pointer as usize) } == 0
        {
            return false;
        }
    }
    unsafe {
        EnableWindow(save, 0);
    }
    true
}

#[allow(clippy::too_many_arguments)]
unsafe fn control(
    parent: HWND,
    class: &str,
    title: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
) -> HWND {
    let handle = unsafe {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(title).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            width,
            height,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    };
    if !handle.is_null() {
        unsafe {
            SendMessageW(
                handle,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
    }
    handle
}
