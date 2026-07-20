use crate::{ApiError, ApiErrorReason, ApiResult};

pub(super) const MAX_IDENTIFIER_BYTES: usize = 128;
pub(super) const MAX_ROLE_BYTES: usize = 128;
pub(super) const MAX_TOOL_BYTES: usize = 256;
pub(super) const MAX_INPUT_PARTS: usize = 128;
pub(super) const MAX_INPUT_BYTES: usize = 1_048_576;
pub(super) const MAX_PAGE_SIZE: usize = 3;
pub(super) const MAX_UPDATE_PAGE_SIZE: usize = 1_024;

pub(super) fn token(value: &str, field: &str, max_bytes: usize) -> ApiResult<()> {
    if value.is_empty() || value.len() > max_bytes || value.trim() != value {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            field,
            format!("{field} must be non-empty and at most {max_bytes} bytes"),
        ));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
    }) {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            field,
            format!("{field} contains unsupported characters"),
        ));
    }
    Ok(())
}

pub(super) fn bounded_text(
    value: &str,
    field: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> ApiResult<()> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > max_bytes {
        return Err(ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            field,
            format!("{field} must be at most {max_bytes} bytes"),
        ));
    }
    Ok(())
}
