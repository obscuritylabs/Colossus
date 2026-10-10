//! Complete application bytes stay private until Runtime's gateway releases them.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_ports::{
    BROWSER_SCREENSHOT_CHUNK_BYTES, BrowserDownloadDescriptor, BrowserDownloadReadRequest,
    BrowserDriverCommand, BrowserUploadArtifact, BrowserUploadPrepareRequest,
    BrowserUploadWriteRequest, MAX_BROWSER_TRANSFER_BYTES,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

/// Exact complete native download, awaiting actual-byte mandatory post-effect policy.
pub struct CapturedBrowserDownload {
    /// Exact current native generation and final permitted origin.
    pub descriptor: BrowserDownloadDescriptor,
    /// Privately assembled complete bytes, never public semantic evidence.
    pub bytes: Zeroizing<Vec<u8>>,
}

impl BrowserCoordinator {
    /// Commit actual pre-authorized artifact bytes under fresh snapshot and ownership fences.
    pub async fn upload(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        element: BrowserElementRef,
        artifact: BrowserUploadArtifact,
        request_control: &colossus_ports::RunControl,
    ) -> Result<BrowserObservation, BrowserError> {
        let action = BrowserAction::Upload {
            element,
            artifact_id: artifact.descriptor.artifact_id.clone(),
            max_bytes: artifact.descriptor.size_bytes,
        };
        match self
            .execute_inner(
                actor,
                lease,
                target,
                action,
                Some(artifact),
                request_control,
            )
            .await?
        {
            capture::Evidence::Observation(value) => Ok(value),
            _ => Err(BrowserError::InvalidEvidence),
        }
    }

    /// Retrieve one exact current link into private custody for actual-byte post authorization.
    pub async fn download(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        element: BrowserElementRef,
        request_control: &colossus_ports::RunControl,
    ) -> Result<CapturedBrowserDownload, BrowserError> {
        match self
            .execute_inner(
                actor,
                lease,
                target,
                BrowserAction::Download {
                    element,
                    max_bytes: MAX_BROWSER_TRANSFER_BYTES,
                },
                None,
                request_control,
            )
            .await?
        {
            capture::Evidence::Download(value) => Ok(value),
            _ => Err(BrowserError::InvalidEvidence),
        }
    }

    pub(super) fn accept_download(
        &self,
        actor: &BrowserActor,
        lease: &BrowserControlLease,
        target: &BrowserTarget,
        guard: &crate::state::DispatchGuard,
        control: &BrowserDriverControl,
        value: CapturedBrowserDownload,
    ) -> Result<CapturedBrowserDownload, BrowserError> {
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
    pub(super) async fn collect_upload(
        &self,
        command: BrowserDriverCommand,
        artifact: BrowserUploadArtifact,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        let metadata = &artifact.descriptor;
        if artifact.bytes.is_empty()
            || artifact.bytes.len() > MAX_BROWSER_TRANSFER_BYTES as usize
            || artifact.bytes.len() != metadata.size_bytes as usize
            || digest(&artifact.bytes) != metadata.sha256
        {
            return Err(BrowserDriverError::Denied);
        }
        let mut receipt = self
            .driver
            .prepare_upload(
                BrowserUploadPrepareRequest {
                    command: command.clone(),
                    descriptor: metadata.clone(),
                },
                control,
            )
            .await?;
        if receipt.next_offset != 0 || !token(&receipt.transfer_id) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let identity = receipt.transfer_id.clone();
        for data in artifact.bytes.chunks(BROWSER_SCREENSHOT_CHUNK_BYTES) {
            if control.is_cancelled() {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let request = BrowserUploadWriteRequest {
                transfer: request(&command, &identity, receipt.next_offset),
                data_base64: BASE64.encode(data),
            };
            let expected = receipt.next_offset + data.len() as u32;
            receipt = self.driver.write_upload_chunk(request, control).await?;
            if receipt.transfer_id != identity || receipt.next_offset != expected {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        if control.is_cancelled() || receipt.next_offset != metadata.size_bytes {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.driver
            .commit_upload(request(&command, &identity, receipt.next_offset), control)
            .await
    }

    pub(super) async fn collect_download(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<CapturedBrowserDownload, BrowserDriverError> {
        let descriptor = self.driver.download(command.clone(), control).await?;
        if descriptor.session_id != command.session_id
            || descriptor.target != command.target
            || descriptor.control_generation != command.control_generation
            || descriptor.size_bytes > MAX_BROWSER_TRANSFER_BYTES
            || !token(&descriptor.transfer_id)
            || descriptor.sha256.len() != 64
            || !descriptor
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let origins = {
            let mut state = lock(&self.state).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            owned(&mut state, &command.binding, &command.session_id)
                .map_err(|_| BrowserDriverError::Denied)?
                .options
                .allowed_origins
                .clone()
        };
        if !origins.contains(&descriptor.origin) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(descriptor.size_bytes as usize));
        // Empty downloads still consume the one-shot native transfer.
        loop {
            if control.is_cancelled() {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let offset = bytes.len() as u32;
            let chunk = self
                .driver
                .read_download_chunk(request(&command, &descriptor.transfer_id, offset), control)
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
            if decoded.len() > BROWSER_SCREENSHOT_CHUNK_BYTES
                || bytes.len() + decoded.len() > descriptor.size_bytes as usize
                || *Zeroizing::new(BASE64.encode(&*decoded)) != chunk.data_base64
                || (decoded.is_empty() && descriptor.size_bytes != 0)
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            bytes.extend_from_slice(&decoded);
            if bytes.len() == descriptor.size_bytes as usize {
                break;
            }
        }
        if digest(&bytes) != descriptor.sha256 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(CapturedBrowserDownload { descriptor, bytes })
    }
}

fn request(
    command: &BrowserDriverCommand,
    transfer_id: &str,
    offset: u32,
) -> BrowserDownloadReadRequest {
    BrowserDownloadReadRequest {
        binding: command.binding.clone(),
        run_id: command.run_id.clone(),
        session_id: command.session_id.clone(),
        target: command.target.clone(),
        control_generation: command.control_generation,
        transfer_id: transfer_id.into(),
        offset,
    }
}
fn token(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
