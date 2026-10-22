//! Native lifetime of a review; cancellation never leaves a usable child.

use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager as _, webview::WebviewBuilder};
use tokio::sync::oneshot;

use super::{
    ClientIdentityDisplay, ClientIdentityReviewState, PendingReview, ReviewDto, WEBVIEW, cancel,
    decision, display, presentation, protocol, unavailable,
};
use crate::{dto::CommandErrorDto, state::AppState};

struct ReviewGuard {
    app: AppHandle,
    ui_token: String,
}

impl Drop for ReviewGuard {
    fn drop(&mut self) {
        cancel(&self.app, Some(&self.ui_token));
    }
}

/// This is native-to-native composition. Dropping the future cancels its UI.
pub(crate) async fn show(
    app: &AppHandle,
    target_id: &str,
    selection_epoch: u64,
    display: ClientIdentityDisplay,
) -> Result<Option<String>, CommandErrorDto> {
    display::validate(&display)?;
    let app_state = app.state::<AppState>();
    let mut selection = app_state.subscribe_selection();
    if !app_state.selection_is_current(target_id, selection_epoch) {
        return Err(unavailable());
    }
    let state = app.state::<ClientIdentityReviewState>();
    let ui_token = uuid::Uuid::new_v4().to_string();
    let deadline = Instant::now() + Duration::from_millis(display.remaining_ms.into());
    let (response, receiver) = oneshot::channel();
    {
        let mut pending = state.pending.lock().map_err(|_| unavailable())?;
        if pending.is_some() || app.get_webview(WEBVIEW).is_some() {
            return Err(CommandErrorDto::busy(
                "A certificate review is already open.",
            ));
        }
        *pending = Some(PendingReview {
            details: ReviewDto {
                ui_token: ui_token.clone(),
                origin: display.origin,
                candidates: display.candidates,
                appearance: *state.appearance.lock().map_err(|_| unavailable())?,
            },
            target_id: target_id.into(),
            selection_epoch,
            deadline,
            response: Some(response),
        });
    }
    let guard = ReviewGuard {
        app: app.clone(),
        ui_token: ui_token.clone(),
    };
    app_state.browser.hide_all();
    crate::terminal_commands::pane::hide(app, false);
    create(app, &ui_token).await?;
    let invalidated = async {
        let _ = selection.changed().await;
    };
    let view_failed = async {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if !is_present(app) {
                return;
            }
        }
    };
    let answer = decision(receiver, invalidated, view_failed, deadline).await?;
    if answer.is_some()
        && !state.pending.lock().is_ok_and(|pending| {
            pending.as_ref().is_some_and(|pending| {
                pending.details.ui_token == ui_token
                    && pending.is_current(&app_state, Instant::now())
            })
        })
    {
        return Err(unavailable());
    }
    drop(guard);
    Ok(answer)
}

fn token_is_current(app: &AppHandle, ui_token: &str) -> bool {
    let state = app.state::<ClientIdentityReviewState>();
    let app_state = app.state::<AppState>();
    state.pending.lock().is_ok_and(|pending| {
        pending.as_ref().is_some_and(|pending| {
            pending.details.ui_token == ui_token && pending.is_current(&app_state, Instant::now())
        })
    })
}

async fn create(app: &AppHandle, ui_token: &str) -> Result<(), CommandErrorDto> {
    let window = app.get_window("main").ok_or_else(unavailable)?;
    let ui_token = ui_token.to_owned();
    let application = app.clone();
    // Tauri's add_child dispatches to the event thread and must not be called
    // from a synchronous Windows event callback. A late blocking task rechecks
    // the native token before showing or focusing anything.
    let task = tauri::async_runtime::spawn_blocking(move || {
        if !token_is_current(&application, &ui_token) {
            return Err(unavailable());
        }
        let builder = WebviewBuilder::new(WEBVIEW, protocol::window_url())
            .use_https_scheme(false)
            .incognito(true)
            .devtools(false)
            .focused(false)
            .on_navigation(protocol::navigation_allowed)
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_download(|_, _| false);
        let view = window
            .add_child(
                builder,
                tauri::LogicalPosition::new(-10_000.0, -10_000.0),
                tauri::LogicalSize::new(16.0, 16.0),
            )
            .map_err(|_| unavailable())?;
        if !token_is_current(&application, &ui_token) {
            let _ = view.close();
            return Err(unavailable());
        }
        presentation::position(&window, &view)?;
        if window.is_focused().unwrap_or(false) {
            view.set_focus().map_err(|_| unavailable())?;
        }
        Ok(())
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
}

fn is_present(app: &AppHandle) -> bool {
    app.get_window("main").is_some()
        && app.get_webview(WEBVIEW).is_some_and(|view| {
            view.window().label() == "main"
                && view
                    .url()
                    .is_ok_and(|url| protocol::navigation_allowed(&url))
        })
}
