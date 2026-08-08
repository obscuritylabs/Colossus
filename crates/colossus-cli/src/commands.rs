use super::*;

#[derive(Subcommand)]
pub(super) enum Command {
    /// Check the fixed stable Colossus release channel.
    Update(UpdateCommand),
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
    /// Manage the Codex/ChatGPT sign-in reused by subscription-backed providers.
    Codex(CodexCommand),
    /// Inspect and query provider-neutral web-search routes.
    Search(SearchCommand),
    /// Inspect model role routing.
    Models(ModelsCommand),
    /// Upload, inspect, and download caller-owned released artifacts.
    Artifacts(ArtifactsCommand),
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
        /// Attach one bounded UTF-8 file as private CLI run input. Repeat as needed.
        #[arg(long = "attach", value_name = "PATH")]
        attachments: Vec<PathBuf>,
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
    /// Recover work, serve IPC, or administer the owner-only public API.
    Worker(WorkerCommand),
    /// Internal authenticated one-shot sandbox helper.
    #[command(name = "__sandbox-helper", hide = true)]
    SandboxHelper,
}

#[derive(Args)]
pub(super) struct UpdateCommand {
    #[command(subcommand)]
    pub(super) command: UpdateAction,
}

#[derive(Subcommand)]
pub(super) enum UpdateAction {
    /// Check whether a newer stable CLI release is available.
    Check,
}

#[derive(Args)]
pub(super) struct WorkerCommand {
    /// Recover and drain once instead of serving local IPC.
    #[arg(
        long,
        conflicts_with_all = [
            "shutdown",
            "status",
            "public_api_dir",
            "enroll_application",
            "revoke_credential"
        ]
    )]
    pub(super) once: bool,
    /// Ask the authenticated local worker to checkpoint and stop.
    #[arg(
        long,
        conflicts_with_all = [
            "once",
            "status",
            "public_api_dir",
            "enroll_application",
            "revoke_credential"
        ]
    )]
    pub(super) shutdown: bool,
    /// Authenticate the configured worker and show readiness.
    #[arg(
        long,
        conflicts_with_all = [
            "once",
            "shutdown",
            "public_api_dir",
            "enroll_application",
            "revoke_credential"
        ]
    )]
    pub(super) status: bool,
    /// Absolute current-user 0700 directory for public API discovery.
    #[arg(long, value_name = "ABS_OWNER_PRIVATE_DIR")]
    pub(super) public_api_dir: Option<PathBuf>,
    /// Enroll an application offline and write its bearer directly to a keyring.
    #[arg(
        long,
        value_name = "APPLICATION_ID",
        requires_all = [
            "public_api_dir",
            "scope",
            "role",
            "credential_keyring_service",
            "credential_keyring_account"
        ],
        conflicts_with = "revoke_credential"
    )]
    pub(super) enroll_application: Option<String>,
    /// Exact public API scope to grant; repeat for additional scopes.
    #[arg(long, requires = "enroll_application")]
    pub(super) scope: Vec<String>,
    /// Exact model role ceiling; repeat for additional roles.
    #[arg(long, requires = "enroll_application")]
    pub(super) role: Vec<String>,
    /// Exact tool-name ceiling; repeat as needed. Omission denies every tool.
    #[arg(long, requires = "enroll_application")]
    pub(super) tool: Vec<String>,
    /// Destination OS-keyring service for the one-time bearer.
    #[arg(long, value_name = "SERVICE", requires = "enroll_application")]
    pub(super) credential_keyring_service: Option<String>,
    /// Destination OS-keyring account; Desktop external enrollment accepts `auto`.
    #[arg(long, value_name = "ACCOUNT", requires = "enroll_application")]
    pub(super) credential_keyring_account: Option<String>,
    /// Explicitly replace an existing destination keyring entry.
    #[arg(
        long,
        requires = "enroll_application",
        conflicts_with = "retire_credential_keyring_service"
    )]
    pub(super) replace_credential: bool,
    /// Source keyring service to revoke and delete after successful enrollment.
    #[arg(
        long,
        value_name = "SERVICE",
        requires_all = ["enroll_application", "retire_credential_keyring_account"],
        conflicts_with = "replace_credential"
    )]
    pub(super) retire_credential_keyring_service: Option<String>,
    /// Source keyring account to revoke and delete after successful enrollment.
    #[arg(
        long,
        value_name = "ACCOUNT",
        requires_all = ["enroll_application", "retire_credential_keyring_service"],
        conflicts_with = "replace_credential"
    )]
    pub(super) retire_credential_keyring_account: Option<String>,
    /// Revoke one public API credential by its non-secret canonical UUID.
    #[arg(
        long,
        value_name = "CREDENTIAL_ID",
        requires = "public_api_dir",
        conflicts_with = "enroll_application"
    )]
    pub(super) revoke_credential: Option<String>,
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
        /// Use isolated redb state for source development.
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
        /// Journal storage protection and key provider.
        #[arg(long, value_enum, default_value = "none")]
        storage_keys: StorageKeys,
    },
    /// Parse and print the active configuration with references intact.
    Show,
    /// Show credential-free effective tool and action resolution.
    Effective,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(super) enum StorageKeys {
    /// Hash-chained plaintext storage with no external key requirement.
    #[default]
    None,
    /// OS credential-store backed journal encryption and signing.
    Platform,
    /// Environment-backed journal encryption and signing references.
    Environment,
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
    /// Fully verify payloads, chain, indexes, and any configured checkpoint/anchor.
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
