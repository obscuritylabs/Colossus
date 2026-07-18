use super::*;

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum MemoryScopeArg {
    Global,
    Repository,
    Session,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum MemoryStatusArg {
    Active,
    Archived,
    Superseded,
    All,
}

impl MemoryStatusArg {
    pub(super) fn status(self) -> Option<MemoryStatus> {
        match self {
            Self::Active => Some(MemoryStatus::Active),
            Self::Archived => Some(MemoryStatus::Archived),
            Self::Superseded => Some(MemoryStatus::Superseded),
            Self::All => None,
        }
    }
}

#[derive(Args)]
pub(super) struct MemoriesCommand {
    #[command(subcommand)]
    pub(super) command: MemoriesAction,
}

#[derive(Subcommand)]
pub(super) enum MemoriesAction {
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
pub(super) struct MemoryIndexCommand {
    #[command(subcommand)]
    pub(super) command: MemoryIndexAction,
}

#[derive(Subcommand)]
pub(super) enum MemoryIndexAction {
    /// Show adapter readiness and journal lag.
    Status,
    /// Retry queued journal-to-index work.
    Sync,
    /// Rebuild from canonical active records.
    Rebuild,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum ResearchDepthArg {
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
pub(super) enum ResearchSourceArg {
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
pub(super) struct ResearchCommand {
    #[command(subcommand)]
    pub(super) command: ResearchAction,
}

#[derive(Subcommand)]
pub(super) enum ResearchAction {
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
pub(super) struct TelemetryCommand {
    #[command(subcommand)]
    pub(super) command: TelemetryAction,
}

#[derive(Subcommand)]
pub(super) enum TelemetryAction {
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
