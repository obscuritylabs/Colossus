//! Thin terminal interface for the Rust runtime.

mod tui_host;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::{Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
use colossus_access::AccessProfile;
use colossus_contracts::{
    ApprovalProof, DecisionPriority, DecisionStatus, EffectRequest, GoalStatus, IntegrationAuth,
    MemoryScope, MemoryStatus, PlanStatus, PlanStep, PolicyDecision, ProviderEvent, ResearchDepth,
    ResearchSourceKind, RunEvent, RunEventEnvelope, SessionSummary, SubagentStatus, TaskStatus,
    ToolCall, UserPromptRequest, UserPromptResponse, WorkflowScheduleMisfirePolicy,
};
use colossus_policy::{AllowApproval, DenyApproval};
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    EventDisplayMode, PresentationBlock, PresentationDocument, PresentationTable, SemanticRenderer,
    StreamDisplayMode, TerminalDocumentRenderer, TerminalPalette, TerminalPreferences,
    ThemeLibrary, ThemeName, TranscriptDensity, document_from_json,
};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions};
use colossus_tui::{BootstrapRequest, ScreenMode, TuiOptions, run_tui};
use colossus_worker::{WorkerApprovalMode, WorkerClient, WorkerOperation, WorkerServer};
use serde_json::{Value, json};
#[cfg(windows)]
use std::fmt;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, BufRead, IsTerminal as _, Write as _},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
mod cli;
mod commands;
mod configuration;
mod desktop_tui_auth;
mod entrypoint;
mod extension_args;
mod line_runner;
mod memory_research_args;
mod output;
mod pickers;
mod presentation_commands;
mod public_api_admin;
mod service_args;
mod terminal_io;
mod webhooks;
mod work_args;
mod worker_dispatch;
mod worker_shell;
mod workflow_args;
mod workflow_commands;

use cli::*;
use commands::*;
use configuration::*;
use desktop_tui_auth::*;
use entrypoint::*;
use extension_args::*;
use line_runner::*;
use memory_research_args::*;
use output::*;
use pickers::*;
use presentation_commands::*;
use public_api_admin::*;
use service_args::*;
use terminal_io::*;
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
