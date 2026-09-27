//! Label both `AppKit` representations of a cell-backed control.

use objc2_app_kit::{NSAccessibility, NSControl};
use objc2_foundation::NSString;

pub(super) fn label(control: &NSControl, text: &NSString) {
    // The view can be ignored by the accessibility hierarchy in favor of its
    // native cell. AppKit does not forward this override from view to cell.
    control.setAccessibilityLabel(Some(text));
    if let Some(cell) = control.cell() {
        cell.setAccessibilityLabel(Some(text));
    }
}
