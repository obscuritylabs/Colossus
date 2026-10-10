//! Closed private upload/download calls, with no transport or artifact authority in models.
use super::*;
use colossus_ports::{
    BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserUploadCommitRequest,
    BrowserUploadPrepareRequest, BrowserUploadReceipt, BrowserUploadWriteRequest,
};
impl BrowserBridgeDriver {
    async fn transfer_call(
        &self,
        request: Request,
        session: &BrowserSessionId,
        generation: u64,
        advance: bool,
        control: &BrowserDriverControl,
    ) -> Result<Response, BrowserDriverError> {
        self.check_generation(session, generation, advance)?;
        self.call(
            request,
            session.clone(),
            generation,
            control,
            Duration::from_millis(u64::from(
                self.enrollment.capabilities.limits.action_timeout_ms,
            )),
        )
        .await
    }
    fn transfer_owner(
        &self,
        request: &BrowserScreenshotReadRequest,
    ) -> Result<(), BrowserDriverError> {
        if request.binding != self.enrollment.binding || request.control_generation == 0 {
            return Err(BrowserDriverError::Denied);
        }
        Ok(())
    }
    pub(super) async fn prepare_upload_private(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if request.command.binding != self.enrollment.binding
            || request.command.control_generation == 0
        {
            return Err(BrowserDriverError::Denied);
        }
        if !self
            .enrollment
            .capabilities
            .actions
            .contains(&request.command.action.kind())
        {
            return Err(BrowserDriverError::Unsupported);
        }
        crate::validation::transfer::upload(&request)?;
        let session = request.command.session_id.clone();
        let generation = request.command.control_generation;
        match self
            .transfer_call(
                Request::BeginUpload {
                    request: Box::new(request),
                },
                &session,
                generation,
                true,
                control,
            )
            .await?
        {
            Response::UploadPrepared { receipt }
                if crate::validation::transfer::receipt(&receipt, None, 0).is_ok() =>
            {
                Ok(receipt)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
    pub(super) async fn write_upload_private(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.transfer_owner(&request.transfer)?;
        let size = crate::validation::transfer::upload_chunk(
            &request.data_base64,
            colossus_ports::MAX_BROWSER_TRANSFER_BYTES,
        )?;
        let next = request
            .transfer
            .offset
            .checked_add(size)
            .filter(|offset| *offset <= colossus_ports::MAX_BROWSER_TRANSFER_BYTES)
            .ok_or(BrowserDriverError::Denied)?;
        let session = request.transfer.session_id.clone();
        let generation = request.transfer.control_generation;
        let token = request.transfer.transfer_id.clone();
        match self
            .transfer_call(
                Request::WriteUpload {
                    request: Box::new(request),
                },
                &session,
                generation,
                false,
                control,
            )
            .await?
        {
            Response::UploadProgress { receipt }
                if crate::validation::transfer::receipt(&receipt, Some(&token), next).is_ok() =>
            {
                Ok(receipt)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
    pub(super) async fn commit_upload_private(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.transfer_owner(&request)?;
        let session = request.session_id.clone();
        let generation = request.control_generation;
        let tab = request.target.tab_id.clone();
        match self
            .transfer_call(
                Request::CommitUpload {
                    request: Box::new(request),
                },
                &session,
                generation,
                false,
                control,
            )
            .await?
        {
            Response::Uploaded { observation }
                if observation.session_id == session
                    && observation.tab.tab_id == tab
                    && observation.snapshot.is_none()
                    && observation.tab.title.len() <= 1024
                    && observation
                        .tab
                        .origin
                        .as_ref()
                        .is_none_or(|origin| self.enrollment.allowed_origins.contains(origin))
                    && crate::validation::bounded_observation(
                        &observation,
                        self.enrollment.capabilities.limits.max_observation_bytes as usize,
                    )
                    .is_ok() =>
            {
                Ok(observation)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
    pub(super) async fn download_private(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        if !matches!(
            command.action,
            colossus_contracts::BrowserAction::Download { .. }
        ) {
            return Err(BrowserDriverError::Unsupported);
        }
        if command.binding != self.enrollment.binding || command.control_generation == 0 {
            return Err(BrowserDriverError::Denied);
        }
        if !self
            .enrollment
            .capabilities
            .actions
            .contains(&command.action.kind())
        {
            return Err(BrowserDriverError::Unsupported);
        }
        let session = command.session_id.clone();
        let generation = command.control_generation;
        match self
            .transfer_call(
                Request::Download {
                    command: Box::new(command.clone()),
                },
                &session,
                generation,
                true,
                control,
            )
            .await?
        {
            Response::Downloaded { descriptor }
                if crate::validation::transfer::download(
                    &descriptor,
                    &command,
                    &self.enrollment.allowed_origins,
                )
                .is_ok() =>
            {
                Ok(descriptor)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
    pub(super) async fn read_download_private(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.transfer_owner(&request)?;
        let session = request.session_id.clone();
        let generation = request.control_generation;
        let offset = request.offset;
        match self
            .transfer_call(
                Request::ReadDownload {
                    request: Box::new(request),
                },
                &session,
                generation,
                false,
                control,
            )
            .await?
        {
            Response::DownloadChunk { chunk }
                if crate::validation::transfer::download_chunk(
                    &chunk,
                    offset,
                    if offset == 0 && chunk.data_base64.is_empty() {
                        0
                    } else {
                        colossus_ports::MAX_BROWSER_TRANSFER_BYTES
                    },
                )
                .is_ok() =>
            {
                Ok(chunk)
            }
            _ => Err(self.invalid_response(&session).await),
        }
    }
}
