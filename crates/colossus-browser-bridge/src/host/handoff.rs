use super::*;
use colossus_ports::BrowserNativeHandoffRequest;

impl Host {
    pub(super) async fn confirm_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        let session = self.existing(&request.session_id)?;
        let _operation = session.operation.lock().await;
        let authority = {
            let state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let open = state.open.as_ref().ok_or(BrowserDriverError::Stale)?;
            crate::validation::native_handoff(&request, open)?;
            if request.binding != self.enrollment.binding
                || state.closing
                || state.quiescing
                || state.authority.is_cancelled()
                || state.generation != 0
                || state.writer_run.is_some()
                || state.cancelled_through.is_some()
                || state.handoff_confirmed
                || state
                    .tabs
                    .get(&request.expected_target.tab_id)
                    .is_none_or(|tab| tab.document_id != request.expected_target.document_id)
            {
                return Err(BrowserDriverError::Stale);
            }
            state.authority.clone()
        };
        let tab = self
            .driver
            .confirm_native_handoff(
                request.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing
            || state.authority.is_cancelled()
            || tab.tab_id != request.confirmed_target.tab_id
            || tab.document_id != request.confirmed_target.document_id
            || tab.title.len() > 1024
            || tab
                .origin
                .as_ref()
                .is_some_and(|origin| !self.enrollment.allowed_origins.contains(origin))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.tabs.insert(tab.tab_id.clone(), tab.clone());
        state.handoff_confirmed = true;
        Ok(tab)
    }
}
