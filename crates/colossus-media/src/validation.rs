use colossus_contracts::ModelImageReference;
use colossus_ports::RunInputMediaError;
use image::{ImageFormat, ImageReader, Limits};
use sha2::{Digest as _, Sha256};
use std::io::Cursor;

/// Maximum image count in one provider-visible context.
pub const MAX_IMAGE_COUNT: usize = 16;
/// Maximum exact bytes for one run-input image.
pub const MAX_IMAGE_BYTES: u64 = 16 * 1_048_576;
/// Maximum combined exact bytes in one provider-visible context.
pub const MAX_COMBINED_IMAGE_BYTES: u64 = 32 * 1_048_576;
/// Maximum decoded width or height.
pub const MAX_IMAGE_SIDE_PIXELS: u32 = 16_384;
/// Maximum decoded pixel count.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const MAX_DECODE_ALLOC_BYTES: u64 = 512 * 1_048_576;

/// Safe verified properties derived from exact image bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedImage {
    /// Normalized detected MIME type.
    pub media_type: String,
    /// Exact byte length.
    pub size_bytes: u64,
    /// Lowercase SHA-256 of the exact bytes.
    pub sha256: String,
    /// Decoded width.
    pub width_pixels: u32,
    /// Decoded height.
    pub height_pixels: u32,
}

/// Validate one exact PNG, JPEG, or static WebP without normalizing its bytes.
pub fn validate_image_bytes(
    file_name: &str,
    declared_media_type: Option<&str>,
    bytes: &[u8],
) -> Result<ValidatedImage, RunInputMediaError> {
    validate_file_name(file_name)?;
    let size_bytes = u64::try_from(bytes.len()).map_err(|_| invalid("image size overflowed"))?;
    if size_bytes == 0 || size_bytes > MAX_IMAGE_BYTES {
        return Err(invalid("image size must be between 1 byte and 16 MiB"));
    }
    let format = image::guess_format(bytes).map_err(|_| invalid("unsupported image format"))?;
    let media_type = match format {
        ImageFormat::Png => {
            reject_animated_png(bytes)?;
            "image/png"
        }
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => {
            reject_animated_webp(bytes)?;
            "image/webp"
        }
        _ => {
            return Err(invalid(
                "only PNG, JPEG, and static WebP images are supported",
            ));
        }
    };
    if let Some(declared) = declared_media_type
        && normalize_media_type(declared) != Some(media_type)
    {
        return Err(invalid("declared MIME type does not match the image bytes"));
    }

    let (width_pixels, height_pixels) = image_dimensions(bytes, format)?;
    validate_dimensions(width_pixels, height_pixels)?;
    // Decode after the allocation and dimension ceilings are installed. This catches
    // corrupt and truncated content while retaining the exact input bytes unchanged.
    let decoded = bounded_reader(bytes, format)
        .decode()
        .map_err(|_| invalid("image is corrupt or truncated"))?;
    if decoded.width() != width_pixels || decoded.height() != height_pixels {
        return Err(invalid("decoded image dimensions are inconsistent"));
    }

    Ok(ValidatedImage {
        media_type: media_type.into(),
        size_bytes,
        sha256: hex::encode(Sha256::digest(bytes)),
        width_pixels,
        height_pixels,
    })
}

/// Reapply provider-visible aggregate image limits to ordered references.
pub fn validate_image_references(
    references: impl IntoIterator<Item = ModelImageReference>,
) -> Result<Vec<ModelImageReference>, RunInputMediaError> {
    let references = references.into_iter().collect::<Vec<_>>();
    if references.len() > MAX_IMAGE_COUNT {
        return Err(invalid("at most 16 images may be provider-visible"));
    }
    let mut combined = 0_u64;
    for reference in &references {
        if reference.size_bytes == 0 || reference.size_bytes > MAX_IMAGE_BYTES {
            return Err(invalid("one image exceeds the 16 MiB bound"));
        }
        validate_dimensions(reference.width_pixels, reference.height_pixels)?;
        combined = combined
            .checked_add(reference.size_bytes)
            .ok_or_else(|| invalid("combined image size overflowed"))?;
        if combined > MAX_COMBINED_IMAGE_BYTES {
            return Err(invalid("combined images exceed the 32 MiB bound"));
        }
    }
    Ok(references)
}

pub(crate) fn normalize_media_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn bounded_reader(bytes: &[u8], format: ImageFormat) -> ImageReader<Cursor<&[u8]>> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE_PIXELS);
    limits.max_image_height = Some(MAX_IMAGE_SIDE_PIXELS);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    reader.limits(limits);
    reader
}

fn image_dimensions(bytes: &[u8], format: ImageFormat) -> Result<(u32, u32), RunInputMediaError> {
    bounded_reader(bytes, format)
        .into_dimensions()
        .map_err(|_| invalid("image header is corrupt or truncated"))
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), RunInputMediaError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE_PIXELS
        || height > MAX_IMAGE_SIDE_PIXELS
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(invalid(
            "image dimensions exceed 16,384 pixels per side or 100 megapixels",
        ));
    }
    Ok(())
}

fn validate_file_name(value: &str) -> Result<(), RunInputMediaError> {
    if value.is_empty()
        || value.len() > 255
        || value.trim() != value
        || matches!(value, "." | "..")
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid("image filename is not a safe bounded display name"));
    }
    Ok(())
}

fn reject_animated_png(bytes: &[u8]) -> Result<(), RunInputMediaError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.get(..8) != Some(SIGNATURE) {
        return Err(invalid("PNG signature is invalid"));
    }
    let mut offset = 8_usize;
    while offset < bytes.len() {
        let length_bytes = bytes
            .get(offset..offset.saturating_add(4))
            .ok_or_else(|| invalid("PNG chunk is truncated"))?;
        let length = usize::try_from(u32::from_be_bytes(
            length_bytes
                .try_into()
                .map_err(|_| invalid("PNG chunk is invalid"))?,
        ))
        .map_err(|_| invalid("PNG chunk length overflowed"))?;
        let kind = bytes
            .get(offset.saturating_add(4)..offset.saturating_add(8))
            .ok_or_else(|| invalid("PNG chunk type is truncated"))?;
        if kind == b"acTL" {
            return Err(invalid("animated PNG images are not supported"));
        }
        offset = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| invalid("PNG chunk length overflowed"))?;
        if offset > bytes.len() {
            return Err(invalid("PNG chunk is truncated"));
        }
        if kind == b"IEND" {
            return (offset == bytes.len())
                .then_some(())
                .ok_or_else(|| invalid("PNG contains bytes after its IEND chunk"));
        }
    }
    Err(invalid("PNG has no complete IEND chunk"))
}

fn reject_animated_webp(bytes: &[u8]) -> Result<(), RunInputMediaError> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(invalid("WebP container header is invalid"));
    }
    let declared = bytes
        .get(4..8)
        .and_then(|value| <[u8; 4]>::try_from(value).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| invalid("WebP container length is truncated"))?;
    if u64::from(declared).saturating_add(8) != bytes.len() as u64 {
        return Err(invalid("WebP container length does not match its bytes"));
    }
    let mut offset = 12_usize;
    while offset < bytes.len() {
        let kind = bytes
            .get(offset..offset.saturating_add(4))
            .ok_or_else(|| invalid("WebP chunk type is truncated"))?;
        let length = bytes
            .get(offset.saturating_add(4)..offset.saturating_add(8))
            .and_then(|value| <[u8; 4]>::try_from(value).ok())
            .map(u32::from_le_bytes)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| invalid("WebP chunk length is invalid"))?;
        let data_start = offset
            .checked_add(8)
            .ok_or_else(|| invalid("WebP chunk length overflowed"))?;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| invalid("WebP chunk length overflowed"))?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| invalid("WebP chunk is truncated"))?;
        if kind == b"ANIM" || kind == b"ANMF" {
            return Err(invalid("animated WebP images are not supported"));
        }
        if kind == b"VP8X" && data.first().is_some_and(|flags| flags & 0x02 != 0) {
            return Err(invalid("animated WebP images are not supported"));
        }
        offset = data_end
            .checked_add(length & 1)
            .ok_or_else(|| invalid("WebP chunk padding overflowed"))?;
    }
    if offset != bytes.len() {
        return Err(invalid("WebP chunk padding is truncated"));
    }
    Ok(())
}

fn invalid(detail: impl Into<String>) -> RunInputMediaError {
    let detail = detail.into();
    RunInputMediaError::Invalid(detail.chars().take(256).collect())
}
