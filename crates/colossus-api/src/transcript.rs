//! Optional read-only canonical conversation disclosure.

use crate::{ApiError, ApiErrorReason, ApiResult};
use serde::{Deserialize, Serialize};

/// Advertised only when the authenticated caller can read canonical transcripts.
pub const SESSION_TRANSCRIPT_CAPABILITY: &str = "conversations.transcript.v1";
/// Maximum public opaque continuation token size in UTF-8 bytes.
pub const MAX_TRANSCRIPT_CURSOR_BYTES: usize = 4096;

/// A canonical session export addressed through an owned or explicitly shared run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadSessionTranscriptRequest {
    /// Authorized anchor; its session, rather than only this run, is exported.
    pub source_run_id: String,
    /// Maximum eligible messages, zero selects 32; accepted range is 1..=32.
    pub page_size: u32,
    /// Caller/source/snapshot-bound continuation from the preceding response.
    pub page_token: Option<String>,
}

impl ReadSessionTranscriptRequest {
    /// Validate before any canonical message payload is read.
    pub fn validate(&self) -> ApiResult<()> {
        crate::validation::token(&self.source_run_id, "source_run_id", 128)?;
        if self.page_size > 32 {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "page_size",
                "page_size must be at most 32",
            ));
        }
        if self
            .page_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > MAX_TRANSCRIPT_CURSOR_BYTES)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "page_token",
                "a bounded nonempty transcript cursor is required",
            ));
        }
        Ok(())
    }

    /// Positive bounded page size for the canonical reader.
    pub fn bounded_page_size(&self) -> usize {
        if self.page_size == 0 {
            32
        } else {
            self.page_size.min(32) as usize
        }
    }
}

/// Explicit fallback for older or custom backends without canonical export support.
pub fn session_transcript_unavailable() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "canonical conversation export is unavailable on this backend",
    )
}
