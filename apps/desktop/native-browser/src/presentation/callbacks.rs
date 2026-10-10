use super::{Owner, ffi};
use colossus_browser_presentation::Input;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    time::Instant,
};

pub(super) unsafe extern "C" fn focus(owner: *mut c_void, focused: i32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if owner.is_null() {
            return;
        }
        // SAFETY: native view retains an Arc at callback entry. This extra retain
        // keeps reentrant destruction from freeing the owner during the callback.
        let owner = unsafe {
            Arc::increment_strong_count(owner.cast::<Owner>());
            Arc::from_raw(owner.cast::<Owner>())
        };
        let Ok(mut guard) = owner.guard.lock() else {
            return;
        };
        let focused = focused == 1;
        if guard.focus(focused, Instant::now()).is_ok() {
            let lease = guard.lease();
            drop(guard);
            (owner.focus)(lease, focused);
        }
    }));
}
pub(super) unsafe extern "C" fn input(owner: *mut c_void, raw: *const ffi::Input) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if owner.is_null() {
            return;
        }
        // SAFETY: native owner retain is live at callback entry; an independent
        // callback retain survives reentrant destruction of the native view.
        let owner = unsafe {
            Arc::increment_strong_count(owner.cast::<Owner>());
            Arc::from_raw(owner.cast::<Owner>())
        };
        // SAFETY: native synchronous callback supplies this borrowed input record.
        let Some(raw) = (unsafe { raw.as_ref() }) else {
            return;
        };
        if raw.version != 1 || raw.modifiers & !0x1fff != 0 {
            return;
        }
        let input = match raw.kind {
            1 => Input::MouseMove {
                x: u32::try_from(raw.x).unwrap_or(u32::MAX),
                y: u32::try_from(raw.y).unwrap_or(u32::MAX),
            },
            2 | 3 => Input::MouseButton {
                x: u32::try_from(raw.x).unwrap_or(u32::MAX),
                y: u32::try_from(raw.y).unwrap_or(u32::MAX),
                button: u8::try_from(raw.button).unwrap_or(u8::MAX),
                pressed: raw.kind == 2,
            },
            4 => Input::MouseWheel {
                x: u32::try_from(raw.x).unwrap_or(u32::MAX),
                y: u32::try_from(raw.y).unwrap_or(u32::MAX),
                delta_x: raw.wheel_x,
                delta_y: raw.wheel_y,
            },
            5 | 6 => {
                let Ok(code) = u16::try_from(raw.key_code) else {
                    return;
                };
                Input::Key {
                    code,
                    pressed: raw.kind == 5,
                }
            }
            7 => {
                let Some(character) = u32::try_from(raw.key_code).ok().and_then(char::from_u32)
                else {
                    return;
                };
                Input::Character {
                    text: character.to_string(),
                }
            }
            8 => {
                if raw.text.is_null() || raw.text_units == 0 || raw.text_units > 4096 {
                    return;
                }
                // SAFETY: native callback borrows this bounded UTF16 buffer only during this call.
                let units = unsafe { std::slice::from_raw_parts(raw.text, raw.text_units) };
                let Ok(text) = String::from_utf16(units) else {
                    return;
                };
                Input::ImeCommit { text }
            }
            9 => Input::ImeCancel,
            _ => return,
        };
        let Ok(guard) = owner.guard.lock() else {
            return;
        };
        if guard
            .authorize_input(guard.lease(), &input, Instant::now())
            .is_ok()
        {
            let lease = guard.lease();
            drop(guard);
            (owner.input)(lease, input, raw.modifiers);
        }
    }));
}
