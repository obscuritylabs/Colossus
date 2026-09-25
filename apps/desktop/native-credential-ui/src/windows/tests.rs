//! Native handle tests exercise real controls without opening visible windows.

use super::*;
use crate::lifecycle::TEST_OWNERSHIP;
use windows_sys::Win32::UI::{
    Controls::{EM_REPLACESEL, EM_SETSEL},
    WindowsAndMessaging::{GetDlgItem, SendMessageW},
};

struct Parent(HWND);

impl Parent {
    fn new() -> Self {
        let window = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Synthetic native credential test").as_ptr(),
                0,
                0,
                0,
                600,
                260,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!window.is_null());
        Self(window)
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

#[test]
fn native_edit_accepts_exact_boundary_values_without_default_32k_truncation() {
    let _ownership = TEST_OWNERSHIP.lock().unwrap();
    for length in [761, 762, 2_560, 2_561, 8_192, 65_536] {
        let parent = Parent::new();
        let (completion, mut result) = Completion::acquire().unwrap();
        let window = unsafe {
            create(
                parent.0,
                Arc::new(AtomicBool::new(false)),
                completion,
                false,
            )
        }
        .unwrap();
        let input = unsafe { GetDlgItem(window, i32::try_from(INPUT_ID).unwrap()) };
        let token = format!("{}END", "X".repeat(length - 3));
        assert_ne!(unsafe { SetWindowTextW(input, wide(&token).as_ptr()) }, 0);
        assert_eq!(
            unsafe { GetWindowTextLengthW(input) },
            i32::try_from(length).unwrap()
        );
        unsafe {
            SendMessageW(window, WM_COMMAND, SAVE_ID, 0);
        }
        let saved = result.try_recv().unwrap().unwrap();
        assert_eq!(saved.expose(), token);
        assert_eq!(unsafe { IsWindow(window) }, 0);
    }
}

#[test]
fn native_replacement_rejects_overflow_and_invalid_characters_without_accepting_a_prefix() {
    let _ownership = TEST_OWNERSHIP.lock().unwrap();
    let parent = Parent::new();
    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe {
        create(
            parent.0,
            Arc::new(AtomicBool::new(false)),
            completion,
            false,
        )
    }
    .unwrap();
    let input = unsafe { GetDlgItem(window, i32::try_from(INPUT_ID).unwrap()) };
    unsafe {
        SetWindowTextW(input, wide("KEEP").as_ptr());
    }
    for rejected in [
        "X".repeat(65_537),
        "bad token".to_owned(),
        "line\nbreak".to_owned(),
        "é".to_owned(),
    ] {
        unsafe {
            SetWindowTextW(input, wide(&rejected).as_ptr());
        }
        assert_eq!(unsafe { GetWindowTextLengthW(input) }, 4);
    }
    unsafe {
        SendMessageW(input, EM_SETSEL, 4, 4);
        SendMessageW(
            input,
            EM_REPLACESEL,
            0,
            wide(&"X".repeat(65_533)).as_ptr() as isize,
        );
    }
    assert_eq!(unsafe { GetWindowTextLengthW(input) }, 4);
    unsafe {
        SendMessageW(window, WM_COMMAND, SAVE_ID, 0);
    }
    assert_eq!(result.try_recv().unwrap().unwrap().expose(), "KEEP");
}

#[test]
fn parent_destruction_and_async_cancellation_close_owned_native_dialogs() {
    let _ownership = TEST_OWNERSHIP.lock().unwrap();
    let parent = Parent::new();
    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe {
        create(
            parent.0,
            Arc::new(AtomicBool::new(false)),
            completion,
            false,
        )
    }
    .unwrap();
    drop(parent);
    assert_eq!(unsafe { IsWindow(window) }, 0);
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));

    let parent = Parent::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe { create(parent.0, cancelled.clone(), completion, false) }.unwrap();
    cancelled.store(true, Ordering::Release);
    unsafe {
        SendMessageW(window, WM_TIMER, POLL_TIMER, 0);
    }
    assert_eq!(unsafe { IsWindow(window) }, 0);
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
    assert_ne!(
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(parent.0) },
        0
    );

    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe {
        create(
            parent.0,
            Arc::new(AtomicBool::new(false)),
            completion,
            false,
        )
    }
    .unwrap();
    unsafe {
        SendMessageW(window, WM_ENDSESSION, 1, 0);
    }
    assert_eq!(unsafe { IsWindow(window) }, 0);
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
}
