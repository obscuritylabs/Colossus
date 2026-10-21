//! One-use certificate choice in a trusted child of the main Desktop window.

use std::{sync::Mutex, time::Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager as _, State, Webview};
use tokio::sync::oneshot;

use crate::{dto::CommandErrorDto, state::AppState};

#[cfg(any(
    test,
    feature = "embedded-chromium-preview",
    feature = "macos-contained-browser-diagnostic",
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
mod display;
#[cfg(any(
    test,
    feature = "embedded-chromium-preview",
    feature = "macos-contained-browser-diagnostic",
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
pub(crate) use display::ClientIdentityDisplay;
#[cfg(any(
    feature = "embedded-chromium-preview",
    feature = "macos-contained-browser-diagnostic",
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
mod runner;
#[cfg(any(
    feature = "embedded-chromium-preview",
    feature = "macos-contained-browser-diagnostic",
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
pub(crate) use runner::show;

mod presentation;
pub(crate) mod protocol;
#[cfg(test)]
mod tests;

pub(crate) const WEBVIEW: &str = "client-identity-review";

/// Display metadata only. The native broker retains the actual challenge key.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientIdentityDisplayCandidate {
    pub(crate) label: String,
    pub(crate) fingerprint_sha256: String,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReviewAppearance {
    color_theme: ColorTheme,
    dark_palette: DarkPalette,
    text_size: TextSize,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum ColorTheme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum DarkPalette {
    #[default]
    Colossus,
    Neutral,
    Black,
    Hacker,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum TextSize {
    Compact,
    #[default]
    Comfortable,
    Large,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewDto {
    ui_token: String,
    origin: String,
    candidates: Vec<ClientIdentityDisplayCandidate>,
    appearance: ReviewAppearance,
}

#[derive(Default)]
pub(crate) struct ClientIdentityReviewState {
    pending: Mutex<Option<PendingReview>>,
    appearance: Mutex<ReviewAppearance>,
}

struct PendingReview {
    details: ReviewDto,
    target_id: String,
    selection_epoch: u64,
    deadline: Instant,
    response: Option<oneshot::Sender<Option<String>>>,
}

impl PendingReview {
    fn is_current(&self, state: &AppState, now: Instant) -> bool {
        now < self.deadline && state.selection_is_current(&self.target_id, self.selection_epoch)
    }

    fn finish(
        &mut self,
        ui_token: &str,
        fingerprint: Option<&str>,
        now: Instant,
        selected: bool,
    ) -> Result<(), CommandErrorDto> {
        if self.details.ui_token != ui_token
            || now >= self.deadline
            || !selected
            || fingerprint.is_some_and(|fingerprint| {
                self.details
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.fingerprint_sha256 == fingerprint)
                    .count()
                    != 1
            })
        {
            return Err(unavailable());
        }
        self.response
            .take()
            .ok_or_else(unavailable)?
            .send(fingerprint.map(str::to_owned))
            .map_err(|_| unavailable())
    }
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "client_identity_review",
        "This certificate request is no longer available.",
    )
}

fn require_document(caller: &Webview) -> Result<(), CommandErrorDto> {
    if !caller.url().is_ok_and(|url| {
        protocol::document_authorized(caller.label(), caller.window().label(), &url)
    }) {
        return Err(unavailable());
    }
    Ok(())
}

/// Only appearance enums enter from the ordinary controller; never request data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn sync_client_identity_appearance(
    caller: Webview,
    state: State<'_, ClientIdentityReviewState>,
    appearance: ReviewAppearance,
) -> Result<(), CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    *state.appearance.lock().map_err(|_| unavailable())? = appearance;
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn client_identity_review_context(
    caller: Webview,
    state: State<'_, ClientIdentityReviewState>,
    app_state: State<'_, AppState>,
) -> Result<ReviewDto, CommandErrorDto> {
    require_document(&caller)?;
    state
        .pending
        .lock()
        .map_err(|_| unavailable())?
        .as_ref()
        .filter(|pending| {
            pending.response.is_some() && pending.is_current(&app_state, Instant::now())
        })
        .map(|pending| pending.details.clone())
        .ok_or_else(unavailable)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn finish_client_identity_review(
    caller: Webview,
    state: State<'_, ClientIdentityReviewState>,
    app_state: State<'_, AppState>,
    ui_token: String,
    fingerprint_sha256: Option<String>,
) -> Result<(), CommandErrorDto> {
    require_document(&caller)?;
    let mut pending = state.pending.lock().map_err(|_| unavailable())?;
    let pending = pending.as_mut().ok_or_else(unavailable)?;
    let now = Instant::now();
    let selected = pending.is_current(&app_state, now);
    pending.finish(&ui_token, fingerprint_sha256.as_deref(), now, selected)
}

#[cfg(any(
    test,
    feature = "embedded-chromium-preview",
    feature = "macos-contained-browser-diagnostic",
    feature = "windows-browser-development",
    feature = "windows-browser-functional-preview"
))]
async fn decision(
    receiver: oneshot::Receiver<Option<String>>,
    invalidated: impl std::future::Future<Output = ()>,
    view_failed: impl std::future::Future<Output = ()>,
    deadline: Instant,
) -> Result<Option<String>, CommandErrorDto> {
    tokio::select! {
        biased;
        () = invalidated => Err(unavailable()),
        () = view_failed => Err(unavailable()),
        () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Ok(None),
        answer = receiver => Ok(if Instant::now() >= deadline { None } else { answer.unwrap_or_default() }),
    }
}

pub(crate) fn is_active(app: &AppHandle) -> bool {
    app.try_state::<ClientIdentityReviewState>()
        .is_some_and(|state| {
            state
                .pending
                .lock()
                .map_or(true, |pending| pending.is_some())
        })
}

fn cancel(app: &AppHandle, ui_token: Option<&str>) {
    let state = app.state::<ClientIdentityReviewState>();
    let cancelled = state.pending.lock().is_ok_and(|mut pending| {
        if pending
            .as_ref()
            .is_some_and(|pending| ui_token.is_none_or(|token| token == pending.details.ui_token))
        {
            pending.take(); // Dropping the sender denies an unfinished request.
            true
        } else {
            false
        }
    });
    if cancelled && let Some(view) = app.get_webview(WEBVIEW) {
        let _ = view.close();
    }
}

pub(crate) fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    presentation::handle_window_event(window, event);
}

pub(crate) fn controller_loading(app: &AppHandle) {
    cancel(app, None);
}
