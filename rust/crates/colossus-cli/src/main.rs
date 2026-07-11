//! Thin terminal interface for the Rust runtime.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::{Args, Parser, Subcommand, ValueEnum};
use colossus_contracts::{
    ApprovalProof, DecisionPriority, DecisionStatus, EffectRequest, GoalStatus, IntegrationAuth,
    MemoryScope, MemoryStatus, PlanStatus, PlanStep, PolicyDecision, ProviderEvent, ResearchDepth,
    ResearchSourceKind, RunEvent, RunEventEnvelope, SubagentStatus, TaskStatus, UserPromptRequest,
    UserPromptResponse,
};
use colossus_policy::{AllowApproval, DenyApproval};
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_presentation::{
    EventDisplayMode, ReplPreferences, SemanticRenderer, StreamDisplayMode, ThemeName,
    TranscriptDensity,
};
use colossus_runtime::{Runtime, RuntimeConfig};
use colossus_worker::{WorkerClient, WorkerOperation, WorkerServer};
use reedline::{
    DefaultPrompt, EditCommand, Emacs, KeyCode, KeyModifiers, Reedline, ReedlineEvent, Signal,
    default_emacs_keybindings,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, BufRead as _, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Parser)]
#[command(
    name = "colossus-rs",
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
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ApprovalMode {
    /// Fail closed without prompting (default outside the REPL).
    Deny,
    /// Prompt on the terminal for every approval obligation.
    Ask,
    /// Prompt explicitly while risk-model evaluation is not yet configured.
    RiskAuto,
    /// Grant approval obligations automatically without expanding policy permissions.
    FullAccess,
}

struct TerminalApproval {
    risk_unavailable: bool,
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
        eprintln!("{}", request.question);
        for (index, choice) in request.choices.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, choice);
        }
        for _ in 0..3 {
            if request.choices.is_empty() {
                eprint!("Answer: ");
            } else if request.allow_free_form {
                eprint!("Choose a number or enter an answer: ");
            } else {
                eprint!("Choose a number: ");
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
        eprintln!("approval required: {} {}", request.action, request.resource);
        eprintln!("reason: {}", decision.reason);
        if self.risk_unavailable {
            eprintln!("risk status: unavailable; explicit approval is required");
        }
        let content = serde_json::to_string_pretty(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        eprintln!("proposed content: {}", bounded_preview(&content, 1200));
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
    preferences: ReplPreferences,
}

impl TerminalStreamObserver {
    fn new(target: StreamTarget) -> Self {
        Self {
            target,
            wrote_text: false,
            preferences: ReplPreferences::default(),
        }
    }

    fn with_preferences(target: StreamTarget, preferences: ReplPreferences) -> Self {
        Self {
            target,
            wrote_text: false,
            preferences,
        }
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.finish_line()?;
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
        if self.wrote_text {
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
}

#[async_trait]
impl RunEventObserver for TerminalStreamObserver {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if let RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } = &envelope.event
        {
            if self.preferences.stream_mode == StreamDisplayMode::Off {
                return Ok(());
            }
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
        if let Some(line) = SemanticRenderer::new(self.preferences.clone())
            .run_event_envelope(&envelope)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?
        {
            self.write_line(&line)
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
    let mode = configured.unwrap_or(if matches!(command, Command::Repl { .. }) {
        ApprovalMode::Ask
    } else {
        ApprovalMode::Deny
    });
    match mode {
        ApprovalMode::Deny => Arc::new(DenyApproval),
        ApprovalMode::Ask | ApprovalMode::RiskAuto => Arc::new(TerminalApproval {
            risk_unavailable: mode == ApprovalMode::RiskAuto,
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
    /// Verify signed offline release bundles.
    Bundle(BundleCommand),
    /// Manage persisted integrations and imported OpenAPI tools.
    Integrations(IntegrationsCommand),
    /// Discover and invoke explicitly configured MCP servers.
    Mcp(McpCommand),
    /// Execute one audited model turn through the configured role.
    Run {
        /// User prompt sent as the complete logical request content.
        prompt: String,
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
    /// Start the modern interactive terminal.
    Repl {
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
    /// Restore and persist default presentation preferences.
    Reset,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create a strict offline configuration without overwriting an existing file.
    Init,
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
    /// Show a reconstructed run.
    Status { run_id: String },
    /// Resume a waiting or interrupted run.
    Resume { run_id: String },
    /// Supply inline JSON or @path input and resume.
    Input { run_id: String, input: String },
    /// Cancel a non-terminal run.
    Cancel { run_id: String },
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
    /// Verify a signed offline bundle without network access.
    Verify { path: PathBuf },
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

fn init_config(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let state = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("state.redb");
    let config = RuntimeConfig::offline_template(state);
    fs::write(path, config.to_yaml()?)?;
    println!("created {}", path.display());
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
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
}

fn repl_editor(multiline: bool) -> Reedline {
    if !multiline {
        return Reedline::create();
    }
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Enter,
        ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
    );
    keybindings.add_binding(KeyModifiers::ALT, KeyCode::Enter, ReedlineEvent::Submit);
    Reedline::create().with_edit_mode(Box::new(Emacs::new(keybindings)))
}

fn handle_presentation_command(
    line: &str,
    preferences: &mut ReplPreferences,
) -> Result<PresentationCommandResult, Box<dyn Error>> {
    let mut changed = false;
    match line {
        "/repl" | "/repl prefs" => print_json(preferences)?,
        "/repl save" => changed = true,
        "/repl reset" => {
            *preferences = ReplPreferences::default();
            changed = true;
        }
        "/theme" => println!(
            "theme={}; available=default,high_contrast,plain",
            preferences.theme.as_str()
        ),
        "/theme default" => {
            preferences.theme = ThemeName::Default;
            changed = true;
        }
        "/theme high_contrast" => {
            preferences.theme = ThemeName::HighContrast;
            changed = true;
        }
        "/theme plain" => {
            preferences.theme = ThemeName::Plain;
            changed = true;
        }
        "/theme reset" => {
            preferences.theme = ThemeName::Default;
            changed = true;
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
            if command.starts_with("/repl ")
                || command.starts_with("/theme ")
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

fn choose_session(
    runtime: &Runtime,
    editor: &mut Reedline,
    prompt: &DefaultPrompt,
    scripted_input: &mut Option<io::StdinLock<'_>>,
    limit: usize,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut sessions = runtime
        .list_sessions(100)?
        .into_iter()
        .filter(|session| session.message_count > 0)
        .collect::<Vec<_>>();
    sessions.truncate(limit);
    if sessions.is_empty() {
        println!("No sessions exist yet.");
        return Ok(None);
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
    println!("Enter a number or exact session id (blank cancels).");
    let Signal::Success(choice) = read_repl_signal(editor, prompt, scripted_input)? else {
        return Ok(None);
    };
    let choice = choice.trim();
    if choice.is_empty() {
        return Ok(None);
    }
    if let Ok(index) = choice.parse::<usize>()
        && let Some(session) = index.checked_sub(1).and_then(|index| sessions.get(index))
    {
        return Ok(Some(session.id.clone()));
    }
    runtime
        .get_session(choice)?
        .map(|session| session.id)
        .ok_or_else(|| cli_error(format!("session not found: {choice}")))
        .map_err(Into::into)
        .map(Some)
}

fn read_repl_signal(
    editor: &mut Reedline,
    prompt: &DefaultPrompt,
    scripted_input: &mut Option<io::StdinLock<'_>>,
) -> Result<Signal, Box<dyn Error>> {
    let Some(input) = scripted_input.as_mut() else {
        return Ok(editor.read_line(prompt)?);
    };
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        Ok(Signal::CtrlD)
    } else {
        Ok(Signal::Success(line))
    }
}

async fn repl(
    runtime: &Runtime,
    initial_session: Option<String>,
    resume_latest: bool,
) -> Result<(), Box<dyn Error>> {
    let mut preferences = runtime.presentation_preferences()?;
    let mut editor = repl_editor(preferences.multiline);
    let prompt = DefaultPrompt::default();
    let stdin = io::stdin();
    let mut scripted_input = (!stdin.is_terminal()).then(|| stdin.lock());
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
    println!(
        "Colossus Rust alpha. session={active_session_id}; /help for commands; Ctrl-D to exit."
    );
    loop {
        match read_repl_signal(&mut editor, &prompt, &mut scripted_input)? {
            Signal::Success(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if matches!(line, "/quit" | "/exit") {
                    break;
                }
                let prior_multiline = preferences.multiline;
                match handle_presentation_command(line, &mut preferences)? {
                    PresentationCommandResult::NotHandled => {}
                    PresentationCommandResult::Handled => continue,
                    PresentationCommandResult::Save => {
                        preferences = runtime
                            .save_presentation_preferences(preferences.clone())
                            .await?;
                        print_json(&preferences)?;
                        if prior_multiline != preferences.multiline {
                            editor = repl_editor(preferences.multiline);
                        }
                        continue;
                    }
                }
                if line == "/help" {
                    println!(
                        "/repl [prefs|reset] | /theme [default|high_contrast|plain] | /stream on|raw|off | /events compact|verbose|off | /reasoning on|off | /transcript comfortable|compact | /multiline on|off|toggle | /trace | /resume [LIMIT] | /sessions | /session show|new|resume ID | /work | /tasks | /decisions | /plans | /goals | /goal OBJECTIVE | /agents | /agents drain | /memories | /memory search QUERY | /research QUESTION | /research list | /telemetry [RUN_ID] | /telemetry metrics | /skills | /skill use|clear|show|resources|read | /packs list|show|verify|validate|install|enable|disable|uninstall|call|trust | /bundle verify | /integrations | /integration show|call|disconnect | /mcp servers|tools|call | /context status|list|compact|restore ID | /workflow list | /audit verify | /tools | /exit"
                    );
                    println!("Any other line is sent through the configured primary model role.");
                } else if line == "/workflow list" {
                    workflow_command(runtime, WorkflowAction::List).await?;
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
                            .search_memories("", Some(&active_session_id), None, 20)
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
                    let (publisher, public_key) =
                        value.trim().split_once(' ').ok_or_else(|| {
                            cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY")
                        })?;
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
                } else if let Some(session_id) = line.strip_prefix("/session resume ") {
                    let session_id = session_id.trim();
                    active_session_id = runtime
                        .get_session(session_id)?
                        .ok_or_else(|| cli_error(format!("session not found: {session_id}")))?
                        .id;
                    println!("session={active_session_id}");
                } else if line == "/resume" || line.starts_with("/resume ") {
                    let limit = line
                        .strip_prefix("/resume ")
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::parse::<usize>)
                        .transpose()?
                        .unwrap_or(10)
                        .clamp(1, 100);
                    if let Some(session_id) =
                        choose_session(runtime, &mut editor, &prompt, &mut scripted_input, limit)?
                    {
                        active_session_id = session_id;
                        println!("session={active_session_id}");
                    }
                } else {
                    let mut observer = TerminalStreamObserver::with_preferences(
                        StreamTarget::Stdout,
                        preferences.clone(),
                    );
                    let result = runtime
                        .run_model_with_skills_stream(
                            "primary",
                            "You are Colossus.",
                            line,
                            None,
                            Some(&active_session_id),
                            &[],
                            &sticky_skills,
                            &mut observer,
                        )
                        .await;
                    observer.finish_line()?;
                    let result = result?;
                    if preferences.stream_mode == StreamDisplayMode::Off {
                        println!("{}", result.output);
                    }
                }
            }
            Signal::CtrlD | Signal::CtrlC => break,
            _ => continue,
        }
    }
    Ok(())
}

async fn dispatch_to_worker_if_active(
    config: &RuntimeConfig,
    command: &Command,
    approval_mode: Option<ApprovalMode>,
) -> Result<bool, Box<dyn Error>> {
    let Some(client) = WorkerClient::discover(config)? else {
        return Ok(false);
    };
    match client.ping().await {
        Ok(_) => {}
        Err(colossus_worker::WorkerError::Unavailable(_)) => return Ok(false),
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
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
            stream,
        } => {
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
            let operation = WorkerOperation::RunModel {
                role: role.clone(),
                instructions: instructions.clone(),
                prompt: prompt.clone(),
                max_turns: *max_turns,
                session_id,
                explicit_skills: skills.clone(),
                sticky_skills: Vec::new(),
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
                BundleAction::Verify { path } => WorkerOperation::BundleVerify {
                    path: path.to_string_lossy().into_owned(),
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
        Command::Repl { session, resume } => {
            worker_repl(&client, session.clone(), *resume).await?;
            Ok(true)
        }
        Command::Preferences(command) => {
            let operation = match command.command {
                PreferencesAction::Show => WorkerOperation::PresentationGet,
                PreferencesAction::Reset => WorkerOperation::PresentationSave {
                    preferences: ReplPreferences::default(),
                },
            };
            print_json(&client.call(operation).await?)?;
            Ok(true)
        }
        Command::Worker { .. } | Command::Config(_) | Command::SandboxHelper => Ok(false),
    }
}

async fn worker_repl(
    client: &WorkerClient,
    requested_session: Option<String>,
    resume: bool,
) -> Result<(), Box<dyn Error>> {
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
    let mut preferences = serde_json::from_value::<ReplPreferences>(
        client.call(WorkerOperation::PresentationGet).await?,
    )?;
    let mut editor = repl_editor(preferences.multiline);
    let prompt = DefaultPrompt::default();
    let stdin = io::stdin();
    let mut scripted_input = (!stdin.is_terminal()).then(|| stdin.lock());
    let mut sticky_skills = Vec::<String>::new();
    println!("Colossus Rust REPL via authenticated worker. Type /help for commands.");
    loop {
        match read_repl_signal(&mut editor, &prompt, &mut scripted_input)? {
            Signal::Success(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if matches!(line, "/quit" | "/exit") {
                    break;
                }
                let prior_multiline = preferences.multiline;
                match handle_presentation_command(line, &mut preferences)? {
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
                        print_json(&preferences)?;
                        if prior_multiline != preferences.multiline {
                            editor = repl_editor(preferences.multiline);
                        }
                        continue;
                    }
                }
                if line == "/help" {
                    println!(
                        "/repl [prefs|reset] | /theme [default|high_contrast|plain] | /stream on|raw|off | /events compact|verbose|off | /reasoning on|off | /transcript comfortable|compact | /multiline on|off|toggle | /trace | /resume [LIMIT] | /sessions | /session show|new|resume ID | /work | /tasks | /decisions | /plans | /goals | /goal OBJECTIVE | /agents | /agents drain | /memories | /memory search QUERY | /research QUESTION | /research list | /telemetry [RUN_ID] | /telemetry metrics | /skills | /skill use|clear|show|resources|read | /packs list|show|verify|validate|install|enable|disable|uninstall|call|trust | /bundle verify | /integrations | /integration show|call|disconnect | /mcp servers|tools|call | /context status|list|compact|restore ID | /workflow list | /audit verify | /tools | /exit"
                    );
                    println!("Any other line is sent through the configured primary model role.");
                } else if line == "/workflow list" {
                    print_json(&client.call(WorkerOperation::WorkflowList).await?)?;
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
                        SemanticRenderer::new(preferences.clone()).work_state(&state)
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
                            .call(WorkerOperation::MemorySearch {
                                query: String::new(),
                                session_id: Some(active_session_id.clone()),
                                repository_id: None,
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
                    let (publisher, public_key) =
                        value.trim().split_once(' ').ok_or_else(|| {
                            cli_error("usage: /packs trust add PUBLISHER BASE64_PUBLIC_KEY")
                        })?;
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
                } else if line == "/skill clear" {
                    sticky_skills.clear();
                    println!("active skills cleared");
                } else if line == "/skill active" {
                    println!("active skills: {}", sticky_skills.join(", "));
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
                        SemanticRenderer::new(preferences.clone()).context_status(&status)
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
                    active_session_id =
                        serde_json::from_value::<colossus_contracts::SessionSummary>(
                            client
                                .call(WorkerOperation::SessionCreate { title: None })
                                .await?,
                        )?
                        .id;
                    println!("session={active_session_id}");
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
                } else if line == "/resume" || line.starts_with("/resume ") {
                    let limit = line
                        .strip_prefix("/resume ")
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::parse::<usize>)
                        .transpose()?
                        .unwrap_or(10)
                        .clamp(1, 100);
                    if let Some(session_id) = choose_worker_session(
                        client,
                        &mut editor,
                        &prompt,
                        &mut scripted_input,
                        limit,
                    )
                    .await?
                    {
                        active_session_id = session_id;
                        println!("session={active_session_id}");
                    }
                } else if line.starts_with('/') {
                    println!("unknown REPL command: {line}; use /help");
                } else {
                    let mut observer = TerminalStreamObserver::with_preferences(
                        StreamTarget::Stdout,
                        preferences.clone(),
                    );
                    let result = client
                        .run_model(
                            WorkerOperation::RunModel {
                                role: "primary".into(),
                                instructions: "You are Colossus.".into(),
                                prompt: line.into(),
                                max_turns: None,
                                session_id: Some(active_session_id.clone()),
                                explicit_skills: Vec::new(),
                                sticky_skills: sticky_skills.clone(),
                            },
                            &mut observer,
                        )
                        .await;
                    observer.finish_line()?;
                    let result = result?;
                    if preferences.stream_mode == StreamDisplayMode::Off {
                        println!("{}", result.output);
                    }
                    client.call(WorkerOperation::Drain).await?;
                }
            }
            Signal::CtrlD | Signal::CtrlC => break,
            _ => continue,
        }
    }
    Ok(())
}

async fn choose_worker_session(
    client: &WorkerClient,
    editor: &mut Reedline,
    prompt: &DefaultPrompt,
    scripted_input: &mut Option<io::StdinLock<'_>>,
    limit: usize,
) -> Result<Option<String>, Box<dyn Error>> {
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
        return Ok(None);
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
    println!("Enter a number or exact session id (blank cancels).");
    let Signal::Success(choice) = read_repl_signal(editor, prompt, scripted_input)? else {
        return Ok(None);
    };
    let choice = choice.trim();
    if choice.is_empty() {
        return Ok(None);
    }
    if let Ok(index) = choice.parse::<usize>()
        && let Some(session) = index.checked_sub(1).and_then(|index| sessions.get(index))
    {
        return Ok(Some(session.id.clone()));
    }
    let session = client
        .call(WorkerOperation::SessionGet {
            session_id: choice.into(),
        })
        .await?;
    if session.is_null() {
        return Err(cli_error(format!("session not found: {choice}")).into());
    }
    Ok(Some(choice.into()))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if matches!(cli.command, Command::SandboxHelper) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    if let Command::Config(ConfigCommand {
        command: ConfigAction::Init,
    }) = &cli.command
    {
        return init_config(&cli.config);
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
            let approvals = approval_provider(&cli.command, cli.approval_mode);
            let server = WorkerServer::open(&config, approvals)?;
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
    if dispatch_to_worker_if_active(&config, &cli.command, cli.approval_mode).await? {
        return Ok(());
    }
    let approvals = approval_provider(&cli.command, cli.approval_mode);
    let user_prompts: Option<Arc<dyn UserPromptProvider>> =
        (matches!(&cli.command, Command::Repl { .. }) && io::stdin().is_terminal()).then(|| {
            Arc::new(TerminalUserPrompt {
                lock: Mutex::new(()),
            }) as Arc<dyn UserPromptProvider>
        });
    let runtime = Runtime::open_with_interfaces(&config, approvals, user_prompts)?;
    match cli.command {
        Command::Config(_) => unreachable!("handled before runtime construction"),
        Command::Preferences(command) => match command.command {
            PreferencesAction::Show => print_json(&runtime.presentation_preferences()?)?,
            PreferencesAction::Reset => print_json(
                &runtime
                    .save_presentation_preferences(ReplPreferences::default())
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
            BundleAction::Verify { path } => print_json(&runtime.verify_bundle(path).await?)?,
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
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
            stream,
        } => {
            let session_id = if resume {
                Some(runtime.latest_session()?.id)
            } else {
                session
            };
            let result = if stream {
                let mut observer = TerminalStreamObserver::new(StreamTarget::Stderr);
                let result = runtime
                    .run_model_with_skills_stream(
                        &role,
                        &instructions,
                        &prompt,
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
                        &prompt,
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
        Command::Repl { session, resume } => repl(&runtime, session, resume).await?,
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
