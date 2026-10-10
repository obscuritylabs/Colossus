use super::*;
use crate::state::DispatchGuard;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_ports::{
    BROWSER_SCREENSHOT_CHUNK_BYTES, BrowserDriverCommand, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, MAX_BROWSER_SCREENSHOT_BYTES, RunControl,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub(super) enum Evidence {
    Observation(BrowserObservation),
    Screenshot(CapturedBrowserScreenshot),
    Download(super::transfer::CapturedBrowserDownload),
}

/// Complete private PNG. Runtime must post-authorize these actual bytes before publication.
pub struct CapturedBrowserScreenshot {
    /// Exact native receipt; transfer identity must never be exposed to model arguments.
    pub descriptor: BrowserScreenshotDescriptor,
    /// Private zeroizing PNG bytes, never a semantic observation or public wire frame.
    pub bytes: Zeroizing<Vec<u8>>,
}

impl BrowserCoordinator {
    /// Capture a viewport under the same writer, document, pending-action and takeover fences.
    pub async fn screenshot(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        max_bytes: u32,
        request_control: &RunControl,
    ) -> Result<CapturedBrowserScreenshot, BrowserError> {
        if max_bytes == 0 || max_bytes > MAX_BROWSER_SCREENSHOT_BYTES {
            return Err(BrowserError::InvalidArguments);
        }
        match self
            .execute_inner(
                actor,
                lease,
                target,
                BrowserAction::Screenshot { max_bytes },
                None,
                request_control,
            )
            .await?
        {
            Evidence::Screenshot(value) => Ok(value),
            Evidence::Observation(_) | Evidence::Download(_) => Err(BrowserError::InvalidEvidence),
        }
    }

    pub(super) async fn collect_capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<CapturedBrowserScreenshot, BrowserDriverError> {
        let BrowserAction::Screenshot { max_bytes } = command.action else {
            return Err(BrowserDriverError::Denied);
        };
        let descriptor = self.driver.capture(command.clone(), control).await?;
        if descriptor.session_id != command.session_id
            || descriptor.target != command.target
            || descriptor.control_generation != command.control_generation
            || descriptor.size_bytes < 33
            || descriptor.size_bytes > max_bytes
            || descriptor.size_bytes > MAX_BROWSER_SCREENSHOT_BYTES
            || !hex_token(&descriptor.transfer_id, 32)
            || !hex_token(&descriptor.sha256, 64)
            || descriptor.width == 0
            || descriptor.height == 0
            || u64::from(descriptor.width) * u64::from(descriptor.height) > 16_777_216
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(descriptor.size_bytes as usize));
        while bytes.len() < descriptor.size_bytes as usize {
            if control.is_cancelled() {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let offset = bytes.len() as u32;
            let chunk = self
                .driver
                .read_screenshot_chunk(
                    BrowserScreenshotReadRequest {
                        binding: command.binding.clone(),
                        run_id: command.run_id.clone(),
                        session_id: command.session_id.clone(),
                        target: command.target.clone(),
                        control_generation: command.control_generation,
                        transfer_id: descriptor.transfer_id.clone(),
                        offset,
                    },
                    control,
                )
                .await?;
            if chunk.offset != offset
                || chunk.data_base64.len() > BROWSER_SCREENSHOT_CHUNK_BYTES.div_ceil(3) * 4
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let decoded = Zeroizing::new(
                BASE64
                    .decode(&chunk.data_base64)
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
            );
            if decoded.is_empty()
                || decoded.len() > BROWSER_SCREENSHOT_CHUNK_BYTES
                || bytes.len() + decoded.len() > descriptor.size_bytes as usize
                || *Zeroizing::new(BASE64.encode(&*decoded)) != chunk.data_base64
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            bytes.extend_from_slice(&decoded);
        }
        let digest: String = Sha256::digest(&*bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if digest != descriptor.sha256
            || png_dimensions(&bytes) != Some((descriptor.width, descriptor.height))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(CapturedBrowserScreenshot { descriptor, bytes })
    }

    pub(super) fn accept_capture(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        guard: &DispatchGuard,
        control: &BrowserDriverControl,
        value: CapturedBrowserScreenshot,
    ) -> Result<CapturedBrowserScreenshot, BrowserError> {
        if control.is_cancelled() {
            return Err(BrowserError::OutcomeUnknown);
        }
        let mut state = lock(&self.state)?;
        let session = owned(&mut state, &actor.binding, &lease.session_id)
            .map_err(|_| BrowserError::OutcomeUnknown)?;
        if session.lease.as_ref().is_none_or(|current| {
            &current.public != lease || current.deadline <= std::time::Instant::now()
        }) || session.summary.lifecycle != BrowserLifecycle::Ready
            || session
                .pending
                .as_ref()
                .is_none_or(|pending| pending.id != guard.dispatch_id)
            || !session
                .summary
                .tabs
                .iter()
                .any(|tab| tab.tab_id == target.tab_id && tab.document_id == target.document_id)
        {
            return Err(BrowserError::OutcomeUnknown);
        }
        session.pending = None;
        Ok(value)
    }
}

fn hex_token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 33
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.get(8..16)? != b"\0\0\0\rIHDR"
    {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
        u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
    ))
}
