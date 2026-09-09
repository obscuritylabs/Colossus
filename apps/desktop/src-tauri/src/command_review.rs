//! Native-owned read-only command review. Only the OS confirmation can authorize.

use std::sync::Mutex;

use colossus_sdk::{ApprovalInteraction, RespondInteractionRequest};
use serde::Serialize;
use tauri::{AppHandle, Manager as _, State, Webview, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

use crate::{
    commands::target_consent_description,
    dto::{CommandApprovalContextDto, CommandErrorDto},
    state::{AppState, TargetHandle},
};

pub(crate) const WINDOW: &str = "command-approval";

#[derive(Default)]
pub(crate) struct CommandReviewState(Mutex<Option<PendingReview>>);

struct PendingReview {
    details: CommandReviewDto,
    response: Option<oneshot::Sender<bool>>,
}

impl PendingReview {
    fn finish(&mut self, review_id: &str, approved: bool) -> Result<(), CommandErrorDto> {
        if self.details.review_id != review_id {
            return Err(unavailable());
        }
        self.response
            .take()
            .ok_or_else(unavailable)?
            .send(approved)
            .map_err(|_| unavailable())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandReviewDto {
    review_id: String,
    target: String,
    command_context: CommandApprovalContextDto,
}

pub(crate) struct ReviewWindow {
    app: AppHandle,
    review_id: String,
}

impl ReviewWindow {
    pub(crate) fn is_current(&self) -> bool {
        self.app.get_webview_window(WINDOW).is_some()
            && self
                .app
                .state::<CommandReviewState>()
                .0
                .lock()
                .is_ok_and(|pending| {
                    pending
                        .as_ref()
                        .is_some_and(|pending| pending.details.review_id == self.review_id)
                })
    }
}

impl Drop for ReviewWindow {
    fn drop(&mut self) {
        cancel_review(&self.app, &self.review_id);
        if let Some(window) = self.app.get_webview_window(WINDOW) {
            let _ = window.close();
        }
    }
}

fn cancel_review(app: &AppHandle, review_id: &str) {
    let state = app.state::<CommandReviewState>();
    if let Ok(mut pending) = state.0.lock()
        && pending
            .as_ref()
            .is_some_and(|pending| pending.details.review_id == review_id)
    {
        pending.take(); // Dropping the one-use sender denies any unfinished review.
    }
}

fn unavailable() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "approval",
        "The command approval is no longer current. Refresh the run.",
    )
}

fn require_review_document(caller: &Webview) -> Result<(), CommandErrorDto> {
    if caller.label() != WINDOW
        || !caller
            .url()
            .is_ok_and(|url| crate::command_review_protocol::navigation_allowed(&url))
    {
        return Err(unavailable());
    }
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn command_review_context(
    caller: Webview,
    state: State<'_, CommandReviewState>,
) -> Result<CommandReviewDto, CommandErrorDto> {
    require_review_document(&caller)?;
    state
        .0
        .lock()
        .map_err(|_| unavailable())?
        .as_ref()
        .map(|pending| pending.details.clone())
        .ok_or_else(unavailable)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::needless_pass_by_value)] // Tauri injects owned command arguments.
pub(crate) fn finish_command_review(
    caller: Webview,
    state: State<'_, CommandReviewState>,
    review_id: String,
    approved: bool,
) -> Result<(), CommandErrorDto> {
    require_review_document(&caller)?;
    let mut pending = state.0.lock().map_err(|_| unavailable())?;
    let pending = pending.as_mut().ok_or_else(unavailable)?;
    pending.finish(&review_id, approved)
}

/// Fetch the exact authoritative challenge; renderer text is never consumed.
pub(crate) async fn pending_approval(
    target: &TargetHandle,
    request: &RespondInteractionRequest,
) -> Result<ApprovalInteraction, CommandErrorDto> {
    crate::approval_adapter::pending(&target.client, request)
        .await
        .map_err(CommandErrorDto::from_api)
}

/// Return a guard keeping full command details visible through OS confirmation.
pub(crate) async fn review_command(
    app: &AppHandle,
    target: &TargetHandle,
    target_id: &str,
    epoch: u64,
    request: &RespondInteractionRequest,
    approval: &ApprovalInteraction,
) -> Result<Option<ReviewWindow>, CommandErrorDto> {
    let context = approval.command_context.clone().ok_or_else(unavailable)?;
    context.validate().map_err(|_| unavailable())?;
    let review_id = uuid::Uuid::new_v4().to_string();
    let (sender, mut receiver) = oneshot::channel();
    let state = app.state::<CommandReviewState>();
    {
        let mut pending = state.0.lock().map_err(|_| unavailable())?;
        if pending.is_some() || app.get_webview_window(WINDOW).is_some() {
            return Err(CommandErrorDto::busy("A command review is already open."));
        }
        *pending = Some(PendingReview {
            details: CommandReviewDto {
                review_id: review_id.clone(),
                target: target_consent_description(&target.consent)?,
                command_context: context.into(),
            },
            response: Some(sender),
        });
    }
    let guard = ReviewWindow {
        app: app.clone(),
        review_id: review_id.clone(),
    };
    let window =
        WebviewWindowBuilder::new(app, WINDOW, crate::command_review_protocol::window_url())
            .use_https_scheme(true)
            .on_navigation(crate::command_review_protocol::navigation_allowed)
            .title("Review Colossus command")
            .inner_size(900.0, 700.0)
            .min_inner_size(420.0, 360.0)
            .center()
            .build()
            .map_err(|_| unavailable())?;
    let application = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            cancel_review(&application, &review_id);
        }
    });
    let app_state = app.state::<AppState>();
    let mut selection = app_state.subscribe_selection();
    let mut refresh = tokio::time::interval(std::time::Duration::from_secs(2));
    let deadline = tokio::time::sleep(std::time::Duration::from_mins(5));
    tokio::pin!(deadline);
    loop {
        if !app_state.selection_is_current(target_id, epoch) {
            return Err(unavailable());
        }
        tokio::select! {
            answer = &mut receiver => return Ok(answer.unwrap_or(false).then_some(guard)),
            _ = selection.changed() => return Err(unavailable()),
            () = &mut deadline => return Ok(None),
            _ = refresh.tick() => {
                if pending_approval(target, request).await? != *approval { return Err(unavailable()); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review() -> (PendingReview, oneshot::Receiver<bool>) {
        let (sender, receiver) = oneshot::channel();
        (
            PendingReview {
                details: CommandReviewDto {
                    review_id: "one-use-review".into(),
                    target: "Managed Local".into(),
                    command_context: colossus_sdk::CommandApprovalContext {
                        justification: "Check the build.".into(),
                        executable: "build".into(),
                        arguments: vec![],
                        working_directory: "/work".into(),
                        redacted: false,
                    }
                    .into(),
                },
                response: Some(sender),
            },
            receiver,
        )
    }

    #[tokio::test]
    async fn review_acknowledgement_is_one_use_and_closing_cancels_it() {
        for approved in [true, false] {
            let (mut pending, mut receiver) = review();
            assert!(pending.finish("stale-review", approved).is_err());
            assert_eq!(
                receiver.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            );
            pending.finish("one-use-review", approved).unwrap();
            assert_eq!(receiver.await.unwrap(), approved);
            assert!(pending.finish("one-use-review", approved).is_err());
        }
        let (pending, receiver) = review();
        drop(pending);
        assert!(receiver.await.is_err());
    }
}
