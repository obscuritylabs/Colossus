//! Appearance updates must preserve the real secure edit and keyboard state.

use super::{Dialog, Session, key, read_text};
use crate::{ColorScheme, DialogAppearance, TextSize};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{ClientToScreen, UpdateWindow},
    UI::{
        Controls::EM_GETPASSWORDCHAR,
        Input::KeyboardAndMouse::GetFocus,
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, GetWindowRect, MINMAXINFO,
            SendMessageW, WM_DPICHANGED, WM_GETMINMAXINFO, WM_THEMECHANGED,
        },
    },
};

pub(super) fn run() {
    for color_scheme in [ColorScheme::Light, ColorScheme::Dark] {
        for text_size in [TextSize::Compact, TextSize::Comfortable, TextSize::Large] {
            let mut dialog = Dialog::with_appearance(DialogAppearance {
                color_scheme,
                text_size,
            });
            // Reproduce the limited desktop width used by headless/RDP hosts,
            // independently of the developer's physical monitor resolution.
            assert_ne!(
                unsafe { SetWindowSubclass(dialog.window, Some(limit_width), 2, 0) },
                0
            );
            dialog.paste("SYNTHETIC-APPEARANCE");
            let session =
                unsafe { GetWindowLongPtrW(dialog.window, GWLP_USERDATA) } as *const Session;
            for dpi in [96_usize, 144, 192, 120] {
                let proposed = RECT {
                    left: 0,
                    top: 0,
                    right: 1200,
                    bottom: 900,
                };
                unsafe {
                    SendMessageW(
                        dialog.window,
                        WM_DPICHANGED,
                        dpi | (dpi << 16),
                        &raw const proposed as isize,
                    );
                    SendMessageW(dialog.window, WM_THEMECHANGED, 0, 0);
                    UpdateWindow(dialog.window);
                }
                assert_eq!(read_text(dialog.input), "SYNTHETIC-APPEARANCE");
                assert_eq!(read_text(dialog.status), "20 / 65,536 bytes");
                assert_eq!(unsafe { GetFocus() }, dialog.input);
                assert_ne!(
                    unsafe { SendMessageW(dialog.input, EM_GETPASSWORDCHAR, 0, 0) },
                    0
                );
                assert_layout(dialog.window, unsafe { &*session });
            }
            key(dialog.input, 0x0d, None);
            dialog.saved("SYNTHETIC-APPEARANCE");
        }
    }
}

fn assert_layout(window: HWND, session: &Session) {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    assert_ne!(unsafe { GetClientRect(window, &raw mut client) }, 0);
    assert_ne!(unsafe { ClientToScreen(window, &raw mut origin) }, 0);
    let mut previous_bottom = origin.y;
    for (name, handle) in [
        ("heading", session.heading),
        ("description", session.description),
        ("label", session.label),
        ("input", session.input),
        ("count", session.status),
        ("error", session.error),
        ("save", session.save),
    ] {
        let bounds = bounds(handle);
        assert!(
            bounds.left >= origin.x && bounds.right <= origin.x + client.right,
            "{name} horizontal bounds {:?} outside client {:?} at DPI {}",
            (bounds.left, bounds.right),
            (origin.x, origin.x + client.right),
            session.visuals.dpi,
        );
        assert!(bounds.top >= previous_bottom && bounds.bottom <= origin.y + client.bottom);
        assert!(bounds.right > bounds.left && bounds.bottom > bounds.top);
        previous_bottom = bounds.bottom;
    }
    let save = bounds(session.save);
    let cancel = bounds(session.cancel);
    assert!(cancel.left > save.right && cancel.right <= origin.x + client.right);
    assert_eq!((save.top, save.bottom), (cancel.top, cancel.bottom));
}

unsafe extern "system" fn limit_width(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if message == WM_GETMINMAXINFO {
        unsafe { (*(lparam as *mut MINMAXINFO)).ptMaxTrackSize.x = 1024 };
    }
    result
}

fn bounds(window: HWND) -> RECT {
    let mut value = RECT::default();
    assert_ne!(unsafe { GetWindowRect(window, &raw mut value) }, 0);
    value
}
