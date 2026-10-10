use super::{CANCEL_ID, SAVE_ID, Session, painting, wide};
use crate::purpose::Purpose;
use crate::validation::{InputError, validate_units};
use colossus_contracts::MAX_HOST_SECRET_BYTES;
use std::slice;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    },
    UI::{
        Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL},
        Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT},
        Shell::{DefSubclassProc, RemoveWindowSubclass},
        WindowsAndMessaging::{
            GetParent, GetWindowTextLengthW, GetWindowTextW, PostMessageW, SendMessageW,
            SetWindowTextW, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_KEYDOWN, WM_NCDESTROY, WM_PASTE,
            WM_SETTEXT, WM_SYSCHAR,
        },
    },
};
use zeroize::Zeroizing;

pub(super) unsafe fn error(pointer: *const Session, error: InputError) {
    unsafe {
        (*pointer).has_error.set(true);
        SetWindowTextW((*pointer).error, wide(error.message()).as_ptr());
        painting::invalidate_input(pointer);
    }
}

unsafe fn replacement_length(window: HWND, inserted: usize) -> usize {
    let mut start = 0_u32;
    let mut end = 0_u32;
    unsafe {
        SendMessageW(
            window,
            EM_GETSEL,
            &raw mut start as usize,
            &raw mut end as isize,
        );
    }
    let existing = usize::try_from(unsafe { GetWindowTextLengthW(window) }).unwrap_or(0);
    existing
        .saturating_sub(end.saturating_sub(start) as usize)
        .saturating_add(inserted)
}

unsafe fn current_units(window: HWND) -> Result<Zeroizing<Vec<u16>>, InputError> {
    let length = usize::try_from(unsafe { GetWindowTextLengthW(window) })
        .map_err(|_| InputError::InvalidPasswordCharacter)?;
    if length > MAX_HOST_SECRET_BYTES {
        return Err(InputError::PasswordTooLong);
    }
    let mut units = Zeroizing::new(vec![0; length + 1]);
    let capacity = i32::try_from(units.len()).map_err(|_| InputError::PasswordTooLong)?;
    let copied = unsafe { GetWindowTextW(window, units.as_mut_ptr(), capacity) };
    if usize::try_from(copied).ok() != Some(length) {
        return Err(InputError::InvalidPasswordCharacter);
    }
    units.truncate(length);
    Ok(units)
}

pub(super) unsafe fn current_password_bytes(window: HWND) -> Result<usize, InputError> {
    let units = unsafe { current_units(window) }?;
    crate::validation::password_units_bytes(units.iter().copied(), units.len(), false)
}

unsafe fn validate_edit(
    window: HWND,
    pointer: *const Session,
    inserted: &[u16],
    replace_all: bool,
    partial: bool,
) -> Result<(), InputError> {
    if unsafe { (*pointer).purpose } == Purpose::Token {
        let length = if replace_all {
            inserted.len()
        } else {
            unsafe { replacement_length(window, inserted.len()) }
        };
        return validate_units(inserted.iter().copied(), length);
    }
    let mut existing = unsafe { current_units(window) }?;
    let (start, end) = if replace_all {
        (0, existing.len())
    } else {
        let (mut start, mut end) = (0_u32, 0_u32);
        unsafe {
            SendMessageW(
                window,
                EM_GETSEL,
                &raw mut start as usize,
                &raw mut end as isize,
            );
        }
        (
            (start as usize).min(existing.len()),
            (end as usize).min(existing.len()),
        )
    };
    let length = existing
        .len()
        .saturating_sub(end.saturating_sub(start))
        .saturating_add(inserted.len());
    if length > MAX_HOST_SECRET_BYTES {
        return Err(InputError::PasswordTooLong);
    }
    existing.splice(start..end, inserted.iter().copied());
    crate::validation::password_units_bytes(existing.iter().copied(), existing.len(), partial)
        .map(|_| ())
}

pub(super) unsafe extern "system" fn control_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let pointer = data as *const Session;

    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(window, Some(control_proc), id);
        }
        return unsafe { DefSubclassProc(window, message, wparam, lparam) };
    }
    unsafe {
        painting::control_state(window, message, pointer);
    }
    if message == WM_SYSCHAR && unsafe { mnemonic(wparam, pointer) } {
        return 0;
    }
    if message == WM_KEYDOWN && unsafe { keyboard(window, wparam, pointer) } {
        return 0;
    }
    if window != unsafe { (*pointer).input } {
        return unsafe { DefSubclassProc(window, message, wparam, lparam) };
    }
    match message {
        WM_PASTE => {
            unsafe {
                paste(window, pointer);
            }
            0
        }
        WM_CHAR => {
            if [1, 3, 22, 24, 0x09, 0x0d, 0x1b].contains(&wparam) {
                return 0;
            }
            if wparam == 8 {
                return unsafe { DefSubclassProc(window, message, wparam, lparam) };
            }
            match u16::try_from(wparam)
                .map_err(|_| InputError::InvalidCharacter)
                .and_then(|unit| unsafe { validate_edit(window, pointer, &[unit], false, true) })
            {
                Ok(()) => unsafe { DefSubclassProc(window, message, wparam, lparam) },
                Err(reason) => {
                    unsafe {
                        error(pointer, reason);
                    }
                    0
                }
            }
        }
        WM_SETTEXT | EM_REPLACESEL => {
            // These messages carry native NUL-terminated UTF-16. Never scan past
            // the bound merely to learn how oversized an input is.
            let raw = lparam as *const u16;
            let units = if raw.is_null() {
                Some(&[][..])
            } else {
                let end =
                    (0..=MAX_HOST_SECRET_BYTES).find(|index| unsafe { *raw.add(*index) } == 0);
                end.map(|length| unsafe { slice::from_raw_parts(raw, length) })
            };
            let result = units.ok_or(InputError::TooLong).and_then(|units| unsafe {
                validate_edit(window, pointer, units, message == WM_SETTEXT, false)
            });
            match result {
                Ok(()) => unsafe { DefSubclassProc(window, message, wparam, lparam) },
                Err(reason) => {
                    unsafe {
                        error(pointer, reason);
                    }
                    0
                }
            }
        }
        _ => unsafe { DefSubclassProc(window, message, wparam, lparam) },
    }
}

unsafe fn paste(window: HWND, pointer: *const Session) {
    if unsafe { OpenClipboard(window) } == 0 {
        return;
    }
    let handle = unsafe { GetClipboardData(13) }; // CF_UNICODETEXT
    if handle.is_null() {
        unsafe {
            CloseClipboard();
        }
        return;
    }
    let allocation_units = unsafe { GlobalSize(handle) } / 2;
    let raw = unsafe { GlobalLock(handle) } as *const u16;
    if raw.is_null() {
        unsafe {
            CloseClipboard();
        }
        return;
    }
    let available = allocation_units.min(MAX_HOST_SECRET_BYTES + 1);
    let units = unsafe { slice::from_raw_parts(raw, available) };
    let result = units
        .iter()
        .position(|unit| *unit == 0)
        .ok_or(InputError::TooLong)
        .and_then(|end| {
            unsafe { validate_edit(window, pointer, &units[..end], false, false) }?;
            Ok(Zeroizing::new(units[..=end].to_vec()))
        });
    unsafe {
        GlobalUnlock(handle);
        CloseClipboard();
    }
    match result {
        Ok(units) => unsafe {
            DefSubclassProc(window, EM_REPLACESEL, 0, units.as_ptr() as isize);
        },
        Err(reason) => unsafe {
            error(pointer, reason);
        },
    }
}

unsafe fn keyboard(window: HWND, wparam: WPARAM, pointer: *const Session) -> bool {
    let parent = unsafe { GetParent(window) };

    if window == unsafe { (*pointer).input } && unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0 {
        if wparam == 0x41 {
            unsafe {
                SendMessageW(window, EM_SETSEL, 0, -1);
            }
            return true;
        }
        if wparam == 0x56 {
            unsafe {
                paste(window, pointer);
            }
            return true;
        }
    }
    match wparam {
        0x1b => {
            unsafe {
                PostMessageW(parent, WM_CLOSE, 0, 0);
            }
            return true;
        }
        0x0d => {
            let command = if window == unsafe { (*pointer).cancel } {
                CANCEL_ID
            } else {
                SAVE_ID
            };
            unsafe {
                PostMessageW(parent, WM_COMMAND, command, 0);
            }
            return true;
        }
        0x09 => {
            let handles = unsafe { [(*pointer).input, (*pointer).save, (*pointer).cancel] };
            let current = handles
                .iter()
                .position(|handle| *handle == window)
                .unwrap_or(0);
            let backwards = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
            let step = if backwards { 2 } else { 1 };
            let mut next = (current + step) % 3;
            if unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(handles[next])
            } == 0
            {
                next = (next + step) % 3;
            }
            unsafe {
                SetFocus(handles[next]);
            }
            return true;
        }
        _ => {}
    }

    false
}

pub(super) unsafe fn mnemonic(character: WPARAM, pointer: *const Session) -> bool {
    let Ok(character) = u8::try_from(character) else {
        return false;
    };
    let session = unsafe { &*pointer };
    let (input_key, confirm_key) = match session.purpose {
        Purpose::Token => (b't', b's'),
        Purpose::Pkcs12Password => (b'p', b'o'),
    };
    match character.to_ascii_lowercase() {
        character if character == input_key => unsafe {
            SetFocus(session.input);
        },
        character if character == b'c' || character == confirm_key => {
            let command = if character.eq_ignore_ascii_case(&b'c') {
                CANCEL_ID
            } else {
                SAVE_ID
            };
            unsafe {
                PostMessageW(GetParent(session.input), WM_COMMAND, command, 0);
            }
        }
        _ => return false,
    }
    true
}
