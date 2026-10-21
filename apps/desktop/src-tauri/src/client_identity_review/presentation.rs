//! Native geometry and focus; ordinary renderer bounds cannot position consent.

use tauri::{Manager as _, Webview};

use super::{WEBVIEW, cancel, is_active, unavailable};
use crate::dto::CommandErrorDto;

pub(super) fn position(window: &tauri::Window, view: &Webview) -> Result<(), CommandErrorDto> {
    if !crate::browser::viewport::window_can_present(window) {
        return view.hide().map_err(|_| unavailable());
    }
    let scale = window.scale_factor().map_err(|_| unavailable())?;
    let size = window
        .inner_size()
        .map_err(|_| unavailable())?
        .to_logical::<f64>(scale);
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width < 16.0
        || size.height < 16.0
    {
        return Err(unavailable());
    }
    view.set_bounds(tauri::Rect {
        position: tauri::LogicalPosition::new(0.0, 0.0).into(),
        size: tauri::LogicalSize::new(size.width, size.height).into(),
    })
    .map_err(|_| unavailable())?;
    view.show().map_err(|_| unavailable())
}

pub(super) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" || !is_active(window.app_handle()) {
        return;
    }
    match event {
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
            cancel(window.app_handle(), None);
        }
        tauri::WindowEvent::Resized(_)
        | tauri::WindowEvent::ScaleFactorChanged { .. }
        | tauri::WindowEvent::Focused(true) => {
            if let Some(view) = window.app_handle().get_webview(WEBVIEW)
                && position(window, &view).is_err()
            {
                cancel(window.app_handle(), None);
            }
        }
        _ => {}
    }
}
