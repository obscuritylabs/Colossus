use crate::{ModelProviderError, StoreError};
use std::fmt;
use thiserror::Error;

/// Independent bounds enforced before a model request can be dispatched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextBudgetResource {
    /// Estimated input tokens after reserving output and safety headroom.
    Tokens,
    /// Serialized request bytes, including projected tool-argument escaping.
    RequestBytes,
}

impl fmt::Display for ContextBudgetResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Tokens => "estimated tokens",
            Self::RequestBytes => "bytes",
        })
    }
}

/// Model-visible context component that could not fit its bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextBudgetScope {
    /// Prepared context with automatic compaction disabled or already applied.
    PreparedRequest,
    /// Newest logical turn, protected from automatic summarization.
    NewestTurn,
    /// Preserved recent messages plus their fixed instructions and bindings.
    PreservedMessages,
    /// Minimum snapshot metadata plus preserved messages.
    SnapshotEnvelope,
    /// Result after bounding a newly compacted snapshot.
    CompactedRequest,
}

impl fmt::Display for ContextBudgetScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PreparedRequest => "the prepared model request",
            Self::NewestTurn => "the newest logical turn",
            Self::PreservedMessages => "preserved recent messages",
            Self::SnapshotEnvelope => "preserved recent messages plus snapshot metadata",
            Self::CompactedRequest => "the compacted model request",
        })
    }
}

/// Safe numeric evidence of a context-size failure; contains no model input.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error(
    "{scope} requires {required} {resource}, exceeding the {limit} {resource} input budget; shorten the message, reduce tool arguments or output, or start a new session"
)]
pub struct ContextBudgetExceeded {
    /// Bound that was exceeded.
    pub resource: ContextBudgetResource,
    /// Required amount, including fixed request or snapshot overhead.
    pub required: u64,
    /// Effective input ceiling for that resource.
    pub limit: u64,
    /// Component that could not be compacted to fit.
    pub scope: ContextBudgetScope,
}

impl ContextBudgetExceeded {
    /// Construct token evidence without retaining model identifiers or request text.
    pub const fn tokens(required: u64, limit: u64, scope: ContextBudgetScope) -> Self {
        Self {
            resource: ContextBudgetResource::Tokens,
            required,
            limit,
            scope,
        }
    }

    /// Construct byte evidence with saturating conversion on wider hosts.
    pub fn request_bytes(required: usize, limit: usize, scope: ContextBudgetScope) -> Self {
        Self {
            resource: ContextBudgetResource::RequestBytes,
            required: u64::try_from(required).unwrap_or(u64::MAX),
            limit: u64::try_from(limit).unwrap_or(u64::MAX),
            scope,
        }
    }

    /// Stable reason carried to public run failures and telemetry.
    pub const fn code(&self) -> &'static str {
        match self.resource {
            ContextBudgetResource::Tokens => "context.token_limit_exceeded",
            ContextBudgetResource::RequestBytes => "context.request_too_large",
        }
    }
}

/// Context preparation or snapshot lifecycle failure.
#[derive(Debug, Error)]
pub enum ContextError {
    /// Context configuration or request cannot satisfy the safety contract.
    #[error("context configuration failed: {0}")]
    Configuration(String),
    /// The model-visible request could not fit its token or byte budget.
    #[error(transparent)]
    BudgetExceeded(#[from] ContextBudgetExceeded),
    /// Canonical snapshot persistence or session reconstruction failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Optional model summarization failed before deterministic fallback could run.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
}
