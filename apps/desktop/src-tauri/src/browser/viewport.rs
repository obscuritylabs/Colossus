use super::{
    BrowserManager,
    dto::{BrowserRect, BrowserViewportRequest},
    manager::error,
};
use crate::{dto::CommandErrorDto, state::AppState};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager as _};

pub(crate) fn valid_rect(rect: BrowserRect, width: f64, height: f64) -> bool {
    [rect.x, rect.y, rect.width, rect.height, width, height]
        .iter()
        .all(|v| v.is_finite())
        && rect.x >= 0.0
        && rect.y >= 48.0
        && rect.width >= 16.0
        && rect.height >= 16.0
        && rect.x + rect.width <= width + 1.0
        && rect.y + rect.height <= height + 1.0
}

impl BrowserManager {
    pub(crate) async fn viewport(
        &self,
        app: &AppHandle,
        request: &BrowserViewportRequest,
    ) -> Result<(), CommandErrorDto> {
        let scope = self.validate(request.generation)?;
        let window = app
            .get_window("main")
            .ok_or_else(|| error("The Desktop window has closed."))?;
        let controller = app
            .get_webview("main")
            .ok_or_else(|| error("The Desktop controller has closed."))?;
        let active = colossus_native_browser::is_active(&controller)
            .await
            .unwrap_or(false);
        let scale = window
            .scale_factor()
            .map_err(|_| error("The browser viewport is unavailable."))?;
        let size = window
            .inner_size()
            .map_err(|_| error("The browser viewport is unavailable."))?
            .to_logical::<f64>(scale);
        let operations = {
            let mut state = self.lock()?;
            if state.draining {
                return Err(error(super::registry::DRAINING_MESSAGE));
            }
            if state.generation != request.generation
                || state.scope.as_deref() != Some(scope.as_str())
            {
                return Err(error(
                    "The selected workspace changed. Reopen the browser pane.",
                ));
            }
            let selected = state.selected.get(&scope).cloned();
            state
                .tabs
                .iter_mut()
                .map(|tab| {
                    let visible = active
                        && tab.scope == scope
                        && Some(&tab.dto.id) == request.tab_id.as_ref()
                        && request.tab_id == selected
                        && tab.dto.error.is_none();
                    let rect = request
                        .rect
                        .filter(|rect| visible && valid_rect(*rect, size.width, size.height));
                    tab.heartbeat = rect.map(|_| Instant::now());
                    (
                        tab.view.clone(),
                        tab.presentation_epoch.clone(),
                        tab.presentation_epoch
                            .fetch_add(1, Ordering::AcqRel)
                            .wrapping_add(1),
                        rect,
                    )
                })
                .collect::<Vec<_>>()
        };
        // Drop registry locks before UI work: native event callbacks also need
        // them. Each queued operation rechecks its independently revocable lease.
        for (view, lease, epoch, rect) in operations {
            match super::presentation::apply(&window, view, lease, epoch, rect).await {
                Ok(()) | Err(colossus_native_browser::BrowserError::Closed) => {}
                Err(_) => return Err(error("The browser viewport could not be updated.")),
            }
        }
        Ok(())
    }

    fn hide_expired(&self) {
        let views = if let Ok(mut state) = self.data.lock() {
            state
                .tabs
                .iter_mut()
                .filter_map(|tab| {
                    if tab
                        .heartbeat
                        .is_some_and(|last| last.elapsed() > Duration::from_secs(2))
                    {
                        tab.presentation_epoch.fetch_add(1, Ordering::AcqRel);
                        tab.heartbeat = None;
                        Some(tab.view.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for view in views {
            let _ = view.hide();
        }
    }
}

pub(crate) fn start_watchdog(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if app.get_window("main").is_none() {
                break;
            }
            app.state::<AppState>().browser.hide_expired();
        }
    });
}

pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
        window.state::<AppState>().browser.hide_all();
        crate::terminal_commands::pane::hide(window.app_handle(), false);
    } else if matches!(event, tauri::WindowEvent::Focused(false)) {
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn(async move {
            let active = match app.get_webview("main") {
                Some(view) => colossus_native_browser::is_active(&view)
                    .await
                    .unwrap_or(false),
                None => false,
            };
            if !active {
                app.state::<AppState>().browser.hide_all();
                crate::terminal_commands::pane::hide(&app, false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_bounds_remain_finite_and_inside_the_content_region() {
        let good = BrowserRect {
            x: 700.0,
            y: 170.0,
            width: 600.0,
            height: 600.0,
        };
        assert!(valid_rect(good, 1440.0, 900.0));
        for bad in [
            BrowserRect { y: 0.0, ..good },
            BrowserRect { x: -1.0, ..good },
            BrowserRect {
                width: f64::NAN,
                ..good
            },
            BrowserRect {
                width: 10_000.0,
                ..good
            },
            BrowserRect {
                height: -1.0,
                ..good
            },
        ] {
            assert!(!valid_rect(bad, 1440.0, 900.0));
        }
    }
}
