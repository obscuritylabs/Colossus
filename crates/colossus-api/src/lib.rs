//! Transport-neutral public application API for Colossus.
//!
//! This crate owns public resource shapes and application-facing ports. Transports
//! authenticate callers and construct [`CallerContext`] values; callers never submit
//! their own actor identity.

#![allow(clippy::missing_errors_doc)]

mod artifacts;
mod error;
mod identity;
mod plugins;
mod repository;
mod runs;
mod validation;

pub use artifacts::{
    ARTIFACT_CHUNK_BYTES, ArtifactApi, ArtifactChunk, ArtifactDownload, ArtifactPurpose,
    ArtifactReference, ArtifactState, ArtifactUploadReservation, CreateArtifactUploadRequest,
    EventSourcedArtifactApi, MAX_ARTIFACT_BYTES,
};
pub use error::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, FieldViolation, OutcomeCertainty,
};
pub use identity::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, IdempotencyKey, RequestId,
    scopes,
};
pub use plugins::*;
pub use repository::{EventSourcedRunRepository, RunRepository};
pub use runs::{
    AgentRunApi, ApprovalRisk, ArchiveThreadRequest, CancelRunRequest, ContentPart,
    CreateRunRequest, CreateRunResponse, GetRunRequest, Idempotent, Interaction, InteractionKind,
    InteractionResponse, InteractionStatus, ListRunsRequest, ListRunsResponse,
    ListSessionActivityRequest, ListSessionActivityResponse, NewRun, PLAN_CONTINUATION_CAPABILITY,
    PlanExecutionStrategy, PlanRunAction, PlanStatus, ReleasedArtifactPurpose,
    ReleasedArtifactReference, ReleasedArtifactState, ReleasedContentPart, ReleasedMessageRole,
    ReleasedSessionMessage, ResearchDepth, ResearchSourceKind, RespondInteractionRequest,
    RestoreThreadRequest, Run, RunBranch, RunBranchContextMode, RunCancellation,
    RunExecutionRequest, RunExecutor, RunFailure, RunMode, RunNotice, RunResult, RunStatus,
    RunUpdate, RunUpdateKind, RunUpdateStream, SESSION_ACTIVITY_CAPABILITY, SessionActivity,
    SessionActivityContent, SessionActivityKind, SessionActivityLane, SessionActivityStatus,
    ThreadLifecycle, TokenUsage, ToolActivity, ToolActivityState, WatchRunRequest,
    validate_public_approval_display,
};

#[cfg(test)]
mod tests;
