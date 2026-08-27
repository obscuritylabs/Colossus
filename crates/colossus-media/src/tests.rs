use super::*;
use crate::resolver::AVAILABLE_EVENT;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, ModelImageDetail, ModelImageReference,
    NewEvent,
};
use colossus_ports::{EventJournal, RunInputMediaError, RunInputMediaResolver};
use colossus_testkit::InMemoryEventJournal;
use image::{DynamicImage, ImageFormat};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{io::Cursor, sync::Arc};

fn encoded(format: ImageFormat) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgba8(2, 3)
        .write_to(&mut bytes, format)
        .expect("encode fixture");
    bytes.into_inner()
}

fn reference(index: usize, size_bytes: u64, width: u32, height: u32) -> ModelImageReference {
    ModelImageReference {
        artifact_id: format!("artifact-{index:064x}"),
        file_name: format!("image-{index}.png"),
        media_type: "image/png".into(),
        size_bytes,
        sha256: format!("{:064x}", index.saturating_add(1)),
        width_pixels: width,
        height_pixels: height,
        detail: ModelImageDetail::Auto,
    }
}

fn jpeg_with_exact_size(target: usize) -> Vec<u8> {
    let jpeg = encoded(ImageFormat::Jpeg);
    assert!(jpeg.starts_with(&[0xff, 0xd8]));
    assert!(target >= jpeg.len().saturating_add(4));
    let mut comments = Vec::with_capacity(target - jpeg.len());
    let mut remaining = target - jpeg.len();
    while remaining > 0 {
        let mut segment_size = remaining.min(65_537);
        if remaining.saturating_sub(segment_size) < 4 && remaining != segment_size {
            segment_size = remaining - 4;
        }
        assert!(segment_size >= 4);
        let payload_size = segment_size - 4;
        comments.extend_from_slice(&[0xff, 0xfe]);
        comments.extend_from_slice(
            &u16::try_from(payload_size + 2)
                .expect("JPEG comment length")
                .to_be_bytes(),
        );
        comments.resize(comments.len() + payload_size, b'x');
        remaining -= segment_size;
    }
    let mut padded = Vec::with_capacity(target);
    padded.extend_from_slice(&jpeg[..2]);
    padded.extend_from_slice(&comments);
    padded.extend_from_slice(&jpeg[2..]);
    assert_eq!(padded.len(), target);
    padded
}

#[test]
fn validates_supported_exact_bytes_and_rejects_spoofing() {
    for (format, media_type) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = encoded(format);
        let validated =
            validate_image_bytes("fixture.img", Some(media_type), &bytes).expect("valid image");
        assert_eq!((validated.width_pixels, validated.height_pixels), (2, 3));
        assert_eq!(validated.size_bytes, bytes.len() as u64);
        assert_eq!(validated.sha256, hex::encode(Sha256::digest(&bytes)));
        assert!(
            validate_image_bytes(
                "fixture.img",
                Some("image/png"),
                &encoded(ImageFormat::Jpeg)
            )
            .is_err()
        );
    }
}

#[test]
fn rejects_truncation_unsupported_formats_and_animation_markers() {
    let mut png = encoded(ImageFormat::Png);
    png.truncate(png.len() / 2);
    assert!(validate_image_bytes("bad.png", Some("image/png"), &png).is_err());
    assert!(validate_image_bytes("bad.gif", Some("image/gif"), b"GIF89a").is_err());

    let mut apng = encoded(ImageFormat::Png);
    apng.splice(33..33, [0, 0, 0, 0, b'a', b'c', b'T', b'L', 0, 0, 0, 0]);
    assert!(validate_image_bytes("animated.png", Some("image/png"), &apng).is_err());

    let mut animated_webp = b"RIFF\0\0\0\0WEBPANIM\0\0\0\0".to_vec();
    let riff_size = u32::try_from(animated_webp.len() - 8).expect("RIFF length");
    animated_webp[4..8].copy_from_slice(&riff_size.to_le_bytes());
    assert!(validate_image_bytes("animated.webp", Some("image/webp"), &animated_webp).is_err());
}

#[test]
fn exact_and_over_limit_byte_boundaries_are_enforced_before_transmission() {
    let exact = jpeg_with_exact_size(MAX_IMAGE_BYTES as usize);
    let validated = validate_image_bytes("maximum.jpg", Some("image/jpeg"), &exact)
        .expect("exactly 16 MiB is valid");
    assert_eq!(validated.size_bytes, MAX_IMAGE_BYTES);

    let mut over = exact;
    over.push(0);
    let error = validate_image_bytes("too-large.jpg", Some("image/jpeg"), &over)
        .expect_err("16 MiB plus one byte must fail");
    assert!(error.to_string().contains("16 MiB"));
}

#[test]
fn aggregate_count_byte_dimension_and_pixel_boundaries_are_exact() {
    validate_image_references(
        (0..16).map(|index| reference(index, MAX_COMBINED_IMAGE_BYTES / 16, 10_000, 10_000)),
    )
    .expect("exact count, byte, side, and pixel bounds");

    assert!(
        validate_image_references((0..17).map(|index| reference(index, 1, 1_024, 1_024))).is_err()
    );
    assert!(
        validate_image_references([
            reference(0, MAX_IMAGE_BYTES, 1_024, 1_024),
            reference(1, MAX_IMAGE_BYTES, 1_024, 1_024),
            reference(2, 1, 1_024, 1_024),
        ])
        .is_err()
    );
    assert!(validate_image_references([reference(0, 1, MAX_IMAGE_SIDE_PIXELS + 1, 1)]).is_err());
    assert!(validate_image_references([reference(0, 1, 10_000, 10_001)]).is_err());
    assert!(validate_image_references([reference(0, 1, 0, 1)]).is_err());
}

#[tokio::test]
async fn artifact_ownership_is_rechecked_after_resolver_restart() {
    let bytes = encoded(ImageFormat::Png);
    let digest = hex::encode(Sha256::digest(&bytes));
    let artifact_id = format!("artifact-{}", "c".repeat(64));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: format!("artifact:{artifact_id}"),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: AVAILABLE_EVENT.into(),
            actor: Actor {
                actor_type: ActorType::Application,
                id: "app:owner-a".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({
                "artifact": {
                    "artifact_id": artifact_id,
                    "file_name": "owned.png",
                    "media_type": "image/png",
                    "size_bytes": bytes.len(),
                    "sha256": digest,
                    "purpose": "run_input",
                    "state": "available",
                    "created_at": "2026-08-26T00:00:00Z",
                },
                "content_base64": BASE64.encode(&bytes),
            }),
        })
        .expect("artifact event");

    let first = JournalRunInputMediaResolver::new(Arc::clone(&journal));
    let reference = first
        .image_reference("app:owner-a", &artifact_id)
        .expect("owner reference");
    assert!(matches!(
        first.image_reference("app:owner-b", &artifact_id),
        Err(RunInputMediaError::Unavailable)
    ));

    let reopened = JournalRunInputMediaResolver::new(journal);
    assert!(matches!(
        reopened.image_reference("app:owner-b", &artifact_id),
        Err(RunInputMediaError::Unavailable)
    ));
    let resolved = reopened
        .resolve_image(&reference)
        .await
        .expect("late verified resolution");
    assert_eq!(resolved.bytes, bytes);
    assert_eq!(resolved.reference, reference);
}
