use super::*;

#[derive(Subcommand)]
pub(super) enum Command {
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
    /// Inspect and query provider-neutral web-search routes.
    Search(SearchCommand),
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
    /// Build, verify, and install signed pack and skill collections.
    Collections(CollectionsCommand),
    /// Pull and push authenticated signed collection transports.
    Registry(RegistryCommand),
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
pub(super) struct ConfigCommand {
    #[command(subcommand)]
    pub(super) command: ConfigAction,
}

#[derive(Args)]
pub(super) struct PreferencesCommand {
    #[command(subcommand)]
    pub(super) command: PreferencesAction,
}

#[derive(Subcommand)]
pub(super) enum PreferencesAction {
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
pub(super) enum ConfigAction {
    /// Create a strict offline configuration without overwriting an existing file.
    Init {
        /// Use isolated redb state and environment keys for source development.
        #[arg(long)]
        development: bool,
        /// Clone non-storage settings from an existing strict configuration.
        #[arg(long, value_name = "PATH", requires = "development")]
        from: Option<PathBuf>,
        /// Unified tool and built-in policy profile.
        #[arg(long, default_value = "development")]
        access_profile: AccessProfile,
        /// Resource sandbox preset; defaults from the selected access profile.
        #[arg(long, value_enum)]
        sandbox_profile: Option<SandboxProfile>,
    },
    /// Parse and print the active configuration with references intact.
    Show,
    /// Show credential-free effective tool and action resolution.
    Effective,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum SandboxProfile {
    OfflineDefault,
    WorkspaceDevelopment,
}

impl SandboxProfile {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::OfflineDefault => "offline-default",
            Self::WorkspaceDevelopment => "workspace-development",
        }
    }
}

#[derive(Args)]
pub(super) struct AuditCommand {
    #[command(subcommand)]
    pub(super) command: AuditAction,
}

#[derive(Subcommand)]
pub(super) enum AuditAction {
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
pub(super) struct PolicyCommand {
    #[command(subcommand)]
    pub(super) command: PolicyAction,
}

#[derive(Subcommand)]
pub(super) enum PolicyAction {
    /// Check readiness, revision metadata, and decision-log safeguards.
    Doctor,
}

#[derive(Args)]
pub(super) struct ProjectionCommand {
    #[command(subcommand)]
    pub(super) command: ProjectionAction,
}

#[derive(Subcommand)]
pub(super) enum ProjectionAction {
    /// Show position, journal head, lag, and readiness.
    Status,
    /// Replay queued journal records into every projection.
    Drain,
    /// Delete and replay one projection, or every projection when omitted.
    Rebuild { name: Option<String> },
}

#[derive(Args)]
pub(super) struct StateCommand {
    #[command(subcommand)]
    pub(super) command: StateAction,
}

#[derive(Subcommand)]
pub(super) enum StateAction {
    /// Check the writer lease, journal head, adapters, and projection lag.
    Doctor,
}

#[derive(Args)]
pub(super) struct SandboxCommand {
    #[command(subcommand)]
    pub(super) command: SandboxAction,
}

#[derive(Subcommand)]
pub(super) enum SandboxAction {
    /// Report native kernel support and configured OCI fallback.
    Doctor,
}

#[derive(Args)]
pub(super) struct ProcessCommand {
    #[command(subcommand)]
    pub(super) command: ProcessAction,
}

#[derive(Subcommand)]
pub(super) enum ProcessAction {
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
pub(super) struct NetworkCommand {
    #[command(subcommand)]
    pub(super) command: NetworkAction,
}

#[derive(Subcommand)]
pub(super) enum NetworkAction {
    /// Fetch one exact HTTP(S) URL through destination enforcement and quarantine.
    Get { url: String },
}
