use super::*;
use colossus_api::{
    Idempotent, Interaction, InteractionResponse, ListRunsRequest, ListRunsResponse, Run,
    RunExecutionRequest, ThreadLifecycle,
};

impl RunRepository for TerminalRaceRepository {
    fn get_run(&self, caller: &CallerContext, run_id: &str) -> ApiResult<Option<Run>> {
        self.inner.get_run(caller, run_id)
    }

    fn updates_after(
        &self,
        caller: &CallerContext,
        run_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> ApiResult<Vec<RunUpdate>> {
        self.page_reads.fetch_add(1, Ordering::AcqRel);
        let updates = self
            .inner
            .updates_after(caller, run_id, after_sequence, limit)?;
        if !self.committed.swap(true, Ordering::AcqRel) {
            assert!(updates.is_empty());
            // Commit exactly after the watch's page read and before its terminal
            // snapshot read. No timing, notifications, or provider execution is used.
            self.inner
                .append_update(caller, run_id, after_sequence, result())?;
            Ok(updates)
        } else if self.hide_terminal_tail {
            Ok(Vec::new())
        } else {
            Ok(updates)
        }
    }

    fn resolve_create_run(
        &self,
        _: &CallerContext,
        _: &CreateRunRequest,
    ) -> ApiResult<Option<Run>> {
        unreachable!()
    }
    fn create_run(
        &self,
        _: &CallerContext,
        _: &CreateRunRequest,
        _: &NewRun,
    ) -> ApiResult<Idempotent<Run>> {
        unreachable!()
    }
    fn execution_request(
        &self,
        _: &CallerContext,
        _: &str,
    ) -> ApiResult<Option<RunExecutionRequest>> {
        unreachable!()
    }
    fn recoverable_run(
        &self,
        _: &CallerContext,
        _: &str,
    ) -> ApiResult<Option<(Run, RunExecutionRequest)>> {
        unreachable!()
    }
    fn list_runs(&self, _: &CallerContext, _: &ListRunsRequest) -> ApiResult<ListRunsResponse> {
        unreachable!()
    }
    fn archive_thread(
        &self,
        _: &CallerContext,
        _: &str,
        _: &IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle> {
        unreachable!()
    }
    fn restore_thread(
        &self,
        _: &CallerContext,
        _: &str,
        _: &IdempotencyKey,
    ) -> ApiResult<ThreadLifecycle> {
        unreachable!()
    }
    fn append_update(
        &self,
        _: &CallerContext,
        _: &str,
        _: u64,
        _: RunUpdateKind,
    ) -> ApiResult<RunUpdate> {
        unreachable!()
    }
    fn request_cancellation(
        &self,
        _: &CallerContext,
        _: &str,
        _: &IdempotencyKey,
    ) -> ApiResult<Idempotent<Run>> {
        unreachable!()
    }
    fn respond_interaction(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
        _: &str,
        _: &IdempotencyKey,
        _: InteractionResponse,
    ) -> ApiResult<Idempotent<Interaction>> {
        unreachable!()
    }
    fn resolve_interaction_response(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
        _: &str,
        _: &IdempotencyKey,
        _: &InteractionResponse,
    ) -> ApiResult<Option<Interaction>> {
        unreachable!()
    }
}
