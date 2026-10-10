//! Native publication of already released screenshot bytes through the ordinary artifact service.

use crate::{
    ApiScope, ApplicationKind, ApplicationPrincipal, ArtifactApi, ArtifactChunk, ArtifactPurpose,
    CallerContext, CreateArtifactUploadRequest, EventSourcedArtifactApi, IdempotencyKey, RequestId,
    scopes,
};
use async_trait::async_trait;
use colossus_contracts::ModelImageReference;
use colossus_ports::{
    BrowserArtifactError, BrowserArtifactPublication, BrowserArtifactPublisher, EventJournal,
    MAX_BROWSER_SCREENSHOT_BYTES,
};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

mod ownership;
mod transfer;

/// Stateless adapter. The trusted runtime passes its exact authoritative encrypted journal.
#[derive(Debug, Default)]
pub struct ReleasedBrowserArtifactPublisher;

#[async_trait]
impl BrowserArtifactPublisher for ReleasedBrowserArtifactPublisher {
    async fn resolve_upload(
        &self,
        journal: Arc<dyn EventJournal>,
        request: colossus_ports::BrowserUploadArtifactRequest,
    ) -> Result<colossus_ports::BrowserUploadArtifact, BrowserArtifactError> {
        transfer::resolve_upload(journal, request).await
    }

    async fn publish_download(
        &self,
        journal: Arc<dyn EventJournal>,
        request: colossus_ports::BrowserDownloadPublication,
    ) -> Result<colossus_ports::BrowserReleasedArtifact, BrowserArtifactError> {
        transfer::publish_download(journal, request).await
    }
    async fn publish(
        &self,
        journal: Arc<dyn EventJournal>,
        request: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError> {
        let invalid = || BrowserArtifactError::Invalid;
        if request.run_id.is_empty()
            || request.call_id.is_empty()
            || request.run_id.len() > 128
            || request.call_id.len() > 128
            || request.bytes.len() > MAX_BROWSER_SCREENSHOT_BYTES as usize
        {
            return Err(invalid());
        }
        let verified = colossus_media::validate_image_bytes(
            "browser-screenshot.png",
            Some("image/png"),
            &request.bytes,
        )
        .map_err(|_| invalid())?;
        let descriptor = &request.descriptor;
        if verified.size_bytes != u64::from(descriptor.size_bytes)
            || verified.sha256 != descriptor.sha256
            || verified.width_pixels != descriptor.width
            || verified.height_pixels != descriptor.height
        {
            return Err(invalid());
        }
        // Only this native port manufactures the caller from the already registered runtime binding.
        // No public artifact request may choose an application owner.
        let owner = ownership::resolve(journal.as_ref(), &request.binding)?;
        let principal = ApplicationPrincipal::authenticated(
            &owner,
            "native-browser-release",
            ApplicationKind::Embedded,
            [ApiScope::new(scopes::ARTIFACTS_WRITE).map_err(|_| invalid())?],
            Vec::<String>::new(),
            Vec::<String>::new(),
        )
        .map_err(|_| invalid())?;
        let mut identity = Sha256::new();
        identity.update(b"colossus-released-browser-artifact-v1\0");
        identity.update(serde_json::to_vec(&request.binding).map_err(|_| invalid())?);
        identity.update(serde_json::to_vec(descriptor).map_err(|_| invalid())?);
        for value in [
            &request.run_id,
            &request.call_id,
            descriptor.transfer_id.as_str(),
        ] {
            identity.update([0]);
            identity.update(value.as_bytes());
        }
        let key = hex::encode(identity.finalize());
        let caller = CallerContext::authenticated(
            principal,
            RequestId::new(format!("browser-release-{key}")).map_err(|_| invalid())?,
        );
        let resolver = colossus_media::JournalRunInputMediaResolver::new(Arc::clone(&journal));
        let service = EventSourcedArtifactApi::new(journal);
        let reservation = service
            .create_upload(
                &caller,
                CreateArtifactUploadRequest {
                    file_name: "browser-screenshot.png".into(),
                    media_type: verified.media_type.clone(),
                    size_bytes: verified.size_bytes,
                    sha256: verified.sha256.clone(),
                    purpose: ArtifactPurpose::RunOutput,
                    idempotency_key: IdempotencyKey::new(format!("browser-{key}"))
                        .map_err(|_| invalid())?,
                },
            )
            .await
            .map_err(|_| BrowserArtifactError::OutcomeUnknown)?;
        let mut offset = 0_u64;
        let chunks = request
            .bytes
            .chunks(reservation.chunk_size_bytes as usize)
            .map(|data| {
                let chunk = ArtifactChunk {
                    offset,
                    data: data.to_vec(),
                };
                offset += data.len() as u64;
                chunk
            })
            .collect();
        let artifact = service
            .upload(&caller, &reservation.upload_id, chunks)
            .await
            .map_err(|_| BrowserArtifactError::OutcomeUnknown)?;
        resolver
            .released_image_reference(&owner, &artifact.artifact_id)
            .map_err(|_| BrowserArtifactError::OutcomeUnknown)
    }
}

#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transfer_tests;
