use super::types::NativeState;
use async_trait::async_trait;
use colossus_contracts::*;
use colossus_ports::*;
use std::sync::{Arc, atomic::Ordering};

/// Registers the exact coordinator allocation before delegating its first native await.
/// Serialized human admissions prevent shared bindings from adopting another instance's page.
pub(in crate::browser_tools) struct Registrar(pub(in crate::browser_tools) Arc<NativeState>);
#[async_trait]
impl BrowserDriver for Registrar {
    fn capabilities(&self) -> BrowserCapabilities {
        self.0.driver.capabilities()
    }
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        self.0.driver.cancellation_disposition()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if request.run_id.is_none() {
            if self.0.stopped.load(Ordering::Acquire) {
                return Err(BrowserDriverError::Denied);
            }
            let entry = self
                .0
                .current
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                .clone()
                .ok_or(BrowserDriverError::Denied)?;
            if request.binding != entry.binding || entry.control.is_cancelled() {
                return Err(BrowserDriverError::Denied);
            }
            let mut slot = entry
                .request
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if slot.is_some() {
                return Err(BrowserDriverError::Denied);
            }
            *slot = Some(request.clone());
        }
        self.0.driver.open_session(request, control).await
    }
    async fn execute(
        &self,
        c: BrowserDriverCommand,
        k: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.0.driver.execute(c, k).await
    }
    async fn capture(
        &self,
        c: BrowserDriverCommand,
        k: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.0.driver.capture(c, k).await
    }
    async fn read_screenshot_chunk(
        &self,
        r: BrowserScreenshotReadRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.0.driver.read_screenshot_chunk(r, k).await
    }
    async fn prepare_upload(
        &self,
        r: BrowserUploadPrepareRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.0.driver.prepare_upload(r, k).await
    }
    async fn write_upload_chunk(
        &self,
        r: BrowserUploadWriteRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.0.driver.write_upload_chunk(r, k).await
    }
    async fn commit_upload(
        &self,
        r: BrowserUploadCommitRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.0.driver.commit_upload(r, k).await
    }
    async fn download(
        &self,
        c: BrowserDriverCommand,
        k: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        self.0.driver.download(c, k).await
    }
    async fn read_download_chunk(
        &self,
        r: BrowserDownloadReadRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserDownloadChunk, BrowserDriverError> {
        self.0.driver.read_download_chunk(r, k).await
    }
    async fn confirm_native_handoff(
        &self,
        r: BrowserNativeHandoffRequest,
        k: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.0.driver.confirm_native_handoff(r, k).await
    }
    async fn cancel_session(&self, s: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.0.driver.cancel_session(s).await
    }
    async fn close_session(&self, s: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.0.driver.close_session(s).await
    }
}
