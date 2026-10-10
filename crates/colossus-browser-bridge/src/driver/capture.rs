use super::*;
use colossus_contracts::BrowserAction;

impl BrowserBridgeDriver {
    pub(super) fn check_generation(
        &self,
        session_id: &BrowserSessionId,
        generation: u64,
        advance: bool,
    ) -> Result<(), BrowserDriverError> {
        if !self.available.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Unavailable);
        }
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let session = sessions
            .get_mut(session_id)
            .ok_or(BrowserDriverError::Stale)?;
        if session.closing
            || generation < session.generation
            || (!advance && generation != session.generation)
        {
            return Err(BrowserDriverError::Stale);
        }
        if advance {
            session.generation = generation;
        }
        Ok(())
    }

    pub(super) async fn invalid_response(&self, session: &BrowserSessionId) -> BrowserDriverError {
        self.available.store(false, Ordering::Release);
        let _ = self.cancel_session(session).await;
        BrowserDriverError::OutcomeUnknown
    }

    pub(super) async fn capture_private(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        if command.binding != self.enrollment.binding || command.control_generation == 0 {
            return Err(BrowserDriverError::Denied);
        }
        if !matches!(command.action, BrowserAction::Screenshot { .. })
            || !self
                .enrollment
                .capabilities
                .actions
                .contains(&command.action.kind())
        {
            return Err(BrowserDriverError::Unsupported);
        }
        self.check_generation(&command.session_id, command.control_generation, true)?;
        let session = command.session_id.clone();
        let response = self
            .call(
                Request::Capture {
                    command: Box::new(command.clone()),
                },
                session.clone(),
                command.control_generation,
                control,
                Duration::from_millis(u64::from(
                    self.enrollment.capabilities.limits.action_timeout_ms,
                )),
            )
            .await?;
        match response {
            Response::Captured { descriptor }
                if crate::validation::screenshot_descriptor(&descriptor, &command).is_ok() =>
            {
                Ok(descriptor)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }

    pub(super) async fn read_screenshot_private(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if request.binding != self.enrollment.binding || request.control_generation == 0 {
            return Err(BrowserDriverError::Denied);
        }
        self.check_generation(&request.session_id, request.control_generation, false)?;
        let session = request.session_id.clone();
        let offset = request.offset;
        let response = self
            .call(
                Request::ReadScreenshot {
                    request: Box::new(request.clone()),
                },
                session.clone(),
                request.control_generation,
                control,
                Duration::from_millis(u64::from(
                    self.enrollment.capabilities.limits.action_timeout_ms,
                )),
            )
            .await?;
        match response {
            Response::ScreenshotChunk { chunk }
                if crate::validation::screenshot_chunk(
                    &chunk,
                    offset,
                    colossus_ports::MAX_BROWSER_SCREENSHOT_BYTES,
                )
                .is_ok() =>
            {
                Ok(chunk)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }

    pub(super) async fn confirm_handoff_private(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if request.binding != self.enrollment.binding || request.native_document_generation == 0 {
            return Err(BrowserDriverError::Denied);
        }
        self.check_generation(&request.session_id, 0, false)?;
        let session = request.session_id.clone();
        let response = self
            .call(
                Request::ConfirmNativeHandoff {
                    request: Box::new(request.clone()),
                },
                session.clone(),
                0,
                control,
                self.cleanup_deadline(),
            )
            .await?;
        match response {
            Response::NativeHandoffConfirmed { tab }
                if tab.tab_id == request.confirmed_target.tab_id
                    && tab.document_id == request.confirmed_target.document_id
                    && tab.title.len() <= 1024
                    && tab
                        .origin
                        .as_ref()
                        .is_none_or(|origin| self.enrollment.allowed_origins.contains(origin)) =>
            {
                Ok(tab)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
}
