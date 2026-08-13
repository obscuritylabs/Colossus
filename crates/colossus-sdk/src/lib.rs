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
#[cfg(all(feature = "sidecar", target_os = "macos"))]
mod macos_code_identity;
#[cfg(all(feature = "sidecar", target_os = "macos"))]
mod macos_verified_process;
#[cfg(feature = "daemon")]
mod native_daemon;
#[cfg(all(feature = "sidecar", unix))]
mod native_sidecar;
#[cfg(all(feature = "sidecar", windows))]
#[path = "native_sidecar_windows.rs"]
mod native_sidecar;
#[cfg(all(feature = "sidecar", not(any(unix, windows))))]
#[path = "native_sidecar_unsupported.rs"]
mod native_sidecar;
mod secret;
#[cfg(feature = "sidecar")]
mod sidecar;
mod stream;
mod types;

#[cfg(feature = "embedded")]
pub use backend::ContextBoundAgentRunClient;
pub use backend::{AgentRunClient, ArtifactClient, Backend, BackendKind};
pub use client::Colossus;
pub use colossus_api::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, ApiScope, FieldViolation, IdempotencyKey,
    PLAN_CONTINUATION_CAPABILITY, scopes,
};
#[cfg(all(feature = "sidecar", target_os = "macos"))]
pub use colossus_darwin_process::{
    DarwinChild as MacosSuspendedChild, SpawnedTty as MacosSuspendedTty,
};
pub use colossus_sidecar_protocol::ManagedExecutionBoundary;
pub use config::{
    ApiMajor, AppPrivateInstanceDir, InstanceId, MacosCodeSigningRequirement, Sha256Digest,
    TlsFingerprint, VerifiedExecutable,
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
#[cfg(all(feature = "sidecar", target_os = "macos"))]
pub use macos_code_identity::MacosCodeIdentity;
#[cfg(all(feature = "sidecar", target_os = "macos"))]
pub use macos_verified_process::{
    spawn_suspended_tty as spawn_suspended_macos_tty,
    validate_suspended_process as validate_suspended_macos_process,
};
#[cfg(feature = "daemon")]
pub use native_daemon::NativeDaemonLifecycle;
#[cfg(feature = "sidecar")]
pub use native_sidecar::NativeSidecarLifecycle;
#[cfg(all(feature = "sidecar", target_os = "macos"))]
pub use native_sidecar::verify_macos_executable_identity;
pub use secret::{CredentialProvider, Secret};
#[cfg(feature = "sidecar")]
pub use sidecar::{
    MANAGED_CONFIG_FILENAME, ManagedAccessProfile, ManagedChatCompletionsOutputTokenParameter,
    ManagedModelCapabilities, ManagedModelConfig, ManagedProviderConfig, ManagedProviderKind,
    ManagedReasoningEffort, ManagedRuntimeConfig, NativeSidecarFailure, NativeSidecarStatus,
    REMOTE_PROVIDER_TIMEOUT_MS, SidecarApplicationGrant, SidecarApprovalBrokerGrant,
    SidecarBootstrapConfig, SidecarHostCredential, SidecarLifecycle, SidecarOptions,
    WorkspaceIdentity, default_managed_provider_timeout_ms, validate_managed_model_identifier,
    validate_managed_provider_base_url,
};
pub use stream::RunUpdates;
pub use types::{
    ApprovalInteraction, ApprovalRisk, ArtifactPurpose, ArtifactReference, ArtifactState,
    CancelRunRequest, CancelRunResponse, CreateRunRequest, CreateRunResponse, DownloadedArtifact,
    GetRunRequest, GetRunResponse, InputContentPart, Interaction, InteractionAnswer,
    InteractionContent, InteractionKind, InteractionStatus, ListRunsRequest, ListRunsResponse,
    MessageContentPart, MessageRole, OutcomeCertainty, PageRequest, PageResponse,
    PlanExecutionStrategy, PlanRunAction, PlanStatus, PromptAnswer, PromptChoice,
    RespondInteractionRequest, RespondInteractionResponse, Run, RunCancellation, RunFailure,
    RunMode, RunResult, RunStatus, RunTerminal, RunUpdate, RunUpdateKind, RunUpdateStream,
    ServerCapabilities, SessionMessage, TokenUsage, ToolActivity, ToolActivityState,
    UploadArtifactRequest, UserPromptInteraction, WatchRunRequest,
};

#[cfg(test)]
mod tests;
