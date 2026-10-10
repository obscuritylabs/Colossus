use super::*;
pub(in crate::browser_tools::tests) enum Pending {
    Upload {
        command: BrowserDriverCommand,
        descriptor: BrowserUploadDescriptor,
        bytes: Vec<u8>,
    },
    Download {
        command: BrowserDriverCommand,
    },
}
impl Driver {
    pub(in crate::browser_tools::tests) fn transfer_prepare(
        &self,
        request: BrowserUploadPrepareRequest,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.transfers.fetch_add(1, Ordering::SeqCst);
        *self.transfer.lock().unwrap() = Some(Pending::Upload {
            command: request.command,
            descriptor: request.descriptor,
            bytes: Vec::new(),
        });
        Ok(BrowserUploadReceipt {
            transfer_id: "d".repeat(32),
            next_offset: 0,
        })
    }
    pub(in crate::browser_tools::tests) fn transfer_write(
        &self,
        request: BrowserUploadWriteRequest,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let mut pending = self.transfer.lock().unwrap();
        let Some(Pending::Upload { command, bytes, .. }) = pending.as_mut() else {
            return Err(BrowserDriverError::Stale);
        };
        if !exact(command, &request.transfer) || request.transfer.offset as usize != bytes.len() {
            return Err(BrowserDriverError::Stale);
        }
        bytes.extend(BASE64.decode(&request.data_base64).unwrap());
        Ok(BrowserUploadReceipt {
            transfer_id: "d".repeat(32),
            next_offset: bytes.len() as u32,
        })
    }
    pub(in crate::browser_tools::tests) fn transfer_commit(
        &self,
        request: BrowserUploadCommitRequest,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let Some(Pending::Upload {
            command,
            descriptor,
            bytes,
        }) = self.transfer.lock().unwrap().take()
        else {
            return Err(BrowserDriverError::Stale);
        };
        if !exact(&command, &request)
            || bytes.len() != request.offset as usize
            || descriptor.sha256 != hash(&bytes)
        {
            return Err(BrowserDriverError::Stale);
        }
        assert_eq!(bytes, CONTENT);
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab: BrowserTabSummary {
                tab_id: command.target.tab_id,
                document_id: command.target.document_id,
                origin: Some(BrowserOrigin::parse("https://example.org").unwrap()),
                title: "fixture".into(),
            },
            snapshot: None,
            truncated: false,
        })
    }
    pub(in crate::browser_tools::tests) fn transfer_download(
        &self,
        command: BrowserDriverCommand,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        self.transfers.fetch_add(1, Ordering::SeqCst);
        let descriptor = BrowserDownloadDescriptor {
            session_id: command.session_id.clone(),
            target: command.target.clone(),
            control_generation: command.control_generation,
            transfer_id: "d".repeat(32),
            size_bytes: CONTENT.len() as u32,
            sha256: hash(CONTENT),
            origin: BrowserOrigin::parse("https://example.org").unwrap(),
        };
        *self.transfer.lock().unwrap() = Some(Pending::Download { command });
        Ok(descriptor)
    }
    pub(in crate::browser_tools::tests) fn transfer_read(
        &self,
        request: BrowserDownloadReadRequest,
    ) -> Result<BrowserDownloadChunk, BrowserDriverError> {
        let Some(Pending::Download { command }) = self.transfer.lock().unwrap().take() else {
            return Err(BrowserDriverError::Stale);
        };
        if !exact(&command, &request) || request.offset != 0 {
            return Err(BrowserDriverError::Stale);
        }
        Ok(BrowserDownloadChunk {
            offset: 0,
            data_base64: BASE64.encode(CONTENT),
        })
    }
}
fn exact(command: &BrowserDriverCommand, request: &BrowserScreenshotReadRequest) -> bool {
    request.binding == command.binding
        && request.run_id == command.run_id
        && request.session_id == command.session_id
        && request.target == command.target
        && request.control_generation == command.control_generation
        && request.transfer_id == "d".repeat(32)
}
