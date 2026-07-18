use super::*;

/// Configured research depth controlling bounded query and worker budgets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchDepth {
    /// Minimal evidence collection for a fast answer.
    Quick,
    /// Balanced default collection and synthesis.
    Standard,
    /// Largest configured evidence budget.
    Deep,
}

/// Supported research evidence lane.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchSourceKind {
    /// Read-only evidence from the active repository.
    Repo,
    /// Policy-authorized web evidence.
    Web,
    /// Policy-authorized MCP evidence.
    Mcp,
}

/// Durable research lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchStatus {
    /// Planning, collection, extraction, or synthesis is active.
    Running,
    /// A cited report and its evidence were durably committed.
    Completed,
    /// The run terminated with bounded failure evidence.
    Failed,
    /// Process loss abandoned the run; it is never silently retried.
    Interrupted,
}

/// Durable outcome for one planned evidence lane and query.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchLaneStatus {
    /// Evidence collection has not reached a terminal outcome.
    Pending,
    /// The lane produced zero or more bounded source records.
    Completed,
    /// Configuration intentionally disabled this lane.
    Disabled,
    /// Policy denied the lane before evidence was released.
    Denied,
    /// The adapter or provider failed with a known outcome.
    Failed,
    /// A bounded scheduler skipped the lane.
    Skipped,
}

/// Canonical outcome of one query against one research lane.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchLane {
    /// Stable lane identifier inside the run.
    pub id: String,
    /// Planned query supplied to the adapter.
    pub query: String,
    /// Evidence adapter class.
    pub kind: ResearchSourceKind,
    /// Current durable outcome.
    pub status: ResearchLaneStatus,
    /// Bounded limitation or outcome detail.
    pub message: String,
    /// Number of canonical source records produced by this lane.
    pub source_count: usize,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// Durable research orchestration phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    /// Bounded query generation.
    Planning,
    /// Evidence adapter execution.
    Collecting,
    /// Source-backed claim extraction.
    Workers,
    /// Citation-bearing report assembly.
    Synthesis,
    /// Startup abandonment detection without implicit retry.
    Recovery,
}

/// Durable progress outcome for one phase action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchProgressStatus {
    /// Work has started.
    Started,
    /// Work completed using its preferred implementation.
    Completed,
    /// Deterministic fallback completed after an unavailable or invalid model result.
    Fallback,
    /// Work was skipped by a configured bound.
    Skipped,
    /// Work failed with a known bounded outcome.
    Failed,
}

/// Canonical bounded progress record retained in the research aggregate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchProgress {
    /// Stable progress identifier.
    pub id: String,
    /// Orchestration phase.
    pub phase: ResearchPhase,
    /// Stable action label such as `queries` or `source:R1`.
    pub action: String,
    /// Current action outcome.
    pub status: ResearchProgressStatus,
    /// Bounded human-readable detail.
    pub message: String,
    /// Optional one-based position.
    pub current: Option<usize>,
    /// Optional bounded total.
    pub total: Option<usize>,
    /// UTC timestamp.
    pub created_at: String,
}

/// Canonical bounded evidence record retained with a research run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchSource {
    /// Stable source identifier.
    pub id: String,
    /// Owning research run.
    pub run_id: String,
    /// Stable report label such as `R1`.
    pub label: String,
    /// Evidence adapter class.
    pub kind: ResearchSourceKind,
    /// Human-readable source title.
    pub title: String,
    /// Bounded source URI or repository path.
    pub uri: String,
    /// Bounded released evidence content.
    pub content: String,
    /// Query that produced this source.
    pub query: String,
    /// Bounded non-secret metadata.
    pub metadata: std::collections::BTreeMap<String, String>,
    /// UTC creation timestamp.
    pub created_at: String,
}

/// One extracted statement tied to canonical source labels.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchClaim {
    /// Stable claim identifier.
    pub id: String,
    /// Owning research run.
    pub run_id: String,
    /// Bounded claim text.
    pub text: String,
    /// One or more canonical evidence labels.
    pub source_labels: Vec<String>,
    /// UTC creation timestamp.
    pub created_at: String,
}

/// Canonical research run reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchRun {
    /// Stable run identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Original research question.
    pub question: String,
    /// Configured evidence depth.
    pub depth: ResearchDepth,
    /// Requested evidence lanes in stable order.
    pub source_kinds: Vec<ResearchSourceKind>,
    /// Current durable lifecycle state.
    pub status: ResearchStatus,
    /// Planned bounded query list.
    pub queries: Vec<String>,
    /// Per-query lane outcomes, including denied and unavailable work.
    pub lanes: Vec<ResearchLane>,
    /// Ordered durable phase activity, including deterministic fallbacks.
    #[serde(default)]
    pub progress: Vec<ResearchProgress>,
    /// Explicit limitations carried into synthesis.
    pub limitations: Vec<String>,
    /// Final citation-bearing Markdown report.
    pub report: String,
    /// Bounded terminal failure detail.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC terminal timestamp.
    pub completed_at: Option<String>,
}

/// Metadata-only persisted event reference used by telemetry details.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryEventRecord {
    /// Durable global journal sequence.
    pub sequence: u64,
    /// Stable journal event identifier.
    pub event_id: String,
    /// Derived run identifier.
    pub run_id: String,
    /// Typed persisted event name.
    pub event_type: String,
    /// Event classification without payload disclosure.
    pub classification: EventClassification,
    /// Actor provenance type.
    pub actor_type: ActorType,
    /// Bounded actor identifier.
    pub actor_id: String,
    /// Correlation and lineage identifiers only.
    pub context: ExecutionContext,
    /// UTC persisted timestamp.
    pub created_at: String,
    /// Hash of the encrypted event's plaintext payload.
    pub payload_hash: String,
    /// Encrypted payload byte estimate, never plaintext.
    pub encrypted_payload_bytes: usize,
}

/// Metadata-only telemetry summary derived from persisted run events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTelemetrySummary {
    /// Stable run identifier.
    pub run_id: String,
    /// Owning session when present.
    pub session_id: Option<String>,
    /// First persisted run event timestamp.
    pub started_at: String,
    /// Last persisted run event timestamp.
    pub last_event_at: String,
    /// Non-negative timestamp-derived wall duration.
    pub duration_seconds: f64,
    /// Total matching persisted events.
    pub events: usize,
    /// Stable event-type histogram.
    pub event_types: std::collections::BTreeMap<String, usize>,
    /// Visible model output character count derived only from typed output events.
    pub model_output_chars: usize,
    /// Requested tool-call count.
    pub tool_calls: usize,
    /// Nonzero tool results plus terminal tool errors.
    pub tool_errors: usize,
    /// Approval decisions requested from an operator.
    pub approval_requests: usize,
    /// Explicit automatic/full-access approval proofs.
    pub auto_approvals: usize,
    /// Persisted risk assessment events.
    pub risk_assessments: usize,
    /// Persisted research lifecycle events.
    pub research_events: usize,
    /// Persisted subagent lifecycle events.
    pub subagent_events: usize,
    /// Context preparations that created or used compaction.
    pub context_compactions: usize,
    /// Recoverable, terminal, failed, denied-release, or unknown errors.
    pub error_events: usize,
    /// Final visible outputs.
    pub final_outputs: usize,
    /// Provider-reported prompt/input tokens.
    pub provider_input_tokens: u64,
    /// Provider-reported completion/output tokens.
    pub provider_output_tokens: u64,
    /// Provider-reported total tokens.
    pub provider_total_tokens: u64,
    /// Provider-reported cached input tokens when available.
    pub provider_cached_input_tokens: u64,
    /// Provider-reported reasoning tokens when available.
    pub provider_reasoning_tokens: u64,
}

/// Bounded telemetry timeline and its summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTelemetryDetail {
    /// Aggregate summary.
    pub summary: RunTelemetrySummary,
    /// Metadata-only records in durable sequence order.
    pub records: Vec<TelemetryEventRecord>,
    /// True when the configured record bound omitted later records.
    pub truncated: bool,
}

/// Aggregate metadata-only metrics over bounded recent runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryMetrics {
    /// Included run count.
    pub run_count: usize,
    /// Included event count.
    pub event_count: usize,
    /// Mean timestamp-derived duration.
    pub average_duration_seconds: f64,
    /// Maximum timestamp-derived duration.
    pub max_duration_seconds: f64,
    /// Visible output character total.
    pub model_output_chars: usize,
    /// Tool-call total.
    pub tool_calls: usize,
    /// Tool-error total.
    pub tool_errors: usize,
    /// Approval-request total.
    pub approval_requests: usize,
    /// Automatic-approval total.
    pub auto_approvals: usize,
    /// Risk-assessment total.
    pub risk_assessments: usize,
    /// Research-event total.
    pub research_events: usize,
    /// Subagent-event total.
    pub subagent_events: usize,
    /// Context-compaction total.
    pub context_compactions: usize,
    /// Error-event total.
    pub error_events: usize,
    /// Final-output total.
    pub final_outputs: usize,
    /// Provider-reported prompt/input token total.
    pub provider_input_tokens: u64,
    /// Provider-reported completion/output token total.
    pub provider_output_tokens: u64,
    /// Provider-reported token total.
    pub provider_total_tokens: u64,
    /// Provider-reported cached input token total.
    pub provider_cached_input_tokens: u64,
    /// Provider-reported reasoning token total.
    pub provider_reasoning_tokens: u64,
    /// Aggregate stable event-type histogram.
    pub event_types: std::collections::BTreeMap<String, usize>,
}
