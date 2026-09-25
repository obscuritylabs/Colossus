use super::{
    Arc, AtomicBool, Completion, DefinedClass, MainThreadMarker, NSWindow, PromptError, Retained,
    accessibility_tests, controller, current_text, insert, open_sheet, parent,
};
use crate::DialogAppearance;
use objc2_app_kit::{
    NSApplication, NSButton, NSEvent, NSEventModifierFlags, NSEventType, NSResponder,
    NSSecureTextField, NSView,
};
use objc2_foundation::{NSPoint, NSString, ns_string};

pub(super) fn run(mtm: MainThreadMarker) {
    keyboard_save(mtm);
    keyboard_cancel(mtm);
}

fn keyboard_save(mtm: MainThreadMarker) {
    let parent = parent(mtm);
    parent.makeKeyAndOrderFront(None);
    let (completion, mut result) = Completion::acquire().unwrap();
    open_sheet(
        &parent,
        mtm,
        Arc::new(AtomicBool::new(false)),
        completion,
        DialogAppearance::default(),
    );
    let (panel, input, save) = {
        let controller = controller();
        let borrowed = controller.ivars().session.borrow();
        let session = borrowed.as_ref().unwrap();
        (
            session.panel.clone(),
            session.input.clone(),
            session.save.clone(),
        )
    };
    accessibility_tests::secure_input(&input);
    accessibility_tests::button_enabled(&save, ns_string!("Save"), false);
    panel.performKeyEquivalent(&key(&panel, "\r", 36, false));
    assert!(result.try_recv().is_err(), "empty Return must not save");

    insert(&input.currentEditor().unwrap(), "SYNTHETIC-KEYBOARD");
    accessibility_tests::button_enabled(&save, ns_string!("Save"), true);
    accessibility_tests::secure_input(&input);
    traversal(&panel, &input, &save, mtm);
    assert_eq!(current_text(&input).to_string(), "SYNTHETIC-KEYBOARD");
    assert!(panel.performKeyEquivalent(&key(&panel, "\r", 36, false)));
    assert_eq!(
        result.try_recv().unwrap().unwrap().expose(),
        "SYNTHETIC-KEYBOARD"
    );
    assert_eq!(input.stringValue().length(), 0);
    parent.close();
}

fn traversal(panel: &NSWindow, input: &NSSecureTextField, save: &NSButton, mtm: MainThreadMarker) {
    // SAFETY: Production explicitly retains every target in this key-view loop.
    let cancel = unsafe {
        let next = input.nextKeyView().unwrap();
        assert!(same_view(&next, save));
        let cancel = save.nextKeyView().unwrap();
        assert!(same_view(&cancel.nextKeyView().unwrap(), input));
        assert!(same_view(&input.previousKeyView().unwrap(), &cancel));
        cancel
    };
    panel.sendEvent(&key(panel, "\t", 48, false));
    if NSApplication::sharedApplication(mtm).isFullKeyboardAccessEnabled() {
        assert!(same_responder(&panel.firstResponder().unwrap(), save));
    } else {
        assert!(
            input.currentEditor().is_some(),
            "text-only keyboard navigation stays in the field"
        );
    }
    panel.sendEvent(&key(panel, "\t", 48, true));
    assert!(
        input.currentEditor().is_some(),
        "Shift-Tab returns to secure entry"
    );
    accessibility_tests::button_enabled(&cancel, ns_string!("Cancel"), true);
}

fn same_view(left: &NSView, right: &NSView) -> bool {
    std::ptr::eq(left, right)
}

fn same_responder(left: &NSResponder, right: &NSResponder) -> bool {
    std::ptr::eq(left, right)
}

fn keyboard_cancel(mtm: MainThreadMarker) {
    let parent = parent(mtm);
    parent.makeKeyAndOrderFront(None);
    let (completion, mut result) = Completion::acquire().unwrap();
    open_sheet(
        &parent,
        mtm,
        Arc::new(AtomicBool::new(false)),
        completion,
        DialogAppearance::default(),
    );
    let (panel, input) = {
        let controller = controller();
        let borrowed = controller.ivars().session.borrow();
        let session = borrowed.as_ref().unwrap();
        (session.panel.clone(), session.input.clone())
    };
    insert(&input.currentEditor().unwrap(), "SYNTHETIC-CANCEL");
    assert!(panel.performKeyEquivalent(&key(&panel, "\u{1b}", 53, false)));
    assert!(matches!(result.try_recv(), Ok(Err(PromptError::Cancelled))));
    assert_eq!(input.stringValue().length(), 0);
    parent.close();
}

fn key(window: &NSWindow, characters: &str, code: u16, shifted: bool) -> Retained<NSEvent> {
    let characters = NSString::from_str(characters);
    NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        if shifted { NSEventModifierFlags::Shift } else { NSEventModifierFlags::empty() },
        0.0,
        window.windowNumber(),
        None,
        &characters,
        &characters,
        false,
        code,
    ).expect("native test key event")
}
