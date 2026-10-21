use crate::feed::RunFeeds;
use colossus_api::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, CallerContext, OutcomeCertainty,
    RunRepository, RunUpdate, RunUpdateKind,
};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

const WATCH_PAGE_SIZE: usize = 16;

pub(super) struct RunWatch {
    pub(super) repository: Arc<dyn RunRepository>,
    pub(super) feeds: Arc<RunFeeds>,
    pub(super) caller: CallerContext,
    pub(super) run_id: String,
    pub(super) cursor: u64,
    pub(super) notifications: watch::Receiver<u64>,
}

impl RunWatch {
    pub(super) async fn run(
        mut self,
        sender: mpsc::Sender<ApiResult<RunUpdate>>,
        poll_access: bool,
        check_access: impl Fn() -> ApiResult<()>,
    ) {
        let mut terminal_gap = None;
        loop {
            if let Err(error) = check_access() {
                let _ = sender.send(Err(error)).await;
                return;
            }
            match self.repository.updates_after(
                &self.caller,
                &self.run_id,
                self.cursor,
                WATCH_PAGE_SIZE,
            ) {
                Ok(updates) => {
                    let had_updates = !updates.is_empty();
                    let terminal_update = updates.iter().any(|update| {
                        matches!(
                            &update.kind,
                            RunUpdateKind::Result { .. }
                                | RunUpdateKind::Failure { .. }
                                | RunUpdateKind::Cancellation { .. }
                        )
                    });
                    for update in updates {
                        self.cursor = update.sequence;
                        if sender.send(Ok(update)).await.is_err() {
                            return;
                        }
                    }
                    if terminal_update {
                        self.feeds.close(&self.run_id);
                        return;
                    }
                    if had_updates {
                        terminal_gap = None;
                        continue;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                    return;
                }
            }
            match self.repository.get_run(&self.caller, &self.run_id) {
                Ok(Some(current)) if current.status.is_terminal() => {
                    if self.cursor < current.last_sequence {
                        // A terminal append can commit between the empty page and
                        // this snapshot. Drain its released tail before EOF. The
                        // API stream is contiguous; a second empty read at the
                        // same terminal head is a storage invariant failure.
                        if terminal_gap == Some(current.last_sequence) {
                            let _ = sender.send(Err(self.missing_tail())).await;
                            return;
                        }
                        terminal_gap = Some(current.last_sequence);
                        continue;
                    }
                    self.feeds.close(&self.run_id);
                    return;
                }
                Ok(Some(_)) => {}
                Ok(None) => return,
                Err(error) => {
                    let _ = sender.send(Err(error)).await;
                    return;
                }
            }
            tokio::select! {
                () = sender.closed() => return,
                changed = self.notifications.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(5)), if poll_access => {}
            }
        }
    }

    fn missing_tail(&self) -> ApiError {
        ApiError {
            code: ApiErrorCode::Internal,
            reason: ApiErrorReason::InternalInvariant,
            message: "the durable run feed is incomplete".into(),
            correlation_id: Some(self.caller.request_id().clone()),
            retryable: false,
            outcome: OutcomeCertainty::Known,
            violations: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
