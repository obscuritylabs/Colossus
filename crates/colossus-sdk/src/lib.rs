//! Stable Rust client surface and secure process lifecycle abstractions for Colossus.
//!
//! Applications use the same typed run API with a shared daemon, an isolated sidecar,
//! or an embedded runtime. Lifecycle adapters remain responsible for platform-specific
//! transport, process, credential-store, and writer-lease mechanics.

#![allow(clippy::missing_errors_doc)]

mod backend;
mod client;
mod config;
#[cfg(feature = "daemon")]
mod daemon;
#[cfg(feature = "embedded")]
mod embedded;
#[cfg(feature = "embedded")]
mod embedded_projection;
mod error;
#[cfg(feature = "daemon")]
mod grpc;
#[cfg(feature = "keyring")]
mod keyring_provider;
#[cfg(feature = "daemon")]
mod native_daemon;
mod secret;
#[cfg(feature = "sidecar")]
mod sidecar;
mod stream;
mod types;

#[cfg(feature = "embedded")]
pub use backend::ContextBoundAgentRunClient;
pub use backend::{AgentRunClient, Backend, BackendKind};
pub use client::Colossus;
pub use colossus_api::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, FieldViolation, IdempotencyKey,
};
pub use config::{
    ApiMajor, AppPrivateInstanceDir, InstanceId, Sha256Digest, TlsFingerprint, VerifiedExecutable,
};
#[cfg(feature = "daemon")]
pub use daemon::{
    DaemonConnectOptions, DaemonDescriptor, DaemonDiscovery, DaemonLaunchGuard,
    DaemonLaunchOptions, DaemonLifecycle,
};
#[cfg(feature = "embedded")]
pub use embedded::{EmbeddedLifecycle, EmbeddedOptions};
pub use error::{SdkError, SdkResult};
#[cfg(feature = "daemon")]
pub use grpc::{GrpcBackend, GrpcConnectOptions};
#[cfg(feature = "keyring")]
pub use keyring_provider::KeyringCredentialProvider;
#[cfg(feature = "daemon")]
pub use native_daemon::NativeDaemonLifecycle;
pub use secret::{CredentialProvider, Secret};
#[cfg(feature = "sidecar")]
pub use sidecar::{SidecarLifecycle, SidecarOptions};
pub use stream::RunUpdates;
pub use types::{
    ApprovalInteraction, ApprovalRisk, ArtifactPurpose, ArtifactReference, ArtifactState,
    CancelRunRequest, CancelRunResponse, CreateRunRequest, CreateRunResponse, GetRunRequest,
    GetRunResponse, InputContentPart, Interaction, InteractionAnswer, InteractionContent,
    InteractionKind, InteractionStatus, ListRunsRequest, ListRunsResponse, MessageContentPart,
    MessageRole, OutcomeCertainty, PageRequest, PageResponse, PromptAnswer, PromptChoice,
    RespondInteractionRequest, RespondInteractionResponse, Run, RunCancellation, RunFailure,
    RunMode, RunResult, RunStatus, RunTerminal, RunUpdate, RunUpdateKind, RunUpdateStream,
    SessionMessage, TokenUsage, ToolActivity, ToolActivityState, UserPromptInteraction,
    WatchRunRequest,
};

#[cfg(test)]
mod tests;
