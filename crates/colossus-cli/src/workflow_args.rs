use super::*;

#[derive(Args)]
pub(super) struct WorkflowCommand {
    #[command(subcommand)]
    pub(super) command: WorkflowAction,
}

#[derive(Subcommand)]
pub(super) enum WorkflowAction {
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
pub(super) enum WorkflowScheduleAction {
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
pub(super) enum WorkflowWebhookAction {
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
pub(super) enum WorkflowSubscriptionAction {
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
pub(super) enum WorkflowScheduleMisfireArg {
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
