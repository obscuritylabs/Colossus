//! Per-writer one-shot bounded file custody. Data never becomes an endpoint or path.
use super::*;
use colossus_contracts::BrowserObservation;
use colossus_ports::{
    BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserScreenshotChunk,
    BrowserScreenshotReadRequest, BrowserUploadCommitRequest, BrowserUploadPrepareRequest,
    BrowserUploadReceipt, BrowserUploadWriteRequest,
};
use tokio::time::Instant;

const TTL: Duration = Duration::from_secs(30);
pub(super) struct Upload {
    request: BrowserUploadPrepareRequest,
    token: String,
    offset: u32,
    expires: Instant,
}
pub(super) struct Download {
    descriptor: BrowserDownloadDescriptor,
    run_id: String,
    offset: u32,
    expires: Instant,
}
fn admitted(
    state: &SessionState,
    request: &BrowserScreenshotReadRequest,
) -> Result<RunControl, BrowserDriverError> {
    crate::validation::screenshot_read(
        request,
        state.open.as_ref().ok_or(BrowserDriverError::Stale)?,
    )?;
    if state.closing
        || state.quiescing
        || state.authority.is_cancelled()
        || state.generation != request.control_generation
        || state.writer_run.as_ref() != Some(&request.run_id)
        || state
            .tabs
            .get(&request.target.tab_id)
            .is_none_or(|tab| tab.document_id != request.target.document_id)
    {
        return Err(BrowserDriverError::Stale);
    }
    Ok(state.authority.clone())
}
fn upload_matches(upload: &Upload, request: &BrowserScreenshotReadRequest) -> bool {
    upload.request.command.run_id == request.run_id
        && upload.request.command.target == request.target
        && upload.token == request.transfer_id
        && upload.offset == request.offset
        && Instant::now() < upload.expires
}
impl Host {
    pub(super) async fn prepare_upload(
        &self,
        request: BrowserUploadPrepareRequest,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        crate::validation::transfer::upload(&request)?;
        let session = self.existing(&request.command.session_id)?;
        let _operation = session.operation.lock().await;
        let authority = self.begin_command(&session, &request.command)?;
        let receipt = self
            .driver
            .prepare_upload(
                request.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        crate::validation::transfer::receipt(&receipt, None, 0)?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.upload = Some(Upload {
            request,
            token: receipt.transfer_id.clone(),
            offset: 0,
            expires: Instant::now() + TTL,
        });
        Ok(receipt)
    }
    pub(super) async fn write_upload(
        &self,
        request: BrowserUploadWriteRequest,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let session = self.existing(&request.transfer.session_id)?;
        let _operation = session.operation.lock().await;
        let (authority, size) = {
            let state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let authority = admitted(&state, &request.transfer)?;
            let upload = state.upload.as_ref().ok_or(BrowserDriverError::Stale)?;
            if !upload_matches(upload, &request.transfer) {
                return Err(BrowserDriverError::Stale);
            }
            let size = crate::validation::transfer::upload_chunk(
                &request.data_base64,
                upload.request.descriptor.size_bytes - upload.offset,
            )?;
            (authority, size)
        };
        let offset = request.transfer.offset + size;
        let token = request.transfer.transfer_id.clone();
        let receipt = self
            .driver
            .write_upload_chunk(
                request,
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        crate::validation::transfer::receipt(&receipt, Some(&token), offset)?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state
            .upload
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?
            .offset = offset;
        Ok(receipt)
    }
    pub(super) async fn commit_upload(
        &self,
        request: BrowserUploadCommitRequest,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let session = self.existing(&request.session_id)?;
        let _operation = session.operation.lock().await;
        let (authority, command) = {
            let mut state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let authority = admitted(&state, &request)?;
            let upload = state.upload.as_ref().ok_or(BrowserDriverError::Stale)?;
            if !upload_matches(upload, &request)
                || upload.offset != upload.request.descriptor.size_bytes
            {
                return Err(BrowserDriverError::Stale);
            }
            let command = state
                .upload
                .take()
                .ok_or(BrowserDriverError::OutcomeUnknown)?
                .request
                .command;
            (authority, command)
        };
        let observation = self
            .driver
            .commit_upload(
                request,
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        crate::validation::observation(
            &observation,
            &command,
            state
                .open
                .as_ref()
                .ok_or(BrowserDriverError::OutcomeUnknown)?,
            &self.enrollment,
        )?;
        state
            .tabs
            .insert(observation.tab.tab_id.clone(), observation.tab.clone());
        Ok(observation)
    }
    pub(super) async fn download(
        &self,
        command: BrowserDriverCommand,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        if !matches!(command.action, BrowserAction::Download { .. }) {
            return Err(BrowserDriverError::Unsupported);
        }
        let session = self.existing(&command.session_id)?;
        let _operation = session.operation.lock().await;
        let authority = self.begin_command(&session, &command)?;
        let descriptor = self
            .driver
            .download(
                command.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        crate::validation::transfer::download(
            &descriptor,
            &command,
            &self.enrollment.allowed_origins,
        )?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.download = Some(Download {
            descriptor: descriptor.clone(),
            run_id: command.run_id,
            offset: 0,
            expires: Instant::now() + TTL,
        });
        Ok(descriptor)
    }
    pub(super) async fn read_download(
        &self,
        request: BrowserDownloadReadRequest,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        let session = self.existing(&request.session_id)?;
        let _operation = session.operation.lock().await;
        let (authority, remaining) = {
            let state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let authority = admitted(&state, &request)?;
            let download = state.download.as_ref().ok_or(BrowserDriverError::Stale)?;
            if download.run_id != request.run_id
                || download.descriptor.target != request.target
                || download.descriptor.transfer_id != request.transfer_id
                || download.offset != request.offset
                || Instant::now() >= download.expires
            {
                return Err(BrowserDriverError::Stale);
            }
            (authority, download.descriptor.size_bytes - download.offset)
        };
        let chunk = self
            .driver
            .read_download_chunk(
                request.clone(),
                &BrowserDriverControl::new(RunControl::default(), authority),
            )
            .await?;
        let length =
            crate::validation::transfer::download_chunk(&chunk, request.offset, remaining)?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let download = state
            .download
            .as_mut()
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        download.offset += length;
        if download.offset == download.descriptor.size_bytes {
            state.download = None;
        }
        Ok(chunk)
    }
}
