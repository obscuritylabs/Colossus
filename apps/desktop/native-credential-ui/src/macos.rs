//! `AppKit` sheet with a secure field and pre-insertion formatter validation.

#[path = "macos/formatter.rs"]
mod formatter;
#[cfg(feature = "native-test-driver")]
#[path = "macos/tests.rs"]
pub(crate) mod tests;

use crate::{PromptError, lifecycle::Completion, validation};
use colossus_contracts::HostSecret;
use formatter::TokenFormatter;
use objc2::{
    DefinedClass, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSAccessibility, NSApplicationWillTerminateNotification, NSBackingStoreType, NSButton,
    NSControlTextEditingDelegate, NSPanel, NSSecureTextField, NSTextField, NSTextFieldDelegate,
    NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowWillCloseNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString, NSTimer, ns_string,
};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use zeroize::Zeroizing;

thread_local! { static DIALOG: RefCell<Option<Retained<Controller>>> = const { RefCell::new(None) }; }

struct Session {
    parent: Retained<NSWindow>,
    panel: Retained<NSPanel>,
    input: Retained<NSSecureTextField>,
    status: Retained<NSTextField>,
    save: Retained<NSButton>,
    timer: Retained<NSTimer>,
    completion: Completion,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct ControllerIvars {
    session: RefCell<Option<Session>>,
}

define_class!(
    // SAFETY: NSObject permits subclassing; all callbacks and retained AppKit
    // objects are confined to the marked main thread. There is no custom Drop.
    #[unsafe(super = NSObject)]
    #[name = "ColossusCredentialControllerV1"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControllerIvars]
    struct Controller;

    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _: &NSNotification) { self.finish(Err(PromptError::Cancelled)); }
    }
    unsafe impl NSControlTextEditingDelegate for Controller {
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, _: &NSNotification) {
            if let Some(session) = self.ivars().session.borrow().as_ref() {
                let text = current_text(&session.input);
                session.status.setStringValue(&NSString::from_str(&format!("{} / 65,536 bytes", text.length())));
                session.save.setEnabled(text.length() > 0);
            }
        }
    }
    unsafe impl NSTextFieldDelegate for Controller {}

    impl Controller {
        #[unsafe(method(save:))]
        fn save(&self, _: Option<&AnyObject>) {
            let result = {
                let borrowed = self.ivars().session.borrow();
                let Some(session) = borrowed.as_ref() else { return; };
                let text = current_text(&session.input);
                if let Err(error) = formatter::validate_native(&text) {
                    session.status.setStringValue(&NSString::from_str(error.message()));
                    return;
                }
                let mut secret = Zeroizing::new(text.to_string());
                if let Err(error) = validation::validate(&secret) {
                    session.status.setStringValue(&NSString::from_str(error.message()));
                    return;
                }
                HostSecret::new(std::mem::take(&mut *secret)).map_err(|_| PromptError::Unavailable)
            };
            self.finish(result);
        }

        #[unsafe(method(cancel:))]
        fn cancel(&self, _: Option<&AnyObject>) { self.finish(Err(PromptError::Cancelled)); }

        #[unsafe(method(parentWillClose:))]
        fn parent_will_close(&self, _: &NSNotification) { self.finish(Err(PromptError::Cancelled)); }

        #[unsafe(method(pollCancellation:))]
        fn poll(&self, _: &NSTimer) {
            let cancel = self.ivars().session.borrow().as_ref()
                .is_some_and(|session| session.cancelled.load(Ordering::Acquire));
            if cancel { self.finish(Err(PromptError::Cancelled)); }
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ControllerIvars::default());
        // SAFETY: NSObject initialization is valid for this main-thread subclass.
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, result: Result<HostSecret, PromptError>) {
        // Keep self alive through timer invalidation and thread-local release.
        let _keep_alive = self.retain();
        let Some(session) = self.ivars().session.borrow_mut().take() else {
            return;
        };
        session.timer.invalidate();
        // SAFETY: The exact observer and registered callbacks are owned here.
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(self);
        }
        if let Some(editor) = session.input.currentEditor() {
            editor.setString(ns_string!(""));
        }
        session.input.setStringValue(ns_string!(""));
        unsafe {
            session.input.setDelegate(None);
        }
        session.panel.setDelegate(None);
        session.parent.endSheet(&session.panel);
        session.panel.orderOut(None);
        session.panel.close();
        DIALOG.with(|dialog| dialog.borrow_mut().take());
        session.completion.finish(result);
    }
}

pub(crate) fn open(
    parent: &tauri::WebviewWindow,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    let Ok(pointer) = parent.ns_window() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    // SAFETY: Tauri returns its live NSWindow on the main thread. Retain it for
    // the sheet lifetime; its close notification cancels and releases the sheet.
    let Some(parent) = (unsafe { Retained::<NSWindow>::retain(pointer.cast()) }) else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    if cancelled.load(Ordering::Acquire) {
        completion.finish(Err(PromptError::Cancelled));
        return;
    }
    open_sheet(&parent, mtm, cancelled, completion);
}

fn open_sheet(
    parent: &NSWindow,
    mtm: MainThreadMarker,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
) {
    let controller = Controller::new(mtm);
    // SAFETY: Window lifetime is explicitly retained; close must not autorelease
    // the owning reference. All views, targets and callbacks stay main-thread.
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect(0.0, 0.0, 580.0, 210.0),
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    panel.setTitle(ns_string!("Save a Colossus credential"));
    let Some(content) = panel.contentView() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    let label = NSTextField::labelWithString(ns_string!("Token"), mtm);
    label.setFrame(rect(20.0, 164.0, 540.0, 24.0));
    let input = NSSecureTextField::initWithFrame(
        NSSecureTextField::alloc(mtm),
        rect(20.0, 126.0, 540.0, 30.0),
    );
    input.setPlaceholderString(Some(ns_string!("Paste your credential")));
    input.setAccessibilityLabel(Some(ns_string!("Token")));
    let status = NSTextField::labelWithString(ns_string!("0 / 65,536 bytes"), mtm);
    status.setFrame(rect(20.0, 64.0, 540.0, 50.0));
    let save = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Save"),
            Some(&controller),
            Some(sel!(save:)),
            mtm,
        )
    };
    save.setFrame(rect(342.0, 20.0, 100.0, 32.0));
    save.setKeyEquivalent(ns_string!("\r"));
    save.setAccessibilityLabel(Some(ns_string!("Save")));
    save.setEnabled(false);
    let cancel = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Cancel"),
            Some(&controller),
            Some(sel!(cancel:)),
            mtm,
        )
    };
    cancel.setFrame(rect(450.0, 20.0, 110.0, 32.0));
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    cancel.setAccessibilityLabel(Some(ns_string!("Cancel")));
    let formatter = TokenFormatter::new(mtm, status.clone());
    input.setFormatter(Some(&formatter));
    unsafe {
        input.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        content.addSubview(&label);
        content.addSubview(&input);
        content.addSubview(&status);
        content.addSubview(&save);
        content.addSubview(&cancel);
    }
    panel.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    // SAFETY: Each target is a live view retained by this sheet for the key loop.
    unsafe {
        input.setNextKeyView(Some(&save));
        save.setNextKeyView(Some(&cancel));
        cancel.setNextKeyView(Some(&input));
    }
    // SAFETY: Timer's selector is implemented above with the required NSTimer
    // argument. Invalidating it during finish breaks its retained-target cycle.
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            0.1,
            &controller,
            sel!(pollCancellation:),
            None,
            true,
        )
    };
    observe_cancellation(&controller, parent);
    controller.ivars().session.replace(Some(Session {
        parent: parent.retain(),
        panel: panel.clone(),
        input: input.clone(),
        status,
        save,
        timer,
        completion,
        cancelled,
    }));
    DIALOG.with(|dialog| dialog.replace(Some(controller)));
    parent.beginSheet_completionHandler(&panel, None);
    panel.makeFirstResponder(Some(&input));
}

fn observe_cancellation(controller: &Controller, parent: &NSWindow) {
    // SAFETY: The controller implements the registered notification callback.
    // Its session owns the parent and removes these observers before teardown.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            controller,
            sel!(parentWillClose:),
            Some(NSWindowWillCloseNotification),
            Some(parent),
        );
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            controller,
            sel!(parentWillClose:),
            Some(NSApplicationWillTerminateNotification),
            None,
        );
    }
}

fn current_text(input: &NSSecureTextField) -> Retained<NSString> {
    input
        .currentEditor()
        .map_or_else(|| input.stringValue(), |editor| editor.string())
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
