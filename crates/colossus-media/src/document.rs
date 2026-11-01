//! Bounded document envelopes. Providers own content parsing; nothing is extracted or executed.
use colossus_contracts::ModelFileReference;
use colossus_ports::RunInputMediaError;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeSet, io::Cursor};

/// Maximum documents in one provider-visible context.
pub const MAX_DOCUMENT_COUNT: usize = colossus_contracts::MAX_DOCUMENT_INPUT_COUNT;
/// Maximum exact bytes for one document, matching artifact admission.
pub const MAX_DOCUMENT_BYTES: u64 = colossus_contracts::MAX_DOCUMENT_INPUT_BYTES;
/// Maximum combined document bytes in one provider-visible context.
pub const MAX_COMBINED_DOCUMENT_BYTES: u64 = colossus_contracts::MAX_COMBINED_DOCUMENT_INPUT_BYTES;
const MAX_OFFICE_ENTRIES: usize = 2048;
const MAX_OFFICE_DIRECTORY_BYTES: usize = 512 * 1024;
const MAX_OFFICE_EXPANDED_BYTES: u64 = 128 * 1_048_576;

/// Metadata derived from a bounded native document envelope and exact bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedDocument {
    /// Normalized supported MIME type.
    pub media_type: String,
    /// Exact byte length.
    pub size_bytes: u64,
    /// SHA-256 of the complete unchanged document.
    pub sha256: String,
}

/// Supported filename extensions; this is a declaration, not byte validation.
pub fn document_media_type_for_file_name(name: &str) -> Option<&'static str> {
    match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "pdf" => Some("application/pdf"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        _ => None,
    }
}

/// Whether a normalized MIME is eligible for native document admission.
pub fn supported_document_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "application/pdf"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    )
}

/// Verify the filename/MIME, bounded PDF/OOXML envelope, and digest without parsing document content.
pub fn validate_document_bytes(
    file_name: &str,
    declared_media_type: Option<&str>,
    bytes: &[u8],
) -> Result<ValidatedDocument, RunInputMediaError> {
    crate::validation::validate_file_name(file_name)?;
    let media_type = document_media_type_for_file_name(file_name)
        .ok_or_else(|| invalid("only PDF, DOCX, XLSX, and PPTX documents are supported"))?;
    if declared_media_type
        .is_some_and(|declared| declared.trim().to_ascii_lowercase() != media_type)
    {
        return Err(invalid("document filename and declared MIME type disagree"));
    }
    let size_bytes = u64::try_from(bytes.len()).map_err(|_| invalid("document size overflowed"))?;
    if size_bytes == 0 || size_bytes > MAX_DOCUMENT_BYTES {
        return Err(invalid("documents must contain between 1 byte and 16 MiB"));
    }
    if media_type == "application/pdf" {
        let header = bytes.get(..8).is_some_and(|header| {
            header.starts_with(b"%PDF-")
                && header[5].is_ascii_digit()
                && header[6] == b'.'
                && header[7].is_ascii_digit()
        });
        let tail = &bytes[bytes.len().saturating_sub(1024)..];
        if !header || !tail.windows(5).any(|part| part == b"%%EOF") {
            return Err(invalid("document does not have a bounded PDF envelope"));
        }
    } else {
        validate_office_envelope(bytes, media_type)?;
    }
    Ok(ValidatedDocument {
        media_type: media_type.into(),
        size_bytes,
        sha256: hex::encode(Sha256::digest(bytes)),
    })
}

/// Recheck aggregate bounds whenever references become provider-visible.
pub fn validate_document_references<'a>(
    references: impl IntoIterator<Item = &'a ModelFileReference>,
) -> Result<(), RunInputMediaError> {
    let mut count = 0_usize;
    let mut combined = 0_u64;
    for file in references {
        count += 1;
        combined = combined
            .checked_add(file.size_bytes)
            .ok_or_else(|| invalid("document size overflowed"))?;
        if count > MAX_DOCUMENT_COUNT
            || file.size_bytes == 0
            || file.size_bytes > MAX_DOCUMENT_BYTES
            || combined > MAX_COMBINED_DOCUMENT_BYTES
            || !supported_document_media_type(&file.media_type)
        {
            return Err(invalid(
                "documents exceed the 4-file, 16 MiB per file, or 32 MiB combined bound",
            ));
        }
    }
    Ok(())
}

fn validate_office_envelope(bytes: &[u8], media_type: &str) -> Result<(), RunInputMediaError> {
    // Bound central-directory allocation before asking the ZIP reader to parse metadata.
    // Office inputs this small never need ZIP64 or a multi-volume archive.
    let eocd = bytes
        .windows(4)
        .rposition(|part| part == b"PK\x05\x06")
        .ok_or_else(|| invalid("Office document has no ZIP directory"))?;
    let footer = bytes
        .get(eocd..)
        .filter(|footer| footer.len() >= 22)
        .ok_or_else(|| invalid("Office ZIP directory is truncated"))?;
    let u16_at = |at| u16::from_le_bytes([footer[at], footer[at + 1]]);
    let u32_at =
        |at| u32::from_le_bytes([footer[at], footer[at + 1], footer[at + 2], footer[at + 3]]);
    let count = usize::from(u16_at(10));
    let directory_size =
        usize::try_from(u32_at(12)).map_err(|_| invalid("Office directory size overflowed"))?;
    let directory_start =
        usize::try_from(u32_at(16)).map_err(|_| invalid("Office directory offset overflowed"))?;
    if bytes.get(..4) != Some(b"PK\x03\x04")
        || u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(8) != u16_at(10)
        || count == 0
        || count > MAX_OFFICE_ENTRIES
        || directory_size > MAX_OFFICE_DIRECTORY_BYTES
        || directory_start.checked_add(directory_size) != Some(eocd)
        || footer.len() != 22 + usize::from(u16_at(20))
    {
        return Err(invalid("Office ZIP directory exceeds its bounded envelope"));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| invalid("Office ZIP directory is invalid"))?;
    if archive.len() != count || archive.offset() != 0 {
        return Err(invalid("Office ZIP directory is inconsistent"));
    }
    archive
        .validate_header_consistency()
        .map_err(|_| invalid("Office ZIP headers disagree"))?;
    let expected_part = match media_type {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            "word/document.xml"
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xl/workbook.xml",
        _ => "ppt/presentation.xml",
    };
    let mut names = BTreeSet::new();
    let mut expanded = 0_u64;
    for index in 0..count {
        let file = archive
            .by_index_raw(index)
            .map_err(|_| invalid("Office ZIP entry is invalid"))?;
        let name = file
            .name()
            .map_err(|_| invalid("Office ZIP entry name is invalid"))?;
        if name.len() > 1024
            || name.contains(['\\', ':'])
            || name.chars().any(char::is_control)
            || name.split('/').any(|part| part == "..")
            || file.enclosed_name().is_none()
            || file.is_symlink()
            || file.encrypted()
            || !names.insert(name.into_owned())
        {
            return Err(invalid(
                "Office ZIP entry is unsafe, duplicated, or encrypted",
            ));
        }
        expanded = expanded
            .checked_add(file.size())
            .ok_or_else(|| invalid("Office expanded size overflowed"))?;
        if expanded > MAX_OFFICE_EXPANDED_BYTES {
            return Err(invalid("Office document expanded size exceeds 128 MiB"));
        }
    }
    if !names.contains("[Content_Types].xml")
        || !names.contains(expected_part)
        || names
            .iter()
            .any(|name| name.to_ascii_lowercase().contains("vbaproject"))
    {
        return Err(invalid(
            "Office document has mismatched format parts or contains macros",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> RunInputMediaError {
    RunInputMediaError::Invalid(message.into())
}
