//! Native handle tests exercise real controls without opening visible windows.

use super::*;
use crate::lifecycle::TEST_OWNERSHIP;
use windows_sys::Win32::UI::{
    Controls::{EM_REPLACESEL, EM_SETSEL},
    WindowsAndMessaging::{GetDlgItem, SendMessageW},
};

struct Parent(HWND);

#[test]
fn provider_prompt_displays_immutable_destination_beside_the_secure_edit() {
    let _ownership = TEST_OWNERSHIP.lock().unwrap();
    for text_size in [
        crate::TextSize::Compact,
        crate::TextSize::Comfortable,
        crate::TextSize::Large,
    ] {
        let parent = Parent::new();
        let (completion, mut result) = Completion::acquire().unwrap();
        let endpoint = "https://models.example.test:8443/custom/v1";
        let context =
            ProviderCredentialContext::new(crate::CredentialProvider::AnthropicMessages, endpoint)
                .unwrap();
        let window = unsafe {
            create_for(
                parent.0,
                Arc::new(AtomicBool::new(false)),
                completion,
                false,
                DialogAppearance {
                    text_size,
                    ..DialogAppearance::default()
                },
                Purpose::Token,
                Some(context),
            )
        }
        .unwrap();
        let session = unsafe { &*(GetWindowLongPtrW(window, GWLP_USERDATA) as *const Session) };
        let text = |handle| {
            let length = unsafe { GetWindowTextLengthW(handle) };
            let mut buffer = vec![0; usize::try_from(length).unwrap() + 1];
            let copied = unsafe { GetWindowTextW(handle, buffer.as_mut_ptr(), length + 1) };
            String::from_utf16(&buffer[..usize::try_from(copied).unwrap()]).unwrap()
        };
        assert_eq!(
            text(session.destination_label),
            "Use this token for Anthropic Messages at:"
        );
        assert_eq!(text(session.destination), endpoint);
        assert!(text(session.input).is_empty());
        let readonly = unsafe {
            GetWindowLongPtrW(
                session.destination,
                windows_sys::Win32::UI::WindowsAndMessaging::GWL_STYLE,
            )
        };
        assert_ne!(
            readonly & windows_sys::Win32::UI::WindowsAndMessaging::ES_READONLY as isize,
            0
        );
        let mut endpoint_bounds = RECT::default();
        let mut token_bounds = RECT::default();
        unsafe {
            GetWindowRect(session.destination, &raw mut endpoint_bounds);
            GetWindowRect(session.label, &raw mut token_bounds);
        }
        assert!(endpoint_bounds.bottom < token_bounds.top);
        unsafe {
            DestroyWindow(window);
        }
        assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
    }
}

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
fn native_dialog_has_a_title_and_stays_in_the_owner_monitor_work_area() {
    let _ownership = TEST_OWNERSHIP.lock().unwrap();
    let parent = Parent::new();
    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe {
        create(
            parent.0,
            Arc::new(AtomicBool::new(false)),
            completion,
            false,
            DialogAppearance::default(),
        )
    }
    .unwrap();
    let mut title = [0_u16; 128];
    let copied = unsafe { GetWindowTextW(window, title.as_mut_ptr(), 128) };
    assert_eq!(
        String::from_utf16(&title[..usize::try_from(copied).unwrap()]).unwrap(),
        "Save a Colossus credential"
    );
    let mut bounds = RECT::default();
    assert_ne!(unsafe { GetWindowRect(window, &raw mut bounds) }, 0);
    let mut monitor = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap(),
        ..Default::default()
    };
    assert_ne!(
        unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(parent.0, MONITOR_DEFAULTTONEAREST),
                &raw mut monitor,
            )
        },
        0
    );
    assert!(bounds.left >= monitor.rcWork.left);
    assert!(bounds.top >= monitor.rcWork.top);
    assert!(bounds.right <= monitor.rcWork.right);
    assert!(bounds.bottom <= monitor.rcWork.bottom);
    unsafe {
        DestroyWindow(window);
    }
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
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
                DialogAppearance::default(),
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
            DialogAppearance::default(),
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
            DialogAppearance::default(),
        )
    }
    .unwrap();
    drop(parent);
    assert_eq!(unsafe { IsWindow(window) }, 0);
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));

    let parent = Parent::new();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (completion, mut result) = Completion::acquire().unwrap();
    let window = unsafe {
        create(
            parent.0,
            cancelled.clone(),
            completion,
            false,
            DialogAppearance::default(),
        )
    }
    .unwrap();
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
            DialogAppearance::default(),
        )
    }
    .unwrap();
    unsafe {
        SendMessageW(window, WM_ENDSESSION, 1, 0);
    }
    assert_eq!(unsafe { IsWindow(window) }, 0);
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
}
