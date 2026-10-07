//! Dialog-local GTK appearance; no application settings or credential state change.

use crate::{ColorScheme, DialogAppearance};
use gtk::prelude::*;
use std::fmt::Write as _;

pub(super) fn pixels(appearance: DialogAppearance, logical: i32) -> i32 {
    (logical * appearance.text_size.root_pixels() + 8) / 16
}

pub(super) fn content(content: &gtk::Box, appearance: DialogAppearance) {
    content.set_spacing(pixels(appearance, 12));
    content.set_margin_start(pixels(appearance, 24));
    content.set_margin_end(pixels(appearance, 24));
    content.set_margin_top(pixels(appearance, 20));
    content.set_margin_bottom(pixels(appearance, 20));
}

pub(super) fn apply(dialog: &gtk::Dialog, appearance: DialogAppearance) -> Result<(), ()> {
    if let Some(button) = dialog.widget_for_response(gtk::ResponseType::Accept) {
        button.style_context().add_class("suggested-action");
    }
    let high_contrast = gtk::Settings::default()
        .and_then(|settings| settings.gtk_theme_name())
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.contains("highcontrast") || name.contains("high-contrast")
        });
    let provider = gtk::CssProvider::new();
    provider
        .load_from_data(stylesheet(appearance, high_contrast).as_bytes())
        .map_err(|_| ())?;
    install(dialog.upcast_ref(), &provider);
    Ok(())
}

fn install(widget: &gtk::Widget, provider: &gtk::CssProvider) {
    widget
        .style_context()
        .add_provider(provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    if let Some(container) = widget.downcast_ref::<gtk::Container>() {
        // Include native internal children, such as the action-area buttons.
        container.forall(|child| install(child, provider));
    }
}

fn stylesheet(appearance: DialogAppearance, high_contrast: bool) -> String {
    let scale = f64::from(appearance.text_size.root_pixels()) / 16.0;
    let mut css = format!(
        "* {{ font-size: {}px; }} entry {{ font-size: {}px; }}",
        14.0 * scale,
        16.0 * scale
    );
    // Keep OS/user colors for System and accessibility themes. The application
    // priority also lets explicit GTK user CSS take precedence over app colors.
    if appearance.color_scheme == ColorScheme::System || high_contrast {
        return css;
    }
    let palette = appearance.palette();
    let _ = write!(
        css,
        "
window {{ background-color: #{surface:06x}; color: #{text:06x}; }}
label {{ color: #{text:06x}; }}
label.credential-label {{ color: #{strong:06x}; }}
label.credential-description {{ color: #{muted:06x}; }}
label.credential-error {{ color: #{danger:06x}; }}
entry, button {{ background-image: none; background-color: #{control:06x}; color: #{text:06x}; border-color: #{border:06x}; }}
button:hover {{ background-color: #{hover:06x}; }}
entry:focus, button:focus {{ border-color: #{focus:06x}; outline-color: #{focus:06x}; }}
button.suggested-action {{ background-color: #{accent:06x}; color: #{on_accent:06x}; border-color: #{accent:06x}; }}
button.suggested-action:hover {{ background-color: #{accent_hover:06x}; }}
entry selection {{ background-color: #{accent:06x}; color: #{on_accent:06x}; }}
",
        surface = palette.surface,
        text = palette.text,
        strong = palette.strong,
        muted = palette.muted,
        danger = palette.danger,
        control = palette.control,
        border = palette.border,
        hover = palette.hover,
        focus = palette.focus,
        accent = palette.accent,
        on_accent = palette.on_accent,
        accent_hover = palette.accent_hover,
    );
    css
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextSize;

    #[test]
    fn system_and_high_contrast_preserve_native_colors_with_requested_text_scale() {
        let appearance = DialogAppearance {
            color_scheme: ColorScheme::Dark,
            text_size: TextSize::Large,
        };
        let high_contrast = stylesheet(appearance, true);
        let system = stylesheet(
            DialogAppearance {
                color_scheme: ColorScheme::System,
                ..appearance
            },
            false,
        );
        assert_eq!(system, high_contrast);
        assert!(system.contains("entry { font-size: 18px; }"));
        assert!(!system.contains("color:"));
        assert!(!system.contains("background"));
    }
}
