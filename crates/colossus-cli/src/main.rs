//! Thin terminal interface for the Rust runtime.

mod tui_host;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::{Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
use colossus_access::AccessProfile;
use colossus_codex_auth::CodexCliAction;
use colossus_contracts::{
    AgentRunMode, AgentRunOutcome, ApprovalProof, ApprovalReviewNotice, AutomaticApprovalNotice,
    DecisionPriority, DecisionStatus, EffectRequest, GoalRunOutcome, GoalStatus, IntegrationAuth,
    MemoryScope, MemoryStatus, PlanDraftTarget, PlanExecutionOutcome, PlanExecutionStrategy,
    PlanRecord, PlanStatus, PlanStep, PolicyDecision, ProviderEvent, ResearchDepth,
    ResearchSourceKind, RiskReviewFallbackNotice, RunEvent, RunEventEnvelope,
    SecurityPostureReport, SessionSummary, SubagentStatus, TaskStatus, ToolCall, UserPromptRequest,
    UserPromptResponse, WorkflowScheduleMisfirePolicy,
};
#[cfg(test)]
use colossus_contracts::{SecurityPostureFinding, SecurityPostureSeverity};
use colossus_home::{
    ColossusHome, ConfinedRoot, HomeError, HomeSurface, detect_workspace_identity,
};
use colossus_policy::{AllowApproval, DenyApproval};
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    EventDisplayMode, PresentationBlock, PresentationDocument, PresentationTable, PresentationTone,
    SemanticRenderer, StreamDisplayMode, TerminalDocumentRenderer, TerminalPalette,
    TerminalPreferences, ThemeLibrary, ThemeName, TranscriptDensity, automatic_approval_document,
    document_from_json, risk_review_fallback_document,
};
use colossus_runtime::{
    Runtime, RuntimeConfig, RuntimeOpenOptions, StorageLocation, WorkspaceIdentityToken,
};
use colossus_tui::{BackgroundNoticeProvider, BootstrapRequest, ScreenMode, TuiOptions, run_tui};
use colossus_update::{
    InstallerKind, UpdateApplyFailure, UpdateApplyReport, UpdateApplyStatus, UpdateCheckReport,
    UpdateCheckStatus, UpdateChecker, UpdateRefusalReason, UpdateService, UpdateUnavailableReason,
};
use colossus_worker::{
    InteractiveWorkerRequest, WorkerApprovalMode, WorkerClient, WorkerError, WorkerOperation,
    WorkerPrompt, WorkerPromptHandler, WorkerPromptKind, WorkerServer,
};
use serde_json::{Value, json};
#[cfg(windows)]
use std::fmt;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, BufRead, IsTerminal as _, Read as _, Write as _},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;
mod artifact_args;
mod artifact_commands;
mod cli;
mod codex_commands;
mod commands;
mod configuration;
mod desktop_tui_auth;
mod entrypoint;
mod extension_args;
mod line_plan;
mod line_runner;
mod mcp_auth;
mod memory_research_args;
mod output;
mod pickers;
mod presentation_commands;
mod public_api_admin;
mod service_args;
mod terminal_io;
mod update_commands;
mod webhooks;
mod work_args;
mod worker_dispatch;
mod worker_shell;
mod workflow_args;
mod workflow_commands;

use artifact_args::*;
use artifact_commands::*;
use cli::*;
use codex_commands::*;
use commands::*;
use configuration::*;
use desktop_tui_auth::*;
use entrypoint::*;
use extension_args::*;
use line_plan::*;
use line_runner::*;
use mcp_auth::*;
use memory_research_args::*;
use output::*;
use pickers::*;
use presentation_commands::*;
use public_api_admin::*;
use service_args::*;
use terminal_io::*;
use update_commands::*;
use webhooks::*;
use work_args::*;
use worker_dispatch::*;
use worker_shell::*;
use workflow_args::*;
use workflow_commands::*;

fn sandbox_helper_requested(mut arguments: impl Iterator<Item = std::ffi::OsString>) -> bool {
    let _binary = arguments.next();
    arguments
        .next()
        .is_some_and(|argument| argument == "__sandbox-helper")
}

fn sandbox_protection_probe_requested(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> bool {
    let _binary = arguments.next();
    arguments
        .next()
        .is_some_and(|argument| argument == "__sandbox-protection-probe")
}

#[cfg(not(windows))]
fn main() -> Result<(), Box<dyn Error>> {
    if sandbox_helper_requested(std::env::args_os()) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    if sandbox_protection_probe_requested(std::env::args_os()) {
        colossus_sandbox::run_native_protection_probe()?;
        return Ok(());
    }
    runtime_main()
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn Error>> {
    if sandbox_helper_requested(std::env::args_os()) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    if sandbox_protection_probe_requested(std::env::args_os()) {
        colossus_sandbox::run_native_protection_probe()?;
        return Ok(());
    }
    // MSVC executables reserve a smaller main-thread stack than the other supported
    // platforms. Debug runtime composition can exceed that reserve before a command is
    // dispatched, so keep the actual async entrypoint on one explicitly bounded thread.
    let outcome = std::thread::Builder::new()
        .name("colossus-main".into())
        .stack_size(WINDOWS_MAIN_STACK_BYTES)
        .spawn(|| runtime_main().map_err(|error| format!("{error:?}")))?
        .join()
        .map_err(|_| WindowsMainError("Colossus main thread panicked".into()))?;
    outcome.map_err(|error| Box::new(WindowsMainError(error)) as Box<dyn Error>)
}

#[cfg(test)]
mod tests;
