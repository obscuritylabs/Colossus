//! Treat external display assets as untrusted protocol data, with no I/O.

use super::{ApiResult, protocol_error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;

const PREFIX: &str = "data:image/png;base64,";
const MAX_BYTES: usize = 64 * 1024;

pub(super) struct NormalizationBudget {
    remaining_pixels: u32,
    remaining_images: u32,
}

impl Default for NormalizationBudget {
    fn default() -> Self {
        Self {
            remaining_pixels: 8 * 1024 * 1024,
            remaining_images: 64,
        }
    }
}

pub(super) fn validated(
    source: String,
    budget: &mut NormalizationBudget,
) -> ApiResult<Option<String>> {
    if source.is_empty() {
        return Ok(None);
    }
    let encoded = source.strip_prefix(PREFIX).ok_or_else(protocol_error)?;
    if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(protocol_error());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| protocol_error())?;
    if bytes.len() > MAX_BYTES {
        return Err(protocol_error());
    }
    // Read the mandatory fixed-size IHDR before constructing a decoder, which
    // may already inflate ancillary chunks. Codec validation still checks the
    // complete image before any retained pixels are released.
    if bytes.len() < 33 || &bytes[..16] != b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR" {
        return Err(protocol_error());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| protocol_error())?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| protocol_error())?);
    if !(1..=512).contains(&width) || !(1..=512).contains(&height) {
        return Err(protocol_error());
    }
    let pixels = width * height;
    if budget.remaining_images == 0 || pixels > budget.remaining_pixels {
        return Ok(None);
    }
    budget.remaining_images -= 1;
    budget.remaining_pixels -= pixels;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader.decode().map_err(|_| protocol_error())?;
    let mut normalized = Cursor::new(Vec::new());
    pixels
        .write_to(&mut normalized, ImageFormat::Png)
        .map_err(|_| protocol_error())?;
    let normalized = normalized.into_inner();
    if normalized.len() > MAX_BYTES {
        return Err(protocol_error());
    }
    Ok(Some(format!("{PREFIX}{}", STANDARD.encode(normalized))))
}
