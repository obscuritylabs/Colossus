//! Embedded-runtime adapter for the backend-neutral Colossus TUI host contract.

use super::{ApprovalMode, TERMINAL_HISTORY_CAPACITY, terminal_completion_values};
use async_trait::async_trait;
use colossus_contracts::{
    ApprovalProof, ApprovalReviewNotice, AutomaticApprovalNotice, ContextStatus, EffectRequest,
    MemoryStatus, PolicyDecision, ProviderRoute, ResearchDepth, ResearchSourceKind,
    RiskReviewFallbackNotice, RunEventEnvelope, SessionMessagePage, SessionSummary,
    TerminalPreferences, UserPromptRequest, UserPromptResponse, WorkStateSnapshot,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    PresentationBlock, PresentationDocument, PresentationTone, ThemeLibrary, ThemeName,
    automatic_approval_document, context_status_document, document_from_json,
    risk_review_fallback_document, work_state_document,
};
use colossus_runtime::Runtime;
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostRunResult, InteractiveHost,
    InteractivePrompt, InteractiveRunRequest, InteractiveSnapshot, PromptResponse, RuntimeCommand,
};
use colossus_worker::{
    WorkerClient, WorkerError, WorkerOperation, WorkerPrompt, WorkerPromptHandler, WorkerPromptKind,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

mod common;
mod embedded;
mod worker;

pub(crate) use common::{TuiApprovalProvider, TuiPromptRouter, TuiUserPromptProvider};
pub(crate) use embedded::EmbeddedInteractiveHost;
pub(crate) use worker::WorkerInteractiveHost;

use common::*;
use worker::parse_toggle;

#[cfg(test)]
mod tests;
