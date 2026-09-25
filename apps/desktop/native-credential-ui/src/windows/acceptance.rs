//! Native message acceptance in a separate process with a private clipboard.

#[path = "acceptance_appearance.rs"]
mod appearance;
#[path = "acceptance_isolation.rs"]
mod isolation;

use super::{CANCEL_ID, INPUT_ID, SAVE_ID, Session, create, wide};
use crate::{DialogAppearance, PromptError, lifecycle::Completion, validation};
use colossus_contracts::HostSecret;
use isolation::{InputIsolation, set_clipboard};
use std::{
    ptr::{null, null_mut},
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::oneshot;
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{EM_GETPASSWORDCHAR, EM_SETSEL},
        Input::KeyboardAndMouse::{
            GetFocus, GetKeyboardState, IsWindowEnabled, SetKeyboardState, VK_CONTROL, VK_SHIFT,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetDlgItem,
            GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsWindow, MSG, PM_REMOVE,
            PeekMessageW, PostMessageW, SendMessageW, TranslateMessage, WM_KEYDOWN, WM_KEYUP,
            WM_PASTE, WS_VISIBLE,
        },
    },
};

pub(crate) fn run() {
    let station = InputIsolation::create();
    paste_boundaries();
    rejected_paste_preserves_exact_input();
    typed_character_boundaries();
    keyboard_traversal_and_actions();
    mnemonic_actions();
    parent_close();
    appearance::run();
    station.close();
    println!(
        "Windows native message acceptance passed: isolated clipboard paste boundaries, overflow rejection, exact saved tokens, keyboard traversal/actions, parent closure and exclusive ownership."
    );
}

struct Dialog {
    parent: HWND,
    window: HWND,
    input: HWND,
    status: HWND,
    error: HWND,
    save: HWND,
    cancel: HWND,
    result: oneshot::Receiver<Result<HostSecret, PromptError>>,
}

impl Dialog {
    fn new() -> Self {
        Self::with_appearance(DialogAppearance::default())
    }

    fn with_appearance(appearance: DialogAppearance) -> Self {
        let parent = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Synthetic parent").as_ptr(),
                WS_VISIBLE,
                0,
                0,
                640,
                300,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        assert!(!parent.is_null());
        let (completion, result) = Completion::acquire().unwrap();
        // The station is noninteractive: visible windows here do not appear on
        // the user's screen. Visibility enables real native keyboard focus.
        let window = unsafe {
            create(
                parent,
                Arc::new(AtomicBool::new(false)),
                completion,
                true,
                appearance,
            )
        }
        .unwrap();
        let input = unsafe { GetDlgItem(window, i32::try_from(INPUT_ID).unwrap()) };
        let session = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Session;
        assert_ne!(unsafe { SendMessageW(input, EM_GETPASSWORDCHAR, 0, 0) }, 0);
        Self {
            parent,
            window,
            input,
            status: unsafe { (*session).status },
            error: unsafe { (*session).error },
            save: unsafe { GetDlgItem(window, i32::try_from(SAVE_ID).unwrap()) },
            cancel: unsafe { GetDlgItem(window, i32::try_from(CANCEL_ID).unwrap()) },
            result,
        }
    }

    fn paste(&self, value: &str) {
        set_clipboard(self.window, value);
        assert_ne!(unsafe { PostMessageW(self.input, WM_PASTE, 0, 0) }, 0);
        dispatch_pending();
    }

    fn saved(&mut self, expected: &str) {
        assert_eq!(self.result.try_recv().unwrap().unwrap().expose(), expected);
        assert_eq!(unsafe { IsWindow(self.window) }, 0);
        assert_ne!(unsafe { IsWindowEnabled(self.parent) }, 0);
    }

    fn cancelled(&mut self) {
        assert!(matches!(
            self.result.try_recv(),
            Ok(Err(PromptError::Cancelled))
        ));
        assert_eq!(unsafe { IsWindow(self.window) }, 0);
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.window);
            DestroyWindow(self.parent);
        }
    }
}

fn paste_boundaries() {
    for length in [761, 762, 2_560, 2_561, 8_192, 65_536] {
        let mut dialog = Dialog::new();
        let token = format!("{}END", "X".repeat(length - 3));
        dialog.paste(&token);
        assert_eq!(read_text(dialog.input), token);
        assert_eq!(read_text(dialog.status), format!("{length} / 65,536 bytes"));
        assert_ne!(unsafe { IsWindowEnabled(dialog.save) }, 0);
        key(dialog.input, 0x0d, None);
        dialog.saved(&token);
    }
}

fn rejected_paste_preserves_exact_input() {
    let mut dialog = Dialog::new();
    dialog.paste("KEEP");
    key(dialog.input, 0x41, Some(VK_CONTROL));
    dialog.paste(&"X".repeat(65_537));
    assert_eq!(read_text(dialog.input), "KEEP");
    assert_eq!(
        read_text(dialog.error),
        validation::InputError::TooLong.message()
    );
    assert_eq!(read_text(dialog.status), "4 / 65,536 bytes");
    for invalid in [
        "X".repeat(65_537),
        "bad token".into(),
        "line\nbreak".into(),
        "é".into(),
    ] {
        dialog.paste(&invalid);
        assert_eq!(read_text(dialog.input), "KEEP");
    }
    let token = "X".repeat(65_536);
    dialog.paste(&token);
    assert_eq!(read_text(dialog.input), token);
    assert!(read_text(dialog.error).is_empty());
    unsafe {
        SendMessageW(dialog.input, EM_SETSEL, 65_536, 65_536);
    }
    set_clipboard(dialog.window, "Y");
    key(dialog.input, 0x56, Some(VK_CONTROL));
    assert_eq!(
        read_text(dialog.input),
        token,
        "aggregate paste overflow must not accept a prefix"
    );
    assert_eq!(
        read_text(dialog.error),
        validation::InputError::TooLong.message()
    );
    key(dialog.input, 0x0d, None);
    dialog.saved(&token);
}

fn keyboard_traversal_and_actions() {
    let mut dialog = Dialog::new();
    assert_eq!(unsafe { IsWindowEnabled(dialog.save) }, 0);
    assert!(matches!(Completion::acquire(), Err(PromptError::Busy)));
    assert_eq!(unsafe { GetFocus() }, dialog.input);
    key(dialog.input, 0x0d, None);
    assert!(
        dialog.result.try_recv().is_err(),
        "empty Return cannot save"
    );
    key(dialog.input, 0x09, None);
    assert_eq!(
        unsafe { GetFocus() },
        dialog.cancel,
        "Tab skips disabled Save"
    );
    key(dialog.cancel, 0x09, Some(VK_SHIFT));
    assert_eq!(unsafe { GetFocus() }, dialog.input);
    set_clipboard(dialog.window, "SYNTHETIC-KEYBOARD");
    key(dialog.input, 0x56, Some(VK_CONTROL));
    assert_eq!(read_text(dialog.input), "SYNTHETIC-KEYBOARD");
    for (from, to) in [
        (dialog.input, dialog.save),
        (dialog.save, dialog.cancel),
        (dialog.cancel, dialog.input),
    ] {
        key(from, 0x09, None);
        assert_eq!(unsafe { GetFocus() }, to);
    }
    for (from, to) in [
        (dialog.input, dialog.cancel),
        (dialog.cancel, dialog.save),
        (dialog.save, dialog.input),
    ] {
        key(from, 0x09, Some(VK_SHIFT));
        assert_eq!(unsafe { GetFocus() }, to);
    }
    key(dialog.input, 0x09, None);
    key(dialog.save, 0x0d, None);
    dialog.saved("SYNTHETIC-KEYBOARD");
    drop(dialog);
    let mut dialog = Dialog::new();
    dialog.paste("SYNTHETIC-CANCEL");
    key(dialog.input, 0x09, Some(VK_SHIFT));
    key(dialog.cancel, 0x0d, None);
    dialog.cancelled();
    drop(dialog);
    let mut dialog = Dialog::new();
    dialog.paste("SYNTHETIC-ESCAPE");
    key(dialog.input, 0x1b, None);
    dialog.cancelled();
}

fn typed_character_boundaries() {
    let mut dialog = Dialog::new();
    let token = "X".repeat(65_536);
    dialog.paste(&token);
    character(dialog.input, 'Y');
    assert_eq!(read_text(dialog.input), token);
    assert_eq!(
        read_text(dialog.error),
        validation::InputError::TooLong.message()
    );
    key(dialog.input, 0x41, Some(VK_CONTROL));
    for invalid in ['\n', ' ', 'é'] {
        character(dialog.input, invalid);
        assert_eq!(read_text(dialog.input), token);
    }
    character(dialog.input, 'Y');
    assert_eq!(
        read_text(dialog.input),
        "Y",
        "typed input replaces the entire 64-KiB selection"
    );
    key(dialog.input, 0x0d, None);
    dialog.saved("Y");
}

fn character(window: HWND, character: char) {
    assert_ne!(
        unsafe {
            PostMessageW(
                window,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
                character as usize,
                1,
            )
        },
        0
    );
    dispatch_pending();
}

fn parent_close() {
    let mut dialog = Dialog::new();
    dialog.paste("SYNTHETIC-PARENT-CLOSE");
    unsafe {
        DestroyWindow(dialog.parent);
    }
    dialog.cancelled();
}

fn mnemonic_actions() {
    let mut dialog = Dialog::new();
    dialog.paste("SYNTHETIC-MNEMONIC");
    key(dialog.input, 0x09, None);
    system_character(dialog.save, 't');
    assert_eq!(unsafe { GetFocus() }, dialog.input);
    system_character(dialog.input, 'S');
    dialog.saved("SYNTHETIC-MNEMONIC");
    drop(dialog);
    let mut dialog = Dialog::new();
    system_character(dialog.window, 'c');
    dialog.cancelled();
}

fn system_character(window: HWND, character: char) {
    assert_ne!(
        unsafe {
            PostMessageW(
                window,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCHAR,
                character as usize,
                1 << 29,
            )
        },
        0
    );
    dispatch_pending();
}

fn key(window: HWND, code: usize, modifier: Option<u16>) {
    let mut previous = [0_u8; 256];
    assert_ne!(unsafe { GetKeyboardState(previous.as_mut_ptr()) }, 0);
    let mut state = [0_u8; 256];
    if let Some(modifier) = modifier {
        state[usize::from(modifier)] = 0x80;
    }
    assert_ne!(unsafe { SetKeyboardState(state.as_ptr()) }, 0);
    assert_ne!(unsafe { PostMessageW(window, WM_KEYDOWN, code, 1) }, 0);
    assert_ne!(unsafe { PostMessageW(window, WM_KEYUP, code, 1) }, 0);
    dispatch_pending();
    assert_ne!(unsafe { SetKeyboardState(previous.as_ptr()) }, 0);
}

fn dispatch_pending() {
    let mut message = MSG::default();
    for _ in 0..100 {
        if unsafe { PeekMessageW(&raw mut message, null_mut(), 0, 0, PM_REMOVE) } == 0 {
            return;
        }
        unsafe {
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    panic!("native test message queue did not drain");
}

fn read_text(window: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(window) };
    let mut units = vec![0; usize::try_from(length).unwrap() + 1];
    assert_eq!(
        unsafe { GetWindowTextW(window, units.as_mut_ptr(), length + 1) },
        length
    );
    String::from_utf16(&units[..units.len() - 1]).unwrap()
}
