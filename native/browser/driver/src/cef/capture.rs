use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_ports::{
    BROWSER_SCREENSHOT_CHUNK_BYTES, BrowserScreenshotChunk, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, MAX_BROWSER_SCREENSHOT_BYTES,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub(super) struct Transfer {
    command: BrowserDriverCommand,
    descriptor: BrowserScreenshotDescriptor,
    bytes: Zeroizing<Vec<u8>>,
    offset: usize,
    deadline: Instant,
    native: u64,
    revision: u64,
    native_document: u64,
    control: BrowserDriverControl,
}

impl Host {
    pub fn capture(
        &mut self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.screenshot = None;
        self.transfer.upload = None;
        self.transfer.download = None;
        let BrowserAction::Screenshot { max_bytes } = command.action else {
            return Err(BrowserDriverError::Denied);
        };
        if max_bytes == 0
            || max_bytes > MAX_BROWSER_SCREENSHOT_BYTES
            || self.session.as_ref() != Some(&command.session_id)
            || self.interrupted(control)
            || command.control_generation == 0
        {
            return Err(BrowserDriverError::Denied);
        }
        self.revoke_human_presentation();
        self.presentation.handoff.revoke();
        self.presentation.control_generation = command.control_generation;
        let tab = self
            .tabs
            .get(&command.target.tab_id)
            .ok_or(BrowserDriverError::Stale)?;
        let (native, revision, native_document) = (tab.native, tab.revision, tab.native_document);
        if tab.summary.document_id != command.target.document_id
            || self.native_document(native)? != native_document
            || self.metadata(native)?.2 != revision
            || self
                .state(|state| state.failed.contains(&native) || state.closed.contains(&native))?
        {
            return Err(BrowserDriverError::Stale);
        }
        self.expected_revision = Some((native, revision, native_document));
        let mut result = self.method(native, "Page.captureScreenshot", json!({"format":"png","fromSurface":true,"captureBeyondViewport":false,"optimizeForSpeed":false}), control)?;
        self.check_document_fence()?;
        let encoded = result
            .get_mut("data")
            .map(Value::take)
            .and_then(|value| match value {
                Value::String(value) => Some(Zeroizing::new(value)),
                _ => None,
            })
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        if encoded.len() > (max_bytes as usize).div_ceil(3) * 4 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let bytes = Zeroizing::new(
            BASE64
                .decode(&*encoded)
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
        );
        if bytes.len() > max_bytes as usize
            || bytes.len() < 33
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes.get(8..16) != Some(b"\0\0\0\rIHDR")
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let dimension = |range: std::ops::Range<usize>| -> Result<u32, BrowserDriverError> {
            let word: [u8; 4] = bytes
                .get(range)
                .and_then(|slice| slice.try_into().ok())
                .ok_or(BrowserDriverError::OutcomeUnknown)?;
            Ok(u32::from_be_bytes(word))
        };
        let (width, height) = (dimension(16..20)?, dimension(20..24)?);
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| BrowserDriverError::Failed)?;
        let descriptor = BrowserScreenshotDescriptor {
            session_id: command.session_id.clone(),
            target: command.target.clone(),
            control_generation: command.control_generation,
            transfer_id: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            size_bytes: bytes.len() as u32,
            sha256: Sha256::digest(&*bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            width,
            height,
        };
        self.screenshot = Some(Transfer {
            command,
            descriptor: descriptor.clone(),
            bytes,
            offset: 0,
            deadline: Instant::now() + Duration::from_secs(30),
            native,
            revision,
            native_document,
            control: control.clone(),
        });
        Ok(descriptor)
    }

    pub fn read_screenshot_chunk(
        &mut self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        // Take ownership before checks so every stale/cancelled/invalid read retires private bytes.
        let mut transfer = self.screenshot.take().ok_or(BrowserDriverError::Stale)?;
        if self.interrupted(control)
            || Instant::now() >= transfer.deadline
            || request.binding != transfer.command.binding
            || request.run_id != transfer.command.run_id
            || request.session_id != transfer.command.session_id
            || request.target != transfer.command.target
            || request.control_generation != transfer.command.control_generation
            || self.presentation.control_generation != request.control_generation
            || request.transfer_id != transfer.descriptor.transfer_id
            || request.offset as usize != transfer.offset
            || self.native_document(transfer.native)? != transfer.native_document
            || self.metadata(transfer.native)?.2 != transfer.revision
        {
            return Err(BrowserDriverError::Stale);
        }
        let end = (transfer.offset + BROWSER_SCREENSHOT_CHUNK_BYTES).min(transfer.bytes.len());
        let chunk = BrowserScreenshotChunk {
            offset: request.offset,
            data_base64: BASE64.encode(&transfer.bytes[transfer.offset..end]),
        };
        transfer.bytes[transfer.offset..end].fill(0);
        transfer.offset = end;
        if end < transfer.bytes.len() {
            self.screenshot = Some(transfer);
        }
        Ok(chunk)
    }

    pub(super) fn retire_capture(&mut self) {
        let stale = self.screenshot.as_ref().is_some_and(|transfer| {
            Instant::now() >= transfer.deadline
                || self.interrupted(&transfer.control)
                || self.native_document(transfer.native).ok() != Some(transfer.native_document)
                || self
                    .metadata(transfer.native)
                    .ok()
                    .map(|metadata| metadata.2)
                    != Some(transfer.revision)
        });
        if stale {
            self.screenshot = None;
        }
    }
}
