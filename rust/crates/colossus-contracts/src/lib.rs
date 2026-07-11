//! Versioned serializable contracts crossing Colossus boundaries.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Serializable actor provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    /// A human operator.
    User,
    /// A model-controlled agent.
    Model,
    /// A durable workflow.
    Workflow,
    /// A delegated child agent.
    Subagent,
    /// A trusted internal service.
    System,
}

/// Serializable journal classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventClassification {
    /// A domain lifecycle event.
    Domain,
    /// An effect lifecycle event.
    Effect,
    /// A policy decision event.
    Policy,
    /// An approval event.
    Approval,
    /// A workflow lifecycle event.
    Workflow,
    /// A trusted runtime event.
    System,
}

/// Serializable authorization phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPhase {
    /// Before adapter execution.
    PreEffect,
    /// Before quarantined content release.
    PostEffect,
}

/// Serializable strict policy outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// Allow with obligations.
    Allow,
    /// Deny.
    Deny,
    /// Require proof and re-evaluation.
    RequireApproval,
}

/// Serializable optional risk availability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskStatus {
    /// Risk input exists.
    Available,
    /// Risk input is unavailable.
    Unavailable,
}

/// Serializable durable workflow status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    /// Accepted but not started.
    Queued,
    /// Executing.
    Running,
    /// Waiting for input or approval.
    Waiting,
    /// Completed.
    Completed,
    /// Failed.
    Failed,
    /// Cancelled.
    Cancelled,
    /// Interrupted by process loss.
    Interrupted,
}

/// Actor identity and immutable provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    /// Provenance category.
    pub actor_type: ActorType,
    /// Stable actor identifier.
    pub id: String,
}

/// Correlation identifiers copied through journal, policy, and workflow events.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionContext {
    /// Correlation identifier for the whole operation.
    pub correlation_id: String,
    /// Identifier of the event or request that caused this operation.
    pub causation_id: Option<String>,
    /// Active session identifier.
    pub session_id: Option<String>,
    /// Active run identifier.
    pub run_id: Option<String>,
    /// Active bounded-autonomy goal.
    pub goal_id: Option<String>,
    /// Approved plan lineage for this run.
    pub plan_id: Option<String>,
    /// Durable child-agent job lineage for this run.
    pub subagent_id: Option<String>,
    /// Declarative active skill identities; these do not grant capabilities.
    #[serde(default)]
    pub skill_ids: Vec<String>,
    /// Pinned workflow identifier.
    pub workflow_id: Option<String>,
    /// Pinned workflow content hash.
    pub workflow_hash: Option<String>,
    /// Active workflow step identifier.
    pub step_id: Option<String>,
    /// One-based workflow attempt number.
    pub attempt: Option<u32>,
}

/// An event before the journal assigns durable envelope fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewEvent {
    /// Event schema version.
    pub event_version: u16,
    /// Aggregate stream identifier.
    pub stream_id: String,
    /// Required optimistic concurrency version.
    pub expected_stream_version: u64,
    /// Security and product classification.
    pub classification: EventClassification,
    /// Versioned event name.
    pub event_type: String,
    /// Actor responsible for the event.
    pub actor: Actor,
    /// Shared execution context.
    pub context: ExecutionContext,
    /// Logical event payload, encrypted by the journal adapter.
    pub payload: Value,
}

/// Descriptor for an encrypted journal payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedPayload {
    /// Key identifier, never key material.
    pub key_id: String,
    /// Authenticated encryption algorithm.
    pub algorithm: String,
    /// Hex-encoded nonce.
    pub nonce: String,
    /// Hex-encoded ciphertext and authentication tag.
    pub ciphertext: String,
    /// Hash of canonical plaintext bytes.
    pub plaintext_hash: String,
}

/// Immutable event journal envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    /// Envelope schema version.
    pub schema_version: u16,
    /// Event schema version.
    pub event_version: u16,
    /// `UUIDv7` event identifier.
    pub event_id: String,
    /// Monotonic sequence across all streams.
    pub global_sequence: u64,
    /// Aggregate stream identifier.
    pub stream_id: String,
    /// One-based stream version.
    pub stream_version: u64,
    /// Security and product classification.
    pub classification: EventClassification,
    /// Versioned event name.
    pub event_type: String,
    /// Actor responsible for the event.
    pub actor: Actor,
    /// Shared execution context.
    pub context: ExecutionContext,
    /// UTC RFC3339 timestamp.
    pub occurred_at: String,
    /// Encrypted event payload.
    pub payload: EncryptedPayload,
    /// Hash of the previous record, or all zeroes for genesis.
    pub previous_hash: String,
    /// Hash of the complete chained record.
    pub record_hash: String,
}

/// An append-only signed chain checkpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCheckpoint {
    /// Last covered global sequence.
    pub global_sequence: u64,
    /// Last covered record hash.
    pub record_hash: String,
    /// Signing key identifier.
    pub key_id: String,
    /// Signature algorithm.
    pub algorithm: String,
    /// Hex-encoded signature.
    pub signature: String,
    /// UTC RFC3339 creation time.
    pub created_at: String,
}

/// One journal event queued for deterministic projection replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionWorkItem {
    /// Global journal sequence to apply.
    pub global_sequence: u64,
    /// Event identifier expected at that sequence.
    pub event_id: String,
}

/// One atomic projection record change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionMutation {
    /// Create or replace a projection value.
    Upsert {
        /// Projection-local record key.
        key: String,
        /// Fully materialized projection value.
        value: Value,
    },
    /// Remove a projection value.
    Delete {
        /// Projection-local record key.
        key: String,
    },
}

/// Atomic projection update and optimistic position advance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionBatch {
    /// Stable projection name and schema version.
    pub projection: String,
    /// Position the caller observed before projecting.
    pub expected_position: u64,
    /// Last global sequence represented by this batch.
    pub through_sequence: u64,
    /// Zero or more record changes. Empty batches still advance the position.
    pub mutations: Vec<ProjectionMutation>,
}

/// Bounded projection readiness and lag report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionStatus {
    /// Projection name.
    pub projection: String,
    /// Last applied global journal sequence.
    pub position: u64,
    /// Current journal head.
    pub journal_head: u64,
    /// Number of unapplied journal records.
    pub lag: u64,
    /// Whether the projection can serve current reads.
    pub ready: bool,
}

/// Declared risk passed into policy as non-authoritative input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskInput {
    /// Whether an assessment is available.
    pub status: RiskStatus,
    /// Optional bounded risk level.
    pub level: Option<String>,
    /// Optional bounded explanation.
    pub reason: Option<String>,
}

/// A reference to a credential whose value is deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReference {
    /// Configuration reference such as `env:OPENAI_API_KEY`.
    pub reference: String,
    /// Hash that can prove which value was used without disclosing it.
    pub value_hash: Option<String>,
}

/// Provider-neutral message role.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMessageRole {
    /// Trusted run instructions.
    System,
    /// Human or application input.
    User,
    /// Prior visible model output.
    Assistant,
    /// Result of an explicitly authorized tool call.
    Tool,
}

/// One provider-neutral model message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelMessage {
    /// Message provenance role.
    pub role: ModelMessageRole,
    /// Visible bounded text.
    pub content: String,
    /// Tool-call identifier for tool results.
    pub tool_call_id: Option<String>,
    /// Strict assistant tool calls preserved for provider continuation.
    #[serde(default)]
    pub tool_calls: Vec<ModelToolCall>,
}

/// Durable reconstructed local session summary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    /// Stable session identifier.
    pub id: String,
    /// Optional bounded human-readable title.
    pub title: Option<String>,
    /// UTC creation timestamp from the canonical creation event.
    pub created_at: String,
    /// UTC timestamp of the last canonical session event.
    pub updated_at: String,
    /// Number of persisted conversation messages.
    pub message_count: u64,
    /// Last attached run identifier.
    pub last_run_id: Option<String>,
    /// Bounded recent user-message preview.
    pub last_user_preview: Option<String>,
}

/// Durable append-only session message record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionMessage {
    /// Owning session identifier.
    pub session_id: String,
    /// Run that produced or consumed the message.
    pub run_id: String,
    /// One-based sequence within the session conversation.
    pub sequence: u64,
    /// Provider-neutral message content and tool correlation.
    pub message: ModelMessage,
    /// UTC timestamp from the canonical message event.
    pub created_at: String,
}

/// Immutable durable context snapshot derived from a bounded session prefix.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSnapshot {
    /// Stable snapshot identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// First canonical message sequence represented by the snapshot.
    pub source_start_sequence: u64,
    /// Last canonical message sequence represented by the snapshot.
    pub source_end_sequence: u64,
    /// Bounded future-context summary.
    pub summary: String,
    /// Durable requirements and facts extracted from the source range.
    pub pinned_facts: Vec<String>,
    /// Unfinished user requests extracted from the source range.
    pub open_tasks: Vec<String>,
    /// Bounded file paths observed in tool results.
    pub files_touched: Vec<String>,
    /// Bounded notable tool outcomes.
    pub notable_tool_results: Vec<String>,
    /// `deterministic` or `hybrid_model`.
    pub strategy: String,
    /// UTC creation timestamp from the journal envelope.
    pub created_at: String,
}

/// Effective context budget and active snapshot status for one session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextStatus {
    /// Owning session identifier.
    pub session_id: String,
    /// Number of canonical messages, which are never deleted by compaction.
    pub message_count: u64,
    /// Estimated tokens in the unmodified logical request.
    pub raw_token_estimate: u64,
    /// Estimated tokens in the provider-visible prepared request.
    pub token_estimate: u64,
    /// Configured fallback context window.
    pub context_window_tokens: u64,
    /// Automatic compaction threshold.
    pub threshold_tokens: u64,
    /// Post-compaction target.
    pub target_tokens: u64,
    /// Currently active snapshot identifier.
    pub active_snapshot_id: Option<String>,
    /// Whether an active snapshot changes provider-visible history.
    pub compacted: bool,
    /// Whether automatic compaction is enabled.
    pub auto_compaction: bool,
}

/// Provider-visible context prepared for one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedContext {
    /// Ordered messages released to the provider.
    pub messages: Vec<ModelMessage>,
    /// Estimated tokens in the released request.
    pub token_estimate: u64,
    /// Estimated tokens before snapshot application.
    pub original_token_estimate: u64,
    /// Configured model context window.
    pub context_window_tokens: u64,
    /// Automatic compaction threshold.
    pub threshold_tokens: u64,
    /// Post-compaction target.
    pub target_tokens: u64,
    /// Snapshot used for this request.
    pub snapshot_id: Option<String>,
    /// Whether history was compacted.
    pub compacted: bool,
    /// Whether this preparation created a new snapshot.
    pub snapshot_created: bool,
    /// Snapshot strategy when compacted.
    pub strategy: Option<String>,
}

/// Durable task lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Accepted but not started.
    Pending,
    /// Currently being worked.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Cannot progress without an external change or input.
    Blocked,
    /// Explicitly abandoned.
    Cancelled,
}

/// Canonical session-scoped task state reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    /// Stable task identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Bounded human-readable title.
    pub title: String,
    /// Bounded supporting detail.
    pub description: String,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// Durable plan lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Proposed work that may still be edited or discarded.
    Draft,
    /// Explicitly approved for one execution or goal handoff.
    Approved,
    /// Consumed by an execution run.
    Executed,
    /// Retained for audit but intentionally abandoned.
    Discarded,
}

/// One ordered, bounded plan step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// One-based stable order within the plan.
    pub index: u32,
    /// Short human-readable action label.
    pub title: String,
    /// Supporting implementation or verification detail.
    pub detail: String,
    /// Whether executing this step may mutate external state.
    pub requires_mutation: bool,
}

/// Canonical session-scoped plan reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRecord {
    /// Stable plan identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Original objective that produced the plan.
    pub prompt: String,
    /// Current lifecycle status.
    pub status: PlanStatus,
    /// Optional bounded Markdown overview.
    pub content: String,
    /// Ordered executable intent without inline code semantics.
    pub steps: Vec<PlanStep>,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// Approval timestamp when approved.
    pub approved_at: Option<String>,
    /// Run that consumed the approved plan.
    pub executed_run_id: Option<String>,
}

/// Durable bounded-autonomy goal status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Further bounded iterations may run.
    Active,
    /// Objective was genuinely achieved.
    Complete,
    /// Progress requires user input or an external state change.
    Blocked,
}

/// Canonical goal state reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRecord {
    /// Stable goal identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Bounded objective preserved across iterations.
    pub objective: String,
    /// Optional approved plan that originated this goal.
    pub source_plan_id: Option<String>,
    /// Current terminal or active state.
    pub status: GoalStatus,
    /// Concise completion or progress summary.
    pub summary: String,
    /// Required explanation when blocked.
    pub blocked_reason: String,
    /// Maximum autonomous iterations.
    pub iteration_budget: u16,
    /// Completed iterations, never greater than the budget.
    pub iterations_completed: u16,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One completed bounded Goal Mode iteration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalIterationResult {
    /// One-based iteration number.
    pub iteration: u16,
    /// Normal agent run identifier.
    pub run_id: String,
    /// Visible final output for this iteration.
    pub output: String,
    /// Durable run event count.
    pub event_count: u64,
    /// Iteration wall time.
    pub elapsed_seconds: f64,
}

/// Result of a bounded Goal Mode loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRunResult {
    /// Final reconstructed goal state.
    pub goal: GoalRecord,
    /// Completed normal agent runs.
    pub iterations: Vec<GoalIterationResult>,
    /// True when the budget ended while the goal remained active.
    pub iteration_budget_exhausted: bool,
    /// Total loop wall time.
    pub elapsed_seconds: f64,
}

/// Durable subagent job status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    /// Waiting for scheduler capacity.
    Queued,
    /// Child agent run is in progress.
    Running,
    /// Child run returned a released final result.
    Completed,
    /// Child run failed with a bounded redacted error.
    Failed,
    /// Operator cancelled the job.
    Cancelled,
    /// Process loss left a previously running job unfinished.
    Interrupted,
}

/// Canonical durable child-agent job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentJob {
    /// Stable job identifier.
    pub id: String,
    /// Parent session identifier.
    pub session_id: String,
    /// Run that requested delegation.
    pub parent_run_id: String,
    /// Tool call that requested delegation.
    pub parent_call_id: String,
    /// Bounded child objective.
    pub task: String,
    /// Configured model role.
    pub role: String,
    /// Current lifecycle state.
    pub status: SubagentStatus,
    /// Isolated durable child session.
    pub child_session_id: String,
    /// Completed child run identifier.
    pub child_run_id: Option<String>,
    /// Bounded released child output.
    pub final_output: String,
    /// Bounded redacted terminal error.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC start timestamp.
    pub started_at: Option<String>,
    /// UTC terminal timestamp.
    pub completed_at: Option<String>,
}

/// Bounded scheduler status snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentQueueStatus {
    /// Total matching jobs.
    pub total: usize,
    /// Jobs awaiting capacity.
    pub queued: usize,
    /// Jobs currently executing.
    pub running: usize,
    /// Successfully completed jobs.
    pub completed: usize,
    /// Failed jobs.
    pub failed: usize,
    /// Cancelled jobs.
    pub cancelled: usize,
    /// Recovery-interrupted jobs.
    pub interrupted: usize,
    /// Configured scheduler ceiling.
    pub max_concurrent: usize,
    /// Currently available local slots.
    pub available_slots: usize,
}

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
    /// Aggregate stable event-type histogram.
    pub event_types: std::collections::BTreeMap<String, usize>,
}

/// Strict declarative skill manifest. Skills carry context, never executable privilege.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillManifest {
    /// Stable skill identifier.
    pub name: String,
    /// Human-readable version.
    pub version: String,
    /// Bounded discovery summary.
    pub description: String,
    /// Prompt terms that may activate the skill.
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Tool names that must already be active; skills never activate tools.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Declarative labels supplied to policy as context only.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Whether the instructions require no network or external integration.
    #[serde(default = "default_true")]
    pub offline_compatible: bool,
}

fn default_true() -> bool {
    true
}

/// Loaded data-only skill and its bounded filesystem provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRecord {
    /// Validated manifest.
    pub manifest: SkillManifest,
    /// Prompt instructions with frontmatter removed.
    pub instructions: String,
    /// Stable provenance label such as `repository:name`.
    pub source: String,
    /// Canonical resource root used only by the trusted resource service.
    pub resource_root: String,
}

/// One deterministic duplicate-resolution result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDuplicate {
    /// Duplicated skill name.
    pub name: String,
    /// Source selected by configured precedence.
    pub selected_source: String,
    /// Every source in precedence order.
    pub sources: Vec<String>,
}

/// Safe metadata for one available or active skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillMetadata {
    /// Skill name.
    pub name: String,
    /// Skill version.
    pub version: String,
    /// Skill description.
    pub description: String,
    /// Provenance label.
    pub source: String,
}

/// Result of deterministic prompt-context skill composition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillComposition {
    /// Original instructions plus bounded skill context.
    pub instructions: String,
    /// Metadata for every enabled skill.
    pub available_skills: Vec<SkillMetadata>,
    /// Metadata for skills activated on this turn.
    pub active_skills: Vec<SkillMetadata>,
}

/// One bounded resource visible under an active data-only skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceEntry {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// File size before reading.
    pub size: u64,
    /// Allowed top-level resource directory.
    pub kind: String,
}

/// One bounded UTF-8 resource read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceRead {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact UTF-8 byte length.
    pub size: u64,
    /// Released text content.
    pub content: String,
}

/// Metadata-only inventory entry for one authorable skill file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileEntry {
    /// POSIX path relative to the skill root.
    pub path: String,
    /// Exact file size.
    pub size: u64,
    /// SHA-256 of the file bytes.
    pub sha256: String,
}

/// Bounded inspection result for one installed or local skill directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInspection {
    /// Validated manifest.
    pub manifest: SkillManifest,
    /// Stable source label without instruction content.
    pub source: String,
    /// Deterministic metadata-only file inventory.
    pub files: Vec<SkillFileEntry>,
    /// Hash over the validated manifest, instructions, and file inventory.
    pub content_sha256: String,
}

/// One bounded UTF-8 authoring read from an installed user skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileRead {
    /// Installed skill name.
    pub name: String,
    /// POSIX path relative to its root.
    pub path: String,
    /// Exact UTF-8 byte length.
    pub size: u64,
    /// SHA-256 used for optimistic writes.
    pub sha256: String,
    /// Released text content.
    pub content: String,
}

/// Result of an atomic optimistic-concurrency authoring write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillWriteResult {
    /// Installed skill name.
    pub name: String,
    /// POSIX path relative to its root.
    pub path: String,
    /// Hash observed before replacement, absent for a new file.
    pub previous_sha256: Option<String>,
    /// Hash of the committed content.
    pub sha256: String,
    /// Whether the file was newly created.
    pub created: bool,
}

/// Result of creating a new installed data-only skill skeleton.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillScaffoldResult {
    /// Installed skill name.
    pub name: String,
    /// Files created relative to the skill root.
    pub files: Vec<String>,
    /// Hash of the validated installed skill.
    pub content_sha256: String,
}

/// Result of validating an installed or workspace-local skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillValidationResult {
    /// Validated skill name.
    pub name: String,
    /// Stable source label.
    pub source: String,
    /// Deterministic file count.
    pub file_count: usize,
    /// Hash of the validated skill.
    pub content_sha256: String,
}

/// Result of installing a validated workspace-local skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillInstallResult {
    /// Installed skill name.
    pub name: String,
    /// Source hash copied into the user library.
    pub content_sha256: String,
    /// Deterministic number of installed files.
    pub file_count: usize,
}

/// Integration protocol or adapter family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    /// Built-in typed connector.
    Native,
    /// Imported JSON OpenAPI operations.
    OpenApi,
    /// Configured Model Context Protocol server.
    Mcp,
}

/// Connection readiness reconstructed from canonical events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    /// Valid and available to the dynamic tool registry.
    Connected,
    /// Structurally valid but its credential reference is currently unresolved.
    PendingAuth,
    /// Explicitly disconnected and hidden from the tool registry.
    Disconnected,
}

/// Credential placement performed only by the permit-bearing adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntegrationAuth {
    /// No credential is required.
    None,
    /// Send a bearer-like authorization value.
    Bearer {
        /// Header name, normally `Authorization`.
        header: String,
        /// Scheme prefix, normally `Bearer`.
        scheme: String,
    },
    /// Send the secret in a configured header with an optional scheme prefix.
    ApiKey {
        /// Header name.
        header: String,
        /// Optional value prefix.
        scheme: Option<String>,
    },
    /// Send a service-account value in a configured header.
    ServiceAccount {
        /// Header name.
        header: String,
    },
}

/// One compiled integration operation and its strict model-visible schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationOperation {
    /// Dynamic namespaced tool specification.
    pub tool: ToolSpec,
    /// Stable source operation identifier.
    pub operation_id: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// Relative path template.
    pub path: String,
    /// Arguments substituted into path placeholders.
    pub path_parameters: Vec<String>,
    /// Arguments encoded into the query string.
    pub query_parameters: Vec<String>,
    /// Whether an optional or required `body` argument is supported.
    pub accepts_body: bool,
}

/// Canonical integration connection state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationConnection {
    /// Stable lowercase connection name.
    pub name: String,
    /// Protocol or adapter family.
    pub kind: IntegrationKind,
    /// Current readiness.
    pub status: IntegrationStatus,
    /// Human-facing title.
    pub title: String,
    /// Bounded description.
    pub description: String,
    /// Canonical API base URL without credentials, query, or fragment.
    pub base_url: String,
    /// Adapter-only credential placement.
    pub auth: IntegrationAuth,
    /// Local credential handle, never its value.
    pub credential_reference: Option<String>,
    /// Declared authorization scopes.
    pub scopes: Vec<String>,
    /// Compiled operations hidden unless status is connected.
    pub operations: Vec<IntegrationOperation>,
    /// SHA-256 of the imported source schema or native manifest.
    pub manifest_sha256: String,
    /// Original creation timestamp.
    pub connected_at: String,
    /// Last lifecycle event timestamp.
    pub updated_at: String,
}

/// Safe connection summary for CLI and diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationSummary {
    /// Connection name.
    pub name: String,
    /// Protocol family.
    pub kind: IntegrationKind,
    /// Current readiness.
    pub status: IntegrationStatus,
    /// Human-facing title.
    pub title: String,
    /// Credential handle without a value.
    pub credential_reference: Option<String>,
    /// Dynamic tool names.
    pub tools: Vec<String>,
    /// Last lifecycle timestamp.
    pub updated_at: String,
}

/// Provenance for a durable key decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// Explicitly supplied by the user.
    User,
    /// Interpreted and recorded by the agent.
    Agent,
}

/// Durable key-decision lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Binding future-facing guidance.
    Active,
    /// Preserved for audit but no longer injected.
    Archived,
    /// Replaced by a newer decision.
    Superseded,
}

/// Binding priority for a durable key decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPriority {
    /// Highest-priority invariant or user commitment.
    Critical,
    /// Important guidance.
    High,
    /// Normal durable guidance.
    Normal,
}

/// Canonical future-facing key decision reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDecision {
    /// Stable decision identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Optional originating goal.
    pub goal_id: Option<String>,
    /// Optional originating plan.
    pub plan_id: Option<String>,
    /// User or agent provenance.
    pub source: DecisionSource,
    /// Active, archived, or superseded.
    pub status: DecisionStatus,
    /// Binding priority.
    pub priority: DecisionPriority,
    /// Bounded label.
    pub title: String,
    /// Interpreted future-facing commitment.
    pub decision: String,
    /// User intent preserved separately from the commitment.
    pub intent: String,
    /// Conditions under which the commitment applies.
    pub applies_when: String,
    /// Bounded supporting rationale.
    pub rationale: String,
    /// Bounded source excerpt, not the entire raw prompt.
    pub source_excerpt: String,
    /// Older decision replaced by this record.
    pub supersedes: Option<String>,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// Provider-neutral assistant tool call preserved between turns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolCall {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Validated object arguments.
    pub arguments: Value,
}

/// One strict function tool exposed to a provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelToolDefinition {
    /// Stable tool name.
    pub name: String,
    /// Bounded human-readable description.
    pub description: String,
    /// JSON Schema for object arguments.
    pub input_schema: Value,
}

/// Application tool specification and effect identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Stable model-visible name.
    pub name: String,
    /// Bounded model-visible description.
    pub description: String,
    /// Strict JSON Schema for object arguments.
    pub input_schema: Value,
    /// Effect action when the tool is effectful; absent for pure computation.
    pub effect_action: Option<String>,
    /// Requested capability identity when effectful.
    pub capability: Option<String>,
    /// Tool-specific normalized output ceiling.
    pub max_output_bytes: u64,
}

/// One validated model-requested tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Strict object arguments.
    pub arguments: Value,
}

/// One bounded tool result suitable for provider continuation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered tool name.
    pub name: String,
    /// Bounded UTF-8 or JSON output.
    pub output: String,
    /// Conventional zero-success exit code.
    pub exit_code: i32,
}

/// Provider-neutral request for one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    /// Selected model identifier.
    pub model: String,
    /// Trusted instructions supplied separately from conversation messages.
    pub instructions: String,
    /// Ordered conversation messages.
    pub messages: Vec<ModelMessage>,
    /// Strict tools available for this turn.
    pub tools: Vec<ModelToolDefinition>,
}

/// Provider-neutral safe event produced by one model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderEvent {
    /// Visible text delta. Non-streaming providers emit one complete delta.
    ModelDelta {
        /// Visible text.
        text: String,
    },
    /// Provider-declared safe reasoning summary, never hidden reasoning.
    ReasoningSummary {
        /// Bounded safe summary.
        summary: String,
    },
    /// Strict validated tool call request.
    ToolCallRequested {
        /// Provider call identifier.
        call_id: String,
        /// Registered tool name.
        name: String,
        /// Parsed JSON object arguments.
        arguments: Value,
    },
    /// Final visible assistant text.
    FinalOutput {
        /// Complete visible output.
        text: String,
    },
}

/// Normalized result of one provider turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTurn {
    /// Configured profile name.
    pub profile: String,
    /// Provider adapter kind.
    pub provider: String,
    /// Model identifier used by the request.
    pub model: String,
    /// Provider response identifier when supplied.
    pub response_id: Option<String>,
    /// Ordered safe normalized events.
    pub events: Vec<ProviderEvent>,
}

/// Resolved provider route metadata without credentials.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRoute {
    /// Requested logical role.
    pub role: String,
    /// Resolved profile name.
    pub profile: String,
    /// Provider adapter kind.
    pub provider: String,
    /// Configured model identifier.
    pub model: String,
}

/// One model visible through a provider catalog endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderModelInfo {
    /// Provider model identifier.
    pub id: String,
    /// Provider object/type label when supplied.
    pub object: Option<String>,
    /// Owning organization when supplied.
    pub owned_by: Option<String>,
}

/// One bounded provider readiness check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderReadinessCheck {
    /// Stable check name.
    pub name: String,
    /// `pass`, `fail`, `not_checked`, or `not_applicable`.
    pub status: String,
    /// Bounded detail without credentials or response bodies.
    pub detail: String,
}

/// Provider profile readiness and capability report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderReadiness {
    /// Profile name.
    pub profile: String,
    /// Adapter kind.
    pub provider: String,
    /// Whether every required check passed.
    pub ready: bool,
    /// Whether the provider supports strict tool calls.
    pub tool_calls: bool,
    /// Whether the provider supports transport streaming.
    pub streaming: bool,
    /// Ordered bounded checks.
    pub checks: Vec<ProviderReadinessCheck>,
}

/// Result of one application-level model run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunResult {
    /// Stable UUIDv7 run identifier.
    pub run_id: String,
    /// Durable session containing this run's conversation messages.
    pub session_id: Option<String>,
    /// Selected model role.
    pub role: String,
    /// Resolved provider profile.
    pub profile: String,
    /// Model used for the turn.
    pub model: String,
    /// Complete visible assistant output.
    pub output: String,
    /// Number of events durably recorded on the run stream, including preparation.
    pub event_count: u64,
    /// Elapsed wall time in fractional seconds.
    pub elapsed_seconds: f64,
}

/// Complete, versioned request sent to a policy decision point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRequest {
    /// Contract schema version.
    pub schema_version: u16,
    /// Stable request identifier.
    pub request_id: String,
    /// Actor and provenance.
    pub actor: Actor,
    /// Canonical action name.
    pub action: String,
    /// Canonical resource name.
    pub resource: String,
    /// Requested capability identities.
    pub capabilities: Vec<String>,
    /// Non-authoritative declared risk.
    pub risk: RiskInput,
    /// Complete proposed logical request or quarantined result content.
    pub content: Value,
    /// Credential references with values removed.
    pub credential_references: Vec<CredentialReference>,
    /// Correlation and workflow context.
    pub context: ExecutionContext,
    /// Optional idempotency key.
    pub idempotency_id: Option<String>,
    /// Pre-execution or post-result phase.
    pub phase: EffectPhase,
    /// Approval proof supplied only during policy re-evaluation.
    pub approval: Option<ApprovalProof>,
}

/// Evidence that a specific actor approved a specific request hash.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalProof {
    /// Approval identifier.
    pub approval_id: String,
    /// Request hash the user saw.
    pub request_hash: String,
    /// Approving actor identifier.
    pub approved_by: String,
    /// UTC RFC3339 approval time.
    pub approved_at: String,
}

/// Non-overridable and policy-provided execution obligations.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyObligations {
    /// Required sandbox backend.
    pub sandbox_backend: String,
    /// Required sandbox profile.
    pub sandbox_profile: String,
    /// Canonical filesystem roots and access modes.
    pub filesystem: Vec<FilesystemGrant>,
    /// Allowed network destination patterns.
    pub network_destinations: Vec<String>,
    /// Exact environment variable names visible to a sandboxed process.
    pub allowed_environment: Vec<String>,
    /// Whether an unavailable isolation backend may downgrade to the broker.
    pub allow_sandbox_downgrade: bool,
    /// Maximum wall-clock time in milliseconds.
    pub timeout_ms: u64,
    /// Maximum released output bytes.
    pub max_output_bytes: u64,
    /// Maximum child process count.
    pub max_processes: u32,
    /// Maximum memory bytes.
    pub max_memory_bytes: u64,
    /// Maximum effect concurrency for the actor/run.
    pub max_concurrency: u32,
    /// Fields or patterns that must be redacted.
    pub required_redactions: Vec<String>,
    /// Whether output must pass a post-effect decision.
    pub require_post_effect: bool,
    /// Labels written to audit events.
    pub audit_labels: BTreeMap<String, String>,
    /// Required retention category.
    pub retention: String,
}

/// A canonical filesystem grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemGrant {
    /// Canonical absolute root.
    pub root: String,
    /// `read`, `write`, `metadata`, or `execute`.
    pub mode: String,
}

/// Strict policy decision returned by built-in or OPA policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDecision {
    /// Stable decision identifier.
    pub decision_id: String,
    /// Policy or bundle revision.
    pub policy_revision: String,
    /// Strict decision outcome.
    pub outcome: DecisionOutcome,
    /// Human-readable bounded reason.
    pub reason: String,
    /// Complete recognized obligations.
    pub obligations: PolicyObligations,
}

/// Bounded adapter output held until optional post-effect authorization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuarantinedEffectResult {
    /// Media type of the captured bytes.
    pub media_type: String,
    /// Captured bytes, not yet released to the requester.
    pub bytes: Vec<u8>,
    /// Whether the adapter believes the external effect succeeded.
    pub effect_succeeded: bool,
}

/// Canonical memory scope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum MemoryScope {
    /// Available across sessions after relevance and policy filtering.
    Global,
    /// Restricted to a canonical repository identifier.
    Repository(String),
    /// Restricted to one session.
    Session(String),
}

/// Canonical memory lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// Eligible for policy-filtered retrieval.
    Active,
    /// Retained for history but excluded from retrieval.
    Archived,
    /// Replaced by another record and excluded from retrieval.
    Superseded,
}

/// Canonical memory record reconstructed from lifecycle events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    /// Stable memory identifier.
    pub id: String,
    /// Retrieval scope.
    pub scope: MemoryScope,
    /// Operator-defined memory kind.
    pub kind: String,
    /// Confidence in the memory, in the inclusive range 0..=1.
    pub confidence: f32,
    /// Bounded provenance label.
    pub source: String,
    /// Current lifecycle status.
    pub status: MemoryStatus,
    /// Canonical text, which must not contain secrets.
    pub text: String,
    /// Bounded rationale.
    pub rationale: String,
    /// UTC RFC3339 creation timestamp.
    pub created_at: String,
    /// UTC RFC3339 update timestamp.
    pub updated_at: String,
    /// Optional UTC RFC3339 expiry.
    pub expires_at: Option<String>,
    /// Replacement memory identifier when superseded.
    pub superseded_by: Option<String>,
}

/// Workflow metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMetadata {
    /// Stable workflow name.
    pub name: String,
    /// Operator-managed semantic version.
    pub version: String,
    /// Human-readable description.
    pub description: String,
}

/// A strict versioned workflow definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDefinition {
    /// Contract version. The initial value is `colossus.dev/v1alpha1`.
    pub api_version: String,
    /// Must be `Workflow`.
    pub kind: String,
    /// Workflow identity and display metadata.
    pub metadata: WorkflowMetadata,
    /// JSON Schema for input.
    pub inputs: Value,
    /// JSON Schema for output.
    pub outputs: Value,
    /// Maximum capabilities any step may request.
    pub capabilities: Vec<String>,
    /// Maximum simultaneously active branches.
    pub max_concurrency: u32,
    /// Maximum total step attempts.
    pub step_budget: u32,
    /// Ordered root steps.
    pub steps: Vec<WorkflowStep>,
}

/// A typed, non-executable workflow step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStep {
    /// Invoke a normal agent turn through the effect gateway.
    Agent {
        /// Stable step identifier.
        id: String,
        /// Logical prompt expression or literal.
        prompt: String,
        /// Explicit idempotency strategy, if any.
        idempotency: Option<String>,
    },
    /// Invoke a registered tool through the effect gateway.
    Tool {
        /// Stable step identifier.
        id: String,
        /// Registered tool name.
        tool: String,
        /// Strict tool arguments.
        arguments: Value,
        /// Explicit idempotency strategy, if any.
        idempotency: Option<String>,
    },
    /// Invoke another registered workflow by exact name and version.
    Workflow {
        /// Stable step identifier.
        id: String,
        /// Referenced workflow name.
        workflow: String,
        /// Referenced workflow version.
        version: String,
        /// Strict subworkflow inputs.
        inputs: Value,
    },
    /// Pause until an approval is supplied.
    Approval {
        /// Stable step identifier.
        id: String,
        /// Bounded prompt shown to the operator.
        prompt: String,
    },
    /// Branch using the non-executable expression grammar.
    Condition {
        /// Stable step identifier.
        id: String,
        /// Restricted condition expression.
        expression: String,
        /// Steps when true.
        then: Vec<WorkflowStep>,
        /// Steps when false.
        otherwise: Vec<WorkflowStep>,
    },
    /// Run bounded branches concurrently.
    Parallel {
        /// Stable step identifier.
        id: String,
        /// Branches, each executed in order internally.
        branches: Vec<Vec<WorkflowStep>>,
        /// Step-local concurrency bound.
        max_concurrency: u32,
    },
    /// Iterate over a bounded JSON array.
    Foreach {
        /// Stable step identifier.
        id: String,
        /// JSON pointer identifying the array.
        items: String,
        /// Hard iteration limit.
        max_items: u32,
        /// Steps executed for each item.
        steps: Vec<WorkflowStep>,
    },
    /// Pause until structured input is supplied.
    WaitForInput {
        /// Stable step identifier.
        id: String,
        /// Bounded operator prompt.
        prompt: String,
        /// JSON Schema for the response.
        schema: Value,
    },
    /// Emit a pure workflow value.
    Emit {
        /// Stable step identifier.
        id: String,
        /// Emitted structured value.
        value: Value,
    },
}

/// Durable workflow run projection reconstructed from events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRun {
    /// Run identifier.
    pub run_id: String,
    /// Workflow name.
    pub workflow_name: String,
    /// Workflow version.
    pub workflow_version: String,
    /// Pinned canonical definition hash.
    pub workflow_hash: String,
    /// Durable status.
    pub status: WorkflowStatus,
    /// Input snapshot.
    pub inputs: Value,
    /// Optional output snapshot.
    pub outputs: Option<Value>,
    /// Last completed root step index.
    pub completed_steps: u32,
}

#[cfg(test)]
mod tests {
    use super::PolicyDecision;

    #[test]
    fn policy_decision_rejects_unknown_fields() {
        let document = r#"{
            "decision_id":"d1","policy_revision":"r1","outcome":"deny",
            "reason":"no","obligations":{"sandbox_backend":"none",
            "sandbox_profile":"none","filesystem":[],"network_destinations":[],
            "allowed_environment":[],"allow_sandbox_downgrade":false,
            "timeout_ms":1,"max_output_bytes":1,"max_processes":0,
            "max_memory_bytes":1,"max_concurrency":1,"required_redactions":[],
            "require_post_effect":false,"audit_labels":{},"retention":"standard"},
            "surprise":true
        }"#;
        assert!(serde_json::from_str::<PolicyDecision>(document).is_err());
    }
}
