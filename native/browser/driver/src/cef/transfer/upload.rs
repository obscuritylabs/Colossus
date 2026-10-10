use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_ports::{
    BrowserUploadPrepareRequest, BrowserUploadReceipt, BrowserUploadWriteRequest,
    MAX_BROWSER_TRANSFER_BYTES,
};

impl Host {
    pub fn prepare_upload(
        &mut self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let BrowserAction::Upload {
            element,
            artifact_id,
            max_bytes,
        } = &request.command.action
        else {
            return Err(BrowserDriverError::Denied);
        };
        let metadata = &request.descriptor;
        if metadata.artifact_id != *artifact_id
            || metadata.size_bytes == 0
            || metadata.size_bytes > *max_bytes
            || *max_bytes > MAX_BROWSER_TRANSFER_BYTES
            || metadata.sha256.len() != 64
            || !metadata
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || metadata.file_name.len() > 64
            || !metadata.file_name.starts_with("upload.")
            || !metadata
                .file_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
        {
            return Err(BrowserDriverError::Denied);
        }
        let element = element.clone();
        let owner = self.transfer_owner(request.command, control)?;
        self.file_input(&owner, &element, control)?;
        let receipt = BrowserUploadReceipt {
            transfer_id: owner.token.clone(),
            next_offset: 0,
        };
        self.transfer.upload = Some(Upload {
            owner,
            descriptor: request.descriptor,
            bytes: Zeroizing::new(Vec::new()),
        });
        Ok(receipt)
    }
    pub fn write_upload_chunk(
        &mut self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        let mut upload = self
            .transfer
            .upload
            .take()
            .ok_or(BrowserDriverError::Stale)?;
        if !self.transfer_matches(&upload.owner, &request.transfer, control)
            || request.transfer.offset as usize != upload.bytes.len()
            || request.data_base64.len()
                > colossus_ports::BROWSER_SCREENSHOT_CHUNK_BYTES.div_ceil(3) * 4
        {
            return Err(BrowserDriverError::Stale);
        }
        let bytes = Zeroizing::new(
            BASE64
                .decode(&request.data_base64)
                .map_err(|_| BrowserDriverError::Denied)?,
        );
        if bytes.is_empty()
            || bytes.len() > colossus_ports::BROWSER_SCREENSHOT_CHUNK_BYTES
            || upload.bytes.len() + bytes.len() > upload.descriptor.size_bytes as usize
            || *Zeroizing::new(BASE64.encode(&*bytes)) != request.data_base64
        {
            return Err(BrowserDriverError::Denied);
        }
        upload.bytes.extend_from_slice(&bytes);
        let receipt = BrowserUploadReceipt {
            transfer_id: upload.owner.token.clone(),
            next_offset: upload.bytes.len() as u32,
        };
        self.transfer.upload = Some(upload);
        Ok(receipt)
    }
    fn file_input(
        &mut self,
        owner: &Owner,
        element: &BrowserElementRef,
        control: &BrowserDriverControl,
    ) -> Result<i32, BrowserDriverError> {
        let (backend, name, attributes) = self.transfer_node(owner, element, control)?;
        if name != "INPUT"
            || !attribute(&attributes, "type")
                .is_some_and(|value| value.eq_ignore_ascii_case("file"))
            || attribute(&attributes, "disabled").is_some()
            || semantic::protected(&attributes)
        {
            return Err(BrowserDriverError::Denied);
        }
        Ok(backend)
    }
    pub fn commit_upload(
        &mut self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let upload = self
            .transfer
            .upload
            .take()
            .ok_or(BrowserDriverError::Stale)?;
        if !self.transfer_matches(&upload.owner, &request, control)
            || request.offset != upload.descriptor.size_bytes
            || upload.bytes.len() != upload.descriptor.size_bytes as usize
            || digest(&upload.bytes) != upload.descriptor.sha256
        {
            return Err(BrowserDriverError::Stale);
        }
        let BrowserAction::Upload { element, .. } = &upload.owner.command.action else {
            return Err(BrowserDriverError::Denied);
        };
        let backend = self.file_input(&upload.owner, element, control)?;
        let staged = self
            .transfer
            .stage
            .as_mut()
            .ok_or(BrowserDriverError::Unavailable)?
            .upload(&upload.bytes, &upload.descriptor.file_name)?;
        let path = staged.path().to_str().ok_or(BrowserDriverError::Denied)?;
        self.file_input(&upload.owner, element, control)?;
        self.method(
            upload.owner.native,
            "DOM.setFileInputFiles",
            json!({"backendNodeId":backend,"files":[path]}),
            control,
        )
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        self.check_document_fence()?;
        if !self.transfer_current(&upload.owner) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let tab = self
            .tabs
            .get(&upload.owner.command.target.tab_id)
            .ok_or(BrowserDriverError::Stale)?;
        Ok(BrowserObservation {
            session_id: upload.owner.command.session_id,
            tab: tab.summary.clone(),
            snapshot: None,
            truncated: false,
        })
    }
}
