use super::*;
use sha2::{Digest as _, Sha256};
#[derive(Default)]
pub(super) struct Native {
    pub executions: AtomicUsize,
    pub empty_download: AtomicBool,
    pub writes: AtomicUsize,
    pub commits: AtomicUsize,
    pub reads: AtomicUsize,
    pub prepares: AtomicUsize,
    pub cancels: AtomicUsize,
    pub closes: AtomicUsize,
    pub bad_receipt: AtomicBool,
    pub outside_download: AtomicBool,
    pub block_write: AtomicBool,
    pub bad_chunk: AtomicBool,
}
#[async_trait]
impl BrowserDriver for Native {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            title: String::new(),
            origin: None,
        })
    }
    async fn execute(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(observation(command, false))
    }
    async fn prepare_upload(
        &self,
        _: BrowserUploadPrepareRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserUploadReceipt {
            transfer_id: "5".repeat(32),
            next_offset: 0,
        })
    }
    async fn write_upload_chunk(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        while self.block_write.load(Ordering::SeqCst) {
            if control.is_cancelled() {
                return Err(BrowserDriverError::Cancelled);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let size = STANDARD.decode(&request.data_base64).unwrap().len() as u32;
        Ok(BrowserUploadReceipt {
            transfer_id: request.transfer.transfer_id.clone(),
            next_offset: request.transfer.offset
                + size
                + u32::from(self.bad_receipt.load(Ordering::SeqCst)),
        })
    }
    async fn commit_upload(
        &self,
        _: BrowserUploadCommitRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(observation(upload().command, true))
    }
    async fn download(
        &self,
        command: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        Ok(BrowserDownloadDescriptor {
            session_id: command.session_id,
            target: command.target,
            control_generation: command.control_generation,
            transfer_id: "6".repeat(32),
            size_bytes: if self.empty_download.load(Ordering::SeqCst) {
                0
            } else {
                70_000
            },
            sha256: format!(
                "{:x}",
                Sha256::digest(vec![
                    9;
                    if self.empty_download.load(Ordering::SeqCst) {
                        0
                    } else {
                        70_000
                    }
                ])
            ),
            origin: BrowserOrigin::parse(if self.outside_download.load(Ordering::SeqCst) {
                "https://outside.test"
            } else {
                "https://example.test"
            })
            .unwrap(),
        })
    }
    async fn read_download_chunk(
        &self,
        request: BrowserDownloadReadRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserScreenshotChunk {
            offset: request.offset,
            data_base64: if self.bad_chunk.load(Ordering::SeqCst) {
                "!".into()
            } else if self.empty_download.load(Ordering::SeqCst) {
                String::new()
            } else {
                STANDARD.encode(vec![
                    9;
                    (70_000 - request.offset as usize)
                        .min(BROWSER_SCREENSHOT_CHUNK_BYTES)
                ])
            },
        })
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn close_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
pub(super) fn observation(command: BrowserDriverCommand, rotate: bool) -> BrowserObservation {
    BrowserObservation {
        session_id: command.session_id,
        tab: BrowserTabSummary {
            tab_id: command.target.tab_id,
            document_id: if rotate {
                command.next_document_id
            } else {
                command.target.document_id
            },
            title: String::new(),
            origin: None,
        },
        snapshot: None,
        truncated: false,
    }
}
pub(super) struct Factory(pub Arc<Native>);
#[async_trait]
impl BrowserHostFactory for Factory {
    fn capabilities(&self) -> BrowserCapabilities {
        let mut caps = enrollment().capabilities;
        caps.actions
            .extend([BrowserActionKind::Upload, BrowserActionKind::Download]);
        caps
    }
    async fn launch(
        &self,
        _: &BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        Ok(self.0.clone())
    }
    async fn reap_failed_launch(
        &self,
        _: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}
