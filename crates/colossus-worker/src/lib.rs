//! Authenticated local IPC for the single-writer Colossus worker.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    AgentRunOutcome, AgentRunResult, ApprovalProof, ApprovalReviewNotice, AutomaticApprovalNotice,
    DecisionPriority, DecisionStatus, EffectRequest, GoalStatus, IntegrationAuth, MemoryScope,
    MemoryStatus, PlanStatus, PlanStep, PolicyDecision, ResearchDepth, ResearchSourceKind,
    RiskReviewFallbackNotice, RunEventEnvelope, SubagentStatus, TaskStatus, TerminalPreferences,
    UserPromptRequest, UserPromptResponse, WorkflowScheduleMisfirePolicy,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_runtime::{
    CredentialResolver, EnvironmentCredentialResolver, Runtime, RuntimeConfig, RuntimeError,
    RuntimeOpenOptions,
};
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;

const PROTOCOL_VERSION: u16 = 5;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLOCK_SKEW_MS: i128 = 30_000;
const REPLAY_WINDOW: usize = 4_096;
#[cfg(not(windows))]
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(windows)]
// A missing pipe is retried briefly by the platform connector, while a pipe
// that is known to be busy receives a longer load-shedding window. Keep the
// outer bound above both so the connector can preserve that distinction.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(65);
#[cfg(not(windows))]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(windows)]
// Hosted Windows debug builds can leave a connected pipe queued behind
// synchronous durable-store work. A connected endpoint is known to be live,
// so wait within the same bound used for Windows pipe saturation.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
type HmacSha256 = Hmac<Sha256>;

mod authentication;
mod authentication_key;
mod client;
mod dispatch;
mod frames;
mod handshake;
mod interactive;
mod observers;
mod operation_names;
mod operations;
mod platform;
mod public_api;
mod public_credentials;
mod server;

pub use authentication_key::WorkerAuthenticationKey;
pub use client::{WorkerClient, WorkerPromptHandler};
pub use frames::{WorkerApprovalMode, WorkerPrompt, WorkerPromptKind};
pub use operations::{WorkerError, WorkerOperation};
pub use public_api::{PublicApiDeploymentMode, PublicApiHostOptions, PublicApiReadyMetadata};
pub use public_credentials::{
    ApplicationGrant, IssuedCredential, PublicApiAuthenticationKey, PublicApiCredentialError,
    PublicApiCredentialManager, PublicApiRotationSourceError,
};
pub use server::WorkerServer;

use authentication::*;
#[cfg(test)]
use client::handshake_timeout_error;
use dispatch::*;
use frames::*;
use handshake::*;
use interactive::*;
use observers::*;
use operation_names::*;
use operations::*;

#[cfg(test)]
mod tests;
