//! Async bridge adapter; all native methods execute on the original main thread.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionId, BrowserTabSummary,
};
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadChunk, BrowserDownloadDescriptor,
    BrowserDownloadReadRequest, BrowserDriver, BrowserDriverCommand, BrowserDriverControl,
    BrowserDriverError, BrowserDriverOpenRequest, BrowserNativeHandoffRequest,
    BrowserScreenshotChunk, BrowserScreenshotDescriptor, BrowserScreenshotReadRequest,
    BrowserUploadCommitRequest, BrowserUploadPrepareRequest, BrowserUploadReceipt,
    BrowserUploadWriteRequest,
};
use tokio::sync::{mpsc, oneshot};
mod transfer;

pub enum Data {
    PrepareUpload(
        BrowserUploadPrepareRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserUploadReceipt, BrowserDriverError>>,
    ),
    WriteUpload(
        BrowserUploadWriteRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserUploadReceipt, BrowserDriverError>>,
    ),
    CommitUpload(
        BrowserUploadCommitRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserObservation, BrowserDriverError>>,
    ),
    Download(
        BrowserDriverCommand,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserDownloadDescriptor, BrowserDriverError>>,
    ),
    ReadDownload(
        BrowserDownloadReadRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserDownloadChunk, BrowserDriverError>>,
    ),
    Open(
        BrowserDriverOpenRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserTabSummary, BrowserDriverError>>,
    ),
    Execute(
        BrowserDriverCommand,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserObservation, BrowserDriverError>>,
    ),
    Capture(
        BrowserDriverCommand,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserScreenshotDescriptor, BrowserDriverError>>,
    ),
    ReadScreenshot(
        BrowserScreenshotReadRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserScreenshotChunk, BrowserDriverError>>,
    ),
    ConfirmHandoff(
        BrowserNativeHandoffRequest,
        BrowserDriverControl,
        oneshot::Sender<Result<BrowserTabSummary, BrowserDriverError>>,
    ),
}
pub enum Control {
    Close(
        BrowserSessionId,
        oneshot::Sender<Result<(), BrowserDriverError>>,
    ),
}
pub struct Driver {
    pub profile: colossus_contracts::BrowserProfileSelection,
    pub capabilities: BrowserCapabilities,
    pub data: mpsc::Sender<Data>,
    pub control: mpsc::Sender<Control>,
    pub cancelled: Arc<AtomicBool>,
}

#[async_trait]
impl BrowserDriver for Driver {
    async fn prepare_upload(
        &self,
        r: BrowserUploadPrepareRequest,
        c: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.prepare_upload_private(r, c).await
    }
    async fn write_upload_chunk(
        &self,
        r: BrowserUploadWriteRequest,
        c: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.write_upload_private(r, c).await
    }
    async fn commit_upload(
        &self,
        r: BrowserUploadCommitRequest,
        c: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.commit_upload_private(r, c).await
    }
    async fn download(
        &self,
        r: BrowserDriverCommand,
        c: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        self.download_private(r, c).await
    }
    async fn read_download_chunk(
        &self,
        r: BrowserDownloadReadRequest,
        c: &BrowserDriverControl,
    ) -> Result<BrowserDownloadChunk, BrowserDriverError> {
        self.read_download_private(r, c).await
    }
    fn capabilities(&self) -> BrowserCapabilities {
        self.capabilities.clone()
    }
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        BrowserCancelDisposition::ClosesContext
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if request.options.profile != self.profile {
            return Err(BrowserDriverError::Unsupported);
        }
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::Open(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::Execute(command, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::Capture(command, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::ReadScreenshot(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        // Revoke immediately, independently of a slow data operation or full queue.
        self.cancelled.store(true, Ordering::Release);
        self.close_session(session).await
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::ConfirmHandoff(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancelled.store(true, Ordering::Release);
        let (sender, receiver) = oneshot::channel();
        self.control
            .send(Control::Close(session.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
}
