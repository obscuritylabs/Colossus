//! Transport-neutral public application API for Colossus.
//!
//! This crate owns public resource shapes and application-facing ports. Transports
//! authenticate callers and construct [`CallerContext`] values; callers never submit
//! their own actor identity.

#![allow(clippy::missing_errors_doc)]

mod error;
mod identity;
mod repository;
mod runs;
mod validation;

pub use error::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, FieldViolation, OutcomeCertainty,
};
pub use identity::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, IdempotencyKey, RequestId,
    scopes,
};
pub use repository::{EventSourcedRunRepository, RunRepository};
pub use runs::{
    AgentRunApi, ApprovalRisk, CancelRunRequest, ContentPart, CreateRunRequest, CreateRunResponse,
    GetRunRequest, Idempotent, Interaction, InteractionKind, InteractionResponse,
    InteractionStatus, ListRunsRequest, ListRunsResponse, NewRun, ReleasedArtifactPurpose,
    ReleasedArtifactReference, ReleasedArtifactState, ReleasedContentPart, ReleasedMessageRole,
    ReleasedSessionMessage, RespondInteractionRequest, Run, RunCancellation, RunExecutionRequest,
    RunExecutor, RunFailure, RunMode, RunNotice, RunResult, RunStatus, RunUpdate, RunUpdateKind,
    RunUpdateStream, TokenUsage, ToolActivity, ToolActivityState, WatchRunRequest,
    validate_public_approval_display,
};

#[cfg(test)]
mod tests;
