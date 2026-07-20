use crate::RequestId;
use colossus_ports::StoreError;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

/// Transport-neutral public error category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// Request fields are invalid.
    InvalidArgument,
    /// Authentication is absent or invalid.
    Unauthenticated,
    /// The authenticated caller lacks authority.
    PermissionDenied,
    /// The requested resource does not exist.
    NotFound,
    /// A resource already exists.
    AlreadyExists,
    /// Optimistic concurrency or idempotency semantics conflict.
    Conflict,
    /// Runtime state prevents this operation.
    FailedPrecondition,
    /// A configured bound was exceeded.
    ResourceExhausted,
    /// The operation was cooperatively cancelled.
    Cancelled,
    /// A transient dependency is unavailable.
    Unavailable,
    /// The server failed without exposing private implementation detail.
    Internal,
    /// An external mutation may have occurred.
    OutcomeUnknown,
}

/// Stable machine-readable Colossus error reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorReason {
    /// One or more request fields are invalid.
    InvalidArgument,
    /// Authentication is required.
    AuthenticationRequired,
    /// Authentication failed.
    AuthenticationFailed,
    /// An exact API scope is absent.
    ScopeDenied,
    /// The requested model role is outside the caller ceiling.
    RoleDenied,
    /// The requested tool is outside the caller ceiling.
    ToolDenied,
    /// A run does not exist or is not visible to the caller.
    RunNotFound,
    /// An idempotency key was reused for a different logical request.
    IdempotencyKeyReused,
    /// Optimistic concurrency failed.
    ConcurrentModification,
    /// A requested run state transition is invalid.
    InvalidRunTransition,
    /// An interaction is absent, expired, already resolved, or belongs to another caller.
    InteractionUnavailable,
    /// A configured concurrency or admission-rate bound was reached.
    CapacityExceeded,
    /// Journal verification placed the runtime in recovery mode.
    RecoveryMode,
    /// Durable storage is unavailable.
    StorageFailure,
    /// An effect or external mutation has an uncertain outcome.
    OutcomeUnknown,
    /// An internal invariant failed.
    InternalInvariant,
}

/// Whether an effectful outcome is known.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCertainty {
    /// The server knows whether the requested mutation occurred.
    Known,
    /// The mutation may have occurred and must not be retried automatically.
    Unknown,
}

/// Safe request validation detail.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldViolation {
    /// Public request field path.
    pub field: String,
    /// Bounded user-safe explanation.
    pub description: String,
}

/// Stable, bounded error returned through every public API backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    /// Broad transport-neutral error category.
    pub code: ApiErrorCode,
    /// Stable Colossus reason.
    pub reason: ApiErrorReason,
    /// Bounded user-safe message without secrets or adapter detail.
    pub message: String,
    /// Server-generated request correlation identifier.
    pub correlation_id: Option<RequestId>,
    /// Whether retry is safe without changing request semantics.
    pub retryable: bool,
    /// Explicit effect outcome certainty.
    pub outcome: OutcomeCertainty,
    /// Structured invalid-field details.
    pub violations: Vec<FieldViolation>,
}

impl ApiError {
    /// Construct one invalid-field error.
    pub fn invalid(
        reason: ApiErrorReason,
        field: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let description = description.into();
        Self {
            code: ApiErrorCode::InvalidArgument,
            reason,
            message: "the request is invalid".into(),
            correlation_id: None,
            retryable: false,
            outcome: OutcomeCertainty::Known,
            violations: vec![FieldViolation {
                field: field.into(),
                description,
            }],
        }
    }

    /// Construct one authorization denial.
    pub fn permission_denied(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::PermissionDenied, reason, message, false)
    }

    /// Construct one not-found error.
    pub fn not_found(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::NotFound, reason, message, false)
    }

    /// Construct one conflict error.
    pub fn conflict(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::Conflict, reason, message, false)
    }

    /// Construct one failed precondition.
    pub fn failed_precondition(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::FailedPrecondition, reason, message, false)
    }

    /// Construct a retryable configured-capacity denial.
    pub fn resource_exhausted(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::ResourceExhausted, reason, message, true)
    }

    /// Construct a permanent configured-work-budget denial.
    pub fn bounded_resource_exhausted(reason: ApiErrorReason, message: impl Into<String>) -> Self {
        Self::known(ApiErrorCode::ResourceExhausted, reason, message, false)
    }

    /// Attach a server-generated correlation identifier.
    pub fn with_correlation_id(mut self, request_id: RequestId) -> Self {
        self.correlation_id = Some(request_id);
        self
    }

    /// Map a storage failure without returning private adapter messages.
    pub fn from_store(error: &StoreError, request_id: &RequestId) -> Self {
        let mapped = match error {
            StoreError::Conflict { .. } => Self::known(
                ApiErrorCode::Conflict,
                ApiErrorReason::ConcurrentModification,
                "the resource changed concurrently",
                true,
            ),
            StoreError::NotFound(_) => Self::known(
                ApiErrorCode::NotFound,
                ApiErrorReason::RunNotFound,
                "the requested resource was not found",
                false,
            ),
            StoreError::Verification(_) | StoreError::RecoveryMode => Self::known(
                ApiErrorCode::FailedPrecondition,
                ApiErrorReason::RecoveryMode,
                "the runtime is in verified read-only recovery mode",
                false,
            ),
            StoreError::OutcomeUnknown(_) => Self {
                code: ApiErrorCode::OutcomeUnknown,
                reason: ApiErrorReason::OutcomeUnknown,
                message: "the external outcome is unknown; do not retry automatically".into(),
                correlation_id: None,
                retryable: false,
                outcome: OutcomeCertainty::Unknown,
                violations: Vec::new(),
            },
            StoreError::KeyUnavailable(_) | StoreError::Adapter(_) => Self::known(
                ApiErrorCode::Internal,
                ApiErrorReason::StorageFailure,
                "durable storage is unavailable",
                false,
            ),
        };
        mapped.with_correlation_id(request_id.clone())
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self::known(
            ApiErrorCode::Internal,
            ApiErrorReason::InternalInvariant,
            message,
            false,
        )
    }

    fn known(
        code: ApiErrorCode,
        reason: ApiErrorReason,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            reason,
            message: message.into(),
            correlation_id: None,
            retryable,
            outcome: OutcomeCertainty::Known,
            violations: Vec::new(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({:?})", self.message, self.reason)
    }
}

impl Error for ApiError {}

/// Result returned by public application API operations.
pub type ApiResult<T> = Result<T, ApiError>;
