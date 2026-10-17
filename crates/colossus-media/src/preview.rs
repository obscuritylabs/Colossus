use crate::validation::decode_validated_image;
use colossus_ports::RunInputMediaError;
use image::ImageFormat;
use std::io::Cursor;

/// Create a bounded static PNG preview from an already owner-authorized artifact.
/// The original bytes remain unchanged; SVG and animated formats are rejected.
pub fn image_preview(
    file_name: &str,
    media_type: &str,
    bytes: &[u8],
    thumbnail: bool,
) -> Result<Vec<u8>, RunInputMediaError> {
    let (_, decoded) = decode_validated_image(file_name, Some(media_type), bytes)?;
    let max_side = if thumbnail { 256 } else { 2048 };
    let rendered = if decoded.width() <= max_side && decoded.height() <= max_side {
        decoded
    } else {
        decoded.thumbnail(max_side, max_side)
    };
    let mut output = Cursor::new(Vec::new());
    rendered
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| RunInputMediaError::Unavailable)?;
    Ok(output.into_inner())
}
