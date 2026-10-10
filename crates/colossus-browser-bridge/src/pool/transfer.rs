//! Supervised custody workers retain ownership before replying to their caller.
use super::*;
use std::time::Duration;
use tokio::time::Instant;
const TTL: Duration = Duration::from_secs(30);
pub(super) struct Upload {
    request: BrowserUploadPrepareRequest,
    receipt: BrowserUploadReceipt,
    expires: Instant,
}
pub(super) struct Download {
    descriptor: BrowserDownloadDescriptor,
    run: String,
    offset: u32,
    expires: Instant,
}
pub(super) fn retire(entry: &Entry) -> Result<(), BrowserDriverError> {
    let mut state = entry
        .state
        .lock()
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
    state.upload = None;
    state.download = None;
    Ok(())
}
fn alive(entry: &Entry, state: &State) -> Result<(), BrowserDriverError> {
    if state.terminal
        || entry.authority.is_cancelled()
        || entry.admission_fenced.load(Ordering::Acquire)
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(())
}
fn upload_matches(upload: &Upload, request: &BrowserScreenshotReadRequest) -> bool {
    upload.request.command.run_id == request.run_id
        && upload.request.command.target == request.target
        && upload.request.command.control_generation == request.control_generation
        && upload.receipt.transfer_id == request.transfer_id
        && upload.receipt.next_offset == request.offset
        && Instant::now() < upload.expires
}
impl BrowserHostPool {
    pub(super) async fn prepare_upload_private(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let entry = self
            .entry(&request.command.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::command_for_open(
            &request.command,
            &entry.request,
            &self.inner.capabilities,
        )?;
        crate::validation::transfer::upload(&request)?;
        let retained = entry.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                retire(&retained)?;
                let receipt = driver.prepare_upload(request.clone(), &control).await?;
                crate::validation::transfer::receipt(&receipt, None, 0)?;
                let mut state = retained
                    .state
                    .lock()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                alive(&retained, &state)?;
                state.upload = Some(Upload {
                    request,
                    receipt: receipt.clone(),
                    expires: Instant::now() + TTL,
                });
                Ok(receipt)
            })
        })
        .await
    }
    pub(super) async fn write_upload_private(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let entry = self
            .entry(&request.transfer.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::screenshot_read(&request.transfer, &entry.request)?;
        let retained = entry.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                let offset = {
                    let state = retained
                        .state
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    let upload = state.upload.as_ref().ok_or(BrowserDriverError::Stale)?;
                    if !upload_matches(upload, &request.transfer) {
                        return Err(BrowserDriverError::Stale);
                    }
                    upload.receipt.next_offset
                        + crate::validation::transfer::upload_chunk(
                            &request.data_base64,
                            upload.request.descriptor.size_bytes - upload.receipt.next_offset,
                        )?
                };
                let token = request.transfer.transfer_id.clone();
                let receipt = driver.write_upload_chunk(request, &control).await?;
                crate::validation::transfer::receipt(&receipt, Some(&token), offset)?;
                let mut state = retained
                    .state
                    .lock()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                alive(&retained, &state)?;
                state
                    .upload
                    .as_mut()
                    .ok_or(BrowserDriverError::OutcomeUnknown)?
                    .receipt = receipt.clone();
                Ok(receipt)
            })
        })
        .await
    }
    pub(super) async fn commit_upload_private(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let entry = self
            .entry(&request.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::screenshot_read(&request, &entry.request)?;
        let retained = entry.clone();
        let ceiling = self.inner.capabilities.limits.max_observation_bytes;
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                let command = {
                    let mut state = retained
                        .state
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    let upload = state.upload.as_ref().ok_or(BrowserDriverError::Stale)?;
                    if !upload_matches(upload, &request)
                        || upload.receipt.next_offset != upload.request.descriptor.size_bytes
                    {
                        return Err(BrowserDriverError::Stale);
                    }
                    state
                        .upload
                        .take()
                        .ok_or(BrowserDriverError::OutcomeUnknown)?
                        .request
                        .command
                };
                let observation = driver.commit_upload(request, &control).await?;
                crate::validation::observed(&observation, &command, &retained.request, ceiling)?;
                Ok(observation)
            })
        })
        .await
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
        let entry = self
            .entry(&command.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::command_for_open(&command, &entry.request, &self.inner.capabilities)?;
        let retained = entry.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                retire(&retained)?;
                let descriptor = driver.download(command.clone(), &control).await?;
                crate::validation::transfer::download(
                    &descriptor,
                    &command,
                    &retained.request.options.allowed_origins,
                )?;
                let mut state = retained
                    .state
                    .lock()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                alive(&retained, &state)?;
                state.download = Some(Download {
                    descriptor: descriptor.clone(),
                    run: command.run_id,
                    offset: 0,
                    expires: Instant::now() + TTL,
                });
                Ok(descriptor)
            })
        })
        .await
    }
    pub(super) async fn read_download_private(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        let entry = self
            .entry(&request.session_id)?
            .ok_or(BrowserDriverError::Stale)?;
        crate::validation::screenshot_read(&request, &entry.request)?;
        let retained = entry.clone();
        self.private_operation(entry, control, move |driver, control| {
            Box::pin(async move {
                let remaining = {
                    let state = retained
                        .state
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    let download = state.download.as_ref().ok_or(BrowserDriverError::Stale)?;
                    if download.run != request.run_id
                        || download.offset != request.offset
                        || download.descriptor.target != request.target
                        || download.descriptor.control_generation != request.control_generation
                        || download.descriptor.transfer_id != request.transfer_id
                        || Instant::now() >= download.expires
                    {
                        return Err(BrowserDriverError::Stale);
                    }
                    download.descriptor.size_bytes - download.offset
                };
                let chunk = driver
                    .read_download_chunk(request.clone(), &control)
                    .await?;
                let size =
                    crate::validation::transfer::download_chunk(&chunk, request.offset, remaining)?;
                let mut state = retained
                    .state
                    .lock()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                alive(&retained, &state)?;
                let download = state
                    .download
                    .as_mut()
                    .ok_or(BrowserDriverError::OutcomeUnknown)?;
                download.offset += size;
                if download.offset == download.descriptor.size_bytes {
                    state.download = None;
                }
                Ok(chunk)
            })
        })
        .await
    }
}
