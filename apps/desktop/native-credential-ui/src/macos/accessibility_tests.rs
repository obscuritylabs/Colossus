//! Inspect the unignored native elements presented to accessibility clients.

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityUnignoredDescendant, NSButton, NSCell, NSSecureTextField,
    NSView,
};
use objc2_foundation::{NSString, ns_string};

pub(super) fn secure_input(input: &NSSecureTextField) {
    let element = exposed(input);
    assert!(element.isAccessibilityElement());
    assert_eq!(
        element.accessibilityLabel().as_deref(),
        Some(ns_string!("Token"))
    );
    assert_eq!(
        element.accessibilitySubrole().as_deref(),
        Some(ns_string!("AXSecureTextField"))
    );
    assert!(element.isAccessibilityEnabled());
    assert!(
        element.accessibilityValue().is_none_or(|value| {
            value
                .downcast_ref::<NSString>()
                .is_some_and(|text| !text.to_string().contains("SYNTHETIC-KEYBOARD"))
        }),
        "secure input must not expose plaintext through accessibility"
    );
}

pub(super) fn save_enabled(button: &NSButton, enabled: bool) {
    let element = exposed(button);
    assert!(element.isAccessibilityElement());
    assert_eq!(
        element.accessibilityLabel().as_deref(),
        Some(ns_string!("Save"))
    );
    assert_eq!(element.isAccessibilityEnabled(), enabled);
}

fn exposed(view: &NSView) -> Retained<ProtocolObject<dyn NSAccessibility>> {
    // AppKit may expose a control's cell and ignore the surrounding NSView. Query
    // its public accessibility hierarchy instead of requiring that wrapper view
    // itself be an accessibility element. Both native representations implement
    // the same protocol; do not change production accessibility to suit the test.
    // SAFETY: The input is a live standard AppKit control on its main thread.
    let element = unsafe { NSAccessibilityUnignoredDescendant(view) }
        .expect("control must expose an accessibility element");
    match element.downcast::<NSCell>() {
        Ok(cell) => ProtocolObject::from_retained(cell),
        Err(element) => {
            ProtocolObject::from_retained(element.downcast::<NSView>().unwrap_or_else(|_element| {
                panic!("standard control must expose a native cell or view")
            }))
        }
    }
}
