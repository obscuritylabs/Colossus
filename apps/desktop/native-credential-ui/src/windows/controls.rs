//! Real native controls retain secure-edit and accessibility behavior.

use super::{CANCEL_ID, INPUT_ID, SAVE_ID, Session, input, wide};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_SETCUEBANNER, EM_SETLIMITTEXT},
        Input::KeyboardAndMouse::EnableWindow,
        Shell::SetWindowSubclass,
        WindowsAndMessaging::{
            BS_OWNERDRAW, CreateWindowExW, ES_AUTOHSCROLL, ES_PASSWORD, GetClientRect, MoveWindow,
            SendMessageW, WM_SETFONT, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
        },
    },
};

const SS_RIGHT: u32 = 2;

pub(super) unsafe fn create(window: HWND, pointer: *mut Session) -> bool {
    // Populate handles before fonts/layout can cause synchronous paint messages.
    unsafe {
        (*pointer).heading = control(window, "STATIC", "Save credential", 0, 0);
        (*pointer).description = control(
            window,
            "STATIC",
            "Your token is saved in the encrypted credential vault.",
            0,
            0,
        );
        (*pointer).label = control(window, "STATIC", "&Token", 0, 0);
        (*pointer).input = control(
            window,
            "EDIT",
            "",
            WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32,
            INPUT_ID,
        );
        (*pointer).status = control(window, "STATIC", "0 / 65,536 bytes", SS_RIGHT, 0);
        (*pointer).error = control(window, "STATIC", "", 0, 0);
        (*pointer).save = control(
            window,
            "BUTTON",
            "&Save",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            SAVE_ID,
        );
        (*pointer).cancel = control(
            window,
            "BUTTON",
            "&Cancel",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            CANCEL_ID,
        );
    }
    let session = unsafe { &*pointer };
    if [
        session.heading,
        session.description,
        session.label,
        session.input,
        session.status,
        session.error,
        session.save,
        session.cancel,
    ]
    .iter()
    .any(|handle| handle.is_null())
    {
        return false;
    }
    // The explicit filter rejects overflows before insertion. The native limit
    // must not silently truncate pasted tokens at the standard edit's 32K default.
    unsafe {
        SendMessageW(session.input, EM_SETLIMITTEXT, 0, 0);
        SendMessageW(
            session.input,
            EM_SETCUEBANNER,
            0,
            wide("Paste your token").as_ptr() as isize,
        );
    }
    for handle in [session.input, session.save, session.cancel] {
        if unsafe { SetWindowSubclass(handle, Some(input::control_proc), 1, pointer as usize) } == 0
        {
            return false;
        }
    }
    unsafe {
        layout(window, pointer);
        EnableWindow(session.save, 0);
    }
    true
}

pub(super) unsafe fn layout(window: HWND, pointer: *const Session) {
    let session = unsafe { &*pointer };
    let visual = &session.visuals;
    let mut client = RECT::default();
    if unsafe { GetClientRect(window, &raw mut client) } == 0 {
        return;
    }
    // Windows may cap an owned form to a small monitor/RDP desktop. Scale fonts
    // vertically as requested, but lay out horizontally in the actual client area.
    let left = visual.px(28);
    let content_width = (client.right - visual.px(56)).max(1);
    for (handle, bounds, font) in [
        (
            session.heading,
            [left, visual.px(28), content_width, visual.px(28)],
            visual.heading_font,
        ),
        (
            session.description,
            [left, visual.px(66), content_width, visual.px(34)],
            visual.body_font,
        ),
        (
            session.label,
            [left, visual.px(110), content_width, visual.px(20)],
            visual.body_font,
        ),
        (
            session.input,
            [
                visual.px(40),
                visual.px(148),
                (client.right - visual.px(80)).max(1),
                visual.px(20),
            ],
            visual.body_font,
        ),
        (
            session.status,
            [left, visual.px(188), content_width, visual.px(18)],
            visual.caption_font,
        ),
        (
            session.error,
            [left, visual.px(208), content_width, visual.px(28)],
            visual.caption_font,
        ),
        (
            session.save,
            [
                client.right - visual.px(260),
                visual.px(256),
                visual.px(108),
                visual.px(40),
            ],
            visual.body_font,
        ),
        (
            session.cancel,
            [
                client.right - visual.px(140),
                visual.px(256),
                visual.px(112),
                visual.px(40),
            ],
            visual.body_font,
        ),
    ] {
        if !handle.is_null() {
            unsafe {
                MoveWindow(handle, bounds[0], bounds[1], bounds[2], bounds[3], 1);
                SendMessageW(handle, WM_SETFONT, font as usize, 1);
            }
        }
    }
}

unsafe fn control(parent: HWND, class: &str, title: &str, style: u32, id: usize) -> HWND {
    unsafe {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(title).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            1,
            1,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null_mut(),
        )
    }
}
