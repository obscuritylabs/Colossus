//! Native publication of policy-released selected-window PNGs.

use crate::{
    ApiScope, ApplicationKind, ApplicationPrincipal, ArtifactApi, ArtifactChunk, ArtifactPurpose,
    CallerContext, CreateArtifactUploadRequest, EventSourcedArtifactApi, IdempotencyKey, RequestId,
    scopes,
};
use async_trait::async_trait;
use colossus_contracts::{ComputerLimits, ComputerScope, ModelImageReference};
use colossus_ports::{
    ComputerArtifactError, ComputerArtifactPublication, ComputerArtifactPublisher, EventJournal,
};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

/// Stateless publisher; ownership comes from the trusted registered run binding.
#[derive(Debug, Default)]
pub struct ReleasedComputerArtifactPublisher;

#[async_trait]
impl ComputerArtifactPublisher for ReleasedComputerArtifactPublisher {
    async fn publish(
        &self,
        journal: Arc<dyn EventJournal>,
        request: ComputerArtifactPublication,
    ) -> Result<ModelImageReference, ComputerArtifactError> {
        validate(&request)?;
        let workflow = match &request.binding.scope {
            ComputerScope::Conversation { .. } => None,
            ComputerScope::Workflow { id } => Some(id.as_str()),
        };
        let owner = crate::browser_artifacts::ownership::resolve_owner(
            journal.as_ref(),
            &request.binding.application_id,
            workflow,
        )
        .map_err(|_| ComputerArtifactError::Invalid)?;
        let principal =
            ApplicationPrincipal::authenticated(
                &owner,
                "native-computer-release",
                ApplicationKind::Embedded,
                [ApiScope::new(scopes::ARTIFACTS_WRITE)
                    .map_err(|_| ComputerArtifactError::Invalid)?],
                Vec::<String>::new(),
                Vec::<String>::new(),
            )
            .map_err(|_| ComputerArtifactError::Invalid)?;
        let key = publication_key(&request)?;
        let caller = CallerContext::authenticated(
            principal,
            RequestId::new(format!("computer-release-{key}"))
                .map_err(|_| ComputerArtifactError::Invalid)?,
        );
        let resolver = colossus_media::JournalRunInputMediaResolver::new(Arc::clone(&journal));
        let service = EventSourcedArtifactApi::new(journal);
        let reservation = service
            .create_upload(
                &caller,
                CreateArtifactUploadRequest {
                    file_name: "computer-screenshot.png".into(),
                    media_type: "image/png".into(),
                    size_bytes: u64::from(request.descriptor.size_bytes),
                    sha256: request.descriptor.sha256.clone(),
                    purpose: ArtifactPurpose::RunOutput,
                    idempotency_key: IdempotencyKey::new(format!("computer-{key}"))
                        .map_err(|_| ComputerArtifactError::Invalid)?,
                },
            )
            .await
            .map_err(|_| ComputerArtifactError::OutcomeUnknown)?;
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
            .map_err(|_| ComputerArtifactError::OutcomeUnknown)?;
        resolver
            .released_image_reference(&owner, &artifact.artifact_id)
            .map_err(|_| ComputerArtifactError::OutcomeUnknown)
    }
}

fn validate(request: &ComputerArtifactPublication) -> Result<(), ComputerArtifactError> {
    let invalid = || ComputerArtifactError::Invalid;
    request.binding.validate().map_err(|_| invalid())?;
    request
        .descriptor
        .validate(&ComputerLimits::HARD_MAX)
        .map_err(|_| invalid())?;
    if request.call_id.is_empty()
        || request.call_id.len() > 128
        || request.call_id.chars().any(char::is_control)
        || request.bytes.len() > ComputerLimits::HARD_MAX.max_screenshot_bytes as usize
    {
        return Err(invalid());
    }
    let verified = colossus_media::validate_image_bytes(
        "computer-screenshot.png",
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
    Ok(())
}

fn publication_key(request: &ComputerArtifactPublication) -> Result<String, ComputerArtifactError> {
    let mut identity = Sha256::new();
    identity.update(b"colossus-released-computer-artifact-v1\0");
    identity
        .update(serde_json::to_vec(&request.binding).map_err(|_| ComputerArtifactError::Invalid)?);
    identity.update([0]);
    identity.update(
        serde_json::to_vec(&request.descriptor).map_err(|_| ComputerArtifactError::Invalid)?,
    );
    identity.update([0]);
    identity.update(request.call_id.as_bytes());
    Ok(hex::encode(identity.finalize()))
}

#[cfg(test)]
mod tests;
