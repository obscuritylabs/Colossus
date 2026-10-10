use super::*;
use colossus_ports::{
    BrowserScreenshotChunk, BrowserScreenshotDescriptor, BrowserScreenshotReadRequest,
};

pub(super) struct Transfer {
    descriptor: BrowserScreenshotDescriptor,
    run_id: String,
    offset: u32,
}

impl Host {
    pub(super) fn existing(
        &self,
        id: &BrowserSessionId,
    ) -> Result<Arc<Session>, BrowserDriverError> {
        self.sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .get(id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)
    }

    pub(super) async fn capture(
        &self,
        command: BrowserDriverCommand,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        if !matches!(command.action, BrowserAction::Screenshot { .. }) {
            return Err(BrowserDriverError::Unsupported);
        }
        let session = self.existing(&command.session_id)?;
        let _operation = session.operation.lock().await;
        let authority = self.begin_command(&session, &command)?;
        let descriptor = self
            .driver
            .capture(
                command.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        crate::validation::screenshot_descriptor(&descriptor, &command)?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.transfer = Some(Transfer {
            descriptor: descriptor.clone(),
            run_id: command.run_id,
            offset: 0,
        });
        Ok(descriptor)
    }

    pub(super) async fn read_screenshot(
        &self,
        request: BrowserScreenshotReadRequest,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        let session = self.existing(&request.session_id)?;
        let _operation = session.operation.lock().await;
        let (authority, remaining) = {
            let state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            crate::validation::screenshot_read(
                &request,
                state.open.as_ref().ok_or(BrowserDriverError::Stale)?,
            )?;
            let transfer = state.transfer.as_ref().ok_or(BrowserDriverError::Stale)?;
            if state.closing
                || state.quiescing
                || state.authority.is_cancelled()
                || state.generation != request.control_generation
                || state.writer_run.as_ref() != Some(&request.run_id)
                || transfer.run_id != request.run_id
                || transfer.descriptor.target != request.target
                || transfer.descriptor.transfer_id != request.transfer_id
                || transfer.offset != request.offset
                || state
                    .tabs
                    .get(&request.target.tab_id)
                    .is_none_or(|tab| tab.document_id != request.target.document_id)
            {
                return Err(BrowserDriverError::Stale);
            }
            (
                state.authority.clone(),
                transfer.descriptor.size_bytes - transfer.offset,
            )
        };
        let chunk = self
            .driver
            .read_screenshot_chunk(
                request.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        let length = crate::validation::screenshot_chunk(&chunk, request.offset, remaining)?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let transfer = state
            .transfer
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        transfer.offset += length;
        if transfer.offset == transfer.descriptor.size_bytes {
            state.transfer = None;
        }
        Ok(chunk)
    }
}
