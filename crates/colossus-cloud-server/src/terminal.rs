//! Human-only terminal admission and bounded, connection-fenced online transport.
mod browser;
mod hub;
pub(crate) use browser::router;
pub(crate) use hub::{TerminalHub, dispatch_local, is_mutation};

use colossus_sdk::{ApiError, ApiErrorCode, ApiErrorReason, ApiOutcomeCertainty};

pub(crate) fn unavailable(mutation: bool) -> ApiError {
    ApiError {
        code: if mutation { ApiErrorCode::OutcomeUnknown } else { ApiErrorCode::Unavailable },
        reason: if mutation { ApiErrorReason::OutcomeUnknown } else { ApiErrorReason::StorageFailure },
        message: if mutation { "The terminal response was lost. Reattach and inspect the shell; input is never retried automatically." } else { "The terminal connection is unavailable. Reconnect to the same terminal." }.into(),
        correlation_id: None,
        retryable: !mutation,
        outcome: if mutation { ApiOutcomeCertainty::Unknown } else { ApiOutcomeCertainty::Known },
        violations: Vec::new(),
    }
}
