use crate::{ValidatedImage, validate_image_bytes};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, EventClassification, ExecutionContext, ModelImageDetail, ModelImageReference, NewEvent,
};
use colossus_ports::{EventJournal, RunInputMediaError};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Retain policy-released image bytes under their trusted run owner.
/// The caller must finish filesystem post-effect authorization before calling this.
pub fn import_released_images(
    journal: &dyn EventJournal,
    owner: Actor,
    context: ExecutionContext,
    images: Vec<(String, Vec<u8>)>,
) -> Result<Vec<ModelImageReference>, RunInputMediaError> {
    if images.is_empty()
        || images.len() > 16
        || images.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() > 32 * 1_048_576
    {
        return Err(RunInputMediaError::Invalid(
            "image views require 1–16 images and at most 32 MiB combined".into(),
        ));
    }
    let validated = images
        .iter()
        .map(|(name, bytes)| validate_image_bytes(name, None, bytes))
        .collect::<Result<Vec<_>, _>>()?;
    images
        .into_iter()
        .zip(validated)
        .map(|((name, bytes), validated)| {
            retain(
                journal,
                owner.clone(),
                context.clone(),
                &name,
                &bytes,
                validated,
            )
        })
        .collect()
}

fn retain(
    journal: &dyn EventJournal,
    owner: Actor,
    context: ExecutionContext,
    file_name: &str,
    bytes: &[u8],
    validated: ValidatedImage,
) -> Result<ModelImageReference, RunInputMediaError> {
    let mut hash = Sha256::new();
    hash.update(b"colossus-viewed-image-v1\0");
    hash.update(owner.id.as_bytes());
    hash.update(b"\0");
    hash.update(uuid::Uuid::now_v7().as_bytes());
    hash.update(validated.sha256.as_bytes());
    let image = ModelImageReference {
        artifact_id: format!("artifact-{}", hex::encode(hash.finalize())),
        file_name: file_name.into(),
        media_type: validated.media_type,
        size_bytes: validated.size_bytes,
        sha256: validated.sha256,
        width_pixels: validated.width_pixels,
        height_pixels: validated.height_pixels,
        detail: ModelImageDetail::Auto,
    };
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| RunInputMediaError::Unavailable)?;
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: format!("artifact:{}", image.artifact_id),
            expected_stream_version: 0,
            classification: EventClassification::System,
            event_type: "artifact.available.v1".into(),
            actor: owner,
            context,
            payload: json!({
                "artifact": {
                    "artifact_id": image.artifact_id,
                    "file_name": image.file_name,
                    "media_type": image.media_type,
                    "size_bytes": image.size_bytes,
                    "sha256": image.sha256,
                    "purpose": "run_input",
                    "state": "available",
                    "created_at": created_at,
                },
                "content_base64": BASE64.encode(bytes),
            }),
        })
        .map_err(|_| RunInputMediaError::Unavailable)?;
    Ok(image)
}
