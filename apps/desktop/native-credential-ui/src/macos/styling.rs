//! `AppKit` presentation only; credential input and completion stay in the controller.

use crate::{ColorScheme, DialogAppearance, appearance::Palette};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAccessibilityHighContrastAqua,
    NSAppearanceNameAccessibilityHighContrastDarkAqua, NSAppearanceNameAqua,
    NSAppearanceNameDarkAqua, NSButton, NSColor, NSControlSize, NSFont, NSSecureTextField,
    NSTextField, NSWindow, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSRect, ns_string};

#[derive(Clone, Copy)]
enum LabelTone {
    Normal,
    Muted,
    Error,
}

pub(super) struct Style {
    scale: f64,
    palette: Palette,
    appearance: Option<Retained<NSAppearance>>,
    high_contrast: bool,
}

impl Style {
    pub(super) fn new(parent: &NSWindow, mut appearance: DialogAppearance) -> Self {
        let high_contrast =
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast();
        // SAFETY: These AppKit-owned appearance names are immutable constants.
        let name = unsafe {
            if appearance.color_scheme == ColorScheme::System {
                let inherited = parent.effectiveAppearance().name();
                appearance.color_scheme = if &*inherited == NSAppearanceNameDarkAqua
                    || &*inherited == NSAppearanceNameAccessibilityHighContrastDarkAqua
                {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                };
            }
            match (appearance.color_scheme, high_contrast) {
                (ColorScheme::Dark, true) => NSAppearanceNameAccessibilityHighContrastDarkAqua,
                (ColorScheme::Dark, false) => NSAppearanceNameDarkAqua,
                (_, true) => NSAppearanceNameAccessibilityHighContrastAqua,
                (_, false) => NSAppearanceNameAqua,
            }
        };
        Self {
            scale: appearance.text_size.scale(),
            palette: appearance.palette(),
            appearance: NSAppearance::appearanceNamed(name),
            high_contrast,
        }
    }

    pub(super) fn rect(&self, x: f64, y: f64, width: f64, height: f64) -> NSRect {
        super::rect(
            x * self.scale,
            y * self.scale,
            width * self.scale,
            height * self.scale,
        )
    }

    pub(super) fn panel(&self, panel: &NSWindow) {
        panel.setAppearance(self.appearance.as_deref());
        panel.setBackgroundColor(Some(
            &self.color(self.palette.surface, NSColor::windowBackgroundColor),
        ));
    }

    pub(super) fn labels(&self, mtm: MainThreadMarker) -> [Retained<NSTextField>; 3] {
        let heading = NSTextField::labelWithString(ns_string!("Save credential"), mtm);
        heading.setFrame(self.rect(28.0, 263.0, 504.0, 33.0));
        heading.setFont(Some(&NSFont::boldSystemFontOfSize(20.0 * self.scale)));
        heading.setTextColor(Some(&self.color(self.palette.strong, NSColor::labelColor)));
        let description = NSTextField::wrappingLabelWithString(
            ns_string!("Your token is saved in the encrypted credential vault."),
            mtm,
        );
        description.setFrame(self.rect(28.0, 227.0, 504.0, 30.0));
        self.label(&description, 14.0, LabelTone::Muted);
        let label = NSTextField::labelWithString(ns_string!("Token"), mtm);
        label.setFrame(self.rect(28.0, 197.0, 504.0, 22.0));
        self.label(&label, 14.0, LabelTone::Normal);
        [heading, description, label]
    }

    pub(super) fn feedback(&self, mtm: MainThreadMarker) -> [Retained<NSTextField>; 2] {
        let count = NSTextField::labelWithString(ns_string!("0 / 65,536 bytes"), mtm);
        count.setFrame(self.rect(28.0, 117.0, 504.0, 20.0));
        self.label(&count, 12.0, LabelTone::Muted);
        let status = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        status.setFrame(self.rect(28.0, 76.0, 504.0, 36.0));
        status.setMaximumNumberOfLines(2);
        self.label(&status, 13.0, LabelTone::Error);
        [count, status]
    }

    fn label(&self, label: &NSTextField, size: f64, tone: LabelTone) {
        label.setFont(Some(&NSFont::systemFontOfSize(size * self.scale)));
        let color = match tone {
            LabelTone::Normal => self.color(self.palette.text, NSColor::labelColor),
            LabelTone::Muted => self.color(self.palette.muted, NSColor::secondaryLabelColor),
            LabelTone::Error => self.color(self.palette.danger, NSColor::systemRedColor),
        };
        label.setTextColor(Some(&color));
    }

    pub(super) fn input(&self, input: &NSSecureTextField) {
        input.setFont(Some(&NSFont::systemFontOfSize(16.0 * self.scale)));
        input.setControlSize(NSControlSize::Large);
        input.setTextColor(Some(
            &self.color(self.palette.text, NSColor::controlTextColor),
        ));
        input.setBackgroundColor(Some(
            &self.color(self.palette.control, NSColor::controlBackgroundColor),
        ));
    }

    pub(super) fn button(&self, button: &NSButton, primary: bool) {
        button.setFont(Some(&NSFont::systemFontOfSize(14.0 * self.scale)));
        button.setControlSize(NSControlSize::Large);
        if primary && !self.high_contrast {
            button.setBezelColor(Some(&rgb(self.palette.accent)));
        }
    }

    fn color(&self, value: u32, system: fn() -> Retained<NSColor>) -> Retained<NSColor> {
        if self.high_contrast {
            system()
        } else {
            rgb(value)
        }
    }
}

fn rgb(value: u32) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from((value >> 16) & 0xff) / 255.0,
        f64::from((value >> 8) & 0xff) / 255.0,
        f64::from(value & 0xff) / 255.0,
        1.0,
    )
}
