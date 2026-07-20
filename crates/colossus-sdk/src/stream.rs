use crate::AgentRunClient;
use crate::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, GetRunRequest, RunUpdate, RunUpdateKind,
    RunUpdateStream, WatchRunRequest,
};
use colossus_api::OutcomeCertainty as ApiOutcomeCertainty;
use futures::{Stream, StreamExt as _, stream};
#[cfg(feature = "daemon")]
use std::time::Duration;
use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

#[cfg(feature = "daemon")]
const INITIAL_RECONNECT_BACKOFF: Duration = Duration::from_millis(250);
#[cfg(feature = "daemon")]
const MAXIMUM_RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Ordered, owned run-update stream suitable for a Tauri async command task.
///
/// A command can repeatedly call [`Self::next_update`] and forward each item through a
/// Tauri channel. Duplicate at-least-once deliveries are removed and a sequence gap or
/// cross-run update fails closed. Daemon and sidecar streams reconnect only this
/// read-only watch operation after `UNAVAILABLE` or a non-terminal clean close,
/// resuming from the last verified durable cursor with bounded exponential backoff.
/// A clean close at a terminal run's exact `last_sequence` completes without
/// reconnecting; the SDK verifies that state with a read-only `GetRun`.
/// Embedded and custom checked streams perform the same exact reconciliation once and
/// fail closed if the summary is unavailable, non-terminal, for another run, or at a
/// different cursor.
/// Dropping this value disconnects only the watcher and never cancels the run.
pub struct RunUpdates {
    inner: RunUpdateStream,
}

impl RunUpdates {
    pub(crate) fn checked(
        client: Arc<dyn AgentRunClient>,
        inner: RunUpdateStream,
        request: WatchRunRequest,
    ) -> Self {
        let state = CheckedState {
            client,
            stream: inner,
            run_id: request.run_id,
            cursor: request.after_sequence,
            done: false,
        };
        Self {
            inner: Box::pin(stream::unfold(state, checked_next)),
        }
    }

    #[cfg(feature = "daemon")]
    pub(crate) fn resilient(
        client: Arc<dyn AgentRunClient>,
        request: WatchRunRequest,
        initial_stream: Option<RunUpdateStream>,
    ) -> Self {
        let state = ResilientState {
            client,
            stream: initial_stream,
            run_id: request.run_id,
            cursor: request.after_sequence,
            backoff: INITIAL_RECONNECT_BACKOFF,
            done: false,
        };
        Self {
            inner: Box::pin(stream::unfold(state, resilient_next)),
        }
    }

    /// Receive the next unique, contiguous durable update.
    pub async fn next_update(&mut self) -> Option<ApiResult<RunUpdate>> {
        self.inner.next().await
    }
}

struct CheckedState {
    client: Arc<dyn AgentRunClient>,
    stream: RunUpdateStream,
    run_id: String,
    cursor: u64,
    done: bool,
}

async fn checked_next(mut state: CheckedState) -> Option<(ApiResult<RunUpdate>, CheckedState)> {
    loop {
        if state.done {
            return None;
        }
        let Some(item) = state.stream.next().await else {
            state.done = true;
            let reconciliation = state
                .client
                .get_run(GetRunRequest {
                    run_id: state.run_id.clone(),
                })
                .await;
            return match reconciliation {
                Ok(response)
                    if response.run.run_id == state.run_id
                        && response.run.terminal.is_some()
                        && response.run.last_sequence == state.cursor =>
                {
                    None
                }
                Ok(response) if response.run.run_id != state.run_id => Some((
                    Err(protocol_error(
                        "the run summary returned a different run during watch reconciliation",
                    )),
                    state,
                )),
                Ok(response) if response.run.last_sequence != state.cursor => Some((
                    Err(protocol_error(
                        "the run summary did not match the verified watch cursor",
                    )),
                    state,
                )),
                Ok(_) => Some((
                    Err(protocol_error(
                        "the run watch closed without an exact terminal run summary",
                    )),
                    state,
                )),
                Err(_) => Some((
                    Err(protocol_error(
                        "the run watch closed and its terminal state could not be reconciled",
                    )),
                    state,
                )),
            };
        };
        match validate_update(&state.run_id, &mut state.cursor, item) {
            Ok(None) => {}
            Ok(Some((update, terminal))) => {
                state.done = terminal;
                return Some((Ok(update), state));
            }
            Err(error) => {
                state.done = true;
                return Some((Err(error), state));
            }
        }
    }
}

#[cfg(feature = "daemon")]
struct ResilientState {
    client: Arc<dyn AgentRunClient>,
    stream: Option<RunUpdateStream>,
    run_id: String,
    cursor: u64,
    backoff: Duration,
    done: bool,
}

#[cfg(feature = "daemon")]
async fn resilient_next(
    mut state: ResilientState,
) -> Option<(ApiResult<RunUpdate>, ResilientState)> {
    loop {
        if state.done {
            return None;
        }
        if state.client.is_closed() {
            state.done = true;
            return Some((Err(closed_watch_error()), state));
        }

        if state.stream.is_none() {
            let request = WatchRunRequest {
                run_id: state.run_id.clone(),
                after_sequence: state.cursor,
            };
            match state.client.watch_run(request).await {
                Ok(stream) => state.stream = Some(stream),
                Err(error) if error.code == ApiErrorCode::Unavailable && error.retryable => {
                    if !reconnect_delay(&mut state).await {
                        state.done = true;
                        return Some((Err(closed_watch_error()), state));
                    }
                    continue;
                }
                Err(error) => {
                    state.done = true;
                    return Some((Err(error), state));
                }
            }
        }

        let item = state
            .stream
            .as_mut()
            .expect("stream is established above")
            .next()
            .await;
        match item {
            Some(item) => match validate_update(&state.run_id, &mut state.cursor, item) {
                Ok(None) => {}
                Ok(Some((update, terminal))) => {
                    state.backoff = INITIAL_RECONNECT_BACKOFF;
                    state.done = terminal;
                    return Some((Ok(update), state));
                }
                Err(error) if error.code == ApiErrorCode::Unavailable && error.retryable => {
                    state.stream = None;
                    if !reconnect_delay(&mut state).await {
                        state.done = true;
                        return Some((Err(closed_watch_error()), state));
                    }
                }
                Err(error) => {
                    state.done = true;
                    return Some((Err(error), state));
                }
            },
            None => {
                state.stream = None;
                match state
                    .client
                    .get_run(GetRunRequest {
                        run_id: state.run_id.clone(),
                    })
                    .await
                {
                    Ok(response) if response.run.run_id != state.run_id => {
                        state.done = true;
                        return Some((
                            Err(protocol_error(
                                "the run summary returned a different run during watch reconciliation",
                            )),
                            state,
                        ));
                    }
                    Ok(response) if response.run.last_sequence < state.cursor => {
                        state.done = true;
                        return Some((
                            Err(protocol_error(
                                "the run summary regressed behind the verified watch cursor",
                            )),
                            state,
                        ));
                    }
                    Ok(response)
                        if response.run.terminal.is_some()
                            && response.run.last_sequence == state.cursor =>
                    {
                        state.done = true;
                        return None;
                    }
                    Ok(_) => {}
                    Err(error) if error.code == ApiErrorCode::Unavailable && error.retryable => {}
                    Err(error) => {
                        state.done = true;
                        return Some((Err(error), state));
                    }
                }
                if !reconnect_delay(&mut state).await {
                    state.done = true;
                    return Some((Err(closed_watch_error()), state));
                }
            }
        }
    }
}

#[cfg(feature = "daemon")]
async fn reconnect_delay(state: &mut ResilientState) -> bool {
    if state.client.is_closed() {
        return false;
    }
    tokio::select! {
        () = tokio::time::sleep(state.backoff) => {}
        () = state.client.wait_closed() => return false,
    }
    state.backoff = state
        .backoff
        .checked_mul(2)
        .unwrap_or(MAXIMUM_RECONNECT_BACKOFF)
        .min(MAXIMUM_RECONNECT_BACKOFF);
    !state.client.is_closed()
}

fn validate_update(
    expected_run_id: &str,
    cursor: &mut u64,
    item: ApiResult<RunUpdate>,
) -> Result<Option<(RunUpdate, bool)>, ApiError> {
    let update = item?;
    if update.run_id != expected_run_id {
        return Err(protocol_error(
            "the run watch returned an update for a different run",
        ));
    }
    if update.sequence <= *cursor {
        return Ok(None);
    }
    if cursor.checked_add(1) != Some(update.sequence) {
        return Err(protocol_error(
            "the run watch returned a non-contiguous update sequence",
        ));
    }

    *cursor = update.sequence;
    let terminal = matches!(
        update.update,
        RunUpdateKind::Result(_) | RunUpdateKind::Failure { .. } | RunUpdateKind::Cancellation(_)
    );
    Ok(Some((update, terminal)))
}

fn protocol_error(message: &'static str) -> ApiError {
    ApiError {
        code: ApiErrorCode::Internal,
        reason: ApiErrorReason::InternalInvariant,
        message: message.into(),
        correlation_id: None,
        retryable: false,
        outcome: ApiOutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

#[cfg(feature = "daemon")]
fn closed_watch_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Unavailable,
        reason: ApiErrorReason::InternalInvariant,
        message: "the Colossus API client was closed".into(),
        correlation_id: None,
        retryable: false,
        outcome: ApiOutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

impl Stream for RunUpdates {
    type Item = ApiResult<RunUpdate>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(context)
    }
}

impl fmt::Debug for RunUpdates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("RunUpdates").finish_non_exhaustive()
    }
}
