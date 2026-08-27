use crate::validation::{ValidatedImage, normalize_media_type, validate_image_bytes};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{ModelImageDetail, ModelImageReference};
use colossus_ports::{
    EventJournal, ResolvedRunInputImage, RunInputMediaError, RunInputMediaResolver,
};
use serde::Deserialize;
use std::sync::Arc;

pub(crate) const AVAILABLE_EVENT: &str = "artifact.available.v1";

/// Journal-backed encrypted artifact resolver shared by runtime interfaces.
pub struct JournalRunInputMediaResolver {
    journal: Arc<dyn EventJournal>,
}

impl JournalRunInputMediaResolver {
    /// Bind the authoritative encrypted event journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    /// Authorize ownership and derive a durable metadata-only image reference.
    pub fn image_reference(
        &self,
        owner_id: &str,
        artifact_id: &str,
    ) -> Result<ModelImageReference, RunInputMediaError> {
        let stored = self.stored(owner_id, artifact_id)?;
        let validated = validate_image_bytes(
            &stored.artifact.file_name,
            Some(&stored.artifact.media_type),
            &stored.bytes,
        )?;
        metadata_matches(&stored.artifact, &validated)?;
        Ok(reference_from(stored.artifact, validated))
    }

    fn stored(
        &self,
        owner_id: &str,
        artifact_id: &str,
    ) -> Result<DecodedStoredArtifact, RunInputMediaError> {
        self.stored_event(artifact_id, Some(owner_id))
    }

    fn stored_without_owner(
        &self,
        artifact_id: &str,
    ) -> Result<DecodedStoredArtifact, RunInputMediaError> {
        self.stored_event(artifact_id, None)
    }

    fn stored_event(
        &self,
        artifact_id: &str,
        owner_id: Option<&str>,
    ) -> Result<DecodedStoredArtifact, RunInputMediaError> {
        validate_artifact_id(artifact_id)?;
        let events = self
            .journal
            .read_stream(&format!("artifact:{artifact_id}"))
            .map_err(|_| RunInputMediaError::Unavailable)?;
        let event = events.first().filter(|event| {
            event.event_type == AVAILABLE_EVENT
                && owner_id.is_none_or(|owner_id| event.actor.id == owner_id)
        });
        let Some(event) = event else {
            return Err(RunInputMediaError::Unavailable);
        };
        let stored: StoredArtifact = serde_json::from_value(
            self.journal
                .decrypt_payload(event)
                .map_err(|_| RunInputMediaError::Unavailable)?,
        )
        .map_err(|_| RunInputMediaError::Unavailable)?;
        if stored.artifact.artifact_id != artifact_id
            || stored.artifact.purpose != "run_input"
            || stored.artifact.state != "available"
        {
            return Err(RunInputMediaError::Unavailable);
        }
        let bytes = BASE64
            .decode(stored.content_base64)
            .map_err(|_| RunInputMediaError::Unavailable)?;
        Ok(DecodedStoredArtifact {
            artifact: stored.artifact,
            bytes,
        })
    }
}

#[async_trait]
impl RunInputMediaResolver for JournalRunInputMediaResolver {
    async fn resolve_image(
        &self,
        reference: &ModelImageReference,
    ) -> Result<ResolvedRunInputImage, RunInputMediaError> {
        let stored = self.stored_without_owner(&reference.artifact_id)?;
        let validated = validate_image_bytes(
            &stored.artifact.file_name,
            Some(&stored.artifact.media_type),
            &stored.bytes,
        )?;
        metadata_matches(&stored.artifact, &validated)?;
        let exact = reference_from(stored.artifact, validated);
        if &exact != reference {
            return Err(RunInputMediaError::Unavailable);
        }
        Ok(ResolvedRunInputImage {
            reference: exact,
            bytes: stored.bytes,
        })
    }
}

fn reference_from(
    artifact: StoredArtifactMetadata,
    validated: ValidatedImage,
) -> ModelImageReference {
    ModelImageReference {
        artifact_id: artifact.artifact_id,
        file_name: artifact.file_name,
        media_type: validated.media_type,
        size_bytes: validated.size_bytes,
        sha256: validated.sha256,
        width_pixels: validated.width_pixels,
        height_pixels: validated.height_pixels,
        detail: ModelImageDetail::Auto,
    }
}

fn validate_artifact_id(value: &str) -> Result<(), RunInputMediaError> {
    let Some(digest) = value.strip_prefix("artifact-") else {
        return Err(RunInputMediaError::Unavailable);
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RunInputMediaError::Unavailable);
    }
    Ok(())
}

fn metadata_matches(
    stored: &StoredArtifactMetadata,
    validated: &ValidatedImage,
) -> Result<(), RunInputMediaError> {
    if stored.size_bytes != validated.size_bytes
        || stored.sha256 != validated.sha256
        || normalize_media_type(&stored.media_type) != Some(validated.media_type.as_str())
    {
        return Err(RunInputMediaError::Unavailable);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifact {
    artifact: StoredArtifactMetadata,
    content_base64: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifactMetadata {
    artifact_id: String,
    file_name: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
    purpose: String,
    state: String,
    #[serde(rename = "created_at")]
    _created_at: String,
}

struct DecodedStoredArtifact {
    artifact: StoredArtifactMetadata,
    bytes: Vec<u8>,
}
