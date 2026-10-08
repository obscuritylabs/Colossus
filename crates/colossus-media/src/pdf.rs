use colossus_contracts::ModelFileReference;
use colossus_ports::RunInputMediaError;
use sha2::{Digest as _, Sha256};

/// Maximum exact bytes for one PDF input, matching public artifact upload limits.
pub const MAX_PDF_BYTES: u64 = 16 * 1_048_576;
/// Maximum PDF count in one provider-visible context.
pub const MAX_PDF_COUNT: usize = 4;
/// Maximum combined PDF bytes in one provider-visible context.
pub const MAX_COMBINED_PDF_BYTES: u64 = 32 * 1_048_576;

/// Validate a bounded PDF envelope without executing or decoding its contents.
/// The provider owns full PDF parsing; Colossus verifies the media envelope and digest.
pub fn validate_pdf_bytes(
    file_name: &str,
    bytes: &[u8],
) -> Result<(u64, String), RunInputMediaError> {
    if file_name.is_empty()
        || file_name.len() > 255
        || file_name.chars().any(char::is_control)
        || file_name.contains(['/', '\\'])
    {
        return Err(RunInputMediaError::Invalid(
            "PDF file name is invalid".into(),
        ));
    }
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let header = bytes.get(..8).is_some_and(|header| {
        header.starts_with(b"%PDF-")
            && header[5].is_ascii_digit()
            && header[6] == b'.'
            && header[7].is_ascii_digit()
    });
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    if size == 0
        || size > MAX_PDF_BYTES
        || !header
        || !tail.windows(5).any(|window| window == b"%%EOF")
    {
        return Err(RunInputMediaError::Invalid(
            "PDF must have a valid envelope and be no larger than 16 MiB".into(),
        ));
    }
    Ok((size, hex::encode(Sha256::digest(bytes))))
}

/// Reapply provider-visible count and byte bounds to verified PDF references.
pub fn validate_pdf_references<'a>(
    references: impl IntoIterator<Item = &'a ModelFileReference>,
) -> Result<(), RunInputMediaError> {
    let mut count = 0_usize;
    let mut combined = 0_u64;
    for file in references {
        count = count.saturating_add(1);
        combined = combined
            .checked_add(file.size_bytes)
            .ok_or_else(|| RunInputMediaError::Invalid("PDF size overflowed".into()))?;
        if file.media_type != "application/pdf"
            || file.size_bytes == 0
            || file.size_bytes > MAX_PDF_BYTES
            || count > MAX_PDF_COUNT
            || combined > MAX_COMBINED_PDF_BYTES
        {
            return Err(RunInputMediaError::Invalid(
                "PDF inputs exceed the format, 4-file, 16 MiB-per-file, or 32 MiB-combined bound"
                    .into(),
            ));
        }
    }
    Ok(())
}
