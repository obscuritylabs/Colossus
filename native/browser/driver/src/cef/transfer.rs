//! Closed file transfer ownership. Paths exist only inside a supervisor-created stage.
use super::*;
use colossus_ports::{BrowserDownloadReadRequest, BrowserUploadCommitRequest};
use zeroize::Zeroizing;
mod download;
mod upload;

pub(super) struct Owner {
    command: BrowserDriverCommand,
    native: u64,
    revision: u64,
    document: u64,
    token: String,
    deadline: Instant,
    control: BrowserDriverControl,
}
pub(super) struct Upload {
    owner: Owner,
    descriptor: colossus_ports::BrowserUploadDescriptor,
    bytes: Zeroizing<Vec<u8>>,
}
pub(super) struct Download {
    owner: Owner,
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
}
#[derive(Default)]
pub(super) struct State {
    pub(super) stage: Option<crate::transfer_stage::Stage>,
    pub(super) upload: Option<Upload>,
    pub(super) download: Option<Download>,
}
impl Host {
    pub fn configure_transfers(
        &mut self,
        profile: &std::path::Path,
    ) -> Result<(), BrowserDriverError> {
        if self.transfer.stage.is_some() || self.session.is_some() {
            return Err(BrowserDriverError::Denied);
        }
        self.transfer.stage = Some(crate::transfer_stage::Stage::create(profile)?);
        Ok(())
    }
    pub(super) fn finish_transfers(&mut self) -> Result<(), BrowserDriverError> {
        if let Some(stage) = self.transfer.stage.as_mut() {
            stage.finish()?;
        }
        self.transfer.stage = None;
        Ok(())
    }
    fn transfer_owner(
        &mut self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<Owner, BrowserDriverError> {
        self.screenshot = None;
        self.transfer.upload = None;
        self.transfer.download = None;
        if self.interrupted(control)
            || self.session.as_ref() != Some(&command.session_id)
            || command.control_generation == 0
            || self.transfer.stage.is_none()
        {
            return Err(BrowserDriverError::Denied);
        }
        let tab = self
            .tabs
            .get(&command.target.tab_id)
            .ok_or(BrowserDriverError::Stale)?;
        let (native, revision, document) = (tab.native, tab.revision, tab.native_document);
        if tab.summary.document_id != command.target.document_id
            || self.native_document(native)? != document
            || self.metadata(native)?.2 != revision
        {
            return Err(BrowserDriverError::Stale);
        }
        self.revoke_human_presentation();
        self.presentation.handoff.revoke();
        self.presentation.control_generation = command.control_generation;
        self.expected_revision = Some((native, revision, document));
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| BrowserDriverError::Failed)?;
        Ok(Owner {
            command,
            native,
            revision,
            document,
            token: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            deadline: Instant::now() + Duration::from_secs(30),
            control: control.clone(),
        })
    }
    fn transfer_current(&self, owner: &Owner) -> bool {
        Instant::now() < owner.deadline
            && !self.presentation.human
            && self.presentation.control_generation == owner.command.control_generation
            && !self.interrupted(&owner.control)
            && self.session.as_ref() == Some(&owner.command.session_id)
            && self.native_document(owner.native).ok() == Some(owner.document)
            && self.metadata(owner.native).ok().map(|metadata| metadata.2) == Some(owner.revision)
            && self
                .tabs
                .get(&owner.command.target.tab_id)
                .is_some_and(|tab| tab.summary.document_id == owner.command.target.document_id)
    }
    fn transfer_matches(
        &self,
        owner: &Owner,
        request: &BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> bool {
        self.transfer_current(owner)
            && !self.interrupted(control)
            && request.binding == owner.command.binding
            && request.run_id == owner.command.run_id
            && request.session_id == owner.command.session_id
            && request.target == owner.command.target
            && request.control_generation == owner.command.control_generation
            && request.transfer_id == owner.token
    }
    fn transfer_node(
        &mut self,
        owner: &Owner,
        element: &BrowserElementRef,
        control: &BrowserDriverControl,
    ) -> Result<(i32, String, Vec<String>), BrowserDriverError> {
        let backend = self.backend(&owner.command.target.tab_id, element)?;
        let result = self.method(
            owner.native,
            "DOM.describeNode",
            json!({"backendNodeId":backend,"depth":0}),
            control,
        )?;
        let node = result.get("node").ok_or(BrowserDriverError::Stale)?;
        let name = node
            .get("nodeName")
            .and_then(Value::as_str)
            .filter(|name| name.len() <= 32)
            .ok_or(BrowserDriverError::Stale)?
            .to_owned();
        let values = node
            .get("attributes")
            .and_then(Value::as_array)
            .ok_or(BrowserDriverError::Denied)?;
        if values.len() > 256 || values.len() % 2 != 0 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let mut attributes = Vec::new();
        let mut size = 0;
        for value in values {
            let value = value.as_str().ok_or(BrowserDriverError::Denied)?;
            size += value.len();
            if size > 32 * 1024 {
                return Err(BrowserDriverError::LimitExceeded);
            }
            attributes.push(value.to_owned());
        }
        self.check_document_fence()?;
        Ok((backend, name, attributes))
    }
    pub(super) fn retire_transfers(&mut self) {
        if self
            .transfer
            .upload
            .as_ref()
            .is_some_and(|upload| !self.transfer_current(&upload.owner))
        {
            self.transfer.upload = None;
        }
        if self
            .transfer
            .download
            .as_ref()
            .is_some_and(|download| !self.transfer_current(&download.owner))
        {
            self.transfer.download = None;
        }
    }
    pub fn read_download_chunk(
        &mut self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<colossus_ports::BrowserDownloadChunk, BrowserDriverError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        let mut download = self
            .transfer
            .download
            .take()
            .ok_or(BrowserDriverError::Stale)?;
        if !self.transfer_matches(&download.owner, &request, control)
            || request.offset as usize != download.offset
        {
            return Err(BrowserDriverError::Stale);
        }
        let end = (download.offset + colossus_ports::BROWSER_SCREENSHOT_CHUNK_BYTES)
            .min(download.bytes.len());
        let chunk = colossus_ports::BrowserDownloadChunk {
            offset: request.offset,
            data_base64: BASE64.encode(&download.bytes[download.offset..end]),
        };
        download.bytes[download.offset..end].fill(0);
        download.offset = end;
        if end < download.bytes.len() {
            self.transfer.download = Some(download);
        }
        Ok(chunk)
    }
}
fn attribute<'a>(attributes: &'a [String], name: &str) -> Option<&'a str> {
    attributes
        .chunks_exact(2)
        .find(|pair| pair[0].eq_ignore_ascii_case(name))
        .map(|pair| pair[1].as_str())
}
fn digest(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
