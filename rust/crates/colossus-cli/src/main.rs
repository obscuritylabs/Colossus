//! Thin terminal interface for the Rust runtime.

use async_trait::async_trait;
use clap::{Args, Parser, Subcommand, ValueEnum};
use colossus_contracts::{
    ApprovalProof, DecisionPriority, DecisionStatus, EffectRequest, GoalStatus, MemoryScope,
    MemoryStatus, PlanStatus, PlanStep, PolicyDecision, ResearchDepth, ResearchSourceKind,
    SubagentStatus, TaskStatus,
};
use colossus_policy::{AllowApproval, DenyApproval};
use colossus_ports::{ApprovalProvider, PolicyError};
use colossus_runtime::{Runtime, RuntimeConfig};
use reedline::{DefaultPrompt, Reedline, Signal};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, Write as _},
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
        /// Recover state and return without repeatedly polling.
        #[arg(long, default_value_t = true)]
        once: bool,
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
    /// List bounded regular resources for an explicitly active skill.
    Resources { name: String },
    /// Read one bounded UTF-8 resource through the effect gateway.
    Read { name: String, path: String },
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
        } => {
            let run = runtime
                .workflows()
                .start_run(
                    &name,
                    &version,
                    parse_json_argument(runtime, &inputs).await?,
                )
                .await?;
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
    let Signal::Success(choice) = editor.read_line(prompt)? else {
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

async fn repl(
    runtime: &Runtime,
    initial_session: Option<String>,
    resume_latest: bool,
) -> Result<(), Box<dyn Error>> {
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::default();
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
        match editor.read_line(&prompt)? {
            Signal::Success(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if matches!(line, "/quit" | "/exit") {
                    break;
                }
                if line == "/help" {
                    println!(
                        "/resume [LIMIT] | /sessions | /session show|new|resume ID | /tasks | /decisions | /plans | /goals | /goal OBJECTIVE | /agents | /agents drain | /memories | /memory search QUERY | /research QUESTION | /research list | /telemetry [RUN_ID] | /telemetry metrics | /skills | /skill use|clear|show|resources|read | /context status|list|compact|restore ID | /workflow list | /audit verify | /tools | /exit"
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
                    print_json(&runtime.context_status(&active_session_id)?)?;
                } else if line == "/context list" {
                    print_json(&runtime.context_snapshots(&active_session_id)?)?;
                } else if line == "/context compact" {
                    print_json(&runtime.compact_context(&active_session_id).await?)?;
                } else if let Some(snapshot_id) = line.strip_prefix("/context restore ") {
                    print_json(&runtime.restore_context(&active_session_id, snapshot_id.trim())?)?;
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
                    if let Some(session_id) = choose_session(runtime, &mut editor, &prompt, limit)?
                    {
                        active_session_id = session_id;
                        println!("session={active_session_id}");
                    }
                } else {
                    let result = runtime
                        .run_model_with_skills(
                            "primary",
                            "You are Colossus.",
                            line,
                            None,
                            Some(&active_session_id),
                            &[],
                            &sticky_skills,
                        )
                        .await?;
                    println!("{}", result.output);
                }
            }
            Signal::CtrlD | Signal::CtrlC => break,
            _ => continue,
        }
    }
    Ok(())
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
    let approvals = approval_provider(&cli.command, cli.approval_mode);
    let runtime = Runtime::open_with_approval(&config, approvals)?;
    match cli.command {
        Command::Config(_) => unreachable!("handled before runtime construction"),
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
        Command::Context(command) => match command.command {
            ContextAction::Status { session_id } => {
                print_json(&runtime.context_status(&session_id)?)?;
            }
            ContextAction::List { session_id } => {
                print_json(&runtime.context_snapshots(&session_id)?)?;
            }
            ContextAction::Compact { session_id } => {
                print_json(&runtime.compact_context(&session_id).await?)?;
            }
            ContextAction::Restore {
                session_id,
                snapshot_id,
            } => print_json(&runtime.restore_context(&session_id, &snapshot_id)?)?,
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
        Command::Run {
            prompt,
            role,
            instructions,
            max_turns,
            session,
            resume,
            skills,
        } => {
            let session_id = if resume {
                Some(runtime.latest_session()?.id)
            } else {
                session
            };
            let result = runtime
                .run_model_with_skills(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &skills,
                    &[],
                )
                .await?;
            runtime.drain_subagents().await?;
            print_json(&result)?;
        }
        Command::Echo { message } => {
            let result = runtime.echo(&message).await?;
            println!("{}", String::from_utf8_lossy(&result.bytes));
        }
        Command::Repl { session, resume } => repl(&runtime, session, resume).await?,
        Command::Worker { once } => {
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
        Command::SandboxHelper => unreachable!("handled before runtime construction"),
    }
    runtime.checkpoint()?;
    Ok(())
}
