use super::{
    AnyObject, Arc, AtomicBool, Completion, Controller, DIALOG, DefinedClass, MainThreadMarker,
    MainThreadOnly, NSBackingStoreType, NSString, NSWindow, NSWindowStyleMask, Ordering,
    PromptError, Retained, current_text, msg_send, ns_string, open_sheet, rect,
};
use crate::{ColorScheme, DialogAppearance, TextSize};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::NSRange;

#[path = "accessibility_tests.rs"]
mod accessibility_tests;
#[path = "keyboard_tests.rs"]
mod keyboard_tests;

pub(crate) fn run() {
    let mtm =
        MainThreadMarker::new().expect("AppKit acceptance must run on the process main thread");
    let application = NSApplication::sharedApplication(mtm);
    assert!(application.setActivationPolicy(NSApplicationActivationPolicy::Regular));
    application.finishLaunching();
    for (length, color_scheme, text_size) in [
        (761, ColorScheme::Light, TextSize::Compact),
        (762, ColorScheme::Dark, TextSize::Compact),
        (2_560, ColorScheme::Light, TextSize::Comfortable),
        (2_561, ColorScheme::Dark, TextSize::Comfortable),
        (8_192, ColorScheme::Light, TextSize::Large),
        (65_536, ColorScheme::Dark, TextSize::Large),
    ] {
        let parent = parent(mtm);
        let (completion, mut result) = Completion::acquire().unwrap();
        open_sheet(
            &parent,
            mtm,
            Arc::new(AtomicBool::new(false)),
            completion,
            DialogAppearance {
                color_scheme,
                text_size,
            },
        );
        let controller = controller();
        let input = controller
            .ivars()
            .session
            .borrow()
            .as_ref()
            .unwrap()
            .input
            .clone();
        let editor = input
            .currentEditor()
            .expect("secure field must own its native field editor");
        let token = format!("{}END", "X".repeat(length - 3));
        insert(&editor, &token);
        assert_eq!(current_text(&input).to_string(), token);
        let (count, status) = {
            let borrowed = controller.ivars().session.borrow();
            let session = borrowed.as_ref().unwrap();
            (session.count.clone(), session.status.clone())
        };
        assert_eq!(
            count.stringValue().to_string(),
            format!("{length} / 65,536 bytes")
        );
        assert_eq!(status.stringValue().length(), 0);
        insert(&editor, &"X".repeat(65_537));
        assert_eq!(
            current_text(&input).to_string(),
            token,
            "oversized native edit must preserve prior contents"
        );
        assert_eq!(
            count.stringValue().to_string(),
            format!("{length} / 65,536 bytes")
        );
        assert_eq!(
            status.stringValue().to_string(),
            crate::validation::InputError::TooLong.message()
        );
        insert(&editor, "SYNTHETIC-REPLACEMENT");
        assert_eq!(
            status.stringValue().length(),
            0,
            "valid input clears the rejected-edit error"
        );
        insert(&editor, &token);
        // SAFETY: Invoke the production action with its registered Objective-C
        // signature, exactly as the native Save button does.
        unsafe {
            let _: () = msg_send![&*controller, save: Option::<&AnyObject>::None];
        }
        assert_eq!(result.try_recv().unwrap().unwrap().expose(), token);
        assert_eq!(input.stringValue().length(), 0);
        assert_eq!(editor.string().length(), 0);
        parent.close();
    }
    cancellation(mtm, false);
    cancellation(mtm, true);
    shutdown(mtm);
    keyboard_tests::run(mtm);
    println!(
        "AppKit native acceptance passed: exact input boundaries, rejected overflow, cleared controls, parent close, cancellation, keyboard actions and accessible state."
    );
}

fn parent(mtm: MainThreadMarker) -> Retained<NSWindow> {
    // SAFETY: The test owns the window and disables close-time autorelease.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, 640.0, 320.0),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window
}

fn controller() -> Retained<Controller> {
    DIALOG.with(|dialog| dialog.borrow().as_ref().unwrap().clone())
}

fn insert(editor: &objc2_app_kit::NSText, value: &str) {
    // SAFETY: AppKit supplies a secure NSTextView as the field editor. This test
    // simulates an input-system replacement without touching the user clipboard.
    unsafe {
        let _: () = msg_send![editor, insertText: &*NSString::from_str(value), replacementRange: NSRange::new(0, editor.string().length())];
    }
}

fn cancellation(mtm: MainThreadMarker, drop_parent: bool) {
    let parent = parent(mtm);
    let cancelled = Arc::new(AtomicBool::new(false));
    let (completion, mut result) = Completion::acquire().unwrap();
    open_sheet(
        &parent,
        mtm,
        cancelled.clone(),
        completion,
        DialogAppearance::default(),
    );
    let controller = controller();
    let (input, timer) = {
        let borrowed = controller.ivars().session.borrow();
        let session = borrowed.as_ref().unwrap();
        (session.input.clone(), session.timer.clone())
    };
    input.setStringValue(ns_string!("SYNTHETIC"));
    if drop_parent {
        parent.close();
    } else {
        cancelled.store(true, Ordering::Release);
        // SAFETY: Invoke the registered timer callback on its main thread.
        unsafe {
            let _: () = msg_send![&*controller, pollCancellation: &*timer];
        }
    }
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
    assert_eq!(input.stringValue().length(), 0);
    assert!(DIALOG.with(|dialog| dialog.borrow().is_none()));
    parent.close();
}

fn shutdown(mtm: MainThreadMarker) {
    let parent = parent(mtm);
    let (completion, mut result) = Completion::acquire().unwrap();
    open_sheet(
        &parent,
        mtm,
        Arc::new(AtomicBool::new(false)),
        completion,
        DialogAppearance::default(),
    );
    let input = controller()
        .ivars()
        .session
        .borrow()
        .as_ref()
        .unwrap()
        .input
        .clone();
    input.setStringValue(ns_string!("SYNTHETIC"));
    // SAFETY: Deliver the normal application notification to the isolated test
    // process; this does not terminate the app or any other process.
    unsafe {
        objc2_foundation::NSNotificationCenter::defaultCenter().postNotificationName_object(
            objc2_app_kit::NSApplicationWillTerminateNotification,
            None,
        );
    }
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
    assert_eq!(input.stringValue().length(), 0);
    parent.close();
}
