//! Closed private file custody validation before allocation and after dispatch.
use super::*;
use colossus_ports::{
    BrowserDownloadDescriptor, BrowserUploadPrepareRequest, BrowserUploadReceipt,
    MAX_BROWSER_TRANSFER_BYTES,
};

pub(crate) fn upload(request: &BrowserUploadPrepareRequest) -> Result<(), BrowserDriverError> {
    let BrowserAction::Upload {
        artifact_id,
        max_bytes,
        element,
    } = &request.command.action
    else {
        return Err(BrowserDriverError::Unsupported);
    };
    let descriptor = &request.descriptor;
    if !identity(&descriptor.artifact_id)
        || artifact_id != &descriptor.artifact_id
        || descriptor.size_bytes == 0
        || descriptor.size_bytes > *max_bytes
        || descriptor.size_bytes > MAX_BROWSER_TRANSFER_BYTES
        || !hex(&descriptor.sha256, 64)
        || descriptor.file_name.is_empty()
        || descriptor.file_name.len() > 255
        || matches!(descriptor.file_name.as_str(), "." | "..")
        || descriptor
            .file_name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        || element.document_id != request.command.target.document_id
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(crate) fn receipt(
    value: &BrowserUploadReceipt,
    token: Option<&str>,
    offset: u32,
) -> Result<(), BrowserDriverError> {
    if !hex(&value.transfer_id, 32)
        || token.is_some_and(|token| token != value.transfer_id)
        || value.next_offset != offset
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(())
}

pub(crate) fn download(
    value: &BrowserDownloadDescriptor,
    command: &BrowserDriverCommand,
    origins: &[colossus_contracts::BrowserOrigin],
) -> Result<(), BrowserDriverError> {
    let BrowserAction::Download { max_bytes, element } = &command.action else {
        return Err(BrowserDriverError::Unsupported);
    };
    if value.session_id != command.session_id
        || value.target != command.target
        || value.control_generation != command.control_generation
        || value.size_bytes > *max_bytes
        || value.size_bytes > MAX_BROWSER_TRANSFER_BYTES
        || !hex(&value.transfer_id, 32)
        || !hex(&value.sha256, 64)
        || !origins.contains(&value.origin)
        || element.document_id != command.target.document_id
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(())
}

pub(crate) fn upload_chunk(data: &str, remaining: u32) -> Result<u32, BrowserDriverError> {
    chunk_length(data, remaining).map_err(|_| BrowserDriverError::Denied)
}

/// One empty canonical read consumes an explicitly empty download, never a screenshot.
pub(crate) fn download_chunk(
    value: &colossus_ports::BrowserScreenshotChunk,
    offset: u32,
    remaining: u32,
) -> Result<u32, BrowserDriverError> {
    if remaining == 0 && offset == 0 && value.offset == 0 && value.data_base64.is_empty() {
        return Ok(0);
    }
    screenshot_chunk(value, offset, remaining)
}
