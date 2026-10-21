//! Explicit normal-completion suspension; cancellation remains terminal.
use super::*;

impl Host {
    pub(super) async fn suspend(
        &self,
        id: &BrowserSessionId,
        generation: u64,
    ) -> Result<(), BrowserDriverError> {
        let session = self.existing(id)?;
        {
            let mut state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if state.closing || state.generation > generation {
                return Err(BrowserDriverError::Stale);
            }
            if state.suspended {
                return Ok(());
            }
            state.authority.cancel();
            state.cancelled_through = Some(generation);
            state.quiescing = true;
            state.transfer = None;
            state.upload = None;
            state.download = None;
        }
        let _operation = session.operation.lock().await;
        self.driver.suspend_session(id, generation).await?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.quiescing = false;
        state.suspended = true;
        Ok(())
    }

    pub(super) async fn resume(
        &self,
        id: &BrowserSessionId,
        run_id: &str,
        generation: u64,
    ) -> Result<(), BrowserDriverError> {
        if run_id.is_empty() || run_id.len() > 256 || run_id.chars().any(char::is_control) {
            return Err(BrowserDriverError::Denied);
        }
        let session = self.existing(id)?;
        let _operation = session.operation.lock().await;
        {
            let state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if state.closing
                || state.quiescing
                || !state.suspended
                || generation <= state.cancelled_through.unwrap_or(state.generation)
            {
                return Err(BrowserDriverError::Stale);
            }
        }
        let authority = RunControl::default();
        let control = BrowserDriverControl::new(RunControl::default(), authority.clone());
        self.driver
            .resume_session(id, run_id, generation, &control)
            .await?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.authority = authority;
        state.generation = generation;
        state.writer_run = Some(run_id.to_owned());
        state.suspended = false;
        Ok(())
    }
}
