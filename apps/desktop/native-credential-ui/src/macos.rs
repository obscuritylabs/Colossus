//! `AppKit` sheet with a secure field and pre-insertion formatter validation.

#[path = "macos/accessibility.rs"]
mod accessibility;
#[path = "macos/formatter.rs"]
mod formatter;
#[path = "macos/styling.rs"]
mod styling;
#[cfg(feature = "native-test-driver")]
#[path = "macos/tests.rs"]
pub(crate) mod tests;

use crate::{DialogAppearance, PromptError, lifecycle::Completion, validation};
use colossus_contracts::HostSecret;
use formatter::TokenFormatter;
use objc2::{
    DefinedClass, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSApplicationWillTerminateNotification, NSBackingStoreType, NSButton,
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
    count: Retained<NSTextField>,
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
                session.count.setStringValue(&NSString::from_str(&format!("{} / 65,536 bytes", text.length())));
                session.status.setStringValue(ns_string!(""));
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
    appearance: DialogAppearance,
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
    open_sheet(&parent, mtm, cancelled, completion, appearance);
}

fn open_sheet(
    parent: &NSWindow,
    mtm: MainThreadMarker,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    appearance: DialogAppearance,
) {
    let controller = Controller::new(mtm);
    let style = styling::Style::new(parent, appearance);
    // SAFETY: Window lifetime is explicitly retained; close must not autorelease
    // the owning reference. All views, targets and callbacks stay main-thread.
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        style.rect(0.0, 0.0, 560.0, 324.0),
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe {
        panel.setReleasedWhenClosed(false);
    }
    panel.setTitle(ns_string!("Save credential"));
    style.panel(&panel);
    let Some(content) = panel.contentView() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    let [heading, description, label] = style.labels(mtm);
    let input = NSSecureTextField::initWithFrame(
        NSSecureTextField::alloc(mtm),
        style.rect(28.0, 145.0, 504.0, 44.0),
    );
    input.setPlaceholderString(Some(ns_string!("Paste your credential")));
    style.input(&input);
    accessibility::label(&input, ns_string!("Token"));
    let [count, status] = style.feedback(mtm);
    let save = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Save"),
            Some(&controller),
            Some(sel!(save:)),
            mtm,
        )
    };
    save.setFrame(style.rect(432.0, 24.0, 100.0, 40.0));
    style.button(&save, true);
    save.setKeyEquivalent(ns_string!("\r"));
    accessibility::label(&save, ns_string!("Save"));
    save.setEnabled(false);
    let cancel = unsafe {
        NSButton::buttonWithTitle_target_action(
            ns_string!("Cancel"),
            Some(&controller),
            Some(sel!(cancel:)),
            mtm,
        )
    };
    cancel.setFrame(style.rect(320.0, 24.0, 100.0, 40.0));
    style.button(&cancel, false);
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    accessibility::label(&cancel, ns_string!("Cancel"));
    let formatter = TokenFormatter::new(mtm, status.clone());
    input.setFormatter(Some(&formatter));
    unsafe {
        input.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        content.addSubview(&heading);
        content.addSubview(&description);
        content.addSubview(&label);
        content.addSubview(&input);
        content.addSubview(&count);
        content.addSubview(&status);
        content.addSubview(&save);
        content.addSubview(&cancel);
    }
    panel.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    // Let AppKit maintain the loop as it inserts/removes the shared field editor
    // and applies the user's full-keyboard-access preference.
    panel.setInitialFirstResponder(Some(&input));
    panel.setAutorecalculatesKeyViewLoop(true);
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
        count,
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
