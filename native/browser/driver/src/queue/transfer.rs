use super::*;
impl Driver {
    pub(super) async fn prepare_upload_private(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::PrepareUpload(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    pub(super) async fn write_upload_private(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::WriteUpload(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    pub(super) async fn commit_upload_private(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::CommitUpload(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    pub(super) async fn download_private(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::Download(command, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    pub(super) async fn read_download_private(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadChunk, BrowserDriverError> {
        if control.is_cancelled() || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Cancelled);
        }
        let (sender, receiver) = oneshot::channel();
        self.data
            .send(Data::ReadDownload(request, control.clone(), sender))
            .await
            .map_err(|_| BrowserDriverError::Unavailable)?;
        receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
}
