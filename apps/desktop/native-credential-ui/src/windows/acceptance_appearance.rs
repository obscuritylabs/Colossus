//! Appearance updates must preserve the real secure edit and keyboard state.

use super::{Dialog, Session, key, read_text};
use crate::{ColorScheme, DialogAppearance, TextSize};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::{ClientToScreen, UpdateWindow},
    UI::{
        Controls::EM_GETPASSWORDCHAR,
        Input::KeyboardAndMouse::GetFocus,
        WindowsAndMessaging::{
            GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, GetWindowRect, SendMessageW,
            WM_DPICHANGED, WM_THEMECHANGED,
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
    for handle in [
        session.heading,
        session.description,
        session.label,
        session.input,
        session.status,
        session.error,
        session.save,
    ] {
        let bounds = bounds(handle);
        assert!(bounds.left >= origin.x && bounds.right <= origin.x + client.right);
        assert!(bounds.top >= previous_bottom && bounds.bottom <= origin.y + client.bottom);
        assert!(bounds.right > bounds.left && bounds.bottom > bounds.top);
        previous_bottom = bounds.bottom;
    }
    let save = bounds(session.save);
    let cancel = bounds(session.cancel);
    assert!(cancel.left > save.right && cancel.right <= origin.x + client.right);
    assert_eq!((save.top, save.bottom), (cancel.top, cancel.bottom));
}

fn bounds(window: HWND) -> RECT {
    let mut value = RECT::default();
    assert_ne!(unsafe { GetWindowRect(window, &raw mut value) }, 0);
    value
}
