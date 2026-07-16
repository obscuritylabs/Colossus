//! Thin terminal interface for the Rust runtime.

mod tui_host;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::{Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
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
use colossus_runtime::{Runtime, RuntimeConfig};
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

#[derive(Parser)]
#[command(
    name = "colossus",
    version,
    about = "Auditable Colossus workflow runtime"
)]
struct Cli {
    /// Fresh Rust YAML configuration path.
    #[arg(long, default_value = ".colossus/config.yaml")]
    config: PathBuf,
    /// Handling for policy decisions that require operator approval.
    #[arg(long, value_enum)]
    approval_mode: Option<ApprovalMode>,
    /// Output format for structured commands. Auto is human on a terminal and JSON when piped.
    #[arg(long, value_enum, default_value_t = OutputMode::Auto)]
    output: OutputMode,
    /// Preserve terminal scrollback by using Ratatui's inline viewport.
    #[arg(long, global = true)]
    no_alt_screen: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ApprovalMode {
    /// Fail closed without prompting (default outside the interactive TUI).
    Deny,
    /// Prompt on the terminal for every approval obligation.
    Ask,
    /// Auto-approve only low-risk shell effects after model-assisted review.
    RiskAuto,
    /// Grant approval obligations automatically without expanding policy permissions.
    FullAccess,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputMode {
    /// Render human tables and cards on terminals, JSON when redirected.
    #[default]
    Auto,
    /// Always render human tables, cards, and Markdown.
    Human,
    /// Always emit stable machine-readable JSON.
    Json,
}

static OUTPUT_MODE: AtomicU8 = AtomicU8::new(0);
static TERMINAL_PREFERENCES: OnceLock<Mutex<TerminalPreferences>> = OnceLock::new();

const TERMINAL_HISTORY_CAPACITY: usize = 1_000;
const TERMINAL_COMPLETIONS: &[&str] = &[
    "/help",
    "/tui prefs",
    "/tui save",
    "/tui reset",
    "/theme",
    "/theme list",
    "/theme preview",
    "/theme validate",
    "/theme scaffold",
    "/theme reset",
    "/stream on",
    "/stream raw",
    "/stream off",
    "/events compact",
    "/events verbose",
    "/events off",
    "/reasoning on",
    "/reasoning off",
    "/transcript comfortable",
    "/transcript compact",
    "/multiline on",
    "/multiline off",
    "/multiline toggle",
    "/trace",
    "/resume",
    "/sessions",
    "/session show",
    "/session new",
    "/session resume",
    "/work",
    "/tasks",
    "/decisions",
    "/plans",
    "/goals",
    "/goal",
    "/agents",
    "/agents drain",
    "/memories",
    "/memory search",
    "/research",
    "/research list",
    "/telemetry",
    "/telemetry metrics",
    "/skills",
    "/skill active",
    "/skill use",
    "/skill clear",
    "/skill show",
    "/skill resources",
    "/skill read",
    "/packs list",
    "/packs show",
    "/packs verify",
    "/packs install",
    "/packs enable",
    "/packs disable",
    "/packs uninstall",
    "/packs call",
    "/packs trust list",
    "/packs trust add",
    "/bundle verify",
    "/integrations",
    "/integration show",
    "/integration call",
    "/integration disconnect",
    "/mcp servers",
    "/mcp tools",
    "/mcp call",
    "/context status",
    "/context list",
    "/context compact",
    "/context restore",
    "/workflow list",
    "/workflow status",
    "/workflow schedule list",
    "/workflow schedule show",
    "/workflow schedule enable",
    "/workflow schedule disable",
    "/workflow schedule tick",
    "/workflow webhook list",
    "/workflow webhook show",
    "/workflow webhook enable",
    "/workflow webhook disable",
    "/workflow subscription list",
    "/workflow subscription show",
    "/workflow subscription enable",
    "/workflow subscription disable",
    "/workflow subscription tick",
    "/audit verify",
    "/projection status",
    "/tools",
    "/exit",
];
#[cfg(windows)]
const WINDOWS_MAIN_STACK_BYTES: usize = 8 * 1024 * 1024;

#[cfg(windows)]
struct WindowsMainError(String);

#[cfg(windows)]
impl fmt::Debug for WindowsMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(windows)]
impl fmt::Display for WindowsMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(windows)]
impl Error for WindowsMainError {}

impl ApprovalMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::RiskAuto => "risk_auto",
            Self::FullAccess => "full_access",
        }
    }
}

struct TerminalApproval {
    risk_auto: bool,
    lock: Mutex<()>,
}

struct TerminalUserPrompt {
    lock: Mutex<()>,
}

#[async_trait]
impl UserPromptProvider for TerminalUserPrompt {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| ToolError::Failed("user prompt terminal lock is poisoned".into()))?;
        let mut choices = PresentationTable::new(["#", "Choice"], "Enter a free-form answer.");
        for (index, choice) in request.choices.iter().enumerate() {
            choices.push_row([(index + 1).to_string(), choice.clone()]);
        }
        write_stderr_document(&PresentationDocument::from_block(PresentationBlock::Card {
            title: "Input needed".into(),
            tone: colossus_presentation::PresentationTone::Warning,
            body: vec![
                PresentationBlock::Markdown(request.question.clone()),
                PresentationBlock::Table(choices),
                PresentationBlock::Markdown(
                    "_The current agent turn is paused. Type an answer and press Enter; leave it blank to cancel this question._"
                        .into(),
                ),
            ],
        }))
        .map_err(|error| ToolError::Failed(error.to_string()))?;
        for _ in 0..3 {
            if request.choices.is_empty() {
                eprint!("Answer (blank cancels): ");
            } else if request.allow_free_form {
                eprint!("Choose a number or enter an answer (blank cancels): ");
            } else {
                eprint!("Choose a number (blank cancels): ");
            }
            io::stderr()
                .flush()
                .map_err(|error| ToolError::Failed(error.to_string()))?;
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .map_err(|error| ToolError::Failed(error.to_string()))?;
            let answer = answer.trim();
            if answer.is_empty() {
                return Err(ToolError::Failed("user cancelled the question".into()));
            }
            if let Ok(index) = answer.parse::<usize>()
                && let Some(choice) = index
                    .checked_sub(1)
                    .and_then(|index| request.choices.get(index))
            {
                return Ok(UserPromptResponse {
                    answer: choice.clone(),
                    selected_index: Some(index - 1),
                });
            }
            if request.allow_free_form {
                return Ok(UserPromptResponse {
                    answer: answer.into(),
                    selected_index: request.choices.iter().position(|choice| choice == answer),
                });
            }
            eprintln!("Enter one of the numbered choices.");
        }
        Err(ToolError::Failed(
            "user did not provide a valid choice after three attempts".into(),
        ))
    }
}

#[async_trait]
impl ApprovalProvider for TerminalApproval {
    fn risk_auto_enabled(&self) -> bool {
        self.risk_auto
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| PolicyError::Unavailable("approval terminal lock is poisoned".into()))?;
        let content = serde_json::to_string_pretty(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut details = vec![
            ("Action".into(), request.action.clone()),
            ("Resource".into(), request.resource.clone()),
            ("Reason".into(), decision.reason.clone()),
        ];
        if let Some(reason) = request.risk.reason.as_deref() {
            let level = request.risk.level.as_deref().unwrap_or("unavailable");
            details.push(("Risk".into(), format!("{level}: {reason}")));
        }
        write_stderr_document(&PresentationDocument::from_block(PresentationBlock::Card {
            title: "Approval required".into(),
            tone: colossus_presentation::PresentationTone::Warning,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Code {
                    language: Some("proposed content".into()),
                    content: bounded_preview(&content, 1200).into(),
                },
            ],
        }))
        .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        eprint!("Approve this effect? [y/N] ");
        io::stderr()
            .flush()
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let approved = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        drop(guard);
        if !approved {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "terminal-user".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

#[derive(Clone, Copy)]
enum StreamTarget {
    Stdout,
    Stderr,
}

struct TerminalStreamObserver {
    target: StreamTarget,
    wrote_text: bool,
    buffered_text: String,
    final_rendered: bool,
    tool_calls: BTreeMap<String, ToolCall>,
    preferences: TerminalPreferences,
    activity: Option<tokio::task::JoinHandle<()>>,
    output_lock: Arc<Mutex<()>>,
}

impl TerminalStreamObserver {
    fn new(target: StreamTarget) -> Self {
        Self {
            target,
            wrote_text: false,
            buffered_text: String::new(),
            final_rendered: false,
            tool_calls: BTreeMap::new(),
            preferences: TerminalPreferences::default(),
            activity: None,
            output_lock: Arc::new(Mutex::new(())),
        }
    }

    fn with_preferences(target: StreamTarget, preferences: TerminalPreferences) -> Self {
        Self {
            target,
            wrote_text: false,
            buffered_text: String::new(),
            final_rendered: false,
            tool_calls: BTreeMap::new(),
            preferences,
            activity: None,
            output_lock: Arc::new(Mutex::new(())),
        }
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.stop_activity()?;
        self.finish_line()?;
        let _guard = self
            .output_lock
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        match self.target {
            StreamTarget::Stdout => {
                println!("{line}");
                io::stdout().flush()
            }
            StreamTarget::Stderr => {
                eprintln!("{line}");
                io::stderr().flush()
            }
        }
    }

    fn finish_line(&mut self) -> io::Result<()> {
        self.stop_activity()?;
        if self.wrote_text {
            let _guard = self
                .output_lock
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?;
            match self.target {
                StreamTarget::Stdout => {
                    println!();
                    io::stdout().flush()?;
                }
                StreamTarget::Stderr => {
                    eprintln!();
                    io::stderr().flush()?;
                }
            }
            self.wrote_text = false;
        }
        Ok(())
    }

    fn finish_response(&mut self, fallback: &str) -> io::Result<()> {
        self.finish_line()?;
        if matches!(self.target, StreamTarget::Stdout)
            && self.preferences.stream_mode != StreamDisplayMode::Raw
            && !self.final_rendered
        {
            let output = if fallback.is_empty() {
                self.buffered_text.clone()
            } else {
                fallback.into()
            };
            self.write_markdown(&output)?;
            self.final_rendered = true;
        }
        Ok(())
    }

    fn write_markdown(&mut self, markdown: &str) -> io::Result<()> {
        let markdown = PresentationBlock::Markdown(markdown.into());
        let document = PresentationDocument::from_block(
            if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                PresentationBlock::Card {
                    title: "Colossus".into(),
                    tone: colossus_presentation::PresentationTone::Neutral,
                    body: vec![markdown],
                }
            } else {
                markdown
            },
        );
        let rendered = TerminalDocumentRenderer::new(self.preferences.clone(), terminal_width())
            .with_color(self.is_terminal())
            .render(&document);
        self.write_line(&rendered)
    }

    fn is_terminal(&self) -> bool {
        match self.target {
            StreamTarget::Stdout => io::stdout().is_terminal(),
            StreamTarget::Stderr => io::stderr().is_terminal(),
        }
    }

    fn start_activity(&mut self, line: &str, elapsed_seconds: f64) -> io::Result<()> {
        if !self.is_terminal() {
            return self.write_line(line);
        }
        self.stop_activity()?;
        let target = self.target;
        let output_lock = Arc::clone(&self.output_lock);
        let template = line.to_owned();
        let palette = TerminalPalette::for_preferences(&self.preferences);
        write_transient_line(target, &output_lock, &template, elapsed_seconds, palette)?;
        let started = std::time::Instant::now();
        self.activity = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let elapsed = elapsed_seconds + started.elapsed().as_secs_f64();
                if write_transient_line(target, &output_lock, &template, elapsed, palette).is_err()
                {
                    break;
                }
            }
        }));
        Ok(())
    }

    fn stop_activity(&mut self) -> io::Result<()> {
        let Some(activity) = self.activity.take() else {
            return Ok(());
        };
        activity.abort();
        let _guard = self
            .output_lock
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        match self.target {
            StreamTarget::Stdout => {
                print!("\r\x1b[2K");
                io::stdout().flush()
            }
            StreamTarget::Stderr => {
                eprint!("\r\x1b[2K");
                io::stderr().flush()
            }
        }
    }
}

impl Drop for TerminalStreamObserver {
    fn drop(&mut self) {
        let _ = self.stop_activity();
    }
}

fn activity_elapsed(event: &RunEvent) -> Option<f64> {
    match event {
        RunEvent::Phase {
            phase:
                colossus_contracts::RunPhase::Preparing
                | colossus_contracts::RunPhase::WaitingForModel
                | colossus_contracts::RunPhase::Responding,
            elapsed_seconds,
            ..
        } => Some(*elapsed_seconds),
        RunEvent::ToolStarted {
            call,
            elapsed_seconds,
            ..
        } if call.name != "user.ask" => Some(*elapsed_seconds),
        _ => None,
    }
}

fn activity_line_at(template: &str, elapsed_seconds: f64) -> String {
    let Some(start) = template.rfind("elapsed=") else {
        return format!("{template} elapsed={elapsed_seconds:.2}s");
    };
    let value_start = start + "elapsed=".len();
    let Some(value_end) = template[value_start..].find('s') else {
        return format!("{template} elapsed={elapsed_seconds:.2}s");
    };
    let suffix_start = value_start + value_end + 1;
    format!(
        "{}elapsed={elapsed_seconds:.2}s{}",
        &template[..start],
        &template[suffix_start..]
    )
}

fn write_transient_line(
    target: StreamTarget,
    output_lock: &Mutex<()>,
    template: &str,
    elapsed_seconds: f64,
    palette: TerminalPalette,
) -> io::Result<()> {
    let line = activity_line_at(template, elapsed_seconds);
    let spinner = palette.activity_frame(elapsed_seconds, true);
    let rendered = format!("{spinner} {line}");
    let _guard = output_lock
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    match target {
        StreamTarget::Stdout => {
            print!("\r\x1b[2K{rendered}");
            io::stdout().flush()
        }
        StreamTarget::Stderr => {
            eprint!("\r\x1b[2K{rendered}");
            io::stderr().flush()
        }
    }
}

#[async_trait]
impl RunEventObserver for TerminalStreamObserver {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if let RunEvent::ToolStarted { call, .. } = &envelope.event {
            self.tool_calls.insert(call.call_id.clone(), call.clone());
        }
        if let RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } = &envelope.event
        {
            self.stop_activity()
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            if self.preferences.stream_mode == StreamDisplayMode::Off {
                return Ok(());
            }
            if self.preferences.stream_mode == StreamDisplayMode::On
                && matches!(self.target, StreamTarget::Stdout)
            {
                self.buffered_text.push_str(text);
                return Ok(());
            }
            let _guard = self
                .output_lock
                .lock()
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            let text = SemanticRenderer::new(self.preferences.clone())
                .with_color(self.is_terminal())
                .assistant_text(text);
            let result = match self.target {
                StreamTarget::Stdout => {
                    print!("{text}");
                    io::stdout().flush()
                }
                StreamTarget::Stderr => {
                    eprint!("{text}");
                    io::stderr().flush()
                }
            };
            result.map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            self.wrote_text = true;
            return Ok(());
        }
        if let RunEvent::ToolCompleted {
            turn,
            result,
            duration_seconds,
            elapsed_seconds,
        } = &envelope.event
        {
            let call = self.tool_calls.remove(&result.call_id);
            if let Some(line) = SemanticRenderer::new(self.preferences.clone())
                .with_color(self.is_terminal())
                .tool_completed_with_call(
                    *turn,
                    result,
                    *duration_seconds,
                    *elapsed_seconds,
                    call.as_ref(),
                )
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?
            {
                self.write_line(&line)
                    .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            }
            return Ok(());
        }
        if let RunEvent::Provider {
            event: ProviderEvent::FinalOutput { text },
        } = &envelope.event
            && matches!(self.target, StreamTarget::Stdout)
            && self.preferences.stream_mode != StreamDisplayMode::Raw
        {
            self.write_markdown(text)
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            self.final_rendered = true;
            return Ok(());
        }
        if let Some(line) = SemanticRenderer::new(self.preferences.clone())
            .with_color(self.is_terminal())
            .run_event_envelope(&envelope)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?
        {
            if let Some(elapsed_seconds) = activity_elapsed(&envelope.event) {
                self.start_activity(&line, elapsed_seconds)
            } else {
                self.write_line(&line)
            }
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
        }
        Ok(())
    }
}

struct SilentStreamObserver;

#[async_trait]
impl RunEventObserver for SilentStreamObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}

fn bounded_preview(value: &str, max_chars: usize) -> &str {
    value
        .char_indices()
        .nth(max_chars)
        .map_or(value, |(end, _)| &value[..end])
}

fn approval_provider(
    command: &Command,
    configured: Option<ApprovalMode>,
) -> Arc<dyn ApprovalProvider> {
    let mode = configured.unwrap_or(if matches!(command, Command::Tui { .. }) {
        ApprovalMode::Ask
    } else {
        ApprovalMode::Deny
    });
    match mode {
        ApprovalMode::Deny => Arc::new(DenyApproval),
        ApprovalMode::Ask | ApprovalMode::RiskAuto => Arc::new(TerminalApproval {
            risk_auto: mode == ApprovalMode::RiskAuto,
            lock: Mutex::new(()),
        }),
        ApprovalMode::FullAccess => Arc::new(AllowApproval {
            approved_by: "terminal-user:full-access".into(),
        }),
    }
}

#[derive(Subcommand)]
enum Command {
    /// Create or inspect fresh YAML configuration.
    Config(ConfigCommand),
    /// Verify and inspect the authoritative journal.
    Audit(AuditCommand),
    /// Diagnose the active built-in or OPA policy channel.
    Policy(PolicyCommand),
    /// Inspect, drain, or rebuild disposable state projections.
    Projection(ProjectionCommand),
    /// Diagnose canonical storage, lease, repositories, and projection readiness.
    State(StateCommand),
    /// Diagnose the native/OCI sandbox helper.
    Sandbox(SandboxCommand),
    /// Execute exact programs without a shell through the effect gateway.
    Process(ProcessCommand),
    /// Perform policy-allowed brokered network requests.
    Network(NetworkCommand),
    /// Validate and operate durable workflows.
    Workflow(WorkflowCommand),
    /// Inspect and diagnose configured model providers.
    Provider(ProviderCommand),
    /// Inspect model role routing.
    Models(ModelsCommand),
    /// Inspect the active strict tool catalog.
    Tools(ToolsCommand),
    /// Create, inspect, and resume durable sessions.
    Sessions(SessionsCommand),
    /// Refresh bounded actionable work for a session.
    Work {
        /// Exact session; defaults to the latest session.
        #[arg(long)]
        session: Option<String>,
    },
    /// Inspect or reset local presentation preferences.
    Preferences(PreferencesCommand),
    /// Inspect, compact, and restore durable long-session context.
    Context(ContextCommand),
    /// Create and inspect durable session tasks.
    Tasks(TasksCommand),
    /// Create and inspect binding key decisions.
    Decisions(DecisionsCommand),
    /// Create, inspect, and approve durable plans.
    Plans(PlansCommand),
    /// Run and inspect bounded durable goals.
    Goals(GoalsCommand),
    /// Inspect and control durable child-agent jobs.
    Agents(AgentsCommand),
    /// Create, search, archive, and supersede durable memories.
    Memories(MemoriesCommand),
    /// Run and inspect durable source-backed research.
    Research(ResearchCommand),
    /// Inspect metadata-only persisted run telemetry.
    Telemetry(TelemetryCommand),
    /// Discover, compose, and read declarative data-only skills.
    Skills(SkillsCommand),
    /// Verify and lifecycle-manage signed capability packs.
    Packs(PacksCommand),
    /// Build, verify, and install signed offline release bundles.
    Bundle(BundleCommand),
    /// Manage persisted integrations and imported OpenAPI tools.
    Integrations(IntegrationsCommand),
    /// Discover and invoke explicitly configured MCP servers.
    Mcp(McpCommand),
    /// Execute one audited model turn through the configured role.
    Run {
        /// User prompt sent as the complete logical request content.
        prompt: Option<String>,
        /// Create a durable plan through structurally non-mutating Plan Mode.
        #[arg(long, conflicts_with = "execute_plan")]
        plan: bool,
        /// Atomically consume and execute an approved plan id.
        #[arg(long, conflicts_with_all = ["plan", "session", "resume"])]
        execute_plan: Option<String>,
        /// Execute --execute-plan through bounded Goal Mode.
        #[arg(long, requires = "execute_plan")]
        goal: bool,
        /// Maximum Goal Mode iterations for --execute-plan --goal.
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u16).range(1..=50))]
        goal_max_iterations: u16,
        /// Configured model role.
        #[arg(long, default_value = "primary")]
        role: String,
        /// System/developer instructions for this turn.
        #[arg(long, default_value = "You are Colossus.")]
        instructions: String,
        /// Override the configured bounded model-turn limit.
        #[arg(long)]
        max_turns: Option<u16>,
        /// Attach to this exact durable session.
        #[arg(long, conflicts_with = "resume")]
        session: Option<String>,
        /// Resume the most recently updated session.
        #[arg(long, conflicts_with = "session")]
        resume: bool,
        /// Explicitly activate one declarative skill. Repeat as needed.
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// Render policy-released text deltas to stderr while preserving JSON on stdout.
        #[arg(long)]
        stream: bool,
    },
    /// Run the credential-free, network-free echo smoke provider.
    Echo {
        /// Text returned by the deterministic provider.
        message: String,
    },
    /// Start the Ratatui interactive terminal.
    Tui {
        /// Start attached to this exact durable session.
        #[arg(long, conflicts_with = "resume")]
        session: Option<String>,
        /// Start attached to the most recently updated session.
        #[arg(long, conflicts_with = "session")]
        resume: bool,
    },
    /// Recover abandoned runs and drain queued resumable work.
    Worker {
        /// Recover and drain once instead of serving local IPC.
        #[arg(long, conflicts_with_all = ["shutdown", "status"])]
        once: bool,
        /// Ask the authenticated local worker to checkpoint and stop.
        #[arg(long, conflicts_with_all = ["once", "status"])]
        shutdown: bool,
        /// Authenticate the configured worker and show readiness.
        #[arg(long, conflicts_with_all = ["once", "shutdown"])]
        status: bool,
    },
    /// Internal authenticated one-shot sandbox helper.
    #[command(name = "__sandbox-helper", hide = true)]
    SandboxHelper,
}

#[derive(Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigAction,
}

#[derive(Args)]
struct PreferencesCommand {
    #[command(subcommand)]
    command: PreferencesAction,
}

#[derive(Subcommand)]
enum PreferencesAction {
    /// Show the strict effective local profile.
    Show,
    /// Show newest encrypted terminal history entries in chronological order.
    History {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Restore and persist default presentation preferences.
    Reset,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create a strict offline configuration without overwriting an existing file.
    Init {
        /// Use isolated redb state and environment keys for source development.
        #[arg(long)]
        development: bool,
        /// Clone non-storage settings from an existing strict configuration.
        #[arg(long, value_name = "PATH", requires = "development")]
        from: Option<PathBuf>,
    },
    /// Parse and print the active configuration with references intact.
    Show,
}

#[derive(Args)]
struct AuditCommand {
    #[command(subcommand)]
    command: AuditAction,
}

#[derive(Subcommand)]
enum AuditAction {
    /// Verify encryption, chain, checkpoint signature, and secure anchor.
    Verify,
    /// Show bounded envelope metadata without decrypted payload content.
    Show {
        /// First global sequence.
        #[arg(long, default_value_t = 1)]
        from: u64,
        /// Maximum records.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Stream bounded redacted envelopes as JSON Lines to stdout.
    Export {
        /// First global sequence.
        #[arg(long, default_value_t = 1)]
        from: u64,
        /// Maximum records.
        #[arg(long, default_value_t = 1_000)]
        limit: usize,
    },
    /// Show the latest signed checkpoint and secure chain head.
    AnchorStatus,
    /// Show configured durable audit-export position, lag, and retry state.
    ExporterStatus,
    /// Drain queued redacted evidence to the configured external sink.
    ExporterDrain,
    /// Reset the external sink consumer for operator-authorized replay.
    ExporterReset,
}

#[derive(Args)]
struct PolicyCommand {
    #[command(subcommand)]
    command: PolicyAction,
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Check readiness, revision metadata, and decision-log safeguards.
    Doctor,
}

#[derive(Args)]
struct ProjectionCommand {
    #[command(subcommand)]
    command: ProjectionAction,
}

#[derive(Subcommand)]
enum ProjectionAction {
    /// Show position, journal head, lag, and readiness.
    Status,
    /// Replay queued journal records into every projection.
    Drain,
    /// Delete and replay one projection, or every projection when omitted.
    Rebuild { name: Option<String> },
}

#[derive(Args)]
struct StateCommand {
    #[command(subcommand)]
    command: StateAction,
}

#[derive(Subcommand)]
enum StateAction {
    /// Check the writer lease, journal head, adapters, and projection lag.
    Doctor,
}

#[derive(Args)]
struct SandboxCommand {
    #[command(subcommand)]
    command: SandboxAction,
}

#[derive(Subcommand)]
enum SandboxAction {
    /// Report native kernel support and configured OCI fallback.
    Doctor,
}

#[derive(Args)]
struct ProcessCommand {
    #[command(subcommand)]
    command: ProcessAction,
}

#[derive(Subcommand)]
enum ProcessAction {
    /// Run one exact executable with literal arguments and an explicit environment.
    Run {
        executable: PathBuf,
        /// Absolute or repository-relative working directory.
        #[arg(long, default_value = ".")]
        cwd: PathBuf,
        /// Explicit KEY=VALUE environment entry. Repeat as needed.
        #[arg(long = "env")]
        environment: Vec<String>,
        /// Literal arguments passed after `--`; no shell interpretation occurs.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Args)]
struct NetworkCommand {
    #[command(subcommand)]
    command: NetworkAction,
}

#[derive(Subcommand)]
enum NetworkAction {
    /// Fetch one exact HTTP(S) URL through destination enforcement and quarantine.
    Get { url: String },
}

#[derive(Args)]
struct WorkflowCommand {
    #[command(subcommand)]
    command: WorkflowAction,
}

#[derive(Subcommand)]
enum WorkflowAction {
    /// Parse and validate a strict workflow YAML file.
    Validate { path: PathBuf },
    /// Validate and register a definition with repository provenance.
    Register { path: PathBuf },
    /// List registered definition change events.
    List,
    /// Show an exact registered definition and pinned content hash.
    Show { name: String, version: String },
    /// Start a durable run.
    Run {
        name: String,
        version: String,
        /// Inline JSON or @path to a JSON document.
        #[arg(long, default_value = "{}")]
        inputs: String,
        /// Queue for a worker instead of executing immediately.
        #[arg(long)]
        queued: bool,
    },
    /// Create, inspect, control, or evaluate persisted workflow schedules.
    Schedule {
        #[command(subcommand)]
        command: WorkflowScheduleAction,
    },
    /// Create, inspect, control, or ingest authenticated workflow webhooks.
    Webhook {
        #[command(subcommand)]
        command: WorkflowWebhookAction,
    },
    /// Create, inspect, control, or evaluate repository-event subscriptions.
    Subscription {
        #[command(subcommand)]
        command: WorkflowSubscriptionAction,
    },
    /// Show a reconstructed run.
    Status { run_id: String },
    /// Resume a waiting or interrupted run.
    Resume { run_id: String },
    /// Supply inline JSON or @path input and resume.
    Input { run_id: String, input: String },
    /// Cancel a non-terminal run.
    Cancel { run_id: String },
}

#[derive(Subcommand)]
enum WorkflowScheduleAction {
    /// Create a hash-pinned fixed-cadence schedule.
    Create {
        schedule_id: String,
        name: String,
        version: String,
        /// Fixed cadence in seconds (60 through 2678400).
        #[arg(long)]
        cadence_seconds: u64,
        /// Inline JSON or @path to a JSON document.
        #[arg(long, default_value = "{}")]
        inputs: String,
        /// Behavior when multiple occurrences are overdue.
        #[arg(long, value_enum, default_value_t = WorkflowScheduleMisfireArg::FireOnce)]
        misfire: WorkflowScheduleMisfireArg,
        /// Create the schedule disabled.
        #[arg(long)]
        disabled: bool,
        /// Optional UTC RFC3339 first occurrence; defaults to now plus one cadence.
        #[arg(long)]
        starts_at: Option<String>,
    },
    /// List persisted schedules in deterministic identifier order.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact persisted schedule.
    Show { schedule_id: String },
    /// Enable one schedule after rechecking pinned workflow trust.
    Enable { schedule_id: String },
    /// Disable one schedule without deleting its audit history.
    Disable { schedule_id: String },
    /// Evaluate due schedules using the real or an explicit UTC clock.
    Tick {
        #[arg(long)]
        at: Option<String>,
    },
}

#[derive(Subcommand)]
enum WorkflowWebhookAction {
    /// Create a hash-pinned HMAC-SHA256 webhook binding.
    Create {
        webhook_id: String,
        name: String,
        version: String,
        /// Late-bound HMAC secret reference, such as env:COLOSSUS_WEBHOOK_SECRET.
        #[arg(long)]
        secret_reference: String,
        /// Maximum accepted signed-delivery age in seconds (60 through 3600).
        #[arg(long, default_value_t = 300)]
        replay_window_seconds: u64,
        /// Maximum accepted raw JSON body size in bytes (1 through 1048576).
        #[arg(long, default_value_t = 1024 * 1024)]
        max_body_bytes: u64,
        /// Create the webhook disabled.
        #[arg(long)]
        disabled: bool,
    },
    /// List persisted webhook bindings in deterministic identifier order.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact persisted webhook binding.
    Show { webhook_id: String },
    /// Enable one webhook after rechecking pinned workflow trust.
    Enable { webhook_id: String },
    /// Disable one webhook without deleting its audit history.
    Disable { webhook_id: String },
    /// Authenticate and durably ingest one JSON delivery.
    Ingest {
        webhook_id: String,
        /// Sender-supplied replay identifier.
        #[arg(long)]
        delivery_id: String,
        /// Sender-supplied signed UTC RFC3339 timestamp.
        #[arg(long)]
        timestamp: String,
        /// HMAC-SHA256 signature (`sha256=<hex>`).
        #[arg(long)]
        signature: String,
        /// Lowercase application HEADER=VALUE entry; repeat as needed.
        #[arg(long = "header")]
        headers: Vec<String>,
        /// Inline JSON or @path to the exact JSON body bytes.
        #[arg(long)]
        body: String,
    },
    /// Serve authenticated deliveries over loopback HTTP.
    Serve {
        /// Loopback socket address exposed to a trusted reverse proxy.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
    },
}

#[derive(Subcommand)]
enum WorkflowSubscriptionAction {
    /// Create a hash-pinned exact domain-event subscription.
    Create {
        subscription_id: String,
        name: String,
        version: String,
        /// Exact versioned domain event type.
        #[arg(long)]
        event_type: String,
        /// Optional aggregate stream prefix used to narrow matching events.
        #[arg(long)]
        stream_prefix: Option<String>,
        /// Create the subscription disabled.
        #[arg(long)]
        disabled: bool,
        /// Begin after this global sequence; defaults to the current journal head.
        #[arg(long)]
        after_sequence: Option<u64>,
    },
    /// List persisted subscriptions in deterministic identifier order.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact persisted subscription.
    Show { subscription_id: String },
    /// Enable one subscription after rechecking pinned workflow trust.
    Enable { subscription_id: String },
    /// Disable one subscription without deleting its audit history.
    Disable { subscription_id: String },
    /// Evaluate bounded canonical journal work for subscriptions.
    Tick,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum WorkflowScheduleMisfireArg {
    Skip,
    FireOnce,
}

impl From<WorkflowScheduleMisfireArg> for WorkflowScheduleMisfirePolicy {
    fn from(value: WorkflowScheduleMisfireArg) -> Self {
        match value {
            WorkflowScheduleMisfireArg::Skip => Self::Skip,
            WorkflowScheduleMisfireArg::FireOnce => Self::FireOnce,
        }
    }
}

#[derive(Args)]
struct ProviderCommand {
    #[command(subcommand)]
    command: ProviderAction,
}

#[derive(Subcommand)]
enum ProviderAction {
    /// Show configured profiles without resolving credentials.
    Profiles,
    /// Exercise the profile model-catalog endpoint through policy.
    Doctor { profile: Option<String> },
    /// List normalized models through policy.
    Models { profile: Option<String> },
}

#[derive(Args)]
struct ModelsCommand {
    #[command(subcommand)]
    command: ModelsAction,
}

#[derive(Subcommand)]
enum ModelsAction {
    /// Show role-to-profile mappings.
    Routes,
    /// Resolve one role to bounded profile/model metadata.
    Route {
        #[arg(default_value = "primary")]
        role: String,
    },
}

#[derive(Args)]
struct ToolsCommand {
    #[command(subcommand)]
    command: ToolsAction,
}

#[derive(Subcommand)]
enum ToolsAction {
    /// List model-visible specifications and effect identities.
    List,
}

#[derive(Args)]
struct SessionsCommand {
    #[command(subcommand)]
    command: SessionsAction,
}

#[derive(Subcommand)]
enum SessionsAction {
    /// List recent sessions newest first.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show one exact session summary.
    Show { session_id: String },
    /// Show append-only messages for one session.
    Messages { session_id: String },
    /// Create an empty session.
    New { title: Option<String> },
}

#[derive(Args)]
struct ContextCommand {
    #[command(subcommand)]
    command: ContextAction,
}

#[derive(Subcommand)]
enum ContextAction {
    /// Show the active context budget and snapshot.
    Status { session_id: String },
    /// List immutable snapshots for one session.
    List { session_id: String },
    /// Force a new snapshot without deleting canonical messages.
    Compact { session_id: String },
    /// Activate an existing snapshot for future turns.
    Restore {
        session_id: String,
        snapshot_id: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum TaskStatusArg {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

impl From<TaskStatusArg> for TaskStatus {
    fn from(value: TaskStatusArg) -> Self {
        match value {
            TaskStatusArg::Pending => Self::Pending,
            TaskStatusArg::InProgress => Self::InProgress,
            TaskStatusArg::Completed => Self::Completed,
            TaskStatusArg::Blocked => Self::Blocked,
            TaskStatusArg::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Args)]
struct TasksCommand {
    #[command(subcommand)]
    command: TasksAction,
}

#[derive(Subcommand)]
enum TasksAction {
    /// List bounded canonical tasks.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        status: Option<TaskStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact task.
    Show { task_id: String },
    /// Create a session-scoped task.
    Create {
        session_id: String,
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, value_enum, default_value = "pending")]
        status: TaskStatusArg,
    },
    /// Update supplied fields on one task.
    Update {
        task_id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<TaskStatusArg>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum DecisionPriorityArg {
    Critical,
    High,
    Normal,
}

impl From<DecisionPriorityArg> for DecisionPriority {
    fn from(value: DecisionPriorityArg) -> Self {
        match value {
            DecisionPriorityArg::Critical => Self::Critical,
            DecisionPriorityArg::High => Self::High,
            DecisionPriorityArg::Normal => Self::Normal,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum DecisionStatusArg {
    Active,
    Archived,
    Superseded,
}

impl From<DecisionStatusArg> for DecisionStatus {
    fn from(value: DecisionStatusArg) -> Self {
        match value {
            DecisionStatusArg::Active => Self::Active,
            DecisionStatusArg::Archived => Self::Archived,
            DecisionStatusArg::Superseded => Self::Superseded,
        }
    }
}

#[derive(Args)]
struct DecisionsCommand {
    #[command(subcommand)]
    command: DecisionsAction,
}

#[derive(Subcommand)]
enum DecisionsAction {
    /// List bounded canonical decisions.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "active")]
        status: DecisionStatusArg,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact decision.
    Show { decision_id: String },
    /// Create one active future-facing commitment.
    Create {
        session_id: String,
        title: String,
        decision: String,
        #[arg(long, value_enum, default_value = "normal")]
        priority: DecisionPriorityArg,
        #[arg(long, default_value = "")]
        intent: String,
        #[arg(long, default_value = "")]
        applies_when: String,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long, default_value = "")]
        source_excerpt: String,
    },
    /// Update mutable content on an active decision.
    Update {
        decision_id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        decision: Option<String>,
        #[arg(long)]
        priority: Option<DecisionPriorityArg>,
        #[arg(long)]
        intent: Option<String>,
        #[arg(long)]
        applies_when: Option<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        source_excerpt: Option<String>,
    },
    /// Archive an active decision without deleting it.
    Archive { decision_id: String },
    /// Atomically replace an active decision and preserve lineage.
    Supersede {
        decision_id: String,
        title: String,
        decision: String,
        #[arg(long, value_enum, default_value = "normal")]
        priority: DecisionPriorityArg,
        #[arg(long, default_value = "")]
        intent: String,
        #[arg(long, default_value = "")]
        applies_when: String,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long, default_value = "")]
        source_excerpt: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum PlanStatusArg {
    Draft,
    Approved,
    Executed,
    Discarded,
}

impl From<PlanStatusArg> for PlanStatus {
    fn from(value: PlanStatusArg) -> Self {
        match value {
            PlanStatusArg::Draft => Self::Draft,
            PlanStatusArg::Approved => Self::Approved,
            PlanStatusArg::Executed => Self::Executed,
            PlanStatusArg::Discarded => Self::Discarded,
        }
    }
}

#[derive(Args)]
struct PlansCommand {
    #[command(subcommand)]
    command: PlansAction,
}

#[derive(Subcommand)]
enum PlansAction {
    /// List bounded canonical plans.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<PlanStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact plan.
    Show { plan_id: String },
    /// Create a draft plan with ordered title-only steps.
    Create {
        session_id: String,
        prompt: String,
        #[arg(long, default_value = "")]
        content: String,
        #[arg(long = "step", required = true)]
        steps: Vec<String>,
    },
    /// Request operator approval for one draft plan.
    Approve { plan_id: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum GoalStatusArg {
    Active,
    Complete,
    Blocked,
}

impl From<GoalStatusArg> for GoalStatus {
    fn from(value: GoalStatusArg) -> Self {
        match value {
            GoalStatusArg::Active => Self::Active,
            GoalStatusArg::Complete => Self::Complete,
            GoalStatusArg::Blocked => Self::Blocked,
        }
    }
}

#[derive(Args)]
struct GoalsCommand {
    #[command(subcommand)]
    command: GoalsAction,
}

#[derive(Subcommand)]
enum GoalsAction {
    /// List bounded canonical goals.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<GoalStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact goal.
    Show { goal_id: String },
    /// Start a bounded Goal Mode loop in an existing session.
    Run {
        objective: String,
        #[arg(long)]
        session: String,
        #[arg(long, default_value = "primary")]
        role: String,
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u16).range(1..=50))]
        max_iterations: u16,
        #[arg(long)]
        source_plan: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum SubagentStatusArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl From<SubagentStatusArg> for SubagentStatus {
    fn from(value: SubagentStatusArg) -> Self {
        match value {
            SubagentStatusArg::Queued => Self::Queued,
            SubagentStatusArg::Running => Self::Running,
            SubagentStatusArg::Completed => Self::Completed,
            SubagentStatusArg::Failed => Self::Failed,
            SubagentStatusArg::Cancelled => Self::Cancelled,
            SubagentStatusArg::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Args)]
struct AgentsCommand {
    #[command(subcommand)]
    command: AgentsAction,
}

#[derive(Subcommand)]
enum AgentsAction {
    /// Queue one durable child-agent job from the terminal.
    Queue {
        session_id: String,
        task: String,
        #[arg(long, default_value = "subagent_default")]
        role: String,
    },
    /// List bounded durable child-agent jobs.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<SubagentStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact child-agent job and bounded result.
    Show { job_id: String },
    /// Show queue counts and available scheduler slots.
    Status {
        #[arg(long)]
        session: Option<String>,
    },
    /// Execute queued jobs up to configured concurrency until empty.
    Drain,
    /// Cancel one queued or running job.
    Cancel { job_id: String },
    /// Requeue one failed, cancelled, or interrupted job.
    Requeue { job_id: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum MemoryScopeArg {
    Global,
    Repository,
    Session,
}

#[derive(Clone, Copy, ValueEnum)]
enum MemoryStatusArg {
    Active,
    Archived,
    Superseded,
    All,
}

impl MemoryStatusArg {
    fn status(self) -> Option<MemoryStatus> {
        match self {
            Self::Active => Some(MemoryStatus::Active),
            Self::Archived => Some(MemoryStatus::Archived),
            Self::Superseded => Some(MemoryStatus::Superseded),
            Self::All => None,
        }
    }
}

#[derive(Args)]
struct MemoriesCommand {
    #[command(subcommand)]
    command: MemoriesAction,
}

#[derive(Subcommand)]
enum MemoriesAction {
    /// List bounded canonical records.
    List {
        #[arg(long, value_enum, default_value = "active")]
        status: MemoryStatusArg,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Read one exact canonical record.
    Show { memory_id: String },
    /// Search candidates and re-filter canonical scope/status/expiry.
    Search {
        query: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        repository: Option<String>,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// Create one active memory.
    Create {
        text: String,
        #[arg(long, value_enum, default_value = "global")]
        scope: MemoryScopeArg,
        /// Required identifier for session or repository scope.
        #[arg(long)]
        scope_id: Option<String>,
        #[arg(long, default_value = "preference")]
        kind: String,
        #[arg(long, default_value_t = 1.0)]
        confidence: f32,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long)]
        expires_at: Option<String>,
    },
    /// Archive one active memory without deleting it.
    Archive { memory_id: String },
    /// Atomically replace one active memory and retain lineage.
    Supersede {
        memory_id: String,
        text: String,
        #[arg(long, default_value = "")]
        rationale: String,
    },
    /// Inspect or rebuild the disposable lexical index.
    Index(MemoryIndexCommand),
}

#[derive(Args)]
struct MemoryIndexCommand {
    #[command(subcommand)]
    command: MemoryIndexAction,
}

#[derive(Subcommand)]
enum MemoryIndexAction {
    /// Show adapter readiness and journal lag.
    Status,
    /// Retry queued journal-to-index work.
    Sync,
    /// Rebuild from canonical active records.
    Rebuild,
}

#[derive(Clone, Copy, ValueEnum)]
enum ResearchDepthArg {
    Quick,
    Standard,
    Deep,
}

impl From<ResearchDepthArg> for ResearchDepth {
    fn from(value: ResearchDepthArg) -> Self {
        match value {
            ResearchDepthArg::Quick => Self::Quick,
            ResearchDepthArg::Standard => Self::Standard,
            ResearchDepthArg::Deep => Self::Deep,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ResearchSourceArg {
    Repo,
    Web,
    Mcp,
}

impl From<ResearchSourceArg> for ResearchSourceKind {
    fn from(value: ResearchSourceArg) -> Self {
        match value {
            ResearchSourceArg::Repo => Self::Repo,
            ResearchSourceArg::Web => Self::Web,
            ResearchSourceArg::Mcp => Self::Mcp,
        }
    }
}

#[derive(Args)]
struct ResearchCommand {
    #[command(subcommand)]
    command: ResearchAction,
}

#[derive(Subcommand)]
enum ResearchAction {
    /// Execute bounded durable research and emit a cited report.
    Run {
        question: String,
        /// Existing session; a fresh session is created when omitted.
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "standard")]
        depth: ResearchDepthArg,
        #[arg(
            long = "source",
            value_enum,
            value_delimiter = ',',
            default_value = "repo,web,mcp"
        )]
        sources: Vec<ResearchSourceArg>,
    },
    /// List bounded canonical research runs.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show one exact canonical research run.
    Show { run_id: String },
    /// Show stable source labels and released evidence.
    Sources { run_id: String },
    /// Show extracted source-backed claims.
    Claims { run_id: String },
}

#[derive(Args)]
struct TelemetryCommand {
    #[command(subcommand)]
    command: TelemetryAction,
}

#[derive(Subcommand)]
enum TelemetryAction {
    /// List recent run summaries newest first.
    Runs {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a bounded metadata-only timeline by full id or unique prefix.
    Show {
        run_id: String,
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
    /// Aggregate metrics over recent runs.
    Metrics {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Args)]
struct SkillsCommand {
    #[command(subcommand)]
    command: SkillsAction,
}

#[derive(Subcommand)]
enum SkillsAction {
    /// List selected skill metadata in deterministic name order.
    List,
    /// Show one selected manifest and its data-only instructions.
    Show { name: String },
    /// Report duplicate names and configured precedence winners.
    Duplicates,
    /// Preview context composition and required-tool validation.
    Compose {
        prompt: String,
        #[arg(long = "skill")]
        skills: Vec<String>,
    },
    /// Create a new installed user skill (approval required).
    Scaffold {
        name: String,
        description: String,
        #[arg(long)]
        instructions: Option<String>,
        #[arg(long = "resource-dir")]
        resource_dirs: Vec<String>,
    },
    /// Inspect an installed user skill without returning file bodies.
    Inspect { name: String },
    /// Read one authorable installed user-skill file.
    FileRead { name: String, path: String },
    /// Write one authorable installed user-skill file (approval required).
    Write {
        name: String,
        path: String,
        content: String,
        #[arg(long)]
        expected_sha256: Option<String>,
    },
    /// Validate an installed name or a workspace-local directory with --local.
    Validate {
        target: String,
        #[arg(long)]
        local: bool,
    },
    /// Install a validated workspace-local skill (approval required).
    Install { path: String },
    /// List bounded regular resources for an explicitly active skill.
    Resources { name: String },
    /// Read one bounded UTF-8 resource through the effect gateway.
    Read { name: String, path: String },
}

#[derive(Args)]
struct IntegrationsCommand {
    #[command(subcommand)]
    command: IntegrationsAction,
}

#[derive(Args)]
struct PacksCommand {
    #[command(subcommand)]
    command: PacksAction,
}

#[derive(Subcommand)]
enum PacksAction {
    /// List canonical pack lifecycles.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one canonical pack lifecycle.
    Show { name: String },
    /// Verify a local pack without installing it.
    Verify { path: PathBuf },
    /// Alias for strict local pack verification.
    Validate { path: PathBuf },
    /// Install a verified local pack (approval required).
    Install {
        path: PathBuf,
        /// Explicit development override for an unsigned pack.
        #[arg(long)]
        allow_untrusted: bool,
    },
    /// Reverify and enable an installed pack (approval required).
    Enable { name: String },
    /// Disable an installed pack (approval required).
    Disable { name: String },
    /// Uninstall a pack while retaining lifecycle history (approval required).
    Uninstall { name: String },
    /// Invoke one active verified fixed-argument pack tool (approval required).
    Call { tool: String },
    /// Manage publisher/key trust bindings.
    Trust(PackTrustCommand),
}

#[derive(Args)]
struct PackTrustCommand {
    #[command(subcommand)]
    command: PackTrustAction,
}

#[derive(Subcommand)]
enum PackTrustAction {
    /// List publisher/key trust bindings.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Bind a publisher to a base64 Ed25519 public key (approval required).
    Add {
        publisher: String,
        #[arg(long)]
        public_key: String,
    },
}

#[derive(Args)]
struct BundleCommand {
    #[command(subcommand)]
    command: BundleAction,
}

#[derive(Subcommand)]
enum BundleAction {
    /// Derive the safe public identity for a referenced signing seed.
    KeyInfo {
        #[arg(long)]
        signing_key_reference: String,
    },
    /// Verify a signed offline bundle without network access.
    Verify { path: PathBuf },
    /// Materialize a signed bundle from a staged payload directory.
    Build {
        source: PathBuf,
        destination: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        publisher: String,
        /// Explicit RFC3339 UTC timestamp for reproducible output.
        #[arg(long)]
        created_at: String,
        #[arg(long)]
        source_revision: Option<String>,
        /// Environment credential reference containing an Ed25519 signing seed.
        #[arg(long)]
        signing_key_reference: String,
    },
    /// Verify and install the current-target executable into a clean prefix.
    Install {
        path: PathBuf,
        #[arg(long)]
        prefix: PathBuf,
    },
}

#[derive(Args)]
struct McpCommand {
    #[command(subcommand)]
    command: McpAction,
}

#[derive(Subcommand)]
enum McpAction {
    /// List configured server names and exact tool allowlists without launching them.
    Servers,
    /// Discover live allowlisted tool schemas through the audited sandbox.
    Tools {
        /// Restrict discovery to one configured server.
        #[arg(long)]
        server: Option<String>,
    },
    /// Discover, validate, and invoke one exact allowlisted tool.
    Call {
        server: String,
        tool: String,
        /// Inline JSON object or @path to a JSON document.
        arguments: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum IntegrationAuthMode {
    None,
    Bearer,
    ApiKey,
    Basic,
    ServiceAccount,
}

#[derive(Subcommand)]
enum IntegrationsAction {
    /// List safe persisted connection summaries.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one canonical connection without resolving credentials.
    Show { name: String },
    /// Connect a first-party GitHub, SearXNG, or OpenSearch adapter.
    Connect {
        name: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, value_enum)]
        auth_type: Option<IntegrationAuthMode>,
        #[arg(long)]
        credential_reference: Option<String>,
        #[arg(long)]
        username_reference: Option<String>,
        #[arg(long)]
        password_reference: Option<String>,
        #[arg(long, default_value = "Authorization")]
        auth_header: String,
        #[arg(long)]
        auth_scheme: Option<String>,
        #[arg(long = "scope")]
        scopes: Vec<String>,
    },
    /// Import a JSON OpenAPI 3 document (approval required).
    ImportOpenapi {
        name: String,
        spec: String,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long, value_enum, default_value_t = IntegrationAuthMode::Bearer)]
        auth_type: IntegrationAuthMode,
        #[arg(long)]
        credential_reference: Option<String>,
        #[arg(long, default_value = "Authorization")]
        auth_header: String,
        #[arg(long)]
        auth_scheme: Option<String>,
        #[arg(long = "scope")]
        scopes: Vec<String>,
    },
    /// Disconnect one connection while preserving lifecycle history (approval required).
    Disconnect { name: String },
    /// Invoke one connected operation with a JSON argument object.
    Call { tool: String, arguments: String },
}

fn integration_auth(
    mode: IntegrationAuthMode,
    header: String,
    scheme: Option<String>,
) -> IntegrationAuth {
    match mode {
        IntegrationAuthMode::None => IntegrationAuth::None,
        IntegrationAuthMode::Bearer => IntegrationAuth::Bearer {
            header,
            scheme: scheme.unwrap_or_else(|| "Bearer".into()),
        },
        IntegrationAuthMode::ApiKey => IntegrationAuth::ApiKey { header, scheme },
        IntegrationAuthMode::Basic => IntegrationAuth::Basic { header },
        IntegrationAuthMode::ServiceAccount => IntegrationAuth::ServiceAccount { header },
    }
}

async fn parse_json_argument(runtime: &Runtime, source: &str) -> Result<Value, Box<dyn Error>> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.to_owned()
    };
    Ok(serde_json::from_str(&document)?)
}

fn init_config(path: &Path, development: bool, from: Option<&Path>) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if !development && from.is_some() {
        return Err("--from requires --development".into());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }
    let state = parent.join(if development {
        "state.dev.redb"
    } else {
        "state.redb"
    });
    let anchor = parent.join("secure-anchor.dev.json");
    if development && (state.exists() || anchor.exists()) {
        return Err(format!(
            "refusing to create {} while isolated development state or anchor already exists; restore the matching config or remove both {} and {}",
            path.display(),
            state.display(),
            anchor.display()
        )
        .into());
    }
    let config = if let Some(source) = from {
        RuntimeConfig::from_path(source)?
    } else {
        RuntimeConfig::offline_template(&state)
    };
    let config = if development {
        config.with_isolated_development_storage(state, anchor)
    } else {
        config
    };
    let mut destination = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    destination.write_all(config.to_yaml()?.as_bytes())?;
    println!("created {}", path.display());
    Ok(())
}

fn set_output_mode(mode: OutputMode) {
    let encoded = match mode {
        OutputMode::Auto => 0,
        OutputMode::Human => 1,
        OutputMode::Json => 2,
    };
    OUTPUT_MODE.store(encoded, Ordering::Relaxed);
}

fn output_mode() -> OutputMode {
    match OUTPUT_MODE.load(Ordering::Relaxed) {
        1 => OutputMode::Human,
        2 => OutputMode::Json,
        _ => OutputMode::Auto,
    }
}

fn set_terminal_preferences(preferences: &TerminalPreferences) {
    *TERMINAL_PREFERENCES
        .get_or_init(|| Mutex::new(TerminalPreferences::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = preferences.clone();
}

fn terminal_preferences() -> TerminalPreferences {
    TERMINAL_PREFERENCES
        .get_or_init(|| Mutex::new(TerminalPreferences::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .or_else(|_| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or(())
        })
        .unwrap_or(100)
        .clamp(40, 240)
}

fn render_structured_output(
    value: &Value,
    mode: OutputMode,
    terminal: bool,
    width: usize,
    preferences: TerminalPreferences,
) -> Result<String, serde_json::Error> {
    let human = mode == OutputMode::Human || mode == OutputMode::Auto && terminal;
    if !human {
        return serde_json::to_string_pretty(value);
    }
    Ok(TerminalDocumentRenderer::new(preferences, width)
        .with_color(terminal)
        .render(&document_from_json(value, None)))
}

fn print_json(value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    let value = serde_json::to_value(value)?;
    println!(
        "{}",
        render_structured_output(
            &value,
            output_mode(),
            io::stdout().is_terminal(),
            terminal_width(),
            terminal_preferences(),
        )?
    );
    Ok(())
}

fn print_theme_library(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&json!({
            "selected": preferences.theme_name(),
            "library": themes.status(),
        }));
    }

    print_terminal_document(
        &themes.status_document(preferences.theme_name()),
        preferences,
        terminal,
    );
    Ok(())
}

fn human_output(terminal: bool) -> bool {
    output_mode() == OutputMode::Human || output_mode() == OutputMode::Auto && terminal
}

fn print_terminal_document(
    document: &PresentationDocument,
    preferences: &TerminalPreferences,
    terminal: bool,
) {
    println!(
        "{}",
        TerminalDocumentRenderer::new(preferences.clone(), terminal_width())
            .with_color(terminal)
            .render(document)
    );
}

fn print_theme_preview(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let snapshot = themes.preview(name)?;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&snapshot);
    }
    let preview_preferences = themes.preview_preferences(name, preferences)?;
    let document = themes.preview_document(name)?;
    print_terminal_document(&document, &preview_preferences, terminal);
    Ok(())
}

fn print_theme_validation(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&json!({
            "valid": true,
            "library": themes.status(),
        }));
    }
    print_terminal_document(&themes.validation_document(), preferences, terminal);
    Ok(())
}

fn print_theme_scaffold(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let scaffold = themes.scaffold(name)?;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&scaffold);
    }
    print_terminal_document(
        &ThemeLibrary::scaffold_document(&scaffold),
        preferences,
        terminal,
    );
    Ok(())
}

fn print_theme_applied(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(preferences);
    }
    print_terminal_document(
        &themes.selection_document(preferences.theme_name()),
        preferences,
        terminal,
    );
    Ok(())
}

fn write_stderr_document(document: &PresentationDocument) -> io::Result<()> {
    let terminal = io::stderr().is_terminal();
    let rendered = TerminalDocumentRenderer::new(terminal_preferences(), terminal_width())
        .with_color(terminal)
        .render(document);
    eprintln!("{rendered}");
    io::stderr().flush()
}

fn print_terminal_help(preferences: &TerminalPreferences) {
    let mut table = PresentationTable::new(
        ["Area", "Commands", "What it does"],
        "No interactive terminal commands are available.",
    );
    for row in [
        [
            "Conversation",
            "/resume · /sessions · /session show|new|resume",
            "Resume or manage durable conversations",
        ],
        [
            "Work",
            "/work · /tasks · /decisions · /plans · /goals · /goal · /agents",
            "Inspect and drive durable work",
        ],
        [
            "Memory & context",
            "/memories · /memory search · /context status|list|compact|restore",
            "Recall canonical memory and manage context",
        ],
        [
            "Agent resources",
            "/tools · /skills · /skill use|active|clear|show|resources|read",
            "Discover tools and activate skills",
        ],
        [
            "Research",
            "/research · /research list · /mcp servers|tools|call",
            "Run research and inspect MCP capabilities",
        ],
        [
            "Extensions",
            "/packs list|show|verify|install|enable|disable|call · /integrations",
            "Manage trusted extension surfaces",
        ],
        [
            "Runtime",
            "/workflow list|status|schedule · /telemetry · /audit verify · /projection status",
            "Inspect durable runs and runtime health",
        ],
        [
            "Appearance",
            "/theme · /stream · /events · /reasoning · /transcript · /multiline",
            "Tune the terminal experience",
        ],
        ["Exit", "/exit · Ctrl-D", "Leave the terminal safely"],
    ] {
        table.push_row(row);
    }
    let document = PresentationDocument {
        blocks: vec![
            PresentationBlock::Markdown(
                "# Colossus Terminal\n\nType a normal message to talk to the configured primary model. Press **Tab** to complete commands and `@skill` names."
                    .into(),
            ),
            PresentationBlock::KeyValue(vec![
                ("Theme".into(), preferences.theme_name().into()),
                ("Stream".into(), preferences.stream_mode.as_str().into()),
                ("Events".into(), preferences.events_mode.as_str().into()),
                (
                    "Reasoning summaries".into(),
                    if preferences.show_reasoning { "on" } else { "off" }.into(),
                ),
                (
                    "Transcript".into(),
                    preferences.transcript_density.as_str().into(),
                ),
                (
                    "Multiline".into(),
                    if preferences.multiline { "on" } else { "off" }.into(),
                ),
            ]),
            PresentationBlock::Table(table),
        ],
    };
    println!(
        "{}",
        TerminalDocumentRenderer::new(preferences.clone(), terminal_width())
            .with_color(io::stdout().is_terminal())
            .render(&document)
    );
}

fn parse_toggle(value: &str) -> Option<bool> {
    match value {
        "on" | "true" => Some(true),
        "off" | "false" => Some(false),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PresentationCommandResult {
    NotHandled,
    Handled,
    Save,
    ChooseTheme,
}

fn terminal_completion_values(skill_names: &[String], themes: &ThemeLibrary) -> Vec<String> {
    let mut completion_values = TERMINAL_COMPLETIONS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    for name in themes.names() {
        completion_values.push(format!("/theme {name}"));
        completion_values.push(format!("/theme preview {name}"));
    }
    completion_values.extend(skill_names.iter().map(|name| format!("@{name}")));
    completion_values
}

fn resolve_skill_mentions(input: &str, skill_names: &[String]) -> (String, Vec<String>) {
    let mut explicit = Vec::new();
    let mut prompt = input.trim_start();
    while let Some(token) = prompt.split_whitespace().next() {
        let Some(name) = token.strip_prefix('@') else {
            break;
        };
        if name.is_empty() || !skill_names.iter().any(|candidate| candidate == name) {
            break;
        }
        if !explicit.iter().any(|candidate| candidate == name) {
            explicit.push(name.into());
        }
        prompt = prompt[token.len()..].trim_start();
    }
    (prompt.into(), explicit)
}

fn remember_history_entry(history_entries: &mut Vec<String>, entry: &str) {
    if history_entries.last().is_some_and(|last| last == entry) {
        return;
    }
    if history_entries.len() == TERMINAL_HISTORY_CAPACITY {
        history_entries.remove(0);
    }
    history_entries.push(entry.into());
}

fn handle_presentation_command(
    line: &str,
    preferences: &mut TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<PresentationCommandResult, Box<dyn Error>> {
    let mut changed = false;
    match line {
        "/tui" | "/tui prefs" => print_json(preferences)?,
        "/tui save" => changed = true,
        "/tui reset" => {
            *preferences = TerminalPreferences::default();
            changed = true;
        }
        "/theme" if human_output(io::stdout().is_terminal()) => {
            return Ok(PresentationCommandResult::ChooseTheme);
        }
        "/theme" | "/theme list" => print_theme_library(preferences, themes)?,
        "/theme reset" => {
            preferences.select_builtin_theme(ThemeName::Default);
            changed = true;
        }
        "/theme preview" => print_theme_library(preferences, themes)?,
        command if command.starts_with("/theme preview ") => {
            match print_theme_preview(
                preferences,
                themes,
                command.trim_start_matches("/theme preview ").trim(),
            ) {
                Ok(()) => {}
                Err(error) => println!("recoverable: {error}"),
            }
        }
        "/theme validate" => print_theme_validation(preferences, themes)?,
        "/theme scaffold" => {
            println!("recoverable: usage: /theme scaffold NAME");
        }
        command if command.starts_with("/theme scaffold ") => {
            match print_theme_scaffold(
                preferences,
                themes,
                command.trim_start_matches("/theme scaffold ").trim(),
            ) {
                Ok(()) => {}
                Err(error) => println!("recoverable: {error}"),
            }
        }
        command if command.starts_with("/theme save ") => {
            println!(
                "note: `/theme save NAME` is deprecated; `/theme NAME` applies and saves immediately."
            );
            match themes.select(
                command.trim_start_matches("/theme save ").trim(),
                preferences,
            ) {
                Ok(()) => changed = true,
                Err(error) => println!("recoverable: {error}"),
            }
        }
        command if command.starts_with("/theme ") => {
            match themes.select(command.trim_start_matches("/theme ").trim(), preferences) {
                Ok(()) => changed = true,
                Err(error) => println!("recoverable: {error}"),
            }
        }
        "/events" => println!("events={}", preferences.events_mode.as_str()),
        "/events compact" => {
            preferences.events_mode = EventDisplayMode::Compact;
            changed = true;
        }
        "/events verbose" => {
            preferences.events_mode = EventDisplayMode::Verbose;
            changed = true;
        }
        "/events off" => {
            preferences.events_mode = EventDisplayMode::Off;
            changed = true;
        }
        "/transcript" => println!("transcript={}", preferences.transcript_density.as_str()),
        "/transcript comfortable" => {
            preferences.transcript_density = TranscriptDensity::Comfortable;
            changed = true;
        }
        "/transcript compact" => {
            preferences.transcript_density = TranscriptDensity::Compact;
            changed = true;
        }
        "/stream" => println!("stream={}", preferences.stream_mode.as_str()),
        "/stream on" => {
            preferences.stream_mode = StreamDisplayMode::On;
            changed = true;
        }
        "/stream raw" => {
            preferences.stream_mode = StreamDisplayMode::Raw;
            changed = true;
        }
        "/stream off" => {
            preferences.stream_mode = StreamDisplayMode::Off;
            changed = true;
        }
        "/reasoning" => println!(
            "reasoning={}",
            if preferences.show_reasoning {
                "on"
            } else {
                "off"
            }
        ),
        command if command.starts_with("/reasoning ") => {
            if let Some(value) = parse_toggle(command.trim_start_matches("/reasoning ")) {
                preferences.show_reasoning = value;
                changed = true;
            } else {
                println!("recoverable: /reasoning expects on or off");
            }
        }
        "/multiline" => println!(
            "multiline={}",
            if preferences.multiline { "on" } else { "off" }
        ),
        command if command.starts_with("/multiline ") => {
            let value = command.trim_start_matches("/multiline ");
            if value == "toggle" {
                preferences.multiline = !preferences.multiline;
                changed = true;
            } else if let Some(value) = parse_toggle(value) {
                preferences.multiline = value;
                changed = true;
            } else {
                println!("recoverable: /multiline expects on, off, or toggle");
            }
        }
        "/trace" => {
            preferences.events_mode = if preferences.events_mode == EventDisplayMode::Off {
                EventDisplayMode::Compact
            } else {
                EventDisplayMode::Off
            };
            changed = true;
        }
        command
            if command.starts_with("/tui ")
                || command.starts_with("/events ")
                || command.starts_with("/transcript ")
                || command.starts_with("/stream ") =>
        {
            println!("recoverable: invalid presentation command; use /help");
        }
        _ => return Ok(PresentationCommandResult::NotHandled),
    }
    if changed {
        Ok(PresentationCommandResult::Save)
    } else {
        Ok(PresentationCommandResult::Handled)
    }
}

fn cli_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}

fn parse_environment(entries: Vec<String>) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut environment = BTreeMap::new();
    for entry in entries {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("environment entry must be KEY=VALUE: {entry}"))?;
        if name.is_empty() || environment.insert(name.into(), value.into()).is_some() {
            return Err(format!("environment name is empty or duplicated: {name}").into());
        }
    }
    Ok(environment)
}

fn parse_headers(entries: Vec<String>) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut headers = BTreeMap::new();
    for entry in entries {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("header entry must be NAME=VALUE: {entry}"))?;
        if name.is_empty() || headers.insert(name.into(), value.into()).is_some() {
            return Err(format!("header name is empty or duplicated: {name}").into());
        }
    }
    Ok(headers)
}

const MAX_WEBHOOK_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_WEBHOOK_HTTP_BODY_BYTES: usize = 1024 * 1024;

struct WebhookHttpDelivery {
    webhook_id: String,
    delivery_id: String,
    timestamp: String,
    signature: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

enum WebhookIngressBackend<'a> {
    Runtime(&'a Runtime),
    Worker(&'a WorkerClient),
}

impl WebhookIngressBackend<'_> {
    async fn ingest(&self, delivery: WebhookHttpDelivery) -> Result<Value, Box<dyn Error>> {
        match self {
            Self::Runtime(runtime) => Ok(serde_json::to_value(
                runtime
                    .ingest_workflow_webhook(
                        &delivery.webhook_id,
                        &delivery.delivery_id,
                        &delivery.timestamp,
                        &delivery.signature,
                        delivery.headers,
                        &delivery.body,
                    )
                    .await?,
            )?),
            Self::Worker(client) => Ok(client
                .call(WorkerOperation::WorkflowWebhookIngest {
                    webhook_id: delivery.webhook_id,
                    delivery_id: delivery.delivery_id,
                    timestamp: delivery.timestamp,
                    signature: delivery.signature,
                    headers: delivery.headers,
                    body_source: String::from_utf8(delivery.body)
                        .map_err(|_| "webhook JSON body must be UTF-8")?,
                })
                .await?),
        }
    }
}

async fn serve_workflow_webhooks(
    bind: SocketAddr,
    backend: WebhookIngressBackend<'_>,
) -> Result<(), Box<dyn Error>> {
    if !bind.ip().is_loopback() {
        return Err("workflow webhook listener must bind to a loopback address".into());
    }
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "workflow webhook listener ready on http://{}/v1/workflow-webhooks/WEBHOOK_ID",
        listener.local_addr()?
    );
    loop {
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(());
            }
        };
        let response = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            read_webhook_http_delivery(&mut stream),
        )
        .await
        {
            Ok(Ok(delivery)) => match backend.ingest(delivery).await {
                Ok(value) => webhook_http_response(202, "Accepted", &value),
                Err(error) => {
                    eprintln!("workflow webhook delivery rejected: {error}");
                    webhook_http_response(
                        400,
                        "Bad Request",
                        &json!({"accepted": false, "error": "delivery rejected"}),
                    )
                }
            },
            Ok(Err(error)) => webhook_http_response(
                400,
                "Bad Request",
                &json!({"accepted": false, "error": error.to_string()}),
            ),
            Err(_) => webhook_http_response(
                408,
                "Request Timeout",
                &json!({"accepted": false, "error": "request timed out"}),
            ),
        };
        let _ = stream.write_all(&response).await;
        let _ = stream.shutdown().await;
    }
}

async fn read_webhook_http_delivery(
    stream: &mut TcpStream,
) -> Result<WebhookHttpDelivery, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() >= MAX_WEBHOOK_HTTP_HEADER_BYTES {
            return Err("webhook HTTP headers exceed 65536 bytes".into());
        }
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err("webhook HTTP request ended before its headers".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    if header_end > MAX_WEBHOOK_HTTP_HEADER_BYTES {
        return Err("webhook HTTP headers exceed 65536 bytes".into());
    }
    let (_, content_length) = parse_webhook_http_head(&bytes[..header_end])?;
    if content_length == 0 || content_length > MAX_WEBHOOK_HTTP_BODY_BYTES {
        return Err("webhook HTTP body must contain 1..=1048576 bytes".into());
    }
    let expected = header_end
        .checked_add(content_length)
        .ok_or("webhook HTTP request size overflow")?;
    while bytes.len() < expected {
        let mut buffer = [0_u8; 8192];
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err("webhook HTTP body ended before Content-Length bytes".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > expected {
            return Err("webhook HTTP request contains bytes after its declared body".into());
        }
    }
    parse_webhook_http_request(&bytes)
}

fn parse_webhook_http_head(
    bytes: &[u8],
) -> Result<(BTreeMap<String, String>, usize), Box<dyn Error>> {
    let text = std::str::from_utf8(bytes).map_err(|_| "webhook HTTP headers must be UTF-8")?;
    let mut lines = text.strip_suffix("\r\n\r\n").unwrap_or(text).split("\r\n");
    let request_line = lines.next().ok_or("webhook HTTP request line is absent")?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "POST" || parts[2] != "HTTP/1.1" {
        return Err("webhook listener requires POST over HTTP/1.1".into());
    }
    if !parts[1].starts_with("/v1/workflow-webhooks/") || parts[1].contains(['?', '#']) {
        return Err("webhook HTTP path must be /v1/workflow-webhooks/WEBHOOK_ID".into());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or("webhook HTTP header is malformed")?;
        let name = name.to_ascii_lowercase();
        if name.is_empty()
            || headers
                .insert(name.clone(), value.trim().to_owned())
                .is_some()
        {
            return Err(format!("webhook HTTP header is empty or duplicated: {name}").into());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err("chunked webhook HTTP requests are not accepted".into());
    }
    let content_length = headers
        .get("content-length")
        .ok_or("webhook HTTP Content-Length is required")?
        .parse::<usize>()
        .map_err(|_| "webhook HTTP Content-Length is invalid")?;
    Ok((headers, content_length))
}

fn parse_webhook_http_request(bytes: &[u8]) -> Result<WebhookHttpDelivery, Box<dyn Error>> {
    let header_end = find_bytes(bytes, b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or("webhook HTTP header delimiter is absent")?;
    let (mut headers, content_length) = parse_webhook_http_head(&bytes[..header_end])?;
    if bytes.len() != header_end + content_length {
        return Err("webhook HTTP body does not match Content-Length".into());
    }
    let request_line = std::str::from_utf8(&bytes[..header_end])?
        .split("\r\n")
        .next()
        .ok_or("webhook HTTP request line is absent")?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or("webhook HTTP path is absent")?;
    let webhook_id = path
        .strip_prefix("/v1/workflow-webhooks/")
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or("webhook HTTP identifier is invalid")?
        .to_owned();
    let delivery_id = headers
        .remove("x-colossus-delivery-id")
        .ok_or("x-colossus-delivery-id is required")?;
    let timestamp = headers
        .remove("x-colossus-timestamp")
        .ok_or("x-colossus-timestamp is required")?;
    let signature = headers
        .remove("x-colossus-signature")
        .ok_or("x-colossus-signature is required")?;
    for transport in ["connection", "content-length", "host"] {
        headers.remove(transport);
    }
    Ok(WebhookHttpDelivery {
        webhook_id,
        delivery_id,
        timestamp,
        signature,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn webhook_http_response(status: u16, reason: &str, value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{\"accepted\":false}".to_vec());
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    response
}

fn memory_scope(
    scope: MemoryScopeArg,
    scope_id: Option<String>,
) -> Result<MemoryScope, Box<dyn Error>> {
    match (scope, scope_id) {
        (MemoryScopeArg::Global, None) => Ok(MemoryScope::Global),
        (MemoryScopeArg::Global, Some(_)) => {
            Err("global memory scope does not accept --scope-id".into())
        }
        (MemoryScopeArg::Repository, Some(id)) if !id.trim().is_empty() => {
            Ok(MemoryScope::Repository(id))
        }
        (MemoryScopeArg::Session, Some(id)) if !id.trim().is_empty() => {
            Ok(MemoryScope::Session(id))
        }
        (MemoryScopeArg::Repository | MemoryScopeArg::Session, _) => {
            Err("session and repository memory scopes require --scope-id".into())
        }
    }
}

async fn workflow_command(
    runtime: &Runtime,
    command: WorkflowAction,
) -> Result<(), Box<dyn Error>> {
    match command {
        WorkflowAction::Validate { path } => {
            let validated = runtime.validate_workflow_path(&path).await?;
            print_json(&json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))?;
        }
        WorkflowAction::Register { path } => {
            let provenance = format!("repo:{}", path.display());
            let validated = runtime.register_workflow_path(&path).await?;
            print_json(&json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))?;
        }
        WorkflowAction::List => {
            let journal = runtime.journal();
            let definitions = journal
                .read_global(1, usize::MAX)?
                .into_iter()
                .filter(|event| event.event_type.starts_with("workflow.definition."))
                .map(|event| {
                    json!({
                        "event_id": event.event_id,
                        "event_type": event.event_type,
                        "stream_id": event.stream_id,
                        "occurred_at": event.occurred_at,
                        "record_hash": event.record_hash,
                    })
                })
                .collect::<Vec<_>>();
            print_json(&definitions)?;
        }
        WorkflowAction::Show { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| format!("workflow {name}:{version} is not registered"))?;
            print_json(&json!({
                "definition": definition,
                "content_hash": content_hash,
            }))?;
        }
        WorkflowAction::Run {
            name,
            version,
            inputs,
            queued,
        } => {
            let inputs = parse_json_argument(runtime, &inputs).await?;
            let run = if queued {
                runtime.workflows().queue_run(&name, &version, inputs)?
            } else {
                runtime
                    .workflows()
                    .start_run(&name, &version, inputs)
                    .await?
            };
            print_json(&run)?;
        }
        WorkflowAction::Schedule { command } => match command {
            WorkflowScheduleAction::Create {
                schedule_id,
                name,
                version,
                cadence_seconds,
                inputs,
                misfire,
                disabled,
                starts_at,
            } => {
                let inputs = parse_json_argument(runtime, &inputs).await?;
                print_json(&runtime.workflows().create_schedule(
                    &schedule_id,
                    &name,
                    &version,
                    inputs,
                    cadence_seconds,
                    misfire.into(),
                    !disabled,
                    starts_at.as_deref(),
                )?)?;
            }
            WorkflowScheduleAction::List { limit } => {
                print_json(&runtime.workflows().list_schedules(limit.clamp(1, 10_000))?)?;
            }
            WorkflowScheduleAction::Show { schedule_id } => {
                print_json(&runtime.workflows().get_schedule(&schedule_id)?)?;
            }
            WorkflowScheduleAction::Enable { schedule_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_schedule_enabled(&schedule_id, true)?,
                )?;
            }
            WorkflowScheduleAction::Disable { schedule_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_schedule_enabled(&schedule_id, false)?,
                )?;
            }
            WorkflowScheduleAction::Tick { at } => {
                let dispatches = match at {
                    Some(at) => runtime.workflows().tick_schedules_at(&at)?,
                    None => runtime.workflows().tick_schedules_now()?,
                };
                print_json(&dispatches)?;
            }
        },
        WorkflowAction::Webhook { command } => match command {
            WorkflowWebhookAction::Create {
                webhook_id,
                name,
                version,
                secret_reference,
                replay_window_seconds,
                max_body_bytes,
                disabled,
            } => print_json(&runtime.workflows().create_webhook(
                &webhook_id,
                &name,
                &version,
                &secret_reference,
                replay_window_seconds,
                max_body_bytes,
                !disabled,
            )?)?,
            WorkflowWebhookAction::List { limit } => {
                print_json(&runtime.workflows().list_webhooks(limit.clamp(1, 10_000))?)?;
            }
            WorkflowWebhookAction::Show { webhook_id } => {
                print_json(&runtime.workflows().get_webhook(&webhook_id)?)?;
            }
            WorkflowWebhookAction::Enable { webhook_id } => {
                print_json(&runtime.workflows().set_webhook_enabled(&webhook_id, true)?)?;
            }
            WorkflowWebhookAction::Disable { webhook_id } => {
                print_json(
                    &runtime
                        .workflows()
                        .set_webhook_enabled(&webhook_id, false)?,
                )?;
            }
            WorkflowWebhookAction::Ingest {
                webhook_id,
                delivery_id,
                timestamp,
                signature,
                headers,
                body,
            } => {
                let body = if let Some(path) = body.strip_prefix('@') {
                    runtime.read_text_file(path).await?
                } else {
                    body
                };
                print_json(
                    &runtime
                        .ingest_workflow_webhook(
                            &webhook_id,
                            &delivery_id,
                            &timestamp,
                            &signature,
                            parse_headers(headers)?,
                            body.as_bytes(),
                        )
                        .await?,
                )?;
            }
            WorkflowWebhookAction::Serve { bind } => {
                serve_workflow_webhooks(bind, WebhookIngressBackend::Runtime(runtime)).await?;
            }
        },
        WorkflowAction::Subscription { command } => match command {
            WorkflowSubscriptionAction::Create {
                subscription_id,
                name,
                version,
                event_type,
                stream_prefix,
                disabled,
                after_sequence,
            } => print_json(&runtime.workflows().create_subscription(
                &subscription_id,
                &name,
                &version,
                &event_type,
                stream_prefix.as_deref(),
                !disabled,
                after_sequence,
            )?)?,
            WorkflowSubscriptionAction::List { limit } => print_json(
                &runtime
                    .workflows()
                    .list_subscriptions(limit.clamp(1, 10_000))?,
            )?,
            WorkflowSubscriptionAction::Show { subscription_id } => {
                print_json(&runtime.workflows().get_subscription(&subscription_id)?)?;
            }
            WorkflowSubscriptionAction::Enable { subscription_id } => print_json(
                &runtime
                    .workflows()
                    .set_subscription_enabled(&subscription_id, true)?,
            )?,
            WorkflowSubscriptionAction::Disable { subscription_id } => print_json(
                &runtime
                    .workflows()
                    .set_subscription_enabled(&subscription_id, false)?,
            )?,
            WorkflowSubscriptionAction::Tick => {
                print_json(&runtime.workflows().tick_subscriptions_now().await?)?;
            }
        },
        WorkflowAction::Status { run_id } => {
            print_json(&runtime.workflows().get_run(&run_id)?)?;
        }
        WorkflowAction::Resume { run_id } => {
            print_json(&runtime.workflows().resume_run(&run_id).await?)?;
        }
        WorkflowAction::Input { run_id, input } => {
            print_json(
                &runtime
                    .workflows()
                    .provide_input(&run_id, parse_json_argument(runtime, &input).await?)
                    .await?,
            )?;
        }
        WorkflowAction::Cancel { run_id } => {
            print_json(&runtime.workflows().cancel_run(&run_id)?)?;
        }
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
enum ThemePickerInput {
    Cancelled,
    Selected(String),
    Preview(String),
    Command(String),
    Invalid,
}

fn resolve_theme_picker_name(choice: &str, names: &[String]) -> Option<String> {
    if let Ok(index) = choice.parse::<usize>() {
        return index
            .checked_sub(1)
            .and_then(|index| names.get(index))
            .cloned();
    }
    let normalized = choice.trim().to_ascii_lowercase().replace('-', "_");
    names.iter().find(|name| **name == normalized).cloned()
}

fn parse_theme_picker_input(choice: &str, names: &[String]) -> ThemePickerInput {
    let choice = choice.trim();
    if choice.is_empty() {
        return ThemePickerInput::Cancelled;
    }
    if choice.starts_with('/') {
        return ThemePickerInput::Command(choice.into());
    }
    if let Some(preview) = choice
        .strip_prefix("p ")
        .or_else(|| choice.strip_prefix("preview "))
    {
        return resolve_theme_picker_name(preview.trim(), names)
            .map_or(ThemePickerInput::Invalid, ThemePickerInput::Preview);
    }
    resolve_theme_picker_name(choice, names)
        .map_or(ThemePickerInput::Invalid, ThemePickerInput::Selected)
}

fn choose_theme(
    scripted_input: &mut dyn BufRead,
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<ThemePickerInput, Box<dyn Error>> {
    let names = themes.names();
    print_theme_library(preferences, themes)?;
    println!(
        "Enter a number or theme name to apply it, `p NUMBER` to preview it, or leave the line blank to cancel."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(ThemePickerInput::Cancelled);
        }
        match parse_theme_picker_input(&choice, &names) {
            ThemePickerInput::Preview(name) => {
                print_theme_preview(preferences, themes, &name)?;
                println!(
                    "Choose a number or theme name to apply it, preview another with `p NUMBER`, or leave the line blank to cancel."
                );
            }
            ThemePickerInput::Invalid => println!(
                "That is not one of the listed themes. Enter 1-{}, a theme name, `p NUMBER`, or leave the line blank to cancel.",
                names.len()
            ),
            result => return Ok(result),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SessionPickerInput {
    Cancelled,
    Selected(String),
    Command(String),
    Invalid,
}

fn parse_session_picker_input(choice: &str, sessions: &[SessionSummary]) -> SessionPickerInput {
    let choice = choice.trim();
    if choice.is_empty() {
        return SessionPickerInput::Cancelled;
    }
    if choice.starts_with('/') {
        return SessionPickerInput::Command(choice.into());
    }
    if let Ok(index) = choice.parse::<usize>()
        && let Some(session) = index.checked_sub(1).and_then(|index| sessions.get(index))
    {
        return SessionPickerInput::Selected(session.id.clone());
    }
    sessions
        .iter()
        .find(|session| session.id == choice)
        .map_or(SessionPickerInput::Invalid, |session| {
            SessionPickerInput::Selected(session.id.clone())
        })
}

fn choose_session(
    runtime: &Runtime,
    scripted_input: &mut dyn BufRead,
    limit: usize,
) -> Result<SessionPickerInput, Box<dyn Error>> {
    let mut sessions = runtime
        .list_sessions(100)?
        .into_iter()
        .filter(|session| session.message_count > 0)
        .collect::<Vec<_>>();
    sessions.truncate(limit);
    if sessions.is_empty() {
        println!("No sessions exist yet.");
        return Ok(SessionPickerInput::Cancelled);
    }
    println!("Choose a session to resume:");
    for (index, session) in sessions.iter().enumerate() {
        println!(
            "  {}. {}  {}  messages={}",
            index + 1,
            session.id,
            session.title.as_deref().unwrap_or("Untitled"),
            session.message_count
        );
    }
    println!(
        "Enter a number or exact session id (blank cancels; /command returns to the terminal)."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(SessionPickerInput::Cancelled);
        }
        let parsed = parse_session_picker_input(&choice, &sessions);
        if parsed != SessionPickerInput::Invalid {
            return Ok(parsed);
        }
        println!(
            "That is not one of the listed sessions. Enter 1-{}, an exact id, or leave it blank to cancel.",
            sessions.len()
        );
    }
}

async fn line_runner(
    runtime: &Runtime,
    initial_session: Option<String>,
    resume_latest: bool,
    _approval_mode: ApprovalMode,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    if output_mode() == OutputMode::Auto {
        set_output_mode(OutputMode::Human);
    }
    let mut preferences = runtime.presentation_preferences()?;
    set_terminal_preferences(&preferences);
    let mut history_entries = runtime.terminal_history(TERMINAL_HISTORY_CAPACITY)?;
    let skill_names = runtime
        .list_skills()?
        .into_iter()
        .map(|skill| skill.manifest.name)
        .collect::<Vec<_>>();
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("interactive terminals must use the TUI".into());
    }
    let mut scripted_input = stdin.lock();
    let mut active_session_id = if resume_latest {
        runtime.latest_session()?.id
    } else if let Some(session_id) = initial_session {
        runtime
            .get_session(&session_id)?
            .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
            .id
    } else {
        runtime.create_session(None)?.id
    };
    let mut sticky_skills = Vec::<String>::new();
    let mut pending_line = None::<String>;
    println!(
        "Colossus Rust {}. session={active_session_id}; /help for commands; Ctrl-D to exit.",
        env!("CARGO_PKG_VERSION")
    );
    loop {
        let line = if let Some(line) = pending_line.take() {
            line
        } else {
            let mut line = String::new();
            if scripted_input.read_line(&mut line)? == 0 {
                break;
            }
            line
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match runtime.append_terminal_history(line).await {
            Ok(entry) => remember_history_entry(&mut history_entries, &entry),
            Err(error) => eprintln!("history was not persisted: {error}"),
        }
        if matches!(line, "/quit" | "/exit") {
            break;
        }
        match handle_presentation_command(line, &mut preferences, themes)? {
            PresentationCommandResult::NotHandled => {}
            PresentationCommandResult::Handled => continue,
            PresentationCommandResult::Save => {
                preferences = runtime
                    .save_presentation_preferences(preferences.clone())
                    .await?;
                set_terminal_preferences(&preferences);
                if line.starts_with("/theme") {
                    print_theme_applied(&preferences, themes)?;
                } else {
                    print_json(&preferences)?;
                }
                continue;
            }
            PresentationCommandResult::ChooseTheme => {
                match choose_theme(&mut scripted_input, &preferences, themes)? {
                    ThemePickerInput::Selected(name) => {
                        themes.select(&name, &mut preferences)?;
                        preferences = runtime
                            .save_presentation_preferences(preferences.clone())
                            .await?;
                        set_terminal_preferences(&preferences);
                        print_theme_applied(&preferences, themes)?;
                    }
                    ThemePickerInput::Command(command) => pending_line = Some(command),
                    ThemePickerInput::Cancelled => {}
                    ThemePickerInput::Preview(_) | ThemePickerInput::Invalid => {
                        unreachable!("picker consumes preview and invalid input")
                    }
                }
                continue;
            }
        }
        if line == "/help" {
            print_terminal_help(&preferences);
        } else if line == "/workflow list" {
            workflow_command(runtime, WorkflowAction::List).await?;
        } else if line == "/workflow schedule list" {
            workflow_command(
                runtime,
                WorkflowAction::Schedule {
                    command: WorkflowScheduleAction::List { limit: 100 },
                },
            )
            .await?;
        } else if line == "/workflow subscription list" {
            workflow_command(
                runtime,
                WorkflowAction::Subscription {
                    command: WorkflowSubscriptionAction::List { limit: 100 },
                },
            )
            .await?;
        } else if let Some(run_id) = line.strip_prefix("/workflow status ") {
            workflow_command(
                runtime,
                WorkflowAction::Status {
                    run_id: run_id.trim().into(),
                },
            )
            .await?;
        } else if line == "/audit verify" {
            print_json(&runtime.journal().verify()?)?;
        } else if line == "/projection status" {
            print_json(&runtime.projection_status()?)?;
        } else if line == "/tools" {
            print_json(&runtime.tool_specs())?;
        } else if line == "/sessions" {
            print_json(&runtime.list_sessions(20)?)?;
        } else if line == "/work" {
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .work_state(&runtime.work_state(&active_session_id)?)
            );
        } else if line == "/tasks" {
            print_json(&runtime.list_tasks(Some(&active_session_id), None, 100)?)?;
        } else if line == "/decisions" {
            print_json(&runtime.list_decisions(
                Some(&active_session_id),
                Some(DecisionStatus::Active),
                100,
            )?)?;
        } else if line == "/plans" {
            print_json(&runtime.list_plans(Some(&active_session_id), None, 100)?)?;
        } else if line == "/goals" {
            print_json(&runtime.list_goals(Some(&active_session_id), None, 100)?)?;
        } else if let Some(objective) = line.strip_prefix("/goal ") {
            print_json(
                &runtime
                    .run_goal("primary", objective.trim(), &active_session_id, 5, None)
                    .await?,
            )?;
        } else if line == "/agents" {
            print_json(&runtime.list_subagents(Some(&active_session_id), None, 100)?)?;
        } else if line == "/agents drain" {
            print_json(&runtime.drain_subagents().await?)?;
        } else if line == "/memories" {
            print_json(
                &runtime
                    .list_memories(Some(MemoryStatus::Active), 20)
                    .await?,
            )?;
        } else if let Some(query) = line.strip_prefix("/memory search ") {
            print_json(
                &runtime
                    .search_memories(query.trim(), Some(&active_session_id), None, 8)
                    .await?,
            )?;
        } else if line == "/research list" {
            print_json(&runtime.list_research_runs(Some(&active_session_id), 20)?)?;
        } else if let Some(question) = line.strip_prefix("/research ") {
            print_json(
                &runtime
                    .run_research(
                        &active_session_id,
                        question.trim(),
                        ResearchDepth::Standard,
                        vec![
                            ResearchSourceKind::Repo,
                            ResearchSourceKind::Web,
                            ResearchSourceKind::Mcp,
                        ],
                    )
                    .await?,
            )?;
        } else if line == "/telemetry" {
            print_json(&runtime.telemetry_runs(Some(&active_session_id), 20)?)?;
        } else if line == "/telemetry metrics" {
            print_json(&runtime.telemetry_metrics(Some(&active_session_id), 100)?)?;
        } else if let Some(run_id) = line.strip_prefix("/telemetry ") {
            print_json(&runtime.telemetry_run(run_id.trim(), 500)?)?;
        } else if line == "/packs" || line == "/packs list" {
            print_json(&runtime.list_packs(100)?)?;
        } else if let Some(name) = line.strip_prefix("/packs show ") {
            let name = name.trim();
            print_json(
                &runtime
                    .get_pack(name)?
                    .ok_or_else(|| cli_error(format!("pack not found: {name}")))?,
            )?;
        } else if let Some(path) = line
            .strip_prefix("/packs verify ")
            .or_else(|| line.strip_prefix("/packs validate "))
        {
            print_json(&runtime.verify_pack(path.trim()).await?)?;
        } else if let Some(value) = line.strip_prefix("/packs install ") {
            let value = value.trim();
            let (path, allow_untrusted) = value
                .strip_suffix(" --allow-untrusted")
                .map_or((value, false), |path| (path.trim(), true));
            print_json(&runtime.install_pack(path, allow_untrusted).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs enable ") {
            print_json(&runtime.enable_pack(name.trim()).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs disable ") {
            print_json(&runtime.disable_pack(name.trim()).await?)?;
        } else if let Some(name) = line.strip_prefix("/packs uninstall ") {
            print_json(&runtime.uninstall_pack(name.trim()).await?)?;
        } else if let Some(tool) = line.strip_prefix("/packs call ") {
            print_json(&runtime.call_pack_tool(tool.trim()).await?)?;
        } else if line == "/packs trust" || line == "/packs trust list" {
            print_json(&runtime.list_pack_trust(100)?)?;
        } else if let Some(value) = line.strip_prefix("/packs trust add ") {
            let (publisher, public_key) = value
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY"))?;
            print_json(&runtime.add_pack_trust(publisher, public_key.trim()).await?)?;
        } else if let Some(path) = line.strip_prefix("/bundle verify ") {
            print_json(&runtime.verify_bundle(path.trim()).await?)?;
        } else if line == "/integrations" {
            print_json(&runtime.list_integrations(100)?)?;
        } else if let Some(name) = line.strip_prefix("/integration show ") {
            print_json(
                &runtime
                    .get_integration(name.trim())?
                    .ok_or_else(|| cli_error(format!("integration not found: {name}")))?,
            )?;
        } else if let Some(name) = line.strip_prefix("/integration disconnect ") {
            print_json(&runtime.disconnect_integration(name.trim()).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/integration call ") {
            let (tool, arguments) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /integration call TOOL JSON"))?;
            let arguments: Value = serde_json::from_str(arguments.trim())?;
            print_json(&runtime.call_integration_tool(tool, arguments).await?)?;
        } else if line == "/mcp servers" {
            print_json(&runtime.mcp_servers())?;
        } else if line == "/mcp tools" {
            print_json(&runtime.mcp_tools(None).await?)?;
        } else if let Some(server) = line.strip_prefix("/mcp tools ") {
            print_json(&runtime.mcp_tools(Some(server.trim())).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/mcp call ") {
            let mut parts = arguments.trim().splitn(3, ' ');
            let server = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let tool = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let arguments = parts
                .next()
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            print_json(
                &runtime
                    .mcp_call(server, tool, serde_json::from_str(arguments.trim())?)
                    .await?,
            )?;
        } else if line == "/skills" {
            let skills = runtime
                .list_skills()?
                .into_iter()
                .map(|skill| {
                    json!({
                        "name": skill.manifest.name,
                        "version": skill.manifest.version,
                        "description": skill.manifest.description,
                        "source": skill.source,
                        "active": sticky_skills.contains(&skill.manifest.name),
                    })
                })
                .collect::<Vec<_>>();
            print_json(&skills)?;
        } else if line == "/skill active" {
            if sticky_skills.is_empty() {
                println!("No skills are active.");
            } else {
                println!("Active skills: {}", sticky_skills.join(", "));
            }
        } else if line == "/skill clear" {
            sticky_skills.clear();
            println!("active skills cleared");
        } else if let Some(name) = line.strip_prefix("/skill use ") {
            let name = name.trim();
            runtime
                .get_skill(name)?
                .ok_or_else(|| cli_error(format!("skill not found: {name}")))?;
            if !sticky_skills.iter().any(|active| active == name) {
                sticky_skills.push(name.into());
            }
            println!("active skill={name}");
        } else if let Some(name) = line.strip_prefix("/skill show ") {
            print_json(
                &runtime
                    .get_skill(name.trim())?
                    .ok_or_else(|| cli_error(format!("skill not found: {name}")))?,
            )?;
        } else if let Some(name) = line.strip_prefix("/skill resources ") {
            print_json(&runtime.skill_resources(name.trim(), &sticky_skills).await?)?;
        } else if let Some(arguments) = line.strip_prefix("/skill read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /skill read NAME PATH"))?;
            print_json(
                &runtime
                    .read_skill_resource(name, path.trim(), &sticky_skills)
                    .await?,
            )?;
        } else if line == "/context" || line == "/context status" {
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .context_status(&runtime.context_status(&active_session_id).await?)
            );
        } else if line == "/context list" {
            print_json(&runtime.context_snapshots(&active_session_id).await?)?;
        } else if line == "/context compact" {
            print_json(&runtime.compact_context(&active_session_id).await?)?;
        } else if let Some(snapshot_id) = line.strip_prefix("/context restore ") {
            print_json(
                &runtime
                    .restore_context(&active_session_id, snapshot_id.trim())
                    .await?,
            )?;
        } else if line == "/session" || line == "/session show" {
            print_json(
                &runtime
                    .get_session(&active_session_id)?
                    .ok_or_else(|| cli_error("active session disappeared"))?,
            )?;
        } else if line == "/session new" {
            active_session_id = runtime.create_session(None)?.id;
            println!("session={active_session_id}");
        } else if line == "/session resume" || line == "/resume" || line.starts_with("/resume ") {
            let limit = if line == "/session resume" {
                10
            } else {
                line.strip_prefix("/resume ")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::parse::<usize>)
                    .transpose()?
                    .unwrap_or(10)
                    .clamp(1, 100)
            };
            match choose_session(runtime, &mut scripted_input, limit)? {
                SessionPickerInput::Selected(session_id) => {
                    active_session_id = session_id;
                    println!("session={active_session_id}");
                }
                SessionPickerInput::Command(command) => pending_line = Some(command),
                SessionPickerInput::Cancelled => {}
                SessionPickerInput::Invalid => unreachable!("picker retries invalid input"),
            }
        } else if let Some(session_id) = line.strip_prefix("/session resume ") {
            let session_id = session_id.trim();
            active_session_id = runtime
                .get_session(session_id)?
                .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
                .id;
            println!("session={active_session_id}");
        } else if line.starts_with('/') {
            println!("unknown terminal command: {line}; use /help");
        } else {
            let (prompt, explicit_skills) = resolve_skill_mentions(line, &skill_names);
            if prompt.is_empty() {
                println!("Add a message after the @skill name.");
                continue;
            }
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let result = runtime
                .run_model_with_skills_stream(
                    "primary",
                    "You are Colossus.",
                    &prompt,
                    None,
                    Some(&active_session_id),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("run failed; terminal input remains available: {error}");
                    continue;
                }
            };
            observer.finish_response(&result.output)?;
        }
    }
    Ok(())
}

async fn dispatch_to_worker_if_active(
    config: &RuntimeConfig,
    config_path: &Path,
    command: &Command,
    approval_mode: Option<ApprovalMode>,
    no_alt_screen: bool,
) -> Result<bool, Box<dyn Error>> {
    let Some(client) = WorkerClient::discover(config)? else {
        return Ok(false);
    };
    match client.ping().await {
        Ok(_) => {}
        Err(error) if worker_probe_allows_embedded_fallback(&error) => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    if approval_mode.is_some() {
        return Err(
            "an active worker owns approval handling; restart it with the desired --approval-mode"
                .into(),
        );
    }
    match command {
        Command::Audit(command) => {
            match &command.command {
                AuditAction::Verify | AuditAction::AnchorStatus => {
                    print_json(&client.call(WorkerOperation::AuditVerify).await?)?;
                }
                AuditAction::Show { from, limit } => {
                    print_json(
                        &client
                            .call(WorkerOperation::AuditRead {
                                from: *from,
                                limit: *limit,
                            })
                            .await?,
                    )?;
                }
                AuditAction::Export { from, limit } => {
                    let events = client
                        .call(WorkerOperation::AuditRead {
                            from: *from,
                            limit: *limit,
                        })
                        .await?;
                    for event in events
                        .as_array()
                        .ok_or_else(|| cli_error("worker audit export is not an array"))?
                    {
                        println!("{}", serde_json::to_string(event)?);
                    }
                }
                AuditAction::ExporterStatus => {
                    print_json(&client.call(WorkerOperation::AuditExportStatus).await?)?;
                }
                AuditAction::ExporterDrain => {
                    print_json(&client.call(WorkerOperation::AuditExportDrain).await?)?;
                }
                AuditAction::ExporterReset => {
                    print_json(&client.call(WorkerOperation::AuditExportReset).await?)?;
                }
            }
            Ok(true)
        }
        Command::Policy(command) => {
            match &command.command {
                PolicyAction::Doctor => {
                    print_json(&client.call(WorkerOperation::PolicyDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Projection(command) => {
            let operation = match &command.command {
                ProjectionAction::Status => WorkerOperation::ProjectionStatus,
                ProjectionAction::Drain => WorkerOperation::ProjectionDrain,
                ProjectionAction::Rebuild { name } => {
                    WorkerOperation::ProjectionRebuild { name: name.clone() }
                }
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::State(command) => {
            match &command.command {
                StateAction::Doctor => {
                    print_json(&client.call(WorkerOperation::StateDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Sandbox(command) => {
            match &command.command {
                SandboxAction::Doctor => {
                    print_json(&client.call(WorkerOperation::SandboxDoctor).await?)?;
                }
            }
            Ok(true)
        }
        Command::Provider(command) => {
            let operation = match &command.command {
                ProviderAction::Profiles => WorkerOperation::ProviderProfiles,
                ProviderAction::Doctor { profile } => WorkerOperation::ProviderDoctor {
                    profile: profile.clone(),
                },
                ProviderAction::Models { profile } => WorkerOperation::ProviderModels {
                    profile: profile.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Models(command) => {
            match &command.command {
                ModelsAction::Routes => {
                    print_json(&client.call(WorkerOperation::ProviderRoutes).await?)?;
                }
                ModelsAction::Route { role } => {
                    print_json(
                        &client
                            .call(WorkerOperation::ProviderRoute { role: role.clone() })
                            .await?,
                    )?;
                }
            }
            Ok(true)
        }
        Command::Tools(command) => {
            match &command.command {
                ToolsAction::List => {
                    print_json(&client.call(WorkerOperation::ToolsList).await?)?;
                }
            }
            Ok(true)
        }
        Command::Process(command) => {
            let operation = match &command.command {
                ProcessAction::Run {
                    executable,
                    cwd,
                    environment,
                    args,
                } => WorkerOperation::ProcessRun {
                    executable: executable.to_string_lossy().into_owned(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    args: args.clone(),
                    environment: parse_environment(environment.clone())?,
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Network(command) => {
            let operation = match &command.command {
                NetworkAction::Get { url } => WorkerOperation::NetworkGet { url: url.clone() },
            };
            let result = client.call(operation).await?;
            let encoded = result
                .get("bytes_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| cli_error("worker network response has no bytes_base64"))?;
            println!("{}", String::from_utf8_lossy(&BASE64.decode(encoded)?));
            Ok(true)
        }
        Command::Run {
            prompt,
            plan,
            execute_plan,
            goal,
            goal_max_iterations,
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
            stream,
        } => {
            if execute_plan.is_some() && *stream {
                return Err(cli_error(
                    "--stream is not supported with --execute-plan; inspect the returned run JSON",
                )
                .into());
            }
            if let Some(plan_id) = execute_plan {
                let result = if *goal {
                    let plan = client
                        .call(WorkerOperation::PlanGet {
                            plan_id: plan_id.clone(),
                        })
                        .await?;
                    let session_id = plan
                        .get("session_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| cli_error("approved plan has no session id"))?;
                    client
                        .call(WorkerOperation::GoalRun {
                            role: role.clone(),
                            objective: String::new(),
                            session_id: session_id.into(),
                            max_iterations: *goal_max_iterations,
                            source_plan_id: Some(plan_id.clone()),
                        })
                        .await?
                } else {
                    client
                        .call(WorkerOperation::PlanRun {
                            role: role.clone(),
                            plan_id: plan_id.clone(),
                            max_turns: *max_turns,
                        })
                        .await?
                };
                client.call(WorkerOperation::Drain).await?;
                print_json(&result)?;
                return Ok(true);
            }
            let prompt = prompt
                .as_deref()
                .ok_or_else(|| cli_error("a prompt or --execute-plan is required"))?;
            let session_id = if *resume {
                Some(
                    serde_json::from_value::<colossus_contracts::SessionSummary>(
                        client.call(WorkerOperation::SessionLatest).await?,
                    )?
                    .id,
                )
            } else {
                session.clone()
            };
            let operation = if *plan {
                WorkerOperation::RunPlan {
                    role: role.clone(),
                    instructions: instructions.clone(),
                    prompt: prompt.into(),
                    max_turns: *max_turns,
                    session_id,
                    explicit_skills: skills.clone(),
                    sticky_skills: Vec::new(),
                }
            } else {
                WorkerOperation::RunModel {
                    role: role.clone(),
                    instructions: instructions.clone(),
                    prompt: prompt.into(),
                    max_turns: *max_turns,
                    session_id,
                    explicit_skills: skills.clone(),
                    sticky_skills: Vec::new(),
                }
            };
            let result = if *stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = client.run_model(operation, &mut observer).await;
                observer.finish_line()?;
                result?
            } else {
                let mut observer = SilentStreamObserver;
                client.run_model(operation, &mut observer).await?
            };
            client.call(WorkerOperation::Drain).await?;
            print_json(&result)?;
            Ok(true)
        }
        Command::Echo { message } => {
            let result = client
                .call(WorkerOperation::Echo {
                    message: message.clone(),
                })
                .await?;
            let encoded = result
                .get("bytes_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| cli_error("worker echo response has no bytes_base64"))?;
            let bytes = BASE64.decode(encoded)?;
            println!("{}", String::from_utf8_lossy(&bytes));
            Ok(true)
        }
        Command::Workflow(command) => {
            if let WorkflowAction::Webhook {
                command: WorkflowWebhookAction::Serve { bind },
            } = &command.command
            {
                serve_workflow_webhooks(*bind, WebhookIngressBackend::Worker(&client)).await?;
                return Ok(true);
            }
            let operation = match &command.command {
                WorkflowAction::Validate { path } => WorkerOperation::WorkflowValidate {
                    path: path.to_string_lossy().into_owned(),
                },
                WorkflowAction::Register { path } => WorkerOperation::WorkflowRegister {
                    path: path.to_string_lossy().into_owned(),
                },
                WorkflowAction::List => WorkerOperation::WorkflowList,
                WorkflowAction::Show { name, version } => WorkerOperation::WorkflowShow {
                    name: name.clone(),
                    version: version.clone(),
                },
                WorkflowAction::Run {
                    name,
                    version,
                    inputs,
                    queued,
                } => WorkerOperation::WorkflowStart {
                    name: name.clone(),
                    version: version.clone(),
                    inputs_source: inputs.clone(),
                    queued: *queued,
                },
                WorkflowAction::Schedule { command } => match command {
                    WorkflowScheduleAction::Create {
                        schedule_id,
                        name,
                        version,
                        cadence_seconds,
                        inputs,
                        misfire,
                        disabled,
                        starts_at,
                    } => WorkerOperation::WorkflowScheduleCreate {
                        schedule_id: schedule_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        inputs_source: inputs.clone(),
                        cadence_seconds: *cadence_seconds,
                        misfire_policy: (*misfire).into(),
                        enabled: !*disabled,
                        starts_at: starts_at.clone(),
                    },
                    WorkflowScheduleAction::List { limit } => {
                        WorkerOperation::WorkflowScheduleList { limit: *limit }
                    }
                    WorkflowScheduleAction::Show { schedule_id } => {
                        WorkerOperation::WorkflowScheduleShow {
                            schedule_id: schedule_id.clone(),
                        }
                    }
                    WorkflowScheduleAction::Enable { schedule_id } => {
                        WorkerOperation::WorkflowScheduleSetEnabled {
                            schedule_id: schedule_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowScheduleAction::Disable { schedule_id } => {
                        WorkerOperation::WorkflowScheduleSetEnabled {
                            schedule_id: schedule_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowScheduleAction::Tick { at } => {
                        WorkerOperation::WorkflowScheduleTick { at: at.clone() }
                    }
                },
                WorkflowAction::Webhook { command } => match command {
                    WorkflowWebhookAction::Create {
                        webhook_id,
                        name,
                        version,
                        secret_reference,
                        replay_window_seconds,
                        max_body_bytes,
                        disabled,
                    } => WorkerOperation::WorkflowWebhookCreate {
                        webhook_id: webhook_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        secret_reference: secret_reference.clone(),
                        replay_window_seconds: *replay_window_seconds,
                        max_body_bytes: *max_body_bytes,
                        enabled: !*disabled,
                    },
                    WorkflowWebhookAction::List { limit } => {
                        WorkerOperation::WorkflowWebhookList { limit: *limit }
                    }
                    WorkflowWebhookAction::Show { webhook_id } => {
                        WorkerOperation::WorkflowWebhookShow {
                            webhook_id: webhook_id.clone(),
                        }
                    }
                    WorkflowWebhookAction::Enable { webhook_id } => {
                        WorkerOperation::WorkflowWebhookSetEnabled {
                            webhook_id: webhook_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowWebhookAction::Disable { webhook_id } => {
                        WorkerOperation::WorkflowWebhookSetEnabled {
                            webhook_id: webhook_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowWebhookAction::Ingest {
                        webhook_id,
                        delivery_id,
                        timestamp,
                        signature,
                        headers,
                        body,
                    } => WorkerOperation::WorkflowWebhookIngest {
                        webhook_id: webhook_id.clone(),
                        delivery_id: delivery_id.clone(),
                        timestamp: timestamp.clone(),
                        signature: signature.clone(),
                        headers: parse_headers(headers.clone())?,
                        body_source: body.clone(),
                    },
                    WorkflowWebhookAction::Serve { .. } => {
                        unreachable!("webhook serve is handled before operation routing")
                    }
                },
                WorkflowAction::Subscription { command } => match command {
                    WorkflowSubscriptionAction::Create {
                        subscription_id,
                        name,
                        version,
                        event_type,
                        stream_prefix,
                        disabled,
                        after_sequence,
                    } => WorkerOperation::WorkflowSubscriptionCreate {
                        subscription_id: subscription_id.clone(),
                        name: name.clone(),
                        version: version.clone(),
                        event_type: event_type.clone(),
                        stream_prefix: stream_prefix.clone(),
                        enabled: !*disabled,
                        after_sequence: *after_sequence,
                    },
                    WorkflowSubscriptionAction::List { limit } => {
                        WorkerOperation::WorkflowSubscriptionList { limit: *limit }
                    }
                    WorkflowSubscriptionAction::Show { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionShow {
                            subscription_id: subscription_id.clone(),
                        }
                    }
                    WorkflowSubscriptionAction::Enable { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionSetEnabled {
                            subscription_id: subscription_id.clone(),
                            enabled: true,
                        }
                    }
                    WorkflowSubscriptionAction::Disable { subscription_id } => {
                        WorkerOperation::WorkflowSubscriptionSetEnabled {
                            subscription_id: subscription_id.clone(),
                            enabled: false,
                        }
                    }
                    WorkflowSubscriptionAction::Tick => WorkerOperation::WorkflowSubscriptionTick,
                },
                WorkflowAction::Status { run_id } => WorkerOperation::WorkflowStatus {
                    run_id: run_id.clone(),
                },
                WorkflowAction::Resume { run_id } => WorkerOperation::WorkflowResume {
                    run_id: run_id.clone(),
                },
                WorkflowAction::Input { run_id, input } => WorkerOperation::WorkflowInput {
                    run_id: run_id.clone(),
                    input_source: input.clone(),
                },
                WorkflowAction::Cancel { run_id } => WorkerOperation::WorkflowCancel {
                    run_id: run_id.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Sessions(command) => {
            let operation = match &command.command {
                SessionsAction::List { limit } => WorkerOperation::SessionList { limit: *limit },
                SessionsAction::Show { session_id } => WorkerOperation::SessionGet {
                    session_id: session_id.clone(),
                },
                SessionsAction::Messages { session_id } => WorkerOperation::SessionMessages {
                    session_id: session_id.clone(),
                },
                SessionsAction::New { title } => WorkerOperation::SessionCreate {
                    title: title.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, SessionsAction::Show { .. }) && result.is_null() {
                return Err("session not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Work { session } => {
            let session_id = if let Some(session_id) = session {
                session_id.clone()
            } else {
                client
                    .call(WorkerOperation::SessionLatest)
                    .await?
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| cli_error("worker latest session response has no id"))?
                    .to_owned()
            };
            print_json(
                &client
                    .call(WorkerOperation::WorkState { session_id })
                    .await?,
            )?;
            Ok(true)
        }
        Command::Context(command) => {
            let operation = match &command.command {
                ContextAction::Status { session_id } => WorkerOperation::ContextStatus {
                    session_id: session_id.clone(),
                },
                ContextAction::List { session_id } => WorkerOperation::ContextList {
                    session_id: session_id.clone(),
                },
                ContextAction::Compact { session_id } => WorkerOperation::ContextCompact {
                    session_id: session_id.clone(),
                },
                ContextAction::Restore {
                    session_id,
                    snapshot_id,
                } => WorkerOperation::ContextRestore {
                    session_id: session_id.clone(),
                    snapshot_id: snapshot_id.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Telemetry(command) => {
            let operation = match &command.command {
                TelemetryAction::Runs { session, limit } => WorkerOperation::TelemetryRuns {
                    session_id: session.clone(),
                    limit: *limit,
                },
                TelemetryAction::Show { run_id, limit } => WorkerOperation::TelemetryShow {
                    id_or_prefix: run_id.clone(),
                    limit: *limit,
                },
                TelemetryAction::Metrics { session, limit } => WorkerOperation::TelemetryMetrics {
                    session_id: session.clone(),
                    limit: *limit,
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Research(command) => {
            let operation = match &command.command {
                ResearchAction::Run {
                    question,
                    session,
                    depth,
                    sources,
                } => WorkerOperation::ResearchRun {
                    question: question.clone(),
                    session_id: session.clone(),
                    depth: (*depth).into(),
                    source_kinds: sources.iter().copied().map(Into::into).collect(),
                },
                ResearchAction::List { session, limit } => WorkerOperation::ResearchList {
                    session_id: session.clone(),
                    limit: *limit,
                },
                ResearchAction::Show { run_id } => WorkerOperation::ResearchGet {
                    run_id: run_id.clone(),
                },
                ResearchAction::Sources { run_id } => WorkerOperation::ResearchSources {
                    run_id: run_id.clone(),
                },
                ResearchAction::Claims { run_id } => WorkerOperation::ResearchClaims {
                    run_id: run_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, ResearchAction::Show { .. }) && result.is_null() {
                return Err("research run not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Skills(command) => {
            let operation = match &command.command {
                SkillsAction::List => WorkerOperation::SkillList,
                SkillsAction::Show { name } => WorkerOperation::SkillGet { name: name.clone() },
                SkillsAction::Duplicates => WorkerOperation::SkillDuplicates,
                SkillsAction::Compose { prompt, skills } => WorkerOperation::SkillCompose {
                    prompt: prompt.clone(),
                    skills: skills.clone(),
                },
                SkillsAction::Scaffold {
                    name,
                    description,
                    instructions,
                    resource_dirs,
                } => WorkerOperation::SkillScaffold {
                    name: name.clone(),
                    description: description.clone(),
                    instructions: instructions.clone().unwrap_or_else(|| {
                        format!("# {name}\n\nAdd data-only instructions here.\n")
                    }),
                    resource_dirs: resource_dirs.clone(),
                },
                SkillsAction::Inspect { name } => {
                    WorkerOperation::SkillInspect { name: name.clone() }
                }
                SkillsAction::FileRead { name, path } => WorkerOperation::SkillFileRead {
                    name: name.clone(),
                    path: path.clone(),
                },
                SkillsAction::Write {
                    name,
                    path,
                    content,
                    expected_sha256,
                } => WorkerOperation::SkillWrite {
                    name: name.clone(),
                    path: path.clone(),
                    content: content.clone(),
                    expected_sha256: expected_sha256.clone(),
                },
                SkillsAction::Validate { target, local } => WorkerOperation::SkillValidate {
                    target: target.clone(),
                    local: *local,
                },
                SkillsAction::Install { path } => {
                    WorkerOperation::SkillInstall { path: path.clone() }
                }
                SkillsAction::Resources { name } => {
                    WorkerOperation::SkillResources { name: name.clone() }
                }
                SkillsAction::Read { name, path } => WorkerOperation::SkillResourceRead {
                    name: name.clone(),
                    path: path.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, SkillsAction::Show { .. }) && result.is_null() {
                return Err("skill not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Packs(command) => {
            let operation = match &command.command {
                PacksAction::List { limit } => WorkerOperation::PackList { limit: *limit },
                PacksAction::Show { name } => WorkerOperation::PackGet { name: name.clone() },
                PacksAction::Verify { path } | PacksAction::Validate { path } => {
                    WorkerOperation::PackVerify {
                        path: path.to_string_lossy().into_owned(),
                    }
                }
                PacksAction::Install {
                    path,
                    allow_untrusted,
                } => WorkerOperation::PackInstall {
                    path: path.to_string_lossy().into_owned(),
                    allow_untrusted: *allow_untrusted,
                },
                PacksAction::Enable { name } => WorkerOperation::PackEnable { name: name.clone() },
                PacksAction::Disable { name } => {
                    WorkerOperation::PackDisable { name: name.clone() }
                }
                PacksAction::Uninstall { name } => {
                    WorkerOperation::PackUninstall { name: name.clone() }
                }
                PacksAction::Call { tool } => WorkerOperation::PackCall { tool: tool.clone() },
                PacksAction::Trust(command) => match &command.command {
                    PackTrustAction::List { limit } => {
                        WorkerOperation::PackTrustList { limit: *limit }
                    }
                    PackTrustAction::Add {
                        publisher,
                        public_key,
                    } => WorkerOperation::PackTrustAdd {
                        publisher: publisher.clone(),
                        public_key: public_key.clone(),
                    },
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, PacksAction::Show { .. }) && result.is_null() {
                return Err("pack not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Bundle(command) => {
            let operation = match &command.command {
                BundleAction::KeyInfo {
                    signing_key_reference,
                } => WorkerOperation::BundleKeyInfo {
                    signing_key_reference: signing_key_reference.clone(),
                },
                BundleAction::Verify { path } => WorkerOperation::BundleVerify {
                    path: path.to_string_lossy().into_owned(),
                },
                BundleAction::Build {
                    source,
                    destination,
                    name,
                    version,
                    publisher,
                    created_at,
                    source_revision,
                    signing_key_reference,
                } => WorkerOperation::BundleBuild {
                    source: source.to_string_lossy().into_owned(),
                    destination: destination.to_string_lossy().into_owned(),
                    name: name.clone(),
                    version: version.clone(),
                    publisher: publisher.clone(),
                    created_at: created_at.clone(),
                    source_revision: source_revision.clone(),
                    signing_key_reference: signing_key_reference.clone(),
                },
                BundleAction::Install { path, prefix } => WorkerOperation::BundleInstall {
                    path: path.to_string_lossy().into_owned(),
                    prefix: prefix.to_string_lossy().into_owned(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Integrations(command) => {
            let operation = match &command.command {
                IntegrationsAction::List { limit } => {
                    WorkerOperation::IntegrationList { limit: *limit }
                }
                IntegrationsAction::Show { name } => {
                    WorkerOperation::IntegrationGet { name: name.clone() }
                }
                IntegrationsAction::Connect {
                    name,
                    base_url,
                    auth_type,
                    credential_reference,
                    username_reference,
                    password_reference,
                    auth_header,
                    auth_scheme,
                    scopes,
                } => {
                    let mode = auth_type.unwrap_or(match name.as_str() {
                        "github" => IntegrationAuthMode::Bearer,
                        "searxng" if credential_reference.is_some() => IntegrationAuthMode::ApiKey,
                        _ => IntegrationAuthMode::None,
                    });
                    let mut credential_references = BTreeMap::new();
                    if let Some(reference) = username_reference {
                        credential_references.insert("username".into(), reference.clone());
                    }
                    if let Some(reference) = password_reference {
                        credential_references.insert("password".into(), reference.clone());
                    }
                    WorkerOperation::IntegrationConnect {
                        name: name.clone(),
                        base_url: base_url.clone(),
                        auth: integration_auth(mode, auth_header.clone(), auth_scheme.clone()),
                        credential_reference: credential_reference.clone(),
                        credential_references,
                        scopes: scopes.clone(),
                    }
                }
                IntegrationsAction::ImportOpenapi {
                    name,
                    spec,
                    base_url,
                    auth_type,
                    credential_reference,
                    auth_header,
                    auth_scheme,
                    scopes,
                } => WorkerOperation::IntegrationImportOpenApi {
                    name: name.clone(),
                    document_source: if spec.starts_with('@') {
                        spec.clone()
                    } else {
                        format!("@{spec}")
                    },
                    base_url: base_url.clone(),
                    auth: integration_auth(*auth_type, auth_header.clone(), auth_scheme.clone()),
                    credential_reference: credential_reference.clone(),
                    scopes: scopes.clone(),
                },
                IntegrationsAction::Disconnect { name } => {
                    WorkerOperation::IntegrationDisconnect { name: name.clone() }
                }
                IntegrationsAction::Call { tool, arguments } => WorkerOperation::IntegrationCall {
                    tool: tool.clone(),
                    arguments_source: arguments.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, IntegrationsAction::Show { .. }) && result.is_null() {
                return Err("integration not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Tasks(command) => {
            let operation = match &command.command {
                TasksAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::TaskList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                TasksAction::Show { task_id } => WorkerOperation::TaskGet {
                    task_id: task_id.clone(),
                },
                TasksAction::Create {
                    session_id,
                    title,
                    description,
                    status,
                } => WorkerOperation::TaskCreate {
                    session_id: session_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    status: (*status).into(),
                },
                TasksAction::Update {
                    task_id,
                    title,
                    description,
                    status,
                } => WorkerOperation::TaskUpdate {
                    task_id: task_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    status: status.map(Into::into),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, TasksAction::Show { .. }) && result.is_null() {
                return Err("task not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Decisions(command) => {
            let operation = match &command.command {
                DecisionsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::DecisionList {
                    session_id: session.clone(),
                    status: Some((*status).into()),
                    limit: *limit,
                },
                DecisionsAction::Show { decision_id } => WorkerOperation::DecisionGet {
                    decision_id: decision_id.clone(),
                },
                DecisionsAction::Create {
                    session_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionCreate {
                    session_id: session_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: (*priority).into(),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
                DecisionsAction::Update {
                    decision_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionUpdate {
                    decision_id: decision_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: priority.map(Into::into),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
                DecisionsAction::Archive { decision_id } => WorkerOperation::DecisionArchive {
                    decision_id: decision_id.clone(),
                },
                DecisionsAction::Supersede {
                    decision_id,
                    title,
                    decision,
                    priority,
                    intent,
                    applies_when,
                    rationale,
                    source_excerpt,
                } => WorkerOperation::DecisionSupersede {
                    decision_id: decision_id.clone(),
                    title: title.clone(),
                    decision: decision.clone(),
                    priority: (*priority).into(),
                    intent: intent.clone(),
                    applies_when: applies_when.clone(),
                    rationale: rationale.clone(),
                    source_excerpt: source_excerpt.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, DecisionsAction::Show { .. }) && result.is_null() {
                return Err("decision not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Plans(command) => {
            let operation = match &command.command {
                PlansAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::PlanList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                PlansAction::Show { plan_id } => WorkerOperation::PlanGet {
                    plan_id: plan_id.clone(),
                },
                PlansAction::Create {
                    session_id,
                    prompt,
                    content,
                    steps,
                } => WorkerOperation::PlanCreate {
                    session_id: session_id.clone(),
                    prompt: prompt.clone(),
                    content: content.clone(),
                    steps: steps
                        .iter()
                        .enumerate()
                        .map(|(index, title)| PlanStep {
                            index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                            title: title.clone(),
                            detail: String::new(),
                            requires_mutation: false,
                        })
                        .collect(),
                },
                PlansAction::Approve { plan_id } => WorkerOperation::PlanApprove {
                    plan_id: plan_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, PlansAction::Show { .. }) && result.is_null() {
                return Err("plan not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Goals(command) => {
            let operation = match &command.command {
                GoalsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::GoalList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                GoalsAction::Show { goal_id } => WorkerOperation::GoalGet {
                    goal_id: goal_id.clone(),
                },
                GoalsAction::Run {
                    objective,
                    session,
                    role,
                    max_iterations,
                    source_plan,
                } => WorkerOperation::GoalRun {
                    role: role.clone(),
                    objective: objective.clone(),
                    session_id: session.clone(),
                    max_iterations: *max_iterations,
                    source_plan_id: source_plan.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, GoalsAction::Show { .. }) && result.is_null() {
                return Err("goal not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Agents(command) => {
            let operation = match &command.command {
                AgentsAction::Queue {
                    session_id,
                    task,
                    role,
                } => WorkerOperation::AgentQueue {
                    session_id: session_id.clone(),
                    task: task.clone(),
                    role: role.clone(),
                },
                AgentsAction::List {
                    session,
                    status,
                    limit,
                } => WorkerOperation::AgentList {
                    session_id: session.clone(),
                    status: status.map(Into::into),
                    limit: *limit,
                },
                AgentsAction::Show { job_id } => WorkerOperation::AgentGet {
                    job_id: job_id.clone(),
                },
                AgentsAction::Status { session } => WorkerOperation::AgentStatus {
                    session_id: session.clone(),
                },
                AgentsAction::Drain => WorkerOperation::AgentDrain,
                AgentsAction::Cancel { job_id } => WorkerOperation::AgentCancel {
                    job_id: job_id.clone(),
                },
                AgentsAction::Requeue { job_id } => WorkerOperation::AgentRequeue {
                    job_id: job_id.clone(),
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, AgentsAction::Show { .. }) && result.is_null() {
                return Err("subagent not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Memories(command) => {
            let operation = match &command.command {
                MemoriesAction::List { status, limit } => WorkerOperation::MemoryList {
                    status: status.status(),
                    limit: *limit,
                },
                MemoriesAction::Show { memory_id } => WorkerOperation::MemoryGet {
                    memory_id: memory_id.clone(),
                },
                MemoriesAction::Search {
                    query,
                    session,
                    repository,
                    limit,
                } => WorkerOperation::MemorySearch {
                    query: query.clone(),
                    session_id: session.clone(),
                    repository_id: repository.clone(),
                    limit: *limit,
                },
                MemoriesAction::Create {
                    text,
                    scope,
                    scope_id,
                    kind,
                    confidence,
                    rationale,
                    expires_at,
                } => WorkerOperation::MemoryCreate {
                    scope: memory_scope(*scope, scope_id.clone())?,
                    memory_kind: kind.clone(),
                    confidence: *confidence,
                    text: text.clone(),
                    rationale: rationale.clone(),
                    expires_at: expires_at.clone(),
                },
                MemoriesAction::Archive { memory_id } => WorkerOperation::MemoryArchive {
                    memory_id: memory_id.clone(),
                },
                MemoriesAction::Supersede {
                    memory_id,
                    text,
                    rationale,
                } => WorkerOperation::MemorySupersede {
                    memory_id: memory_id.clone(),
                    text: text.clone(),
                    rationale: rationale.clone(),
                },
                MemoriesAction::Index(command) => match &command.command {
                    MemoryIndexAction::Status => WorkerOperation::MemoryIndexStatus,
                    MemoryIndexAction::Sync => WorkerOperation::MemoryIndexSync,
                    MemoryIndexAction::Rebuild => WorkerOperation::MemoryIndexRebuild,
                },
            };
            let result = client.call(operation).await?;
            if matches!(&command.command, MemoriesAction::Show { .. }) && result.is_null() {
                return Err("memory not found".into());
            }
            print_json(&result)?;
            Ok(true)
        }
        Command::Mcp(command) => {
            let operation = match &command.command {
                McpAction::Servers => WorkerOperation::McpServers,
                McpAction::Tools { server } => WorkerOperation::McpTools {
                    server: server.clone(),
                },
                McpAction::Call {
                    server,
                    tool,
                    arguments,
                } => WorkerOperation::McpCall {
                    server: server.clone(),
                    tool: tool.clone(),
                    arguments_source: arguments.clone(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Tui { session, resume } => {
            let themes = ThemeLibrary::load_for_config(config_path)?;
            if io::stdin().is_terminal() && io::stdout().is_terminal() {
                if output_mode() == OutputMode::Json {
                    return Err("interactive --output json is not supported; omit it for the TUI or redirect line-mode input".into());
                }
                let host = Arc::new(tui_host::WorkerInteractiveHost::new(
                    client,
                    themes,
                    ApprovalMode::Ask,
                ));
                run_tui(
                    host,
                    TuiOptions {
                        bootstrap: BootstrapRequest {
                            session_id: session.clone(),
                            resume_latest: *resume,
                        },
                        screen_mode: if no_alt_screen {
                            ScreenMode::Inline
                        } else {
                            ScreenMode::Alternate
                        },
                    },
                )
                .await?;
            } else {
                worker_line_runner(&client, session.clone(), *resume, &themes).await?;
            }
            Ok(true)
        }
        Command::Preferences(command) => {
            let operation = match command.command {
                PreferencesAction::Show => WorkerOperation::PresentationGet,
                PreferencesAction::History { limit } => {
                    WorkerOperation::PresentationHistory { limit }
                }
                PreferencesAction::Reset => WorkerOperation::PresentationSave {
                    preferences: TerminalPreferences::default(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Worker { .. } | Command::Config(_) | Command::SandboxHelper => Ok(false),
    }
}

fn worker_probe_allows_embedded_fallback(error: &colossus_worker::WorkerError) -> bool {
    matches!(error, colossus_worker::WorkerError::Unavailable(_))
}

async fn worker_line_runner(
    client: &WorkerClient,
    requested_session: Option<String>,
    resume: bool,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    if output_mode() == OutputMode::Auto {
        set_output_mode(OutputMode::Human);
    }
    let mut active_session_id = if let Some(session_id) = requested_session {
        let session = client
            .call(WorkerOperation::SessionGet {
                session_id: session_id.clone(),
            })
            .await?;
        if session.is_null() {
            return Err(format!("session not found: {session_id}").into());
        }
        session_id
    } else if resume {
        serde_json::from_value::<colossus_contracts::SessionSummary>(
            client.call(WorkerOperation::SessionLatest).await?,
        )?
        .id
    } else {
        serde_json::from_value::<colossus_contracts::SessionSummary>(
            client
                .call(WorkerOperation::SessionCreate { title: None })
                .await?,
        )?
        .id
    };
    let mut preferences = serde_json::from_value::<TerminalPreferences>(
        client.call(WorkerOperation::PresentationGet).await?,
    )?;
    set_terminal_preferences(&preferences);
    let mut history_entries = serde_json::from_value::<Vec<String>>(
        client
            .call(WorkerOperation::PresentationHistory {
                limit: TERMINAL_HISTORY_CAPACITY,
            })
            .await?,
    )?;
    let skill_names = client
        .call(WorkerOperation::SkillList)
        .await?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| skill.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err("interactive terminals must use the TUI".into());
    }
    let mut scripted_input = stdin.lock();
    let mut sticky_skills = Vec::<String>::new();
    let mut pending_line = None::<String>;
    println!("Colossus Rust line runner via authenticated worker. Type /help for commands.");
    loop {
        let line = if let Some(line) = pending_line.take() {
            line
        } else {
            let mut line = String::new();
            if scripted_input.read_line(&mut line)? == 0 {
                break;
            }
            line
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match client
            .call(WorkerOperation::PresentationHistoryAppend { entry: line.into() })
            .await
        {
            Ok(value) => match serde_json::from_value::<String>(value) {
                Ok(entry) => remember_history_entry(&mut history_entries, &entry),
                Err(error) => eprintln!("history was not persisted: {error}"),
            },
            Err(error) => eprintln!("history was not persisted: {error}"),
        }
        if matches!(line, "/quit" | "/exit") {
            break;
        }
        match handle_presentation_command(line, &mut preferences, themes)? {
            PresentationCommandResult::NotHandled => {}
            PresentationCommandResult::Handled => continue,
            PresentationCommandResult::Save => {
                preferences = serde_json::from_value(
                    client
                        .call(WorkerOperation::PresentationSave {
                            preferences: preferences.clone(),
                        })
                        .await?,
                )?;
                set_terminal_preferences(&preferences);
                if line.starts_with("/theme") {
                    print_theme_applied(&preferences, themes)?;
                } else {
                    print_json(&preferences)?;
                }
                continue;
            }
            PresentationCommandResult::ChooseTheme => {
                match choose_theme(&mut scripted_input, &preferences, themes)? {
                    ThemePickerInput::Selected(name) => {
                        themes.select(&name, &mut preferences)?;
                        preferences = serde_json::from_value(
                            client
                                .call(WorkerOperation::PresentationSave {
                                    preferences: preferences.clone(),
                                })
                                .await?,
                        )?;
                        set_terminal_preferences(&preferences);
                        print_theme_applied(&preferences, themes)?;
                    }
                    ThemePickerInput::Command(command) => pending_line = Some(command),
                    ThemePickerInput::Cancelled => {}
                    ThemePickerInput::Preview(_) | ThemePickerInput::Invalid => {
                        unreachable!("picker consumes preview and invalid input")
                    }
                }
                continue;
            }
        }
        if line == "/help" {
            print_terminal_help(&preferences);
        } else if line == "/workflow list" {
            print_json(&client.call(WorkerOperation::WorkflowList).await?)?;
        } else if line == "/workflow schedule list" {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowScheduleList { limit: 100 })
                    .await?,
            )?;
        } else if line == "/workflow subscription list" {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowSubscriptionList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(run_id) = line.strip_prefix("/workflow status ") {
            print_json(
                &client
                    .call(WorkerOperation::WorkflowStatus {
                        run_id: run_id.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/audit verify" {
            print_json(&client.call(WorkerOperation::AuditVerify).await?)?;
        } else if line == "/projection status" {
            print_json(&client.call(WorkerOperation::ProjectionStatus).await?)?;
        } else if line == "/tools" {
            print_json(&client.call(WorkerOperation::ToolsList).await?)?;
        } else if line == "/sessions" {
            print_json(
                &client
                    .call(WorkerOperation::SessionList { limit: 20 })
                    .await?,
            )?;
        } else if line == "/work" {
            let state = serde_json::from_value::<colossus_contracts::WorkStateSnapshot>(
                client
                    .call(WorkerOperation::WorkState {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .work_state(&state)
            );
        } else if line == "/tasks" {
            print_json(
                &client
                    .call(WorkerOperation::TaskList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/decisions" {
            print_json(
                &client
                    .call(WorkerOperation::DecisionList {
                        session_id: Some(active_session_id.clone()),
                        status: Some(DecisionStatus::Active),
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/plans" {
            print_json(
                &client
                    .call(WorkerOperation::PlanList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/goals" {
            print_json(
                &client
                    .call(WorkerOperation::GoalList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if let Some(objective) = line.strip_prefix("/goal ") {
            print_json(
                &client
                    .call(WorkerOperation::GoalRun {
                        role: "primary".into(),
                        objective: objective.trim().into(),
                        session_id: active_session_id.clone(),
                        max_iterations: 5,
                        source_plan_id: None,
                    })
                    .await?,
            )?;
        } else if line == "/agents" {
            print_json(
                &client
                    .call(WorkerOperation::AgentList {
                        session_id: Some(active_session_id.clone()),
                        status: None,
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if line == "/agents drain" {
            print_json(&client.call(WorkerOperation::AgentDrain).await?)?;
        } else if line == "/memories" {
            print_json(
                &client
                    .call(WorkerOperation::MemoryList {
                        status: Some(MemoryStatus::Active),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if let Some(query) = line.strip_prefix("/memory search ") {
            print_json(
                &client
                    .call(WorkerOperation::MemorySearch {
                        query: query.trim().into(),
                        session_id: Some(active_session_id.clone()),
                        repository_id: None,
                        limit: 8,
                    })
                    .await?,
            )?;
        } else if line == "/research list" {
            print_json(
                &client
                    .call(WorkerOperation::ResearchList {
                        session_id: Some(active_session_id.clone()),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if let Some(question) = line.strip_prefix("/research ") {
            print_json(
                &client
                    .call(WorkerOperation::ResearchRun {
                        question: question.trim().into(),
                        session_id: Some(active_session_id.clone()),
                        depth: ResearchDepth::Standard,
                        source_kinds: vec![
                            ResearchSourceKind::Repo,
                            ResearchSourceKind::Web,
                            ResearchSourceKind::Mcp,
                        ],
                    })
                    .await?,
            )?;
        } else if line == "/telemetry" {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryRuns {
                        session_id: Some(active_session_id.clone()),
                        limit: 20,
                    })
                    .await?,
            )?;
        } else if line == "/telemetry metrics" {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryMetrics {
                        session_id: Some(active_session_id.clone()),
                        limit: 100,
                    })
                    .await?,
            )?;
        } else if let Some(run_id) = line.strip_prefix("/telemetry ") {
            print_json(
                &client
                    .call(WorkerOperation::TelemetryShow {
                        id_or_prefix: run_id.trim().into(),
                        limit: 500,
                    })
                    .await?,
            )?;
        } else if line == "/packs" || line == "/packs list" {
            print_json(
                &client
                    .call(WorkerOperation::PackList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs show ") {
            let name = name.trim();
            let pack = client
                .call(WorkerOperation::PackGet { name: name.into() })
                .await?;
            if pack.is_null() {
                return Err(cli_error(format!("pack not found: {name}")).into());
            }
            print_json(&pack)?;
        } else if let Some(path) = line
            .strip_prefix("/packs verify ")
            .or_else(|| line.strip_prefix("/packs validate "))
        {
            print_json(
                &client
                    .call(WorkerOperation::PackVerify {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(value) = line.strip_prefix("/packs install ") {
            let value = value.trim();
            let (path, allow_untrusted) = value
                .strip_suffix(" --allow-untrusted")
                .map_or((value, false), |path| (path.trim(), true));
            print_json(
                &client
                    .call(WorkerOperation::PackInstall {
                        path: path.into(),
                        allow_untrusted,
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs enable ") {
            print_json(
                &client
                    .call(WorkerOperation::PackEnable {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs disable ") {
            print_json(
                &client
                    .call(WorkerOperation::PackDisable {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/packs uninstall ") {
            print_json(
                &client
                    .call(WorkerOperation::PackUninstall {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(tool) = line.strip_prefix("/packs call ") {
            print_json(
                &client
                    .call(WorkerOperation::PackCall {
                        tool: tool.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/packs trust" || line == "/packs trust list" {
            print_json(
                &client
                    .call(WorkerOperation::PackTrustList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(value) = line.strip_prefix("/packs trust add ") {
            let (publisher, public_key) = value
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY"))?;
            print_json(
                &client
                    .call(WorkerOperation::PackTrustAdd {
                        publisher: publisher.into(),
                        public_key: public_key.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(path) = line.strip_prefix("/bundle verify ") {
            print_json(
                &client
                    .call(WorkerOperation::BundleVerify {
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/integrations" {
            print_json(
                &client
                    .call(WorkerOperation::IntegrationList { limit: 100 })
                    .await?,
            )?;
        } else if let Some(name) = line.strip_prefix("/integration show ") {
            let name = name.trim();
            let integration = client
                .call(WorkerOperation::IntegrationGet { name: name.into() })
                .await?;
            if integration.is_null() {
                return Err(cli_error(format!("integration not found: {name}")).into());
            }
            print_json(&integration)?;
        } else if let Some(name) = line.strip_prefix("/integration disconnect ") {
            print_json(
                &client
                    .call(WorkerOperation::IntegrationDisconnect {
                        name: name.trim().into(),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/integration call ") {
            let (tool, arguments) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /integration call TOOL JSON"))?;
            print_json(
                &client
                    .call(WorkerOperation::IntegrationCall {
                        tool: tool.into(),
                        arguments_source: arguments.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/mcp servers" {
            print_json(&client.call(WorkerOperation::McpServers).await?)?;
        } else if line == "/mcp tools" {
            print_json(
                &client
                    .call(WorkerOperation::McpTools { server: None })
                    .await?,
            )?;
        } else if let Some(server) = line.strip_prefix("/mcp tools ") {
            print_json(
                &client
                    .call(WorkerOperation::McpTools {
                        server: Some(server.trim().into()),
                    })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/mcp call ") {
            let mut parts = arguments.trim().splitn(3, ' ');
            let server = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let tool = parts
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            let arguments_source = parts
                .next()
                .ok_or_else(|| cli_error("usage: /mcp call SERVER TOOL JSON"))?;
            print_json(
                &client
                    .call(WorkerOperation::McpCall {
                        server: server.into(),
                        tool: tool.into(),
                        arguments_source: arguments_source.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/skills" {
            let mut skills = client.call(WorkerOperation::SkillList).await?;
            if let Some(skills) = skills.as_array_mut() {
                for skill in skills {
                    let is_active = skill
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| sticky_skills.iter().any(|item| item == name));
                    if let Some(skill) = skill.as_object_mut() {
                        skill.insert("active".into(), Value::Bool(is_active));
                    }
                }
            }
            print_json(&skills)?;
        } else if line == "/skill active" {
            if sticky_skills.is_empty() {
                println!("No skills are active.");
            } else {
                println!("Active skills: {}", sticky_skills.join(", "));
            }
        } else if line == "/skill clear" {
            sticky_skills.clear();
            println!("active skills cleared");
        } else if let Some(name) = line.strip_prefix("/skill use ") {
            let name = name.trim();
            if name.is_empty() {
                return Err("skill name is required".into());
            }
            let skill = client
                .call(WorkerOperation::SkillGet { name: name.into() })
                .await?;
            if skill.is_null() {
                return Err(cli_error(format!("skill not found: {name}")).into());
            }
            if !sticky_skills.iter().any(|active| active == name) {
                sticky_skills.push(name.into());
            }
            println!("active skill={name}");
        } else if let Some(name) = line.strip_prefix("/skill show ") {
            let name = name.trim();
            let skill = client
                .call(WorkerOperation::SkillGet { name: name.into() })
                .await?;
            if skill.is_null() {
                return Err(cli_error(format!("skill not found: {name}")).into());
            }
            print_json(&skill)?;
        } else if let Some(name) = line.strip_prefix("/skill resources ") {
            let name = name.trim();
            if !sticky_skills.iter().any(|active| active == name) {
                return Err(cli_error(format!("skill is not active: {name}")).into());
            }
            print_json(
                &client
                    .call(WorkerOperation::SkillResources { name: name.into() })
                    .await?,
            )?;
        } else if let Some(arguments) = line.strip_prefix("/skill read ") {
            let (name, path) = arguments
                .trim()
                .split_once(' ')
                .ok_or_else(|| cli_error("usage: /skill read NAME PATH"))?;
            if !sticky_skills.iter().any(|active| active == name) {
                return Err(cli_error(format!("skill is not active: {name}")).into());
            }
            print_json(
                &client
                    .call(WorkerOperation::SkillResourceRead {
                        name: name.into(),
                        path: path.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/context" || line == "/context status" {
            let status = serde_json::from_value::<colossus_contracts::ContextStatus>(
                client
                    .call(WorkerOperation::ContextStatus {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
            println!(
                "{}",
                SemanticRenderer::new(preferences.clone())
                    .with_color(io::stdout().is_terminal())
                    .context_status(&status)
            );
        } else if line == "/context list" {
            print_json(
                &client
                    .call(WorkerOperation::ContextList {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
        } else if line == "/context compact" {
            print_json(
                &client
                    .call(WorkerOperation::ContextCompact {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
        } else if let Some(snapshot_id) = line.strip_prefix("/context restore ") {
            print_json(
                &client
                    .call(WorkerOperation::ContextRestore {
                        session_id: active_session_id.clone(),
                        snapshot_id: snapshot_id.trim().into(),
                    })
                    .await?,
            )?;
        } else if line == "/session" || line == "/session show" {
            print_json(
                &client
                    .call(WorkerOperation::SessionGet {
                        session_id: active_session_id.clone(),
                    })
                    .await?,
            )?;
        } else if line == "/session new" {
            active_session_id = serde_json::from_value::<colossus_contracts::SessionSummary>(
                client
                    .call(WorkerOperation::SessionCreate { title: None })
                    .await?,
            )?
            .id;
            println!("session={active_session_id}");
        } else if line == "/session resume" || line == "/resume" || line.starts_with("/resume ") {
            let limit = if line == "/session resume" {
                10
            } else {
                line.strip_prefix("/resume ")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::parse::<usize>)
                    .transpose()?
                    .unwrap_or(10)
                    .clamp(1, 100)
            };
            match choose_worker_session(client, &mut scripted_input, limit).await? {
                SessionPickerInput::Selected(session_id) => {
                    active_session_id = session_id;
                    println!("session={active_session_id}");
                }
                SessionPickerInput::Command(command) => pending_line = Some(command),
                SessionPickerInput::Cancelled => {}
                SessionPickerInput::Invalid => unreachable!("picker retries invalid input"),
            }
        } else if let Some(session_id) = line.strip_prefix("/session resume ") {
            let session_id = session_id.trim();
            let session = client
                .call(WorkerOperation::SessionGet {
                    session_id: session_id.into(),
                })
                .await?;
            if session.is_null() {
                return Err(format!("session not found: {session_id}").into());
            }
            active_session_id = session_id.into();
            println!("session={active_session_id}");
        } else if line.starts_with('/') {
            println!("unknown terminal command: {line}; use /help");
        } else {
            let (prompt, explicit_skills) = resolve_skill_mentions(line, &skill_names);
            if prompt.is_empty() {
                println!("Add a message after the @skill name.");
                continue;
            }
            let mut observer =
                TerminalStreamObserver::with_preferences(StreamTarget::Stdout, preferences.clone());
            let result = client
                .run_model(
                    WorkerOperation::RunModel {
                        role: "primary".into(),
                        instructions: "You are Colossus.".into(),
                        prompt,
                        max_turns: None,
                        session_id: Some(active_session_id.clone()),
                        explicit_skills,
                        sticky_skills: sticky_skills.clone(),
                    },
                    &mut observer,
                )
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("run failed; terminal input remains available: {error}");
                    continue;
                }
            };
            observer.finish_response(&result.output)?;
            client.call(WorkerOperation::Drain).await?;
        }
    }
    Ok(())
}

async fn choose_worker_session(
    client: &WorkerClient,
    scripted_input: &mut dyn BufRead,
    limit: usize,
) -> Result<SessionPickerInput, Box<dyn Error>> {
    let mut sessions = serde_json::from_value::<Vec<colossus_contracts::SessionSummary>>(
        client
            .call(WorkerOperation::SessionList { limit: 100 })
            .await?,
    )?
    .into_iter()
    .filter(|session| session.message_count > 0)
    .collect::<Vec<_>>();
    sessions.truncate(limit);
    if sessions.is_empty() {
        println!("No sessions exist yet.");
        return Ok(SessionPickerInput::Cancelled);
    }
    println!("Choose a session to resume:");
    for (index, session) in sessions.iter().enumerate() {
        println!(
            "  {}. {}  {}  messages={}",
            index + 1,
            session.id,
            session.title.as_deref().unwrap_or("Untitled"),
            session.message_count
        );
    }
    println!(
        "Enter a number or exact session id (blank cancels; /command returns to the terminal)."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(SessionPickerInput::Cancelled);
        }
        let parsed = parse_session_picker_input(&choice, &sessions);
        if parsed != SessionPickerInput::Invalid {
            return Ok(parsed);
        }
        println!(
            "That is not one of the listed sessions. Enter 1-{}, an exact id, or leave it blank to cancel.",
            sessions.len()
        );
    }
}

#[cfg(not(windows))]
fn main() -> Result<(), Box<dyn Error>> {
    runtime_main()
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn Error>> {
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

#[tokio::main]
async fn runtime_main() -> Result<(), Box<dyn Error>> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if error.kind() == ErrorKind::MissingSubcommand => {
            let mut arguments = std::env::args_os().collect::<Vec<_>>();
            arguments.push("tui".into());
            Cli::parse_from(arguments)
        }
        Err(error) => error.exit(),
    };
    set_output_mode(cli.output);
    if matches!(cli.command, Command::SandboxHelper) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    if let Command::Config(ConfigCommand {
        command: ConfigAction::Init { development, from },
    }) = &cli.command
    {
        return init_config(&cli.config, *development, from.as_deref());
    }
    let config = RuntimeConfig::from_path(&cli.config)?;
    if matches!(
        cli.command,
        Command::Config(ConfigCommand {
            command: ConfigAction::Show
        })
    ) {
        print!("{}", config.to_yaml()?);
        return Ok(());
    }
    match &cli.command {
        Command::Worker {
            once: false,
            shutdown: false,
            status: false,
        } => {
            let mode = match cli.approval_mode.unwrap_or(ApprovalMode::Ask) {
                ApprovalMode::Deny => WorkerApprovalMode::Deny,
                ApprovalMode::Ask => WorkerApprovalMode::Ask,
                ApprovalMode::RiskAuto => WorkerApprovalMode::RiskAuto,
                ApprovalMode::FullAccess => WorkerApprovalMode::FullAccess,
            };
            let server = WorkerServer::open_with_mode(&config, mode)?;
            eprintln!("worker listening on {}", server.endpoint());
            server.serve().await?;
            return Ok(());
        }
        Command::Worker { shutdown: true, .. } => {
            let client = WorkerClient::from_config(&config)?;
            print_json(&client.call(WorkerOperation::Shutdown).await?)?;
            return Ok(());
        }
        Command::Worker { status: true, .. } => {
            let client = WorkerClient::from_config(&config)?;
            print_json(&client.ping().await?)?;
            return Ok(());
        }
        _ => {}
    }
    if dispatch_to_worker_if_active(
        &config,
        &cli.config,
        &cli.command,
        cli.approval_mode,
        cli.no_alt_screen,
    )
    .await?
    {
        return Ok(());
    }
    let interactive_tui = matches!(&cli.command, Command::Tui { .. })
        && io::stdin().is_terminal()
        && io::stdout().is_terminal();
    if interactive_tui && cli.output == OutputMode::Json {
        return Err(
            "interactive --output json is not supported; omit it for the TUI or redirect line-mode input"
                .into(),
        );
    }
    let prompt_router = interactive_tui.then(|| Arc::new(tui_host::TuiPromptRouter::default()));
    let configured_approval = cli.approval_mode.unwrap_or(ApprovalMode::Ask);
    let approvals: Arc<dyn ApprovalProvider> = if let Some(router) = prompt_router.as_ref()
        && matches!(
            configured_approval,
            ApprovalMode::Ask | ApprovalMode::RiskAuto
        ) {
        Arc::new(tui_host::TuiApprovalProvider {
            router: Arc::clone(router),
            risk_auto: configured_approval == ApprovalMode::RiskAuto,
        })
    } else {
        approval_provider(&cli.command, cli.approval_mode)
    };
    let user_prompts: Option<Arc<dyn UserPromptProvider>> =
        if let Some(router) = prompt_router.as_ref() {
            Some(Arc::new(tui_host::TuiUserPromptProvider {
                router: Arc::clone(router),
            }))
        } else if matches!(&cli.command, Command::Tui { .. }) && io::stdin().is_terminal() {
            Some(Arc::new(TerminalUserPrompt {
                lock: Mutex::new(()),
            }))
        } else {
            None
        };
    let runtime = Arc::new(Runtime::open_with_interfaces(
        &config,
        approvals,
        user_prompts,
    )?);
    match cli.command {
        Command::Config(_) => unreachable!("handled before runtime construction"),
        Command::Preferences(command) => match command.command {
            PreferencesAction::Show => print_json(&runtime.presentation_preferences()?)?,
            PreferencesAction::History { limit } => {
                print_json(&runtime.terminal_history(limit.clamp(1, TERMINAL_HISTORY_CAPACITY))?)?
            }
            PreferencesAction::Reset => print_json(
                &runtime
                    .save_presentation_preferences(TerminalPreferences::default())
                    .await?,
            )?,
        },
        Command::Audit(command) => match command.command {
            AuditAction::Verify | AuditAction::AnchorStatus => {
                print_json(&runtime.journal().verify()?)?;
            }
            AuditAction::Show { from, limit } => {
                print_json(&runtime.journal().read_global(from, limit)?)?;
            }
            AuditAction::Export { from, limit } => {
                for event in runtime.journal().read_global(from, limit)? {
                    println!("{}", serde_json::to_string(&event)?);
                }
            }
            AuditAction::ExporterStatus => print_json(&runtime.audit_export_status()?)?,
            AuditAction::ExporterDrain => print_json(&runtime.drain_audit_exports().await?)?,
            AuditAction::ExporterReset => print_json(&runtime.reset_audit_exports()?)?,
        },
        Command::Policy(command) => match command.command {
            PolicyAction::Doctor => print_json(&runtime.policy_doctor().await?)?,
        },
        Command::Projection(command) => match command.command {
            ProjectionAction::Status => print_json(&runtime.projection_status()?)?,
            ProjectionAction::Drain => print_json(&runtime.drain_projections()?)?,
            ProjectionAction::Rebuild { name } => {
                print_json(&runtime.rebuild_projection(name.as_deref())?)?;
            }
        },
        Command::State(command) => match command.command {
            StateAction::Doctor => print_json(&runtime.state_doctor()?)?,
        },
        Command::Sandbox(command) => match command.command {
            SandboxAction::Doctor => print_json(&runtime.sandbox_doctor())?,
        },
        Command::Process(command) => match command.command {
            ProcessAction::Run {
                executable,
                cwd,
                environment,
                args,
            } => print_json(
                &runtime
                    .run_process(executable, cwd, args, parse_environment(environment)?)
                    .await?,
            )?,
        },
        Command::Network(command) => match command.command {
            NetworkAction::Get { url } => {
                let result = runtime.http_get(&url).await?;
                println!("{}", String::from_utf8_lossy(&result.bytes));
            }
        },
        Command::Workflow(command) => workflow_command(&runtime, command.command).await?,
        Command::Provider(command) => match command.command {
            ProviderAction::Profiles => print_json(&runtime.provider_profiles())?,
            ProviderAction::Doctor { profile } => {
                print_json(&runtime.provider_doctor(profile.as_deref()).await?)?;
            }
            ProviderAction::Models { profile } => {
                print_json(&runtime.provider_models(profile.as_deref()).await?)?;
            }
        },
        Command::Models(command) => match command.command {
            ModelsAction::Routes => print_json(&runtime.provider_routes())?,
            ModelsAction::Route { role } => print_json(&runtime.provider_route(&role)?)?,
        },
        Command::Tools(command) => match command.command {
            ToolsAction::List => print_json(&runtime.tool_specs())?,
        },
        Command::Sessions(command) => match command.command {
            SessionsAction::List { limit } => print_json(&runtime.list_sessions(limit)?)?,
            SessionsAction::Show { session_id } => print_json(
                &runtime
                    .get_session(&session_id)?
                    .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?,
            )?,
            SessionsAction::Messages { session_id } => {
                print_json(&runtime.session_messages(&session_id)?)?;
            }
            SessionsAction::New { title } => {
                print_json(&runtime.create_session(title.as_deref())?)?;
            }
        },
        Command::Work { session } => {
            let session_id = session
                .map(Ok)
                .unwrap_or_else(|| runtime.latest_session().map(|session| session.id))?;
            print_json(&runtime.work_state(&session_id)?)?;
        }
        Command::Context(command) => match command.command {
            ContextAction::Status { session_id } => {
                print_json(&runtime.context_status(&session_id).await?)?;
            }
            ContextAction::List { session_id } => {
                print_json(&runtime.context_snapshots(&session_id).await?)?;
            }
            ContextAction::Compact { session_id } => {
                print_json(&runtime.compact_context(&session_id).await?)?;
            }
            ContextAction::Restore {
                session_id,
                snapshot_id,
            } => print_json(&runtime.restore_context(&session_id, &snapshot_id).await?)?,
        },
        Command::Tasks(command) => match command.command {
            TasksAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_tasks(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            TasksAction::Show { task_id } => print_json(
                &runtime
                    .get_task(&task_id)?
                    .ok_or_else(|| cli_error(format!("task not found: {task_id}")))?,
            )?,
            TasksAction::Create {
                session_id,
                title,
                description,
                status,
            } => print_json(
                &runtime
                    .create_task(&session_id, &title, &description, status.into())
                    .await?,
            )?,
            TasksAction::Update {
                task_id,
                title,
                description,
                status,
            } => print_json(
                &runtime
                    .update_task(
                        &task_id,
                        title.as_deref(),
                        description.as_deref(),
                        status.map(Into::into),
                    )
                    .await?,
            )?,
        },
        Command::Decisions(command) => match command.command {
            DecisionsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_decisions(
                session.as_deref(),
                Some(status.into()),
                limit,
            )?)?,
            DecisionsAction::Show { decision_id } => print_json(
                &runtime
                    .get_decision(&decision_id)?
                    .ok_or_else(|| cli_error(format!("decision not found: {decision_id}")))?,
            )?,
            DecisionsAction::Create {
                session_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .create_decision(
                        &session_id,
                        &title,
                        &decision,
                        priority.into(),
                        &intent,
                        &applies_when,
                        &rationale,
                        &source_excerpt,
                    )
                    .await?,
            )?,
            DecisionsAction::Update {
                decision_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .update_decision(
                        &decision_id,
                        title.as_deref(),
                        decision.as_deref(),
                        priority.map(Into::into),
                        intent.as_deref(),
                        applies_when.as_deref(),
                        rationale.as_deref(),
                        source_excerpt.as_deref(),
                    )
                    .await?,
            )?,
            DecisionsAction::Archive { decision_id } => {
                print_json(&runtime.archive_decision(&decision_id).await?)?;
            }
            DecisionsAction::Supersede {
                decision_id,
                title,
                decision,
                priority,
                intent,
                applies_when,
                rationale,
                source_excerpt,
            } => print_json(
                &runtime
                    .supersede_decision(
                        &decision_id,
                        &title,
                        &decision,
                        priority.into(),
                        &intent,
                        &applies_when,
                        &rationale,
                        &source_excerpt,
                    )
                    .await?,
            )?,
        },
        Command::Plans(command) => match command.command {
            PlansAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_plans(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            PlansAction::Show { plan_id } => print_json(
                &runtime
                    .get_plan(&plan_id)?
                    .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?,
            )?,
            PlansAction::Create {
                session_id,
                prompt,
                content,
                steps,
            } => {
                let steps = steps
                    .into_iter()
                    .enumerate()
                    .map(|(index, title)| PlanStep {
                        index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                        title,
                        detail: String::new(),
                        requires_mutation: false,
                    })
                    .collect();
                print_json(
                    &runtime
                        .create_plan(&session_id, &prompt, &content, steps)
                        .await?,
                )?;
            }
            PlansAction::Approve { plan_id } => {
                print_json(&runtime.approve_plan(&plan_id).await?)?;
            }
        },
        Command::Goals(command) => match command.command {
            GoalsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_goals(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            GoalsAction::Show { goal_id } => print_json(
                &runtime
                    .get_goal(&goal_id)?
                    .ok_or_else(|| cli_error(format!("goal not found: {goal_id}")))?,
            )?,
            GoalsAction::Run {
                objective,
                session,
                role,
                max_iterations,
                source_plan,
            } => print_json(
                &runtime
                    .run_goal(
                        &role,
                        &objective,
                        &session,
                        max_iterations,
                        source_plan.as_deref(),
                    )
                    .await?,
            )?,
        },
        Command::Agents(command) => match command.command {
            AgentsAction::Queue {
                session_id,
                task,
                role,
            } => print_json(&runtime.queue_subagent(&session_id, &task, &role).await?)?,
            AgentsAction::List {
                session,
                status,
                limit,
            } => print_json(&runtime.list_subagents(
                session.as_deref(),
                status.map(Into::into),
                limit,
            )?)?,
            AgentsAction::Show { job_id } => print_json(
                &runtime
                    .get_subagent(&job_id)?
                    .ok_or_else(|| cli_error(format!("subagent not found: {job_id}")))?,
            )?,
            AgentsAction::Status { session } => {
                print_json(&runtime.subagent_queue_status(session.as_deref())?)?;
            }
            AgentsAction::Drain => print_json(&runtime.drain_subagents().await?)?,
            AgentsAction::Cancel { job_id } => {
                print_json(&runtime.cancel_subagent(&job_id).await?)?;
            }
            AgentsAction::Requeue { job_id } => {
                print_json(&runtime.requeue_subagent(&job_id).await?)?;
            }
        },
        Command::Memories(command) => match command.command {
            MemoriesAction::List { status, limit } => {
                print_json(&runtime.list_memories(status.status(), limit).await?)?;
            }
            MemoriesAction::Show { memory_id } => print_json(
                &runtime
                    .get_memory(&memory_id)
                    .await?
                    .ok_or_else(|| cli_error(format!("memory not found: {memory_id}")))?,
            )?,
            MemoriesAction::Search {
                query,
                session,
                repository,
                limit,
            } => print_json(
                &runtime
                    .search_memories(&query, session.as_deref(), repository.as_deref(), limit)
                    .await?,
            )?,
            MemoriesAction::Create {
                text,
                scope,
                scope_id,
                kind,
                confidence,
                rationale,
                expires_at,
            } => print_json(
                &runtime
                    .create_memory(
                        memory_scope(scope, scope_id)?,
                        &kind,
                        confidence,
                        &text,
                        &rationale,
                        expires_at,
                    )
                    .await?,
            )?,
            MemoriesAction::Archive { memory_id } => {
                print_json(&runtime.archive_memory(&memory_id).await?)?;
            }
            MemoriesAction::Supersede {
                memory_id,
                text,
                rationale,
            } => print_json(
                &runtime
                    .supersede_memory(&memory_id, &text, &rationale)
                    .await?,
            )?,
            MemoriesAction::Index(command) => match command.command {
                MemoryIndexAction::Status => {
                    print_json(&runtime.memory_index_status().await?)?;
                }
                MemoryIndexAction::Sync => {
                    print_json(&runtime.sync_memory_index().await?)?;
                }
                MemoryIndexAction::Rebuild => {
                    print_json(&runtime.rebuild_memory_index().await?)?;
                }
            },
        },
        Command::Research(command) => match command.command {
            ResearchAction::Run {
                question,
                session,
                depth,
                sources,
            } => {
                let session_id = match session {
                    Some(session_id) => {
                        runtime
                            .get_session(&session_id)?
                            .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
                            .id
                    }
                    None => runtime.create_session(Some("Research"))?.id,
                };
                print_json(
                    &runtime
                        .run_research(
                            &session_id,
                            &question,
                            depth.into(),
                            sources.into_iter().map(Into::into).collect(),
                        )
                        .await?,
                )?;
            }
            ResearchAction::List { session, limit } => {
                print_json(&runtime.list_research_runs(session.as_deref(), limit)?)?;
            }
            ResearchAction::Show { run_id } => print_json(
                &runtime
                    .get_research_run(&run_id)?
                    .ok_or_else(|| cli_error(format!("research run not found: {run_id}")))?,
            )?,
            ResearchAction::Sources { run_id } => {
                print_json(&runtime.research_sources(&run_id)?)?;
            }
            ResearchAction::Claims { run_id } => {
                print_json(&runtime.research_claims(&run_id)?)?;
            }
        },
        Command::Telemetry(command) => match command.command {
            TelemetryAction::Runs { session, limit } => {
                print_json(&runtime.telemetry_runs(session.as_deref(), limit)?)?;
            }
            TelemetryAction::Show { run_id, limit } => {
                print_json(&runtime.telemetry_run(&run_id, limit)?)?;
            }
            TelemetryAction::Metrics { session, limit } => {
                print_json(&runtime.telemetry_metrics(session.as_deref(), limit)?)?;
            }
        },
        Command::Skills(command) => match command.command {
            SkillsAction::List => {
                let skills = runtime
                    .list_skills()?
                    .into_iter()
                    .map(|skill| {
                        json!({
                            "name": skill.manifest.name,
                            "version": skill.manifest.version,
                            "description": skill.manifest.description,
                            "offline_compatible": skill.manifest.offline_compatible,
                            "source": skill.source,
                        })
                    })
                    .collect::<Vec<_>>();
                print_json(&skills)?;
            }
            SkillsAction::Show { name } => print_json(
                &runtime
                    .get_skill(&name)?
                    .ok_or_else(|| cli_error(format!("skill not found: {name}")))?,
            )?,
            SkillsAction::Duplicates => print_json(&runtime.skill_duplicates()?)?,
            SkillsAction::Compose { prompt, skills } => {
                print_json(&runtime.compose_skills("You are Colossus.", &prompt, &skills, &[])?)?
            }
            SkillsAction::Scaffold {
                name,
                description,
                instructions,
                resource_dirs,
            } => {
                let instructions = instructions
                    .unwrap_or_else(|| format!("# {name}\n\nAdd data-only instructions here.\n"));
                print_json(
                    &runtime
                        .scaffold_skill(&name, &description, &instructions, &resource_dirs)
                        .await?,
                )?;
            }
            SkillsAction::Inspect { name } => {
                print_json(&runtime.inspect_skill(&name).await?)?;
            }
            SkillsAction::FileRead { name, path } => {
                print_json(&runtime.read_skill_file(&name, &path).await?)?;
            }
            SkillsAction::Write {
                name,
                path,
                content,
                expected_sha256,
            } => {
                print_json(
                    &runtime
                        .write_skill_file(&name, &path, &content, expected_sha256.as_deref())
                        .await?,
                )?;
            }
            SkillsAction::Validate { target, local } => {
                if local {
                    print_json(&runtime.validate_local_skill(&target).await?)?;
                } else {
                    print_json(&runtime.validate_installed_skill(&target).await?)?;
                }
            }
            SkillsAction::Install { path } => {
                print_json(&runtime.install_local_skill(&path).await?)?;
            }
            SkillsAction::Resources { name } => {
                print_json(
                    &runtime
                        .skill_resources(&name, std::slice::from_ref(&name))
                        .await?,
                )?;
            }
            SkillsAction::Read { name, path } => print_json(
                &runtime
                    .read_skill_resource(&name, &path, std::slice::from_ref(&name))
                    .await?,
            )?,
        },
        Command::Packs(command) => match command.command {
            PacksAction::List { limit } => print_json(&runtime.list_packs(limit)?)?,
            PacksAction::Show { name } => print_json(
                &runtime
                    .get_pack(&name)?
                    .ok_or_else(|| cli_error(format!("pack not found: {name}")))?,
            )?,
            PacksAction::Verify { path } | PacksAction::Validate { path } => {
                print_json(&runtime.verify_pack(path).await?)?;
            }
            PacksAction::Install {
                path,
                allow_untrusted,
            } => print_json(&runtime.install_pack(path, allow_untrusted).await?)?,
            PacksAction::Enable { name } => print_json(&runtime.enable_pack(&name).await?)?,
            PacksAction::Disable { name } => print_json(&runtime.disable_pack(&name).await?)?,
            PacksAction::Uninstall { name } => {
                print_json(&runtime.uninstall_pack(&name).await?)?;
            }
            PacksAction::Call { tool } => print_json(&runtime.call_pack_tool(&tool).await?)?,
            PacksAction::Trust(command) => match command.command {
                PackTrustAction::List { limit } => {
                    print_json(&runtime.list_pack_trust(limit)?)?;
                }
                PackTrustAction::Add {
                    publisher,
                    public_key,
                } => print_json(&runtime.add_pack_trust(&publisher, &public_key).await?)?,
            },
        },
        Command::Bundle(command) => match command.command {
            BundleAction::KeyInfo {
                signing_key_reference,
            } => print_json(
                &runtime
                    .bundle_signing_key_info(&signing_key_reference)
                    .await?,
            )?,
            BundleAction::Verify { path } => print_json(&runtime.verify_bundle(path).await?)?,
            BundleAction::Build {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                source_revision,
                signing_key_reference,
            } => print_json(
                &runtime
                    .build_bundle(
                        source,
                        destination,
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        source_revision.as_deref(),
                        &signing_key_reference,
                    )
                    .await?,
            )?,
            BundleAction::Install { path, prefix } => {
                print_json(&runtime.install_bundle(path, prefix).await?)?
            }
        },
        Command::Integrations(command) => match command.command {
            IntegrationsAction::List { limit } => {
                print_json(&runtime.list_integrations(limit)?)?;
            }
            IntegrationsAction::Show { name } => print_json(
                &runtime
                    .get_integration(&name)?
                    .ok_or_else(|| cli_error(format!("integration not found: {name}")))?,
            )?,
            IntegrationsAction::Connect {
                name,
                base_url,
                auth_type,
                credential_reference,
                username_reference,
                password_reference,
                auth_header,
                auth_scheme,
                scopes,
            } => {
                let mode = auth_type.unwrap_or(match name.as_str() {
                    "github" => IntegrationAuthMode::Bearer,
                    "searxng" if credential_reference.is_some() => IntegrationAuthMode::ApiKey,
                    _ => IntegrationAuthMode::None,
                });
                let auth = integration_auth(mode, auth_header, auth_scheme);
                let mut named = BTreeMap::new();
                if let Some(reference) = username_reference {
                    named.insert("username".into(), reference);
                }
                if let Some(reference) = password_reference {
                    named.insert("password".into(), reference);
                }
                print_json(
                    &runtime
                        .connect_native_integration(
                            &name,
                            base_url.as_deref(),
                            auth,
                            credential_reference.as_deref(),
                            &named,
                            &scopes,
                        )
                        .await?,
                )?;
            }
            IntegrationsAction::ImportOpenapi {
                name,
                spec,
                base_url,
                auth_type,
                credential_reference,
                auth_header,
                auth_scheme,
                scopes,
            } => {
                let source = if spec.starts_with('@') {
                    spec
                } else {
                    format!("@{spec}")
                };
                let document = parse_json_argument(&runtime, &source).await?;
                let auth = integration_auth(auth_type, auth_header, auth_scheme);
                print_json(
                    &runtime
                        .import_openapi_integration(
                            &name,
                            document,
                            base_url.as_deref(),
                            auth,
                            credential_reference.as_deref(),
                            &scopes,
                        )
                        .await?,
                )?;
            }
            IntegrationsAction::Disconnect { name } => {
                print_json(&runtime.disconnect_integration(&name).await?)?;
            }
            IntegrationsAction::Call { tool, arguments } => {
                let arguments = parse_json_argument(&runtime, &arguments).await?;
                print_json(&runtime.call_integration_tool(&tool, arguments).await?)?;
            }
        },
        Command::Mcp(command) => match command.command {
            McpAction::Servers => print_json(&runtime.mcp_servers())?,
            McpAction::Tools { server } => {
                print_json(&runtime.mcp_tools(server.as_deref()).await?)?;
            }
            McpAction::Call {
                server,
                tool,
                arguments,
            } => {
                let arguments = parse_json_argument(&runtime, &arguments).await?;
                print_json(&runtime.mcp_call(&server, &tool, arguments).await?)?;
            }
        },
        Command::Run {
            prompt,
            plan,
            execute_plan,
            goal,
            goal_max_iterations,
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
            stream,
        } => {
            if execute_plan.is_some() && stream {
                return Err(cli_error(
                    "--stream is not supported with --execute-plan; inspect the returned run JSON",
                )
                .into());
            }
            if let Some(plan_id) = execute_plan {
                let result = if goal {
                    let approved = runtime
                        .get_plan(&plan_id)?
                        .ok_or_else(|| cli_error(format!("plan not found: {plan_id}")))?;
                    serde_json::to_value(
                        runtime
                            .run_goal(
                                &role,
                                "",
                                &approved.session_id,
                                goal_max_iterations,
                                Some(&plan_id),
                            )
                            .await?,
                    )?
                } else {
                    serde_json::to_value(
                        runtime
                            .run_approved_plan(&role, &plan_id, max_turns)
                            .await?,
                    )?
                };
                runtime.drain_subagents().await?;
                print_json(&result)?;
                return Ok(());
            }
            let prompt = prompt
                .as_deref()
                .ok_or_else(|| cli_error("a prompt or --execute-plan is required"))?;
            let session_id = if resume {
                Some(runtime.latest_session()?.id)
            } else {
                session
            };
            let result = if plan && stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = runtime
                    .run_plan_with_skills_stream(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                        &mut observer,
                    )
                    .await;
                observer.finish_line()?;
                result?
            } else if plan {
                runtime
                    .run_plan_with_skills(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                    )
                    .await?
            } else if stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = runtime
                    .run_model_with_skills_stream(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                        &mut observer,
                    )
                    .await;
                observer.finish_line()?;
                result?
            } else {
                runtime
                    .run_model_with_skills(
                        &role,
                        &instructions,
                        prompt,
                        max_turns,
                        session_id.as_deref(),
                        &skills,
                        &[],
                    )
                    .await?
            };
            runtime.drain_subagents().await?;
            print_json(&result)?;
        }
        Command::Echo { message } => {
            let result = runtime.echo(&message).await?;
            println!("{}", String::from_utf8_lossy(&result.bytes));
        }
        Command::Tui { session, resume } if interactive_tui => {
            let themes = ThemeLibrary::load_for_config(&cli.config)?;
            let router = prompt_router
                .clone()
                .ok_or_else(|| cli_error("interactive prompt router is unavailable"))?;
            let host = Arc::new(tui_host::EmbeddedInteractiveHost::new(
                Arc::clone(&runtime),
                themes,
                router,
                configured_approval,
            ));
            run_tui(
                host,
                TuiOptions {
                    bootstrap: BootstrapRequest {
                        session_id: session,
                        resume_latest: resume,
                    },
                    screen_mode: if cli.no_alt_screen {
                        ScreenMode::Inline
                    } else {
                        ScreenMode::Alternate
                    },
                },
            )
            .await?;
        }
        Command::Tui { session, resume } => {
            let themes = ThemeLibrary::load_for_config(&cli.config)?;
            line_runner(&runtime, session, resume, configured_approval, &themes).await?
        }
        Command::Worker {
            once,
            shutdown: false,
            status: false,
        } => {
            let recovered = runtime.workflows().recover_interrupted()?;
            let drained = runtime.workflows().drain().await?;
            let projections = runtime.drain_projections()?;
            let subagents = runtime.drain_subagents().await?;
            print_json(&json!({
                "once": once,
                "recovered": recovered,
                "projections": projections,
                "drained": drained,
                "subagents": subagents,
            }))?;
        }
        Command::Worker { shutdown: true, .. } => {
            unreachable!("handled before runtime construction")
        }
        Command::Worker { status: true, .. } => {
            unreachable!("handled before runtime construction")
        }
        Command::SandboxHelper => unreachable!("handled before runtime construction"),
    }
    runtime.checkpoint()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fallback_requires_an_absent_worker_not_a_busy_worker() {
        assert!(worker_probe_allows_embedded_fallback(
            &colossus_worker::WorkerError::Unavailable("worker-endpoint".into())
        ));
        assert!(!worker_probe_allows_embedded_fallback(
            &colossus_worker::WorkerError::Busy("worker-endpoint".into())
        ));
    }

    fn session_summary(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: Some("Test session".into()),
            created_at: "2026-07-15T00:00:00Z".into(),
            updated_at: "2026-07-15T00:00:00Z".into(),
            message_count: 1,
            last_run_id: None,
            last_user_preview: None,
        }
    }

    #[test]
    fn transient_activity_refresh_preserves_semantic_suffix() {
        assert_eq!(
            activity_line_at("[activity] waiting elapsed=1.00s", 2.5),
            "[activity] waiting elapsed=2.50s"
        );
        assert_eq!(
            activity_line_at(
                "[activity] tool=filesystem.read elapsed=0.25s arguments={}",
                3.75,
            ),
            "[activity] tool=filesystem.read elapsed=3.75s arguments={}"
        );
        assert_eq!(
            activity_line_at("[activity] waiting", 1.0),
            "[activity] waiting elapsed=1.00s"
        );
        assert_eq!(
            activity_elapsed(&RunEvent::ToolStarted {
                turn: 1,
                call: ToolCall {
                    call_id: "call-ask".into(),
                    name: "user.ask".into(),
                    arguments: json!({"question": "What should I remember?"}),
                },
                elapsed_seconds: 0.5,
            }),
            None,
            "interactive input must not keep a transient activity repaint alive"
        );
    }

    #[test]
    fn structured_output_is_human_for_terminals_and_json_for_automation() {
        let value = json!([
            {"name": "filesystem.read", "status": "ready"},
            {"name": "filesystem.search", "status": "ready"}
        ]);
        let redirected = render_structured_output(
            &value,
            OutputMode::Auto,
            false,
            80,
            TerminalPreferences::default(),
        )
        .expect("redirected output");
        assert_eq!(
            serde_json::from_str::<Value>(&redirected).expect("json"),
            value
        );

        let terminal = render_structured_output(
            &value,
            OutputMode::Auto,
            true,
            80,
            TerminalPreferences::default(),
        )
        .expect("terminal output");
        assert!(terminal.contains("Name"));
        assert!(terminal.contains("filesystem.read"));
        assert!(terminal.contains('┌'));

        let explicit_json = render_structured_output(
            &value,
            OutputMode::Json,
            true,
            80,
            TerminalPreferences::default(),
        )
        .expect("explicit json");
        assert_eq!(
            serde_json::from_str::<Value>(&explicit_json).expect("json"),
            value
        );
    }

    #[test]
    fn terminal_completion_catalog_includes_commands_and_discovered_skills() {
        let themes = ThemeLibrary::default();
        let values =
            terminal_completion_values(&["skill-creator".into(), "repo-review".into()], &themes);
        assert!(values.contains(&"/help".into()));
        assert!(values.contains(&"/tui prefs".into()));
        assert!(values.contains(&"/workflow status".into()));
        assert!(values.contains(&"/workflow schedule list".into()));
        assert!(values.contains(&"/workflow schedule tick".into()));
        assert!(values.contains(&"/workflow webhook list".into()));
        assert!(values.contains(&"/workflow subscription list".into()));
        assert!(values.contains(&"/theme hacker".into()));
        assert!(values.contains(&"/theme preview high_contrast".into()));
        assert!(values.contains(&"@skill-creator".into()));
        assert!(values.contains(&"@repo-review".into()));

        let (prompt, skills) = resolve_skill_mentions(
            "@skill-creator @repo-review Review this repository",
            &["skill-creator".into(), "repo-review".into()],
        );
        assert_eq!(prompt, "Review this repository");
        assert_eq!(skills, vec!["skill-creator", "repo-review"]);
        let (prompt, skills) = resolve_skill_mentions("@someone hello", &["repo-review".into()]);
        assert_eq!(prompt, "@someone hello");
        assert!(skills.is_empty());
    }

    #[test]
    fn development_config_init_clones_settings_and_isolates_storage() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = directory.path().join("config.yaml");
        let destination = directory.path().join("config.dev.yaml");
        let mut source_config =
            RuntimeConfig::offline_template(directory.path().join("state.redb"));
        source_config.agent.max_turns = 7;
        fs::write(
            &source,
            source_config.to_yaml().expect("source configuration YAML"),
        )
        .expect("source configuration");

        init_config(&destination, true, Some(&source)).expect("development configuration");
        let development =
            RuntimeConfig::from_path(&destination).expect("strict development config");
        assert_eq!(development.agent.max_turns, 7);
        assert_eq!(
            development.storage.path,
            directory.path().join("state.dev.redb")
        );
        assert_eq!(
            development.storage.adapter,
            colossus_runtime::StorageAdapter::Redb
        );
        assert!(development.storage.postgres.is_none());
        match development.storage.keys {
            colossus_runtime::KeyConfig::Environment {
                journal_variable,
                journal_key_id,
                signing_variable,
                anchor_path,
            } => {
                assert_eq!(journal_variable, "COLOSSUS_DEV_JOURNAL_KEY");
                assert!(journal_key_id.starts_with("journal-development-"));
                assert_eq!(signing_variable, "COLOSSUS_DEV_SIGNING_KEY");
                assert_eq!(anchor_path, directory.path().join("secure-anchor.dev.json"));
            }
            colossus_runtime::KeyConfig::Platform { .. } => {
                panic!("development config must not use the platform credential store")
            }
        }
        assert!(init_config(&destination, true, Some(&source)).is_err());
    }

    #[test]
    fn config_init_from_requires_development_mode() {
        let error = Cli::try_parse_from([
            "colossus",
            "config",
            "init",
            "--from",
            ".colossus/config.yaml",
        ])
        .err()
        .expect("--from without --development must fail");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn development_config_init_refuses_orphaned_state() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let destination = directory.path().join("config.dev.yaml");
        fs::write(directory.path().join("state.dev.redb"), b"orphaned state")
            .expect("orphaned development state");

        let error =
            init_config(&destination, true, None).expect_err("orphaned state must fail closed");
        assert!(error.to_string().contains("restore the matching config"));
        assert!(!destination.exists());
    }

    #[test]
    fn workflow_schedule_cli_parses_the_complete_creation_contract() {
        let cli = Cli::try_parse_from([
            "colossus",
            "workflow",
            "schedule",
            "create",
            "nightly",
            "smoke",
            "1.0.0",
            "--cadence-seconds",
            "3600",
            "--inputs",
            r#"{"message":"scheduled"}"#,
            "--misfire",
            "skip",
            "--disabled",
            "--starts-at",
            "2026-01-01T12:00:00Z",
        ])
        .expect("workflow schedule command");
        let Command::Workflow(WorkflowCommand {
            command:
                WorkflowAction::Schedule {
                    command:
                        WorkflowScheduleAction::Create {
                            schedule_id,
                            name,
                            version,
                            cadence_seconds,
                            inputs,
                            misfire,
                            disabled,
                            starts_at,
                        },
                },
        }) = cli.command
        else {
            panic!("expected workflow schedule creation command");
        };
        assert_eq!(schedule_id, "nightly");
        assert_eq!(name, "smoke");
        assert_eq!(version, "1.0.0");
        assert_eq!(cadence_seconds, 3_600);
        assert_eq!(inputs, r#"{"message":"scheduled"}"#);
        assert_eq!(misfire, WorkflowScheduleMisfireArg::Skip);
        assert!(disabled);
        assert_eq!(starts_at.as_deref(), Some("2026-01-01T12:00:00Z"));
    }

    #[test]
    fn workflow_webhook_cli_parses_creation_and_delivery_contracts() {
        let create = Cli::try_parse_from([
            "colossus",
            "workflow",
            "webhook",
            "create",
            "github-main",
            "smoke",
            "1.0.0",
            "--secret-reference",
            "env:COLOSSUS_WEBHOOK_SECRET",
            "--replay-window-seconds",
            "600",
            "--max-body-bytes",
            "4096",
        ])
        .expect("workflow webhook create command");
        assert!(matches!(
            create.command,
            Command::Workflow(WorkflowCommand {
                command: WorkflowAction::Webhook {
                    command: WorkflowWebhookAction::Create {
                        webhook_id,
                        replay_window_seconds: 600,
                        max_body_bytes: 4096,
                        ..
                    }
                }
            }) if webhook_id == "github-main"
        ));

        let ingest = Cli::try_parse_from([
            "colossus",
            "workflow",
            "webhook",
            "ingest",
            "github-main",
            "--delivery-id",
            "delivery-1",
            "--timestamp",
            "2026-07-16T12:00:00Z",
            "--signature",
            "sha256=abcd",
            "--header",
            "content-type=application/json",
            "--body",
            r#"{"event":"push"}"#,
        ])
        .expect("workflow webhook ingest command");
        assert!(matches!(
            ingest.command,
            Command::Workflow(WorkflowCommand {
                command: WorkflowAction::Webhook {
                    command: WorkflowWebhookAction::Ingest {
                        delivery_id,
                        headers,
                        ..
                    }
                }
            }) if delivery_id == "delivery-1" && headers == vec!["content-type=application/json"]
        ));
    }

    #[test]
    fn workflow_subscription_cli_parses_the_complete_creation_contract() {
        let cli = Cli::try_parse_from([
            "colossus",
            "workflow",
            "subscription",
            "create",
            "new-tasks",
            "smoke",
            "1.0.0",
            "--event-type",
            "task.created.v1",
            "--stream-prefix",
            "task:",
            "--after-sequence",
            "41",
            "--disabled",
        ])
        .expect("workflow subscription command");
        let Command::Workflow(WorkflowCommand {
            command:
                WorkflowAction::Subscription {
                    command:
                        WorkflowSubscriptionAction::Create {
                            subscription_id,
                            name,
                            version,
                            event_type,
                            stream_prefix,
                            disabled,
                            after_sequence,
                        },
                },
        }) = cli.command
        else {
            panic!("expected workflow subscription creation command");
        };
        assert_eq!(subscription_id, "new-tasks");
        assert_eq!(name, "smoke");
        assert_eq!(version, "1.0.0");
        assert_eq!(event_type, "task.created.v1");
        assert_eq!(stream_prefix.as_deref(), Some("task:"));
        assert!(disabled);
        assert_eq!(after_sequence, Some(41));
    }

    #[test]
    fn workflow_webhook_http_parser_is_bounded_and_strips_auth_headers() {
        let body = br#"{"event":"push"}"#;
        let request = format!(
            "POST /v1/workflow-webhooks/github-main HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nX-Colossus-Delivery-Id: delivery-1\r\nX-Colossus-Timestamp: 2026-07-16T12:00:00Z\r\nX-Colossus-Signature: sha256={}\r\nX-Github-Event: push\r\n\r\n{}",
            body.len(),
            "a".repeat(64),
            String::from_utf8_lossy(body),
        );
        let delivery = parse_webhook_http_request(request.as_bytes()).expect("webhook request");
        assert_eq!(delivery.webhook_id, "github-main");
        assert_eq!(delivery.delivery_id, "delivery-1");
        assert_eq!(delivery.body, body);
        assert_eq!(delivery.headers.get("x-github-event"), Some(&"push".into()));
        assert!(!delivery.headers.contains_key("x-colossus-signature"));
        assert!(!delivery.headers.contains_key("content-length"));

        let duplicate = request.replacen(
            "Host: 127.0.0.1\r\n",
            "Host: 127.0.0.1\r\nHost: duplicate\r\n",
            1,
        );
        assert!(parse_webhook_http_request(duplicate.as_bytes()).is_err());
        let chunked = request.replacen(
            "Content-Length:",
            "Transfer-Encoding: chunked\r\nContent-Length:",
            1,
        );
        assert!(parse_webhook_http_request(chunked.as_bytes()).is_err());
    }

    #[test]
    fn tui_parses_with_the_global_inline_flag_and_repl_is_rejected() {
        let tui =
            Cli::try_parse_from(["colossus", "tui", "--no-alt-screen"]).expect("explicit TUI");
        assert!(tui.no_alt_screen);
        assert!(matches!(tui.command, Command::Tui { .. }));

        let error = Cli::try_parse_from(["colossus", "--no-alt-screen", "repl", "--resume"])
            .err()
            .expect("removed REPL command");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn resume_picker_recognizes_selection_cancellation_commands_and_bad_input() {
        let sessions = vec![
            session_summary("session-one"),
            session_summary("session-two"),
        ];

        assert_eq!(
            parse_session_picker_input("2", &sessions),
            SessionPickerInput::Selected("session-two".into())
        );
        assert_eq!(
            parse_session_picker_input("session-one", &sessions),
            SessionPickerInput::Selected("session-one".into())
        );
        assert_eq!(
            parse_session_picker_input(" /session ", &sessions),
            SessionPickerInput::Command("/session".into())
        );
        assert_eq!(
            parse_session_picker_input("", &sessions),
            SessionPickerInput::Cancelled
        );
        assert_eq!(
            parse_session_picker_input("99", &sessions),
            SessionPickerInput::Invalid
        );
        assert_eq!(
            parse_session_picker_input("not a session", &sessions),
            SessionPickerInput::Invalid
        );
    }

    #[test]
    fn theme_picker_accepts_numbers_names_previews_commands_and_cancellation() {
        let names = ThemeLibrary::default().names();
        assert_eq!(
            parse_theme_picker_input("2", &names),
            ThemePickerInput::Selected("mono".into())
        );
        assert_eq!(
            parse_theme_picker_input("high-contrast", &names),
            ThemePickerInput::Selected("high_contrast".into())
        );
        assert_eq!(
            parse_theme_picker_input("p 5", &names),
            ThemePickerInput::Preview("hacker".into())
        );
        assert_eq!(
            parse_theme_picker_input("preview carrot", &names),
            ThemePickerInput::Preview("carrot".into())
        );
        assert_eq!(
            parse_theme_picker_input("/help", &names),
            ThemePickerInput::Command("/help".into())
        );
        assert_eq!(
            parse_theme_picker_input("", &names),
            ThemePickerInput::Cancelled
        );
        assert_eq!(
            parse_theme_picker_input("99", &names),
            ThemePickerInput::Invalid
        );
    }
}
