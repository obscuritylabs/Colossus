//! Transport-neutral public application API for Colossus.
//!
//! This crate owns public resource shapes and application-facing ports. Transports
//! authenticate callers and construct [`CallerContext`] values; callers never submit
//! their own actor identity.

#![allow(clippy::missing_errors_doc)]

mod agent_tasks;
mod artifacts;
mod communication;
pub use agent_tasks::*;
pub use communication::*;
mod error;
mod identity;
mod plugins;
mod posture;
mod process_sessions;
mod repository;
mod runs;
mod sharing;
pub use posture::*;
pub use sharing::{
    ListVisibleRunsResponse, SetWorkspaceSharingRequest, VisibleRun, WorkspaceSharingState,
};
mod validation;
mod workflows;
pub use workflows::*;

pub use artifacts::{
    ARTIFACT_CHUNK_BYTES, ArtifactApi, ArtifactChunk, ArtifactDownload, ArtifactPurpose,
    ArtifactReference, ArtifactState, ArtifactUploadReservation, CreateArtifactUploadRequest,
    EventSourcedArtifactApi, MAX_ARTIFACT_BYTES,
};
pub use colossus_contracts::CommandApprovalContext;
pub use error::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, FieldViolation, OutcomeCertainty,
};
pub use identity::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, IdempotencyKey, RequestId,
    scopes,
};
pub use plugins::*;
pub use process_sessions::*;
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
    validate_public_approval_display, validate_public_command_context,
};

/// Safe automatic provider recovery progress.
pub use colossus_contracts::{ProviderRetry, ProviderRetryState};

#[cfg(test)]
mod tests;
