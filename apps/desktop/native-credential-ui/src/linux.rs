//! Owned GTK password entry on Tauri's existing UI thread; no nested event loop.
use crate::{DialogAppearance, PromptError, lifecycle::Completion, validation};
use colossus_contracts::HostSecret;
use gtk::{glib, prelude::*};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use zeroize::Zeroizing;

mod styling;

pub(crate) fn open(
    parent: &tauri::Window,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    appearance: DialogAppearance,
) {
    let Ok(parent) = parent.gtk_window() else {
        completion.finish(Err(PromptError::Unavailable));
        return;
    };
    let _ = create(parent.upcast_ref(), cancelled, completion, appearance);
}

fn create(
    parent: &gtk::Window,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    appearance: DialogAppearance,
) -> Result<(gtk::Dialog, gtk::Entry), PromptError> {
    let dialog = gtk::Dialog::builder()
        .title("Save a credential")
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(styling::pixels(appearance, 520))
        .resizable(false)
        .build();
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Save credential", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Accept);
    let content = dialog.content_area();
    styling::content(&content, appearance);
    let description = gtk::Label::new(Some(
        "This token stays in native credential storage and is never sent to the app's web view.",
    ));
    description.set_line_wrap(true);
    description.set_xalign(0.0);
    description
        .style_context()
        .add_class("credential-description");
    let label = gtk::Label::new(Some("_Token"));
    label.set_use_underline(true);
    label.set_xalign(0.0);
    label.style_context().add_class("credential-label");
    let entry = gtk::Entry::new();
    entry.set_visibility(false);
    entry.set_input_purpose(gtk::InputPurpose::Password);
    entry.set_activates_default(true);
    label.set_mnemonic_widget(Some(&entry));
    // GTK's password controls already suppress copying. Keep that guarantee
    // explicit for keyboard/menu signal paths without changing the user's clipboard.
    entry.connect_copy_clipboard(|entry| entry.stop_signal_emission_by_name("copy-clipboard"));
    entry.connect_cut_clipboard(|entry| entry.stop_signal_emission_by_name("cut-clipboard"));
    let error = gtk::Label::new(None);
    error.set_line_wrap(true);
    error.set_xalign(0.0);
    error.style_context().add_class("credential-error");
    content.add(&description);
    content.add(&label);
    content.add(&entry);
    content.add(&error);
    if styling::apply(&dialog, appearance).is_err() {
        dialog.close();
        completion.finish(Err(PromptError::Unavailable));
        return Err(PromptError::Unavailable);
    }
    let completion = Rc::new(RefCell::new(Some(completion)));
    let result = Rc::new(RefCell::new(None));
    dialog.connect_response({
        let result = result.clone();
        let entry = entry.clone();
        move |dialog, response| {
            // GtkWindow::close emits a second delete response. Preserve the
            // accepted result until destruction releases native ownership.
            if result.borrow().is_some() {
                return;
            }
            if response == gtk::ResponseType::Accept {
                let text = Zeroizing::new(entry.text().to_string());
                if let Err(invalid) = validation::validate(&text) {
                    error.set_text(invalid.message());
                    entry.grab_focus();
                    return;
                }
                *result.borrow_mut() =
                    Some(HostSecret::new(text.to_string()).map_err(|_| PromptError::Unavailable));
            } else {
                *result.borrow_mut() = Some(Err(PromptError::Cancelled));
            }
            entry.set_text("");
            dialog.close();
        }
    });
    dialog.connect_destroy({
        let entry = entry.clone();
        move |_| {
            entry.set_text("");
            if let Some(completion) = completion.borrow_mut().take() {
                completion.finish(
                    result
                        .borrow_mut()
                        .take()
                        .unwrap_or(Err(PromptError::Cancelled)),
                );
            }
        }
    });
    let weak = dialog.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(dialog) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if cancelled.load(Ordering::Acquire) {
            dialog.response(gtk::ResponseType::Cancel);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    dialog.show_all();
    entry.grab_focus();
    Ok((dialog, entry))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_for_completion(
        received: &mut tokio::sync::oneshot::Receiver<Result<HostSecret, PromptError>>,
    ) -> Result<HostSecret, PromptError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match received.try_recv() {
                Ok(result) => return result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(error) => panic!("native completion closed: {error}"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native dialog did not finish closing"
            );
            let context = glib::MainContext::default();
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    #[ignore = "operator-owned: xvfb-run and a GTK display; see cloud development guide"]
    fn native_linux_password_entry_and_cancel() {
        let _ownership = crate::lifecycle::TEST_OWNERSHIP.lock().unwrap();
        gtk::init().unwrap();
        let parent = gtk::Window::new(gtk::WindowType::Toplevel);
        parent.show_all();
        assert_appearance(&parent);
        let (completion, mut received) = Completion::acquire().unwrap();
        let (dialog, entry) = create(
            &parent,
            Arc::new(AtomicBool::new(false)),
            completion,
            DialogAppearance::default(),
        )
        .unwrap();
        assert!(!gtk::prelude::EntryExt::is_visible(&entry));
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text("unrelated-synthetic-clipboard");
        entry.set_text("synthetic-token");
        entry.select_region(0, -1);
        entry.emit_copy_clipboard();
        assert_eq!(
            clipboard.wait_for_text().as_deref(),
            Some("unrelated-synthetic-clipboard")
        );
        entry.set_text("bad token");
        dialog.response(gtk::ResponseType::Accept);
        assert!(received.try_recv().is_err());
        entry.set_text("synthetic-token");
        dialog.response(gtk::ResponseType::Accept);
        wait_for_completion(&mut received).expect("valid native credential entry");
        let (completion, mut received) = Completion::acquire().unwrap();
        let (dialog, _) = create(
            &parent,
            Arc::new(AtomicBool::new(false)),
            completion,
            DialogAppearance::default(),
        )
        .unwrap();
        dialog.response(gtk::ResponseType::Cancel);
        assert_eq!(
            wait_for_completion(&mut received).unwrap_err(),
            PromptError::Cancelled
        );
        parent.close();
    }

    fn assert_appearance(parent: &gtk::Window) {
        use crate::{ColorScheme, TextSize};
        let parent_color = parent.style_context().color(gtk::StateFlags::NORMAL);
        for scheme in [ColorScheme::Light, ColorScheme::Dark] {
            let mut sizes = Vec::new();
            let mut widths = Vec::new();
            for text_size in [TextSize::Compact, TextSize::Comfortable, TextSize::Large] {
                let appearance = DialogAppearance {
                    color_scheme: scheme,
                    text_size,
                };
                let (completion, mut received) = Completion::acquire().unwrap();
                let (dialog, entry) = create(
                    parent,
                    Arc::new(AtomicBool::new(false)),
                    completion,
                    appearance,
                )
                .unwrap();
                let context = entry.style_context();
                sizes.push(context.font(gtk::StateFlags::NORMAL).size());
                widths.push(dialog.default_size().0);
                let color = context.color(gtk::StateFlags::NORMAL);
                let expected = appearance.palette().text;
                for (actual, shift) in [(color.red(), 16), (color.green(), 8), (color.blue(), 0)] {
                    assert!((actual - f64::from((expected >> shift) & 0xff) / 255.0).abs() < 0.005);
                }
                assert_eq!(
                    parent.style_context().color(gtk::StateFlags::NORMAL),
                    parent_color
                );
                assert!(!gtk::prelude::EntryExt::is_visible(&entry));
                dialog.response(gtk::ResponseType::Cancel);
                assert_eq!(
                    wait_for_completion(&mut received).unwrap_err(),
                    PromptError::Cancelled
                );
            }
            assert!(sizes[0] < sizes[1] && sizes[1] < sizes[2]);
            assert!(widths[0] < widths[1] && widths[1] < widths[2]);
        }
    }
}
