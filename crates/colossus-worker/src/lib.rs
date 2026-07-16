//! Authenticated local IPC for the single-writer Colossus worker.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    AgentRunOutcome, AgentRunResult, ApprovalProof, DecisionPriority, DecisionStatus,
    EffectRequest, GoalStatus, IntegrationAuth, MemoryScope, MemoryStatus, PlanStatus, PlanStep,
    PolicyDecision, ResearchDepth, ResearchSourceKind, RunEventEnvelope, SubagentStatus,
    TaskStatus, TerminalPreferences, UserPromptRequest, UserPromptResponse,
    WorkflowScheduleMisfirePolicy,
};
use colossus_policy::AllowApproval;
use colossus_ports::{
    ApprovalProvider, ModelProviderError, PolicyError, RunControl, RunEventObserver, ToolError,
    UserPromptProvider,
};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeError};
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use uuid::Uuid;

const PROTOCOL_VERSION: u16 = 4;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLOCK_SKEW_MS: i128 = 30_000;
const REPLAY_WINDOW: usize = 4_096;
#[cfg(not(windows))]
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(windows)]
// A missing pipe is retried briefly by the platform connector, while a pipe
// that is known to be busy receives a longer load-shedding window. Keep the
// outer bound above both so the connector can preserve that distinction.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const INTERACTIVE_PROMPT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
type HmacSha256 = Hmac<Sha256>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientHello {
    version: u16,
    challenge: String,
}

#[derive(Serialize)]
struct UnsignedServerHello<'a> {
    version: u16,
    challenge: &'a str,
    server_nonce: &'a str,
    timestamp_ms: i128,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerHello {
    version: u16,
    challenge: String,
    server_nonce: String,
    timestamp_ms: i128,
    authentication_tag: String,
}

/// Local worker transport or strict-contract failure.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// Local transport failed.
    #[error("worker transport failed: {0}")]
    Io(#[from] std::io::Error),
    /// Runtime composition or operation failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Journal or repository operation failed.
    #[error(transparent)]
    Store(#[from] colossus_ports::StoreError),
    /// Workflow validation or lifecycle operation failed.
    #[error(transparent)]
    Workflow(#[from] colossus_workflow::WorkflowError),
    /// Strict JSON serialization failed.
    #[error("worker JSON contract failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Request or response violated the authenticated protocol.
    #[error("worker protocol rejected message: {0}")]
    Protocol(String),
    /// The worker returned an application error.
    #[error("worker operation failed: {0}")]
    Remote(String),
    /// No worker answered at the configured endpoint.
    #[error("worker is unavailable at {0}")]
    Unavailable(String),
    /// A live worker could not accept another connection before the bounded deadline.
    #[error("worker is busy at {0}")]
    Busy(String),
}

/// Versioned operations exposed by the local worker application API.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerOperation {
    /// Authenticate the endpoint and return bounded readiness metadata.
    Ping,
    /// Verify the authoritative journal chain and anchors.
    AuditVerify,
    /// Read bounded redacted event envelopes.
    AuditRead {
        /// First global sequence.
        from: u64,
        /// Maximum records.
        limit: usize,
    },
    /// Inspect configured durable audit-export readiness.
    AuditExportStatus,
    /// Drain queued external audit evidence.
    AuditExportDrain,
    /// Reset and replay the external audit-export consumer.
    AuditExportReset,
    /// Check policy readiness.
    PolicyDoctor,
    /// List projection positions and lag.
    ProjectionStatus,
    /// Drain projection outbox work.
    ProjectionDrain,
    /// Rebuild one or every disposable projection.
    ProjectionRebuild {
        /// Optional exact projection name.
        name: Option<String>,
    },
    /// Inspect storage and writer readiness.
    StateDoctor,
    /// Inspect sandbox readiness.
    SandboxDoctor,
    /// List provider profile readiness without network access.
    ProviderProfiles,
    /// Exercise one provider diagnostic path.
    ProviderDoctor {
        /// Optional exact profile.
        profile: Option<String>,
    },
    /// List normalized models for one provider.
    ProviderModels {
        /// Optional exact profile.
        profile: Option<String>,
    },
    /// Show role-to-profile routing.
    ProviderRoutes,
    /// Resolve one role to bounded provider metadata without network access.
    ProviderRoute {
        /// Logical provider role.
        role: String,
    },
    /// List active model-visible tool schemas.
    ToolsList,
    /// Execute the normal audited model application path.
    RunModel {
        /// Logical role.
        role: String,
        /// Composed caller instructions.
        instructions: String,
        /// User prompt.
        prompt: String,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Optional exact durable session.
        session_id: Option<String>,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute a model run with protocol-v4 prompts and cooperative cancellation.
    RunModelControlled {
        /// Logical role.
        role: String,
        /// Composed caller instructions.
        instructions: String,
        /// User prompt.
        prompt: String,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Exact durable session.
        session_id: String,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute structurally read-only Plan Mode.
    RunPlan {
        /// Logical role.
        role: String,
        /// Caller instructions composed with mandatory planning constraints.
        instructions: String,
        /// Planning prompt.
        prompt: String,
        /// Optional bounded turn override.
        max_turns: Option<u16>,
        /// Optional exact durable session.
        session_id: Option<String>,
        /// Explicit declarative skills.
        explicit_skills: Vec<String>,
        /// TUI-sticky declarative skills.
        sticky_skills: Vec<String>,
    },
    /// Execute the permit-bound offline echo effect.
    Echo {
        /// Exact echo input.
        message: String,
    },
    /// Create an empty durable session.
    SessionCreate {
        /// Optional bounded title.
        title: Option<String>,
    },
    /// Reconstruct one session summary.
    SessionGet {
        /// Exact session identifier.
        session_id: String,
    },
    /// List recent sessions.
    SessionList {
        /// Bounded result limit.
        limit: usize,
    },
    /// Reconstruct append-only messages for one session.
    SessionMessages {
        /// Exact session identifier.
        session_id: String,
    },
    /// Reconstruct one bounded canonical session-message page.
    SessionMessagesPage {
        /// Exact session identifier.
        session_id: String,
        /// Exclusive upper sequence bound for older paging.
        before_sequence: Option<u64>,
        /// Maximum records, clamped to 100.
        limit: usize,
    },
    /// Resolve the newest session.
    SessionLatest,
    /// Refresh bounded actionable work for one session.
    WorkState {
        /// Exact session identifier.
        session_id: String,
    },
    /// Reconstruct the canonical presentation profile.
    PresentationGet,
    /// Reconstruct newest encrypted terminal history entries.
    PresentationHistory {
        /// Maximum entries in chronological order.
        limit: usize,
    },
    /// Persist a complete presentation profile through the runtime gateway.
    PresentationSave {
        /// Strict complete replacement profile.
        preferences: TerminalPreferences,
    },
    /// Append one encrypted terminal history entry through the runtime gateway.
    PresentationHistoryAppend {
        /// Exact submitted entry.
        entry: String,
    },
    /// Show context budget status.
    ContextStatus {
        /// Exact session identifier.
        session_id: String,
    },
    /// List immutable context snapshots.
    ContextList {
        /// Exact session identifier.
        session_id: String,
    },
    /// Force context compaction.
    ContextCompact {
        /// Exact session identifier.
        session_id: String,
    },
    /// Activate one context snapshot.
    ContextRestore {
        /// Exact session identifier.
        session_id: String,
        /// Exact snapshot identifier.
        snapshot_id: String,
    },
    /// List metadata-only run telemetry.
    TelemetryRuns {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// Show one metadata-only run timeline.
    TelemetryShow {
        /// Full or uniquely prefixed run identifier.
        id_or_prefix: String,
        /// Maximum timeline records.
        limit: usize,
    },
    /// Aggregate metadata-only run metrics.
    TelemetryMetrics {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum runs.
        limit: usize,
    },
    /// List canonical tasks.
    TaskList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<TaskStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one task.
    TaskGet {
        /// Exact task identifier.
        task_id: String,
    },
    /// Create a task through the effect gateway.
    TaskCreate {
        /// Owning session.
        session_id: String,
        /// Task title.
        title: String,
        /// Task detail.
        description: String,
        /// Initial lifecycle status.
        status: TaskStatus,
    },
    /// Update supplied task fields.
    TaskUpdate {
        /// Exact task identifier.
        task_id: String,
        /// Optional replacement title.
        title: Option<String>,
        /// Optional replacement detail.
        description: Option<String>,
        /// Optional replacement status.
        status: Option<TaskStatus>,
    },
    /// List canonical key decisions.
    DecisionList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<DecisionStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one key decision.
    DecisionGet {
        /// Exact decision identifier.
        decision_id: String,
    },
    /// Create an active key decision.
    DecisionCreate {
        /// Owning session.
        session_id: String,
        /// Decision title.
        title: String,
        /// Binding decision content.
        decision: String,
        /// Decision priority.
        priority: DecisionPriority,
        /// Future intent.
        intent: String,
        /// Applicability condition.
        applies_when: String,
        /// Supporting rationale.
        rationale: String,
        /// Bounded source excerpt.
        source_excerpt: String,
    },
    /// Update supplied decision fields.
    DecisionUpdate {
        /// Exact decision identifier.
        decision_id: String,
        /// Optional replacement title.
        title: Option<String>,
        /// Optional replacement decision.
        decision: Option<String>,
        /// Optional replacement priority.
        priority: Option<DecisionPriority>,
        /// Optional replacement intent.
        intent: Option<String>,
        /// Optional replacement applicability.
        applies_when: Option<String>,
        /// Optional replacement rationale.
        rationale: Option<String>,
        /// Optional replacement source excerpt.
        source_excerpt: Option<String>,
    },
    /// Archive one active decision.
    DecisionArchive {
        /// Exact decision identifier.
        decision_id: String,
    },
    /// Atomically supersede one active decision.
    DecisionSupersede {
        /// Exact decision identifier.
        decision_id: String,
        /// Replacement title.
        title: String,
        /// Replacement decision.
        decision: String,
        /// Replacement priority.
        priority: DecisionPriority,
        /// Replacement intent.
        intent: String,
        /// Replacement applicability.
        applies_when: String,
        /// Replacement rationale.
        rationale: String,
        /// Replacement source excerpt.
        source_excerpt: String,
    },
    /// List canonical plans.
    PlanList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<PlanStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one plan.
    PlanGet {
        /// Exact plan identifier.
        plan_id: String,
    },
    /// Create a draft plan.
    PlanCreate {
        /// Owning session.
        session_id: String,
        /// Source prompt.
        prompt: String,
        /// Plan content.
        content: String,
        /// Ordered steps.
        steps: Vec<PlanStep>,
    },
    /// Approve one draft plan.
    PlanApprove {
        /// Exact plan identifier.
        plan_id: String,
    },
    /// Atomically consume and execute one approved plan.
    PlanRun {
        /// Logical model role.
        role: String,
        /// Exact approved plan identifier.
        plan_id: String,
        /// Optional bounded model-turn override.
        max_turns: Option<u16>,
    },
    /// List canonical goals.
    GoalList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<GoalStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one goal.
    GoalGet {
        /// Exact goal identifier.
        goal_id: String,
    },
    /// Execute bounded Goal Mode.
    GoalRun {
        /// Logical model role.
        role: String,
        /// Goal objective.
        objective: String,
        /// Existing session.
        session_id: String,
        /// Iteration ceiling.
        max_iterations: u16,
        /// Optional approved source plan.
        source_plan_id: Option<String>,
    },
    /// Queue a durable child-agent job.
    AgentQueue {
        /// Owning session.
        session_id: String,
        /// Delegated task.
        task: String,
        /// Child model role.
        role: String,
    },
    /// List durable child-agent jobs.
    AgentList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Optional status filter.
        status: Option<SubagentStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one child-agent job.
    AgentGet {
        /// Exact job identifier.
        job_id: String,
    },
    /// Show child-agent scheduler status.
    AgentStatus {
        /// Optional session filter.
        session_id: Option<String>,
    },
    /// Drain queued child-agent jobs.
    AgentDrain,
    /// Cancel one child-agent job.
    AgentCancel {
        /// Exact job identifier.
        job_id: String,
    },
    /// Requeue one stopped child-agent job.
    AgentRequeue {
        /// Exact job identifier.
        job_id: String,
    },
    /// List canonical memories.
    MemoryList {
        /// Optional status filter.
        status: Option<MemoryStatus>,
        /// Maximum records.
        limit: usize,
    },
    /// Read one canonical memory.
    MemoryGet {
        /// Exact memory identifier.
        memory_id: String,
    },
    /// Search canonical re-filtered memories.
    MemorySearch {
        /// Search query.
        query: String,
        /// Optional session scope.
        session_id: Option<String>,
        /// Optional repository scope.
        repository_id: Option<String>,
        /// Maximum records.
        limit: usize,
    },
    /// Create one canonical memory.
    MemoryCreate {
        /// Canonical scope.
        scope: MemoryScope,
        /// Memory kind.
        memory_kind: String,
        /// Confidence in zero through one.
        confidence: f32,
        /// Canonical text.
        text: String,
        /// Supporting rationale.
        rationale: String,
        /// Optional UTC expiry.
        expires_at: Option<String>,
    },
    /// Archive one active memory.
    MemoryArchive {
        /// Exact memory identifier.
        memory_id: String,
    },
    /// Supersede one active memory.
    MemorySupersede {
        /// Exact memory identifier.
        memory_id: String,
        /// Replacement text.
        text: String,
        /// Replacement rationale.
        rationale: String,
    },
    /// Show memory-index readiness.
    MemoryIndexStatus,
    /// Retry queued memory-index work.
    MemoryIndexSync,
    /// Rebuild the disposable memory index.
    MemoryIndexRebuild,
    /// Run bounded durable research.
    ResearchRun {
        /// Research question.
        question: String,
        /// Existing session or none to create one.
        session_id: Option<String>,
        /// Research depth.
        depth: ResearchDepth,
        /// Enabled source lanes.
        source_kinds: Vec<ResearchSourceKind>,
    },
    /// List canonical research runs.
    ResearchList {
        /// Optional session filter.
        session_id: Option<String>,
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one research run.
    ResearchGet {
        /// Exact run identifier.
        run_id: String,
    },
    /// List research evidence sources.
    ResearchSources {
        /// Exact run identifier.
        run_id: String,
    },
    /// List source-backed research claims.
    ResearchClaims {
        /// Exact run identifier.
        run_id: String,
    },
    /// Execute one exact process through the sandbox boundary.
    ProcessRun {
        /// Exact executable path.
        executable: String,
        /// Exact working directory.
        cwd: String,
        /// Literal argument vector.
        args: Vec<String>,
        /// Explicit environment values.
        environment: BTreeMap<String, String>,
    },
    /// Fetch one exact URL through policy and quarantine.
    NetworkGet {
        /// Exact URL.
        url: String,
    },
    /// List configured MCP servers without launching them.
    McpServers,
    /// Discover allowlisted MCP tools.
    McpTools {
        /// Optional exact server filter.
        server: Option<String>,
    },
    /// Invoke one allowlisted MCP tool.
    McpCall {
        /// Exact server name.
        server: String,
        /// Exact tool name.
        tool: String,
        /// Inline JSON or a server-local `@path` reference.
        arguments_source: String,
    },
    /// List selected declarative skill summaries.
    SkillList,
    /// Read one selected declarative skill.
    SkillGet {
        /// Exact skill name.
        name: String,
    },
    /// Report duplicate skill names and winners.
    SkillDuplicates,
    /// Preview deterministic skill composition.
    SkillCompose {
        /// User prompt.
        prompt: String,
        /// Explicit skill names.
        skills: Vec<String>,
    },
    /// Scaffold one installed user skill.
    SkillScaffold {
        /// Skill name.
        name: String,
        /// Skill description.
        description: String,
        /// Data-only instructions.
        instructions: String,
        /// Declared resource directories.
        resource_dirs: Vec<String>,
    },
    /// Inspect installed skill metadata and hashes.
    SkillInspect {
        /// Exact skill name.
        name: String,
    },
    /// Read one authorable skill file.
    SkillFileRead {
        /// Exact skill name.
        name: String,
        /// Relative authorable path.
        path: String,
    },
    /// Write one authorable skill file.
    SkillWrite {
        /// Exact skill name.
        name: String,
        /// Relative authorable path.
        path: String,
        /// Replacement content.
        content: String,
        /// Optional optimistic content hash.
        expected_sha256: Option<String>,
    },
    /// Validate an installed or workspace-local skill.
    SkillValidate {
        /// Installed name or local path.
        target: String,
        /// Whether target is a local path.
        local: bool,
    },
    /// Install one validated workspace-local skill.
    SkillInstall {
        /// Server-local skill directory.
        path: String,
    },
    /// List bounded resources for one explicitly active skill.
    SkillResources {
        /// Exact skill name.
        name: String,
    },
    /// Read one bounded skill resource.
    SkillResourceRead {
        /// Exact skill name.
        name: String,
        /// Relative resource path.
        path: String,
    },
    /// List canonical pack lifecycles.
    PackList {
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one pack lifecycle.
    PackGet {
        /// Exact pack name.
        name: String,
    },
    /// Verify one server-local pack.
    PackVerify {
        /// Server-local pack path.
        path: String,
    },
    /// Install one verified pack.
    PackInstall {
        /// Server-local pack path.
        path: String,
        /// Explicit development override.
        allow_untrusted: bool,
    },
    /// Enable one installed pack.
    PackEnable {
        /// Exact pack name.
        name: String,
    },
    /// Disable one installed pack.
    PackDisable {
        /// Exact pack name.
        name: String,
    },
    /// Uninstall one installed pack.
    PackUninstall {
        /// Exact pack name.
        name: String,
    },
    /// Invoke one active fixed-argument pack tool.
    PackCall {
        /// Exact generated tool name.
        tool: String,
    },
    /// List pack publisher trust bindings.
    PackTrustList {
        /// Maximum records.
        limit: usize,
    },
    /// Add a pack publisher trust binding.
    PackTrustAdd {
        /// Publisher identifier.
        publisher: String,
        /// Base64 Ed25519 public key.
        public_key: String,
    },
    /// Verify a signed offline release bundle.
    BundleVerify {
        /// Server-local bundle path.
        path: String,
    },
    /// Derive the public identity for a referenced bundle signing seed.
    BundleKeyInfo {
        /// Environment reference for the signing seed.
        signing_key_reference: String,
    },
    /// Build and sign a server-local staged bundle payload.
    BundleBuild {
        /// Server-local staged payload path.
        source: String,
        /// Server-local destination path.
        destination: String,
        /// Bundle identity.
        name: String,
        /// Release version.
        version: String,
        /// Trusted publisher identity.
        publisher: String,
        /// Explicit RFC3339 UTC timestamp.
        created_at: String,
        /// Optional source revision.
        source_revision: Option<String>,
        /// Environment reference for the signing seed.
        signing_key_reference: String,
    },
    /// Verify and install the current-target bundle artifact.
    BundleInstall {
        /// Server-local bundle path.
        path: String,
        /// Server-local clean installation prefix.
        prefix: String,
    },
    /// List safe integration summaries.
    IntegrationList {
        /// Maximum records.
        limit: usize,
    },
    /// Reconstruct one integration without credentials.
    IntegrationGet {
        /// Exact integration name.
        name: String,
    },
    /// Connect one first-party integration.
    IntegrationConnect {
        /// Exact integration name.
        name: String,
        /// Optional base URL.
        base_url: Option<String>,
        /// Authentication shape with no values.
        auth: IntegrationAuth,
        /// Optional primary credential reference.
        credential_reference: Option<String>,
        /// Named credential references.
        credential_references: BTreeMap<String, String>,
        /// Declared scopes.
        scopes: Vec<String>,
    },
    /// Import one OpenAPI document.
    IntegrationImportOpenApi {
        /// Exact integration name.
        name: String,
        /// Inline JSON or a server-local `@path` reference.
        document_source: String,
        /// Optional base URL override.
        base_url: Option<String>,
        /// Authentication shape with no values.
        auth: IntegrationAuth,
        /// Optional credential reference.
        credential_reference: Option<String>,
        /// Declared scopes.
        scopes: Vec<String>,
    },
    /// Disconnect one integration.
    IntegrationDisconnect {
        /// Exact integration name.
        name: String,
    },
    /// Invoke one connected integration tool.
    IntegrationCall {
        /// Exact dynamic tool name.
        tool: String,
        /// Inline JSON or a server-local `@path` reference.
        arguments_source: String,
    },
    /// Validate a workflow file through the normal filesystem policy boundary.
    WorkflowValidate {
        /// Server-local workflow path.
        path: String,
    },
    /// Register a workflow file through the normal filesystem policy boundary.
    WorkflowRegister {
        /// Server-local workflow path.
        path: String,
    },
    /// List registered workflow definition audit metadata.
    WorkflowList,
    /// Show one exact registered workflow.
    WorkflowShow {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
    },
    /// Start a workflow run.
    WorkflowStart {
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Inline JSON or a server-local `@path` reference.
        inputs_source: String,
        /// Leave the run queued for worker drain instead of executing immediately.
        queued: bool,
    },
    /// Create one persisted hash-pinned cadence schedule.
    WorkflowScheduleCreate {
        /// Stable schedule identifier.
        schedule_id: String,
        /// Workflow name.
        name: String,
        /// Workflow version.
        version: String,
        /// Inline JSON or a server-local `@path` reference.
        inputs_source: String,
        /// Fixed bounded cadence in seconds.
        cadence_seconds: u64,
        /// Explicit multiple-occurrence behavior.
        misfire_policy: WorkflowScheduleMisfirePolicy,
        /// Initial enabled state.
        enabled: bool,
        /// Optional UTC RFC3339 first occurrence boundary.
        starts_at: Option<String>,
    },
    /// List persisted workflow schedules.
    WorkflowScheduleList {
        /// Maximum schedules.
        limit: usize,
    },
    /// Show one persisted workflow schedule.
    WorkflowScheduleShow {
        /// Exact schedule identifier.
        schedule_id: String,
    },
    /// Explicitly enable or disable one persisted schedule.
    WorkflowScheduleSetEnabled {
        /// Exact schedule identifier.
        schedule_id: String,
        /// Requested enabled state.
        enabled: bool,
    },
    /// Evaluate due workflow schedules against a real or explicit clock.
    WorkflowScheduleTick {
        /// Optional UTC RFC3339 clock used for deterministic operation.
        at: Option<String>,
    },
    /// Reconstruct one workflow run.
    WorkflowStatus {
        /// Exact run identifier.
        run_id: String,
    },
    /// Resume one safe interrupted/waiting workflow run.
    WorkflowResume {
        /// Exact run identifier.
        run_id: String,
    },
    /// Provide structured input to a waiting run.
    WorkflowInput {
        /// Exact run identifier.
        run_id: String,
        /// Inline JSON or a server-local `@path` reference.
        input_source: String,
    },
    /// Cancel a non-terminal workflow run.
    WorkflowCancel {
        /// Exact run identifier.
        run_id: String,
    },
    /// Drain safe queued work and projections once.
    Drain,
    /// Request clean checkpoint and worker shutdown.
    Shutdown,
}

#[derive(Serialize)]
struct UnsignedRequest<'a> {
    version: u16,
    request_id: &'a str,
    timestamp_ms: i128,
    nonce: &'a str,
    connection_nonce: &'a str,
    operation: &'a WorkerOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerRequest {
    version: u16,
    request_id: String,
    timestamp_ms: i128,
    nonce: String,
    connection_nonce: String,
    operation: WorkerOperation,
    authentication_tag: String,
}

/// Worker-side policy mode used by attached and headless clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerApprovalMode {
    /// Deny approval obligations without prompting.
    Deny,
    /// Ask an attached protocol-v4 interactive client.
    Ask,
    /// Preserve model-assisted low-risk auto-approval and ask otherwise.
    RiskAuto,
    /// Mint approval obligations without a prompt.
    FullAccess,
}

/// Kind of one authenticated worker-to-client prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPromptKind {
    /// Policy approval obligation.
    Approval,
    /// Tool-requested operator input.
    UserInput,
}

/// Bounded authenticated prompt transported to an attached interactive client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPrompt {
    /// One-use prompt identity bound to the request connection.
    pub prompt_id: String,
    /// Prompt purpose.
    pub kind: WorkerPromptKind,
    /// Short overlay title.
    pub title: String,
    /// Policy-released question or reason.
    pub question: String,
    /// Optional exact answer choices.
    pub choices: Vec<String>,
    /// Whether an answer outside the exact choices is valid.
    pub allow_free_form: bool,
    /// Bounded released details suitable for a semantic card.
    pub details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerFrameContent {
    Event { event: RunEventEnvelope },
    Prompt { prompt: WorkerPrompt },
    Complete { result: Value },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ClientFrameContent {
    PromptResponse {
        prompt_id: String,
        answer: Option<String>,
    },
    Cancel,
}

#[derive(Serialize)]
struct UnsignedClientFrame<'a> {
    version: u16,
    request_id: &'a str,
    connection_nonce: &'a str,
    sequence: u64,
    timestamp_ms: i128,
    content_base64: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerClientFrame {
    version: u16,
    request_id: String,
    connection_nonce: String,
    sequence: u64,
    timestamp_ms: i128,
    content_base64: String,
    authentication_tag: String,
}

#[derive(Serialize)]
struct UnsignedFrame<'a> {
    version: u16,
    request_id: &'a str,
    sequence: u64,
    timestamp_ms: i128,
    content_base64: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerFrame {
    version: u16,
    request_id: String,
    sequence: u64,
    timestamp_ms: i128,
    content_base64: String,
    authentication_tag: String,
}

#[derive(Default)]
struct ReplayGuard {
    order: VecDeque<String>,
    entries: BTreeSet<String>,
}

impl ReplayGuard {
    fn accept(&mut self, nonce: &str) -> Result<(), WorkerError> {
        if nonce.is_empty() || nonce.len() > 128 || !self.entries.insert(nonce.into()) {
            return Err(WorkerError::Protocol(
                "empty, oversized, or replayed request nonce".into(),
            ));
        }
        self.order.push_back(nonce.into());
        while self.order.len() > REPLAY_WINDOW {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
        Ok(())
    }
}

/// Client-side handler for authenticated worker approval and input prompts.
#[async_trait]
pub trait WorkerPromptHandler: Send + Sync {
    /// Return one bounded answer, or `None` to fail closed.
    async fn prompt(&self, prompt: WorkerPrompt) -> Result<Option<String>, WorkerError>;
}

/// Authenticated one-request-per-connection worker client.
#[derive(Clone)]
pub struct WorkerClient {
    endpoint: String,
    authentication_key: [u8; 32],
}

impl WorkerClient {
    /// Resolve a client only when a platform endpoint may currently exist.
    pub fn discover(config: &RuntimeConfig) -> Result<Option<Self>, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Ok(None);
        }
        Ok(Some(Self {
            endpoint,
            authentication_key: config.worker_ipc_auth_key()?,
        }))
    }

    /// Resolve the platform endpoint and authentication key from runtime configuration.
    pub fn from_config(config: &RuntimeConfig) -> Result<Self, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Err(WorkerError::Unavailable(endpoint));
        }
        Ok(Self {
            endpoint,
            authentication_key: config.worker_ipc_auth_key()?,
        })
    }

    /// Exact configured Unix socket or Windows named-pipe endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return readiness metadata when an authenticated worker is listening.
    pub async fn ping(&self) -> Result<Value, WorkerError> {
        self.call(WorkerOperation::Ping).await
    }

    /// Execute a non-streaming worker operation.
    pub async fn call(&self, operation: WorkerOperation) -> Result<Value, WorkerError> {
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, &self.authentication_key),
        )
        .await
        .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))??;
        let request = signed_request(&self.authentication_key, operation, &connection_nonce)?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
        let content = validate_frame(
            &self.authentication_key,
            &request.request_id,
            &mut sequence,
            &frame,
        )?;
        match content {
            WorkerFrameContent::Event { .. } => Err(WorkerError::Protocol(
                "non-streaming call received a run event".into(),
            )),
            WorkerFrameContent::Prompt { .. } => Err(WorkerError::Protocol(
                "non-interactive call received a prompt and failed closed".into(),
            )),
            WorkerFrameContent::Complete { result } => Ok(result),
            WorkerFrameContent::Error { message } => Err(WorkerError::Remote(message)),
        }
    }

    /// Execute one model run while forwarding authenticated released run events.
    pub async fn run_model(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn colossus_ports::RunEventObserver,
    ) -> Result<AgentRunResult, WorkerError> {
        if !matches!(
            operation,
            WorkerOperation::RunModel { .. } | WorkerOperation::RunPlan { .. }
        ) {
            return Err(WorkerError::Protocol(
                "run_model requires a run_model or run_plan operation".into(),
            ));
        }
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, &self.authentication_key),
        )
        .await
        .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))??;
        let request = signed_request(&self.authentication_key, operation, &connection_nonce)?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        loop {
            let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
            let content = validate_frame(
                &self.authentication_key,
                &request.request_id,
                &mut sequence,
                &frame,
            )?;
            match content {
                WorkerFrameContent::Event { event } => observer
                    .observe(event)
                    .await
                    .map_err(|error| WorkerError::Remote(error.to_string()))?,
                WorkerFrameContent::Complete { result } => {
                    return serde_json::from_value(result).map_err(|error| {
                        WorkerError::Protocol(format!("invalid run result: {error}"))
                    });
                }
                WorkerFrameContent::Prompt { .. } => {
                    return Err(WorkerError::Protocol(
                        "uncontrolled model call received a prompt and failed closed".into(),
                    ));
                }
                WorkerFrameContent::Error { message } => return Err(WorkerError::Remote(message)),
            }
        }
    }

    /// Execute a protocol-v4 run with authenticated prompts and cooperative cancellation.
    pub async fn run_model_controlled(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn RunEventObserver,
        prompts: &dyn WorkerPromptHandler,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, WorkerError> {
        if !matches!(operation, WorkerOperation::RunModelControlled { .. }) {
            return Err(WorkerError::Protocol(
                "run_model_controlled requires run_model_controlled".into(),
            ));
        }
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, &self.authentication_key),
        )
        .await
        .map_err(|_| WorkerError::Unavailable(self.endpoint.clone()))??;
        let request = signed_request(&self.authentication_key, operation, &connection_nonce)?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(32);
        let reader_task = tokio::spawn(async move {
            loop {
                let frame = read_message::<_, WorkerFrame>(&mut reader, MAX_FRAME_BYTES).await;
                let finished = frame.is_err();
                if frame_tx.send(frame).await.is_err() || finished {
                    break;
                }
            }
        });
        let mut server_sequence = 0_u64;
        let mut client_sequence = 0_u64;
        let mut cancellation_sent = false;
        let mut cancellation_poll = tokio::time::interval(Duration::from_millis(50));
        cancellation_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let frame = tokio::select! {
                frame = frame_rx.recv() => frame.ok_or_else(|| {
                    WorkerError::Protocol("worker response stream closed".into())
                })??,
                _ = cancellation_poll.tick(), if !cancellation_sent => {
                    if control.is_cancelled() {
                        client_sequence = client_sequence.saturating_add(1);
                        write_signed_client_frame(
                            &mut writer,
                            &self.authentication_key,
                            &request.request_id,
                            &connection_nonce,
                            client_sequence,
                            ClientFrameContent::Cancel,
                        )
                        .await?;
                        cancellation_sent = true;
                    }
                    continue;
                }
            };
            let content = validate_frame(
                &self.authentication_key,
                &request.request_id,
                &mut server_sequence,
                &frame,
            )?;
            match content {
                WorkerFrameContent::Event { event } => observer
                    .observe(event)
                    .await
                    .map_err(|error| WorkerError::Remote(error.to_string()))?,
                WorkerFrameContent::Prompt { prompt } => {
                    let prompt_id = prompt.prompt_id.clone();
                    let answer = prompts.prompt(prompt).await?;
                    client_sequence = client_sequence.saturating_add(1);
                    write_signed_client_frame(
                        &mut writer,
                        &self.authentication_key,
                        &request.request_id,
                        &connection_nonce,
                        client_sequence,
                        ClientFrameContent::PromptResponse { prompt_id, answer },
                    )
                    .await?;
                }
                WorkerFrameContent::Complete { result } => {
                    reader_task.abort();
                    return serde_json::from_value(result).map_err(|error| {
                        WorkerError::Protocol(format!("invalid controlled run result: {error}"))
                    });
                }
                WorkerFrameContent::Error { message } => {
                    reader_task.abort();
                    return Err(WorkerError::Remote(message));
                }
            }
        }
    }

    async fn connect(&self) -> Result<platform::ClientStream, WorkerError> {
        match tokio::time::timeout(CONNECT_TIMEOUT, platform::connect(&self.endpoint)).await {
            Err(_) => Err(WorkerError::Unavailable(self.endpoint.clone())),
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(error)) if platform::connection_is_busy(&error) => {
                Err(WorkerError::Busy(self.endpoint.clone()))
            }
            Ok(Err(_)) => Err(WorkerError::Unavailable(self.endpoint.clone())),
        }
    }
}

#[derive(Clone)]
struct InteractiveRunBridge {
    prompts: tokio::sync::mpsc::Sender<WorkerPrompt>,
    responses:
        Arc<tokio::sync::Mutex<BTreeMap<String, tokio::sync::oneshot::Sender<Option<String>>>>>,
}

impl InteractiveRunBridge {
    async fn request(&self, prompt: WorkerPrompt) -> Result<Option<String>, String> {
        self.request_with_timeout(prompt, INTERACTIVE_PROMPT_TIMEOUT)
            .await
    }

    async fn request_with_timeout(
        &self,
        prompt: WorkerPrompt,
        timeout: Duration,
    ) -> Result<Option<String>, String> {
        let prompt_id = prompt.prompt_id.clone();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        if self
            .responses
            .lock()
            .await
            .insert(prompt_id.clone(), response_tx)
            .is_some()
        {
            return Err("duplicate interactive prompt id".into());
        }
        if self.prompts.send(prompt).await.is_err() {
            self.responses.lock().await.remove(&prompt_id);
            return Err("interactive worker client disconnected".into());
        }
        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err("interactive worker response channel closed".into()),
            Err(_) => {
                self.responses.lock().await.remove(&prompt_id);
                Err("interactive worker prompt timed out".into())
            }
        }
    }

    async fn respond(&self, prompt_id: &str, answer: Option<String>) -> Result<(), WorkerError> {
        let response = self
            .responses
            .lock()
            .await
            .remove(prompt_id)
            .ok_or_else(|| WorkerError::Protocol("unknown, replayed, or wrong prompt id".into()))?;
        response
            .send(answer)
            .map_err(|_| WorkerError::Protocol("prompt response arrived after closure".into()))
    }

    async fn cancel_all(&self) {
        let pending = std::mem::take(&mut *self.responses.lock().await);
        for (_, response) in pending {
            let _ = response.send(None);
        }
    }
}

tokio::task_local! {
    static ACTIVE_INTERACTIVE_RUN: InteractiveRunBridge;
}

struct WorkerInteractiveApproval {
    mode: WorkerApprovalMode,
}

#[async_trait]
impl ApprovalProvider for WorkerInteractiveApproval {
    fn risk_auto_enabled(&self) -> bool {
        self.mode == WorkerApprovalMode::RiskAuto
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        match self.mode {
            WorkerApprovalMode::Deny => return Ok(None),
            WorkerApprovalMode::FullAccess => {
                return ApprovalProvider::request_approval(
                    &AllowApproval {
                        approved_by: "worker:full-access".into(),
                    },
                    request,
                    request_hash,
                    decision,
                )
                .await;
            }
            WorkerApprovalMode::Ask | WorkerApprovalMode::RiskAuto => {}
        }
        let bridge = ACTIVE_INTERACTIVE_RUN.try_with(Clone::clone).map_err(|_| {
            PolicyError::Unavailable("no interactive worker client attached".into())
        })?;
        let answer = bridge
            .request(WorkerPrompt {
                prompt_id: Uuid::now_v7().to_string(),
                kind: WorkerPromptKind::Approval,
                title: "Approval required".into(),
                question: decision.reason.clone(),
                choices: vec!["Allow once".into(), "Deny".into()],
                allow_free_form: false,
                details: json!({
                    "action": request.action,
                    "resource": request.resource,
                    "content": request.content,
                    "decision_id": decision.decision_id,
                }),
            })
            .await
            .map_err(PolicyError::Unavailable)?;
        if answer.as_deref() != Some("Allow once") {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "worker:interactive".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

struct WorkerInteractiveUserPrompt;

#[async_trait]
impl UserPromptProvider for WorkerInteractiveUserPrompt {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let bridge = ACTIVE_INTERACTIVE_RUN
            .try_with(Clone::clone)
            .map_err(|_| ToolError::Failed("no interactive worker client attached".into()))?;
        let answer = bridge
            .request(WorkerPrompt {
                prompt_id: Uuid::now_v7().to_string(),
                kind: WorkerPromptKind::UserInput,
                title: "Input needed".into(),
                question: request.question.clone(),
                choices: request.choices.clone(),
                allow_free_form: request.allow_free_form,
                details: Value::Null,
            })
            .await
            .map_err(ToolError::Failed)?
            .ok_or_else(|| ToolError::Failed("user cancelled the question".into()))?;
        let selected_index = request.choices.iter().position(|choice| choice == &answer);
        if selected_index.is_none() && !request.allow_free_form {
            return Err(ToolError::Failed(
                "user response did not match an allowed choice".into(),
            ));
        }
        Ok(UserPromptResponse {
            answer,
            selected_index,
        })
    }
}

/// Long-running single-writer runtime owner and authenticated IPC server.
pub struct WorkerServer {
    endpoint: String,
    authentication_key: [u8; 32],
    runtime: Arc<Runtime>,
    replay: Arc<Mutex<ReplayGuard>>,
    maintenance: Arc<tokio::sync::Mutex<()>>,
}

impl WorkerServer {
    /// Open the runtime (and therefore acquire the writer lease) before binding IPC.
    pub fn open(
        config: &RuntimeConfig,
        approvals: Arc<dyn colossus_ports::ApprovalProvider>,
    ) -> Result<Self, WorkerError> {
        Ok(Self {
            endpoint: config.worker_ipc_endpoint()?,
            authentication_key: config.worker_ipc_auth_key()?,
            runtime: Arc::new(Runtime::open_with_approval(config, approvals)?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Open a worker whose protocol-v4 attached clients own prompts and cancellation.
    pub fn open_with_mode(
        config: &RuntimeConfig,
        approval_mode: WorkerApprovalMode,
    ) -> Result<Self, WorkerError> {
        let approvals: Arc<dyn ApprovalProvider> = Arc::new(WorkerInteractiveApproval {
            mode: approval_mode,
        });
        let user_prompts: Arc<dyn UserPromptProvider> = Arc::new(WorkerInteractiveUserPrompt);
        Ok(Self {
            endpoint: config.worker_ipc_endpoint()?,
            authentication_key: config.worker_ipc_auth_key()?,
            runtime: Arc::new(Runtime::open_with_interfaces(
                config,
                approvals,
                Some(user_prompts),
            )?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Exact bound endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Serve until Ctrl-C or an authenticated shutdown request, then checkpoint cleanly.
    pub async fn serve(self) -> Result<(), WorkerError> {
        let mut listener = platform::Listener::bind(&self.endpoint).await?;
        let runtime = Arc::clone(&self.runtime);
        let replay = Arc::clone(&self.replay);
        let maintenance = Arc::clone(&self.maintenance);
        let key = self.authentication_key;
        let mut drain_interval = tokio::time::interval(Duration::from_secs(1));
        drain_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        drain_interval.tick().await;
        let draining = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut tasks = tokio::task::JoinSet::new();
        let mut stop = false;
        while !stop {
            tokio::select! {
                accepted = listener.accept() => {
                    let stream = accepted?;
                    let runtime = Arc::clone(&runtime);
                    let replay = Arc::clone(&replay);
                    let maintenance = Arc::clone(&maintenance);
                    let shutdown = shutdown_tx.clone();
                    tasks.spawn(async move {
                        if handle_connection(
                            stream,
                            &key,
                            runtime.as_ref(),
                            replay.as_ref(),
                            maintenance.as_ref(),
                        )
                            .await
                            .is_ok_and(|stopping| stopping)
                        {
                            let _ = shutdown.send(()).await;
                        }
                    });
                }
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    stop = true;
                }
                requested = shutdown_rx.recv() => {
                    stop = requested.is_some();
                }
                _ = drain_interval.tick() => {
                    if draining.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        let runtime = Arc::clone(&runtime);
                        let draining = Arc::clone(&draining);
                        let maintenance = Arc::clone(&maintenance);
                        tasks.spawn(async move {
                            let _ = drain_once(runtime.as_ref(), maintenance.as_ref()).await;
                            draining.store(false, Ordering::Release);
                        });
                    }
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(result) = completed {
                        result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
                    }
                }
            }
        }
        drop(shutdown_tx);
        while let Some(result) = tasks.join_next().await {
            result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
        }
        runtime.checkpoint()?;
        listener.cleanup();
        Ok(())
    }
}

async fn handle_connection<S>(
    mut stream: S,
    key: &[u8; 32],
    runtime: &Runtime,
    replay: &Mutex<ReplayGuard>,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<bool, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let connection_nonce =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, server_handshake(&mut stream, key))
            .await
            .map_err(|_| WorkerError::Protocol("worker client handshake timed out".into()))
            .and_then(std::convert::identity)
        {
            Ok(nonce) => nonce,
            Err(error) => {
                runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
                return Err(error);
            }
        };
    let request: WorkerRequest = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        read_message(&mut stream, MAX_REQUEST_BYTES),
    )
    .await
    .map_err(|_| WorkerError::Protocol("worker request framing timed out".into()))
    .and_then(std::convert::identity)
    {
        Ok(request) => request,
        Err(error) => {
            runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
            return Err(error);
        }
    };
    if let Err(error) = validate_request(key, &request, replay, &connection_nonce) {
        runtime.record_worker_ipc_audit(
            false,
            Some(&request.request_id),
            Some(operation_name(&request.operation)),
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    runtime.record_worker_ipc_audit(
        true,
        Some(&request.request_id),
        Some(operation_name(&request.operation)),
        None,
    )?;
    let request_id = request.request_id.clone();
    match request.operation {
        WorkerOperation::RunModelControlled {
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(256);
            let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(16);
            let responses = Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));
            let bridge = InteractiveRunBridge {
                prompts: prompt_tx,
                responses,
            };
            let control = RunControl::default();
            let mut observer = ChannelWorkerObserver { sender: event_tx };
            let run = ACTIVE_INTERACTIVE_RUN.scope(
                bridge.clone(),
                runtime.run_model_with_skills_stream_controlled(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    Some(&session_id),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                    &control,
                ),
            );
            tokio::pin!(run);
            let (mut reader, mut writer) = tokio::io::split(stream);
            let (client_tx, mut client_rx) = tokio::sync::mpsc::channel(16);
            let reader_key = *key;
            let reader_request_id = request_id.clone();
            let reader_connection_nonce = connection_nonce.clone();
            let reader_task = tokio::spawn(async move {
                let mut sequence = 0_u64;
                loop {
                    let frame =
                        read_message::<_, WorkerClientFrame>(&mut reader, MAX_REQUEST_BYTES)
                            .await
                            .and_then(|frame| {
                                validate_client_frame(
                                    &reader_key,
                                    &reader_request_id,
                                    &reader_connection_nonce,
                                    &mut sequence,
                                    &frame,
                                )
                            });
                    let finished = frame.is_err();
                    if client_tx.send(frame).await.is_err() || finished {
                        break;
                    }
                }
            });
            let mut sequence = 0_u64;
            loop {
                tokio::select! {
                    result = &mut run => {
                        sequence = sequence.saturating_add(1);
                        let content = match result {
                            Ok(outcome) => WorkerFrameContent::Complete {
                                result: serde_json::to_value(outcome)?,
                            },
                            Err(error) => WorkerFrameContent::Error {
                                message: bounded_error(&error.to_string()),
                            },
                        };
                        write_signed_frame(&mut writer, key, &request_id, sequence, content).await?;
                        reader_task.abort();
                        bridge.cancel_all().await;
                        return Ok(false);
                    }
                    event = event_rx.recv() => {
                        let Some(event) = event else { continue; };
                        sequence = sequence.saturating_add(1);
                        write_signed_frame(
                            &mut writer,
                            key,
                            &request_id,
                            sequence,
                            WorkerFrameContent::Event { event },
                        ).await?;
                    }
                    prompt = prompt_rx.recv() => {
                        let Some(prompt) = prompt else { continue; };
                        sequence = sequence.saturating_add(1);
                        write_signed_frame(
                            &mut writer,
                            key,
                            &request_id,
                            sequence,
                            WorkerFrameContent::Prompt { prompt },
                        ).await?;
                    }
                    client = client_rx.recv() => {
                        match client {
                            Some(Ok(ClientFrameContent::PromptResponse { prompt_id, answer })) => {
                                bridge.respond(&prompt_id, answer).await?;
                            }
                            Some(Ok(ClientFrameContent::Cancel)) => control.cancel(),
                            Some(Err(error)) => {
                                control.cancel();
                                bridge.cancel_all().await;
                                reader_task.abort();
                                return Err(error);
                            }
                            None => {
                                control.cancel();
                                bridge.cancel_all().await;
                                reader_task.abort();
                                return Err(WorkerError::Protocol(
                                    "interactive worker client disconnected".into(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        WorkerOperation::RunModel {
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let mut observer = IpcRunObserver {
                stream: &mut stream,
                key,
                request_id: &request_id,
                sequence: 0,
            };
            let result = runtime
                .run_model_with_skills_stream(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            match result {
                Ok(result) => observer.complete(serde_json::to_value(result)?).await?,
                Err(error) => observer.error(error.to_string()).await?,
            }
            Ok(false)
        }
        WorkerOperation::RunPlan {
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let mut observer = IpcRunObserver {
                stream: &mut stream,
                key,
                request_id: &request_id,
                sequence: 0,
            };
            let result = runtime
                .run_plan_with_skills_stream(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            match result {
                Ok(result) => observer.complete(serde_json::to_value(result)?).await?,
                Err(error) => observer.error(error.to_string()).await?,
            }
            Ok(false)
        }
        operation => {
            let shutdown = matches!(operation, WorkerOperation::Shutdown);
            let result = dispatch(runtime, operation, maintenance).await;
            let succeeded = result.is_ok();
            let content = match result {
                Ok(result) => WorkerFrameContent::Complete { result },
                Err(error) => WorkerFrameContent::Error {
                    message: bounded_error(&error.to_string()),
                },
            };
            write_signed_frame(&mut stream, key, &request_id, 1, content).await?;
            Ok(shutdown && succeeded)
        }
    }
}

fn operation_name(operation: &WorkerOperation) -> &'static str {
    match operation {
        WorkerOperation::Ping => "ping",
        WorkerOperation::AuditVerify => "audit_verify",
        WorkerOperation::AuditRead { .. } => "audit_read",
        WorkerOperation::AuditExportStatus => "audit_export_status",
        WorkerOperation::AuditExportDrain => "audit_export_drain",
        WorkerOperation::AuditExportReset => "audit_export_reset",
        WorkerOperation::PolicyDoctor => "policy_doctor",
        WorkerOperation::ProjectionStatus => "projection_status",
        WorkerOperation::ProjectionDrain => "projection_drain",
        WorkerOperation::ProjectionRebuild { .. } => "projection_rebuild",
        WorkerOperation::StateDoctor => "state_doctor",
        WorkerOperation::SandboxDoctor => "sandbox_doctor",
        WorkerOperation::ProviderProfiles => "provider_profiles",
        WorkerOperation::ProviderDoctor { .. } => "provider_doctor",
        WorkerOperation::ProviderModels { .. } => "provider_models",
        WorkerOperation::ProviderRoutes => "provider_routes",
        WorkerOperation::ProviderRoute { .. } => "provider_route",
        WorkerOperation::ToolsList => "tools_list",
        WorkerOperation::RunModel { .. } => "run_model",
        WorkerOperation::RunModelControlled { .. } => "run_model_controlled",
        WorkerOperation::RunPlan { .. } => "run_plan",
        WorkerOperation::Echo { .. } => "echo",
        WorkerOperation::SessionCreate { .. } => "session_create",
        WorkerOperation::SessionGet { .. } => "session_get",
        WorkerOperation::SessionList { .. } => "session_list",
        WorkerOperation::SessionMessages { .. } => "session_messages",
        WorkerOperation::SessionMessagesPage { .. } => "session_messages_page",
        WorkerOperation::SessionLatest => "session_latest",
        WorkerOperation::WorkState { .. } => "work_state",
        WorkerOperation::PresentationGet => "presentation_get",
        WorkerOperation::PresentationHistory { .. } => "presentation_history",
        WorkerOperation::PresentationSave { .. } => "presentation_save",
        WorkerOperation::PresentationHistoryAppend { .. } => "presentation_history_append",
        WorkerOperation::ContextStatus { .. } => "context_status",
        WorkerOperation::ContextList { .. } => "context_list",
        WorkerOperation::ContextCompact { .. } => "context_compact",
        WorkerOperation::ContextRestore { .. } => "context_restore",
        WorkerOperation::TelemetryRuns { .. } => "telemetry_runs",
        WorkerOperation::TelemetryShow { .. } => "telemetry_show",
        WorkerOperation::TelemetryMetrics { .. } => "telemetry_metrics",
        WorkerOperation::TaskList { .. } => "task_list",
        WorkerOperation::TaskGet { .. } => "task_get",
        WorkerOperation::TaskCreate { .. } => "task_create",
        WorkerOperation::TaskUpdate { .. } => "task_update",
        WorkerOperation::DecisionList { .. } => "decision_list",
        WorkerOperation::DecisionGet { .. } => "decision_get",
        WorkerOperation::DecisionCreate { .. } => "decision_create",
        WorkerOperation::DecisionUpdate { .. } => "decision_update",
        WorkerOperation::DecisionArchive { .. } => "decision_archive",
        WorkerOperation::DecisionSupersede { .. } => "decision_supersede",
        WorkerOperation::PlanList { .. } => "plan_list",
        WorkerOperation::PlanGet { .. } => "plan_get",
        WorkerOperation::PlanCreate { .. } => "plan_create",
        WorkerOperation::PlanApprove { .. } => "plan_approve",
        WorkerOperation::PlanRun { .. } => "plan_run",
        WorkerOperation::GoalList { .. } => "goal_list",
        WorkerOperation::GoalGet { .. } => "goal_get",
        WorkerOperation::GoalRun { .. } => "goal_run",
        WorkerOperation::AgentQueue { .. } => "agent_queue",
        WorkerOperation::AgentList { .. } => "agent_list",
        WorkerOperation::AgentGet { .. } => "agent_get",
        WorkerOperation::AgentStatus { .. } => "agent_status",
        WorkerOperation::AgentDrain => "agent_drain",
        WorkerOperation::AgentCancel { .. } => "agent_cancel",
        WorkerOperation::AgentRequeue { .. } => "agent_requeue",
        WorkerOperation::MemoryList { .. } => "memory_list",
        WorkerOperation::MemoryGet { .. } => "memory_get",
        WorkerOperation::MemorySearch { .. } => "memory_search",
        WorkerOperation::MemoryCreate { .. } => "memory_create",
        WorkerOperation::MemoryArchive { .. } => "memory_archive",
        WorkerOperation::MemorySupersede { .. } => "memory_supersede",
        WorkerOperation::MemoryIndexStatus => "memory_index_status",
        WorkerOperation::MemoryIndexSync => "memory_index_sync",
        WorkerOperation::MemoryIndexRebuild => "memory_index_rebuild",
        WorkerOperation::ResearchRun { .. } => "research_run",
        WorkerOperation::ResearchList { .. } => "research_list",
        WorkerOperation::ResearchGet { .. } => "research_get",
        WorkerOperation::ResearchSources { .. } => "research_sources",
        WorkerOperation::ResearchClaims { .. } => "research_claims",
        WorkerOperation::ProcessRun { .. } => "process_run",
        WorkerOperation::NetworkGet { .. } => "network_get",
        WorkerOperation::McpServers => "mcp_servers",
        WorkerOperation::McpTools { .. } => "mcp_tools",
        WorkerOperation::McpCall { .. } => "mcp_call",
        WorkerOperation::SkillList => "skill_list",
        WorkerOperation::SkillGet { .. } => "skill_get",
        WorkerOperation::SkillDuplicates => "skill_duplicates",
        WorkerOperation::SkillCompose { .. } => "skill_compose",
        WorkerOperation::SkillScaffold { .. } => "skill_scaffold",
        WorkerOperation::SkillInspect { .. } => "skill_inspect",
        WorkerOperation::SkillFileRead { .. } => "skill_file_read",
        WorkerOperation::SkillWrite { .. } => "skill_write",
        WorkerOperation::SkillValidate { .. } => "skill_validate",
        WorkerOperation::SkillInstall { .. } => "skill_install",
        WorkerOperation::SkillResources { .. } => "skill_resources",
        WorkerOperation::SkillResourceRead { .. } => "skill_resource_read",
        WorkerOperation::PackList { .. } => "pack_list",
        WorkerOperation::PackGet { .. } => "pack_get",
        WorkerOperation::PackVerify { .. } => "pack_verify",
        WorkerOperation::PackInstall { .. } => "pack_install",
        WorkerOperation::PackEnable { .. } => "pack_enable",
        WorkerOperation::PackDisable { .. } => "pack_disable",
        WorkerOperation::PackUninstall { .. } => "pack_uninstall",
        WorkerOperation::PackCall { .. } => "pack_call",
        WorkerOperation::PackTrustList { .. } => "pack_trust_list",
        WorkerOperation::PackTrustAdd { .. } => "pack_trust_add",
        WorkerOperation::BundleVerify { .. } => "bundle_verify",
        WorkerOperation::BundleKeyInfo { .. } => "bundle_key_info",
        WorkerOperation::BundleBuild { .. } => "bundle_build",
        WorkerOperation::BundleInstall { .. } => "bundle_install",
        WorkerOperation::IntegrationList { .. } => "integration_list",
        WorkerOperation::IntegrationGet { .. } => "integration_get",
        WorkerOperation::IntegrationConnect { .. } => "integration_connect",
        WorkerOperation::IntegrationImportOpenApi { .. } => "integration_import_open_api",
        WorkerOperation::IntegrationDisconnect { .. } => "integration_disconnect",
        WorkerOperation::IntegrationCall { .. } => "integration_call",
        WorkerOperation::WorkflowValidate { .. } => "workflow_validate",
        WorkerOperation::WorkflowRegister { .. } => "workflow_register",
        WorkerOperation::WorkflowList => "workflow_list",
        WorkerOperation::WorkflowShow { .. } => "workflow_show",
        WorkerOperation::WorkflowStart { .. } => "workflow_start",
        WorkerOperation::WorkflowScheduleCreate { .. } => "workflow_schedule_create",
        WorkerOperation::WorkflowScheduleList { .. } => "workflow_schedule_list",
        WorkerOperation::WorkflowScheduleShow { .. } => "workflow_schedule_show",
        WorkerOperation::WorkflowScheduleSetEnabled { .. } => "workflow_schedule_set_enabled",
        WorkerOperation::WorkflowScheduleTick { .. } => "workflow_schedule_tick",
        WorkerOperation::WorkflowStatus { .. } => "workflow_status",
        WorkerOperation::WorkflowResume { .. } => "workflow_resume",
        WorkerOperation::WorkflowInput { .. } => "workflow_input",
        WorkerOperation::WorkflowCancel { .. } => "workflow_cancel",
        WorkerOperation::Drain => "drain",
        WorkerOperation::Shutdown => "shutdown",
    }
}

async fn client_handshake<S>(stream: &mut S, key: &[u8; 32]) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut challenge = [0_u8; 32];
    getrandom::fill(&mut challenge).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let challenge = hex::encode(challenge);
    write_message(
        stream,
        &ClientHello {
            version: PROTOCOL_VERSION,
            challenge: challenge.clone(),
        },
        1024,
    )
    .await?;
    let hello: ServerHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge != challenge
        || hello.server_nonce.len() != 64
        || hex::decode(&hello.server_nonce).map_or(true, |bytes| bytes.len() != 32)
        || (now_ms() - hello.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "worker server protocol is incompatible or its handshake is invalid; restart the worker with this Colossus version".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedServerHello {
            version: hello.version,
            challenge: &hello.challenge,
            server_nonce: &hello.server_nonce,
            timestamp_ms: hello.timestamp_ms,
        },
        &hello.authentication_tag,
        "worker server handshake",
    )?;
    Ok(hello.server_nonce)
}

async fn server_handshake<S>(stream: &mut S, key: &[u8; 32]) -> Result<String, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello: ClientHello = read_message(stream, 1024).await?;
    if hello.version != PROTOCOL_VERSION
        || hello.challenge.len() != 64
        || hex::decode(&hello.challenge).map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(WorkerError::Protocol(
            "worker client protocol is incompatible or its handshake is invalid; restart the worker and client with the same Colossus version".into(),
        ));
    }
    let mut server_nonce = [0_u8; 32];
    getrandom::fill(&mut server_nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let server_nonce = hex::encode(server_nonce);
    let timestamp_ms = now_ms();
    let authentication_tag = request_tag(
        key,
        &UnsignedServerHello {
            version: PROTOCOL_VERSION,
            challenge: &hello.challenge,
            server_nonce: &server_nonce,
            timestamp_ms,
        },
    )?;
    write_message(
        stream,
        &ServerHello {
            version: PROTOCOL_VERSION,
            challenge: hello.challenge,
            server_nonce: server_nonce.clone(),
            timestamp_ms,
            authentication_tag,
        },
        1024,
    )
    .await?;
    Ok(server_nonce)
}

async fn dispatch(
    runtime: &Runtime,
    operation: WorkerOperation,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    match operation {
        WorkerOperation::Ping => Ok(json!({
            "ready": true,
            "protocol_version": PROTOCOL_VERSION,
            "pid": std::process::id(),
        })),
        WorkerOperation::AuditVerify => Ok(serde_json::to_value(runtime.journal().verify()?)?),
        WorkerOperation::AuditRead { from, limit } => Ok(serde_json::to_value(
            runtime
                .journal()
                .read_global(from.max(1), limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::AuditExportStatus => {
            Ok(serde_json::to_value(runtime.audit_export_status()?)?)
        }
        WorkerOperation::AuditExportDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_audit_exports().await?)?)
        }
        WorkerOperation::AuditExportReset => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.reset_audit_exports()?)?)
        }
        WorkerOperation::PolicyDoctor => Ok(runtime.policy_doctor().await?),
        WorkerOperation::ProjectionStatus => {
            Ok(serde_json::to_value(runtime.projection_status()?)?)
        }
        WorkerOperation::ProjectionDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_projections()?)?)
        }
        WorkerOperation::ProjectionRebuild { name } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime.rebuild_projection(name.as_deref())?,
            )?)
        }
        WorkerOperation::StateDoctor => Ok(runtime.state_doctor()?),
        WorkerOperation::SandboxDoctor => Ok(serde_json::to_value(runtime.sandbox_doctor())?),
        WorkerOperation::ProviderProfiles => Ok(serde_json::to_value(runtime.provider_profiles())?),
        WorkerOperation::ProviderDoctor { profile } => Ok(serde_json::to_value(
            runtime.provider_doctor(profile.as_deref()).await?,
        )?),
        WorkerOperation::ProviderModels { profile } => Ok(serde_json::to_value(
            runtime.provider_models(profile.as_deref()).await?,
        )?),
        WorkerOperation::ProviderRoutes => Ok(runtime.provider_routes()),
        WorkerOperation::ProviderRoute { role } => {
            Ok(serde_json::to_value(runtime.provider_route(&role)?)?)
        }
        WorkerOperation::ToolsList => Ok(serde_json::to_value(runtime.tool_specs())?),
        WorkerOperation::Echo { message } => {
            let result = runtime.echo(&message).await?;
            Ok(json!({
                "media_type": result.media_type,
                "bytes_base64": BASE64.encode(result.bytes),
            }))
        }
        WorkerOperation::SessionCreate { title } => Ok(serde_json::to_value(
            runtime.create_session(title.as_deref())?,
        )?),
        WorkerOperation::SessionGet { session_id } => {
            Ok(serde_json::to_value(runtime.get_session(&session_id)?)?)
        }
        WorkerOperation::SessionList { limit } => Ok(serde_json::to_value(
            runtime.list_sessions(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::SessionMessages { session_id } => Ok(serde_json::to_value(
            runtime.session_messages(&session_id)?,
        )?),
        WorkerOperation::SessionMessagesPage {
            session_id,
            before_sequence,
            limit,
        } => Ok(serde_json::to_value(runtime.session_messages_page(
            &session_id,
            before_sequence,
            limit.clamp(1, 100),
        )?)?),
        WorkerOperation::SessionLatest => Ok(serde_json::to_value(runtime.latest_session()?)?),
        WorkerOperation::WorkState { session_id } => {
            Ok(serde_json::to_value(runtime.work_state(&session_id)?)?)
        }
        WorkerOperation::PresentationGet => {
            Ok(serde_json::to_value(runtime.presentation_preferences()?)?)
        }
        WorkerOperation::PresentationHistory { limit } => Ok(serde_json::to_value(
            runtime.terminal_history(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::PresentationSave { preferences } => Ok(serde_json::to_value(
            runtime.save_presentation_preferences(preferences).await?,
        )?),
        WorkerOperation::PresentationHistoryAppend { entry } => Ok(serde_json::to_value(
            runtime.append_terminal_history(&entry).await?,
        )?),
        WorkerOperation::ContextStatus { session_id } => Ok(serde_json::to_value(
            runtime.context_status(&session_id).await?,
        )?),
        WorkerOperation::ContextList { session_id } => Ok(serde_json::to_value(
            runtime.context_snapshots(&session_id).await?,
        )?),
        WorkerOperation::ContextCompact { session_id } => Ok(serde_json::to_value(
            runtime.compact_context(&session_id).await?,
        )?),
        WorkerOperation::ContextRestore {
            session_id,
            snapshot_id,
        } => Ok(serde_json::to_value(
            runtime.restore_context(&session_id, &snapshot_id).await?,
        )?),
        WorkerOperation::TelemetryRuns { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_runs(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::TelemetryShow {
            id_or_prefix,
            limit,
        } => Ok(serde_json::to_value(
            runtime.telemetry_run(&id_or_prefix, limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::TelemetryMetrics { session_id, limit } => Ok(serde_json::to_value(
            runtime.telemetry_metrics(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::TaskList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_tasks(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::TaskGet { task_id } => {
            Ok(serde_json::to_value(runtime.get_task(&task_id)?)?)
        }
        WorkerOperation::TaskCreate {
            session_id,
            title,
            description,
            status,
        } => Ok(serde_json::to_value(
            runtime
                .create_task(&session_id, &title, &description, status)
                .await?,
        )?),
        WorkerOperation::TaskUpdate {
            task_id,
            title,
            description,
            status,
        } => Ok(serde_json::to_value(
            runtime
                .update_task(&task_id, title.as_deref(), description.as_deref(), status)
                .await?,
        )?),
        WorkerOperation::DecisionList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_decisions(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::DecisionGet { decision_id } => {
            Ok(serde_json::to_value(runtime.get_decision(&decision_id)?)?)
        }
        WorkerOperation::DecisionCreate {
            session_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .create_decision(
                    &session_id,
                    &title,
                    &decision,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                )
                .await?,
        )?),
        WorkerOperation::DecisionUpdate {
            decision_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .update_decision(
                    &decision_id,
                    title.as_deref(),
                    decision.as_deref(),
                    priority,
                    intent.as_deref(),
                    applies_when.as_deref(),
                    rationale.as_deref(),
                    source_excerpt.as_deref(),
                )
                .await?,
        )?),
        WorkerOperation::DecisionArchive { decision_id } => Ok(serde_json::to_value(
            runtime.archive_decision(&decision_id).await?,
        )?),
        WorkerOperation::DecisionSupersede {
            decision_id,
            title,
            decision,
            priority,
            intent,
            applies_when,
            rationale,
            source_excerpt,
        } => Ok(serde_json::to_value(
            runtime
                .supersede_decision(
                    &decision_id,
                    &title,
                    &decision,
                    priority,
                    &intent,
                    &applies_when,
                    &rationale,
                    &source_excerpt,
                )
                .await?,
        )?),
        WorkerOperation::PlanList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_plans(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::PlanGet { plan_id } => {
            Ok(serde_json::to_value(runtime.get_plan(&plan_id)?)?)
        }
        WorkerOperation::PlanCreate {
            session_id,
            prompt,
            content,
            steps,
        } => Ok(serde_json::to_value(
            runtime
                .create_plan(&session_id, &prompt, &content, steps)
                .await?,
        )?),
        WorkerOperation::PlanApprove { plan_id } => {
            Ok(serde_json::to_value(runtime.approve_plan(&plan_id).await?)?)
        }
        WorkerOperation::PlanRun {
            role,
            plan_id,
            max_turns,
        } => Ok(serde_json::to_value(
            runtime
                .run_approved_plan(&role, &plan_id, max_turns)
                .await?,
        )?),
        WorkerOperation::GoalList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_goals(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::GoalGet { goal_id } => {
            Ok(serde_json::to_value(runtime.get_goal(&goal_id)?)?)
        }
        WorkerOperation::GoalRun {
            role,
            objective,
            session_id,
            max_iterations,
            source_plan_id,
        } => Ok(serde_json::to_value(
            runtime
                .run_goal(
                    &role,
                    &objective,
                    &session_id,
                    max_iterations,
                    source_plan_id.as_deref(),
                )
                .await?,
        )?),
        WorkerOperation::AgentQueue {
            session_id,
            task,
            role,
        } => Ok(serde_json::to_value(
            runtime.queue_subagent(&session_id, &task, &role).await?,
        )?),
        WorkerOperation::AgentList {
            session_id,
            status,
            limit,
        } => Ok(serde_json::to_value(runtime.list_subagents(
            session_id.as_deref(),
            status,
            limit.clamp(1, 1_000),
        )?)?),
        WorkerOperation::AgentGet { job_id } => {
            Ok(serde_json::to_value(runtime.get_subagent(&job_id)?)?)
        }
        WorkerOperation::AgentStatus { session_id } => Ok(serde_json::to_value(
            runtime.subagent_queue_status(session_id.as_deref())?,
        )?),
        WorkerOperation::AgentDrain => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.drain_subagents().await?)?)
        }
        WorkerOperation::AgentCancel { job_id } => Ok(serde_json::to_value(
            runtime.cancel_subagent(&job_id).await?,
        )?),
        WorkerOperation::AgentRequeue { job_id } => Ok(serde_json::to_value(
            runtime.requeue_subagent(&job_id).await?,
        )?),
        WorkerOperation::MemoryList { status, limit } => Ok(serde_json::to_value(
            runtime.list_memories(status, limit.clamp(1, 1_000)).await?,
        )?),
        WorkerOperation::MemoryGet { memory_id } => {
            Ok(serde_json::to_value(runtime.get_memory(&memory_id).await?)?)
        }
        WorkerOperation::MemorySearch {
            query,
            session_id,
            repository_id,
            limit,
        } => Ok(serde_json::to_value(
            runtime
                .search_memories(
                    &query,
                    session_id.as_deref(),
                    repository_id.as_deref(),
                    limit.clamp(1, 100),
                )
                .await?,
        )?),
        WorkerOperation::MemoryCreate {
            scope,
            memory_kind,
            confidence,
            text,
            rationale,
            expires_at,
        } => Ok(serde_json::to_value(
            runtime
                .create_memory(
                    scope,
                    &memory_kind,
                    confidence,
                    &text,
                    &rationale,
                    expires_at,
                )
                .await?,
        )?),
        WorkerOperation::MemoryArchive { memory_id } => Ok(serde_json::to_value(
            runtime.archive_memory(&memory_id).await?,
        )?),
        WorkerOperation::MemorySupersede {
            memory_id,
            text,
            rationale,
        } => Ok(serde_json::to_value(
            runtime
                .supersede_memory(&memory_id, &text, &rationale)
                .await?,
        )?),
        WorkerOperation::MemoryIndexStatus => Ok(runtime.memory_index_status().await?),
        WorkerOperation::MemoryIndexSync => {
            let _guard = maintenance.lock().await;
            Ok(runtime.sync_memory_index().await?)
        }
        WorkerOperation::MemoryIndexRebuild => {
            let _guard = maintenance.lock().await;
            Ok(runtime.rebuild_memory_index().await?)
        }
        WorkerOperation::ResearchRun {
            question,
            session_id,
            depth,
            source_kinds,
        } => {
            let session_id = match session_id {
                Some(session_id) => {
                    runtime
                        .get_session(&session_id)?
                        .ok_or_else(|| {
                            WorkerError::Remote(format!("session {session_id} not found"))
                        })?
                        .id
                }
                None => runtime.create_session(Some("Research"))?.id,
            };
            Ok(serde_json::to_value(
                runtime
                    .run_research(&session_id, &question, depth, source_kinds)
                    .await?,
            )?)
        }
        WorkerOperation::ResearchList { session_id, limit } => Ok(serde_json::to_value(
            runtime.list_research_runs(session_id.as_deref(), limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::ResearchGet { run_id } => {
            Ok(serde_json::to_value(runtime.get_research_run(&run_id)?)?)
        }
        WorkerOperation::ResearchSources { run_id } => {
            Ok(serde_json::to_value(runtime.research_sources(&run_id)?)?)
        }
        WorkerOperation::ResearchClaims { run_id } => {
            Ok(serde_json::to_value(runtime.research_claims(&run_id)?)?)
        }
        WorkerOperation::ProcessRun {
            executable,
            cwd,
            args,
            environment,
        } => Ok(runtime
            .run_process(executable, cwd, args, environment)
            .await?),
        WorkerOperation::NetworkGet { url } => {
            let released = runtime.http_get(&url).await?;
            Ok(json!({
                "media_type": released.media_type,
                "bytes_base64": BASE64.encode(released.bytes),
            }))
        }
        WorkerOperation::McpServers => Ok(serde_json::to_value(runtime.mcp_servers())?),
        WorkerOperation::McpTools { server } => Ok(serde_json::to_value(
            runtime.mcp_tools(server.as_deref()).await?,
        )?),
        WorkerOperation::McpCall {
            server,
            tool,
            arguments_source,
        } => {
            let arguments = parse_json_source(runtime, &arguments_source).await?;
            Ok(serde_json::to_value(
                runtime.mcp_call(&server, &tool, arguments).await?,
            )?)
        }
        WorkerOperation::SkillList => {
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
            Ok(serde_json::to_value(skills)?)
        }
        WorkerOperation::SkillGet { name } => Ok(serde_json::to_value(runtime.get_skill(&name)?)?),
        WorkerOperation::SkillDuplicates => Ok(serde_json::to_value(runtime.skill_duplicates()?)?),
        WorkerOperation::SkillCompose { prompt, skills } => Ok(serde_json::to_value(
            runtime.compose_skills("You are Colossus.", &prompt, &skills, &[])?,
        )?),
        WorkerOperation::SkillScaffold {
            name,
            description,
            instructions,
            resource_dirs,
        } => Ok(serde_json::to_value(
            runtime
                .scaffold_skill(&name, &description, &instructions, &resource_dirs)
                .await?,
        )?),
        WorkerOperation::SkillInspect { name } => {
            Ok(serde_json::to_value(runtime.inspect_skill(&name).await?)?)
        }
        WorkerOperation::SkillFileRead { name, path } => Ok(serde_json::to_value(
            runtime.read_skill_file(&name, &path).await?,
        )?),
        WorkerOperation::SkillWrite {
            name,
            path,
            content,
            expected_sha256,
        } => Ok(serde_json::to_value(
            runtime
                .write_skill_file(&name, &path, &content, expected_sha256.as_deref())
                .await?,
        )?),
        WorkerOperation::SkillValidate { target, local } => {
            if local {
                Ok(serde_json::to_value(
                    runtime.validate_local_skill(&target).await?,
                )?)
            } else {
                Ok(serde_json::to_value(
                    runtime.validate_installed_skill(&target).await?,
                )?)
            }
        }
        WorkerOperation::SkillInstall { path } => Ok(serde_json::to_value(
            runtime.install_local_skill(&path).await?,
        )?),
        WorkerOperation::SkillResources { name } => Ok(serde_json::to_value(
            runtime
                .skill_resources(&name, std::slice::from_ref(&name))
                .await?,
        )?),
        WorkerOperation::SkillResourceRead { name, path } => Ok(serde_json::to_value(
            runtime
                .read_skill_resource(&name, &path, std::slice::from_ref(&name))
                .await?,
        )?),
        WorkerOperation::PackList { limit } => Ok(serde_json::to_value(
            runtime.list_packs(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::PackGet { name } => Ok(serde_json::to_value(runtime.get_pack(&name)?)?),
        WorkerOperation::PackVerify { path } => {
            Ok(serde_json::to_value(runtime.verify_pack(path).await?)?)
        }
        WorkerOperation::PackInstall {
            path,
            allow_untrusted,
        } => Ok(serde_json::to_value(
            runtime.install_pack(path, allow_untrusted).await?,
        )?),
        WorkerOperation::PackEnable { name } => {
            Ok(serde_json::to_value(runtime.enable_pack(&name).await?)?)
        }
        WorkerOperation::PackDisable { name } => {
            Ok(serde_json::to_value(runtime.disable_pack(&name).await?)?)
        }
        WorkerOperation::PackUninstall { name } => {
            Ok(serde_json::to_value(runtime.uninstall_pack(&name).await?)?)
        }
        WorkerOperation::PackCall { tool } => Ok(runtime.call_pack_tool(&tool).await?),
        WorkerOperation::PackTrustList { limit } => Ok(serde_json::to_value(
            runtime.list_pack_trust(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::PackTrustAdd {
            publisher,
            public_key,
        } => Ok(serde_json::to_value(
            runtime.add_pack_trust(&publisher, &public_key).await?,
        )?),
        WorkerOperation::BundleVerify { path } => {
            Ok(serde_json::to_value(runtime.verify_bundle(path).await?)?)
        }
        WorkerOperation::BundleKeyInfo {
            signing_key_reference,
        } => Ok(serde_json::to_value(
            runtime
                .bundle_signing_key_info(&signing_key_reference)
                .await?,
        )?),
        WorkerOperation::BundleBuild {
            source,
            destination,
            name,
            version,
            publisher,
            created_at,
            source_revision,
            signing_key_reference,
        } => Ok(serde_json::to_value(
            runtime
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
        )?),
        WorkerOperation::BundleInstall { path, prefix } => Ok(serde_json::to_value(
            runtime.install_bundle(path, prefix).await?,
        )?),
        WorkerOperation::IntegrationList { limit } => Ok(serde_json::to_value(
            runtime.list_integrations(limit.clamp(1, 1_000))?,
        )?),
        WorkerOperation::IntegrationGet { name } => {
            Ok(serde_json::to_value(runtime.get_integration(&name)?)?)
        }
        WorkerOperation::IntegrationConnect {
            name,
            base_url,
            auth,
            credential_reference,
            credential_references,
            scopes,
        } => Ok(serde_json::to_value(
            runtime
                .connect_native_integration(
                    &name,
                    base_url.as_deref(),
                    auth,
                    credential_reference.as_deref(),
                    &credential_references,
                    &scopes,
                )
                .await?,
        )?),
        WorkerOperation::IntegrationImportOpenApi {
            name,
            document_source,
            base_url,
            auth,
            credential_reference,
            scopes,
        } => {
            let document = parse_json_source(runtime, &document_source).await?;
            Ok(serde_json::to_value(
                runtime
                    .import_openapi_integration(
                        &name,
                        document,
                        base_url.as_deref(),
                        auth,
                        credential_reference.as_deref(),
                        &scopes,
                    )
                    .await?,
            )?)
        }
        WorkerOperation::IntegrationDisconnect { name } => Ok(serde_json::to_value(
            runtime.disconnect_integration(&name).await?,
        )?),
        WorkerOperation::IntegrationCall {
            tool,
            arguments_source,
        } => {
            let arguments = parse_json_source(runtime, &arguments_source).await?;
            Ok(runtime.call_integration_tool(&tool, arguments).await?)
        }
        WorkerOperation::WorkflowValidate { path } => {
            let validated = runtime.validate_workflow_path(path).await?;
            Ok(json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))
        }
        WorkerOperation::WorkflowRegister { path } => {
            let provenance = format!("repo:{path}");
            let validated = runtime.register_workflow_path(path).await?;
            Ok(json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))
        }
        WorkerOperation::WorkflowList => {
            let definitions = runtime
                .journal()
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
            Ok(serde_json::to_value(definitions)?)
        }
        WorkerOperation::WorkflowShow { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| {
                    WorkerError::Remote(format!("workflow {name}:{version} not found"))
                })?;
            Ok(json!({"definition": definition, "content_hash": content_hash}))
        }
        WorkerOperation::WorkflowStart {
            name,
            version,
            inputs_source,
            queued,
        } => {
            let inputs = parse_json_source(runtime, &inputs_source).await?;
            let run = if queued {
                runtime.workflows().queue_run(&name, &version, inputs)?
            } else {
                runtime
                    .workflows()
                    .start_run(&name, &version, inputs)
                    .await?
            };
            Ok(serde_json::to_value(run)?)
        }
        WorkerOperation::WorkflowScheduleCreate {
            schedule_id,
            name,
            version,
            inputs_source,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at,
        } => {
            let inputs = parse_json_source(runtime, &inputs_source).await?;
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(runtime.workflows().create_schedule(
                &schedule_id,
                &name,
                &version,
                inputs,
                cadence_seconds,
                misfire_policy,
                enabled,
                starts_at.as_deref(),
            )?)?)
        }
        WorkerOperation::WorkflowScheduleList { limit } => Ok(serde_json::to_value(
            runtime.workflows().list_schedules(limit.clamp(1, 10_000))?,
        )?),
        WorkerOperation::WorkflowScheduleShow { schedule_id } => Ok(serde_json::to_value(
            runtime.workflows().get_schedule(&schedule_id)?,
        )?),
        WorkerOperation::WorkflowScheduleSetEnabled {
            schedule_id,
            enabled,
        } => {
            let _guard = maintenance.lock().await;
            Ok(serde_json::to_value(
                runtime
                    .workflows()
                    .set_schedule_enabled(&schedule_id, enabled)?,
            )?)
        }
        WorkerOperation::WorkflowScheduleTick { at } => {
            let _guard = maintenance.lock().await;
            let dispatches = match at {
                Some(at) => runtime.workflows().tick_schedules_at(&at)?,
                None => runtime.workflows().tick_schedules_now()?,
            };
            Ok(serde_json::to_value(dispatches)?)
        }
        WorkerOperation::WorkflowStatus { run_id } => {
            Ok(serde_json::to_value(runtime.workflows().get_run(&run_id)?)?)
        }
        WorkerOperation::WorkflowResume { run_id } => Ok(serde_json::to_value(
            runtime.workflows().resume_run(&run_id).await?,
        )?),
        WorkerOperation::WorkflowInput {
            run_id,
            input_source,
        } => {
            let input = parse_json_source(runtime, &input_source).await?;
            Ok(serde_json::to_value(
                runtime.workflows().provide_input(&run_id, input).await?,
            )?)
        }
        WorkerOperation::WorkflowCancel { run_id } => Ok(serde_json::to_value(
            runtime.workflows().cancel_run(&run_id)?,
        )?),
        WorkerOperation::Drain => drain_once(runtime, maintenance).await,
        WorkerOperation::Shutdown => Ok(json!({"stopping": true})),
        WorkerOperation::RunModel { .. }
        | WorkerOperation::RunModelControlled { .. }
        | WorkerOperation::RunPlan { .. } => Err(WorkerError::Protocol(
            "model runs must use the streaming dispatch path".into(),
        )),
    }
}

async fn drain_once(
    runtime: &Runtime,
    maintenance: &tokio::sync::Mutex<()>,
) -> Result<Value, WorkerError> {
    let _guard = maintenance.lock().await;
    let schedules = runtime.workflows().tick_schedules_now()?;
    let workflows = runtime.workflows().drain().await?;
    // Durable execution queues take precedence over disposable projections so
    // a large projection backlog cannot starve queued child work.
    let subagents = runtime.drain_subagents().await?;
    let projections = runtime.drain_projections()?;
    let audit_exports = runtime.drain_audit_exports().await?;
    Ok(json!({
        "schedules": schedules,
        "workflows": workflows,
        "projections": projections,
        "subagents": subagents,
        "audit_exports": audit_exports,
    }))
}

async fn parse_json_source(runtime: &Runtime, source: &str) -> Result<Value, WorkerError> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.into()
    };
    serde_json::from_str(&document)
        .map_err(|error| WorkerError::Protocol(format!("invalid JSON input: {error}")))
}

struct ChannelWorkerObserver {
    sender: tokio::sync::mpsc::Sender<RunEventEnvelope>,
}

#[async_trait]
impl RunEventObserver for ChannelWorkerObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.sender
            .send(event)
            .await
            .map_err(|_| ModelProviderError::Failed("worker event client disconnected".into()))
    }
}

struct IpcRunObserver<'a, S> {
    stream: &'a mut S,
    key: &'a [u8; 32],
    request_id: &'a str,
    sequence: u64,
}

impl<S> IpcRunObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    async fn complete(&mut self, result: Value) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Complete { result }).await
    }

    async fn error(&mut self, message: String) -> Result<(), WorkerError> {
        self.send(WorkerFrameContent::Error {
            message: bounded_error(&message),
        })
        .await
    }

    async fn send(&mut self, content: WorkerFrameContent) -> Result<(), WorkerError> {
        self.sequence = self.sequence.saturating_add(1);
        write_signed_frame(
            self.stream,
            self.key,
            self.request_id,
            self.sequence,
            content,
        )
        .await
    }
}

#[async_trait]
impl<S> colossus_ports::RunEventObserver for IpcRunObserver<'_, S>
where
    S: AsyncWrite + Unpin + Send,
{
    async fn observe(
        &mut self,
        event: RunEventEnvelope,
    ) -> Result<(), colossus_ports::ModelProviderError> {
        self.send(WorkerFrameContent::Event { event })
            .await
            .map_err(|error| colossus_ports::ModelProviderError::Failed(error.to_string()))
    }
}

fn signed_request(
    key: &[u8; 32],
    operation: WorkerOperation,
    connection_nonce: &str,
) -> Result<WorkerRequest, WorkerError> {
    let request_id = Uuid::now_v7().to_string();
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let nonce = hex::encode(nonce);
    let timestamp_ms = now_ms();
    let tag = request_tag(
        key,
        &UnsignedRequest {
            version: PROTOCOL_VERSION,
            request_id: &request_id,
            timestamp_ms,
            nonce: &nonce,
            connection_nonce,
            operation: &operation,
        },
    )?;
    Ok(WorkerRequest {
        version: PROTOCOL_VERSION,
        request_id,
        timestamp_ms,
        nonce,
        connection_nonce: connection_nonce.into(),
        operation,
        authentication_tag: tag,
    })
}

fn validate_request(
    key: &[u8; 32],
    request: &WorkerRequest,
    replay: &Mutex<ReplayGuard>,
    connection_nonce: &str,
) -> Result<(), WorkerError> {
    if request.version != PROTOCOL_VERSION
        || request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.connection_nonce != connection_nonce
        || (now_ms() - request.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "unsupported version, invalid id, or expired timestamp".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedRequest {
            version: request.version,
            request_id: &request.request_id,
            timestamp_ms: request.timestamp_ms,
            nonce: &request.nonce,
            connection_nonce: &request.connection_nonce,
            operation: &request.operation,
        },
        &request.authentication_tag,
        "worker request",
    )?;
    replay
        .lock()
        .map_err(|error| WorkerError::Protocol(error.to_string()))?
        .accept(&request.nonce)
}

fn validate_frame(
    key: &[u8; 32],
    request_id: &str,
    sequence: &mut u64,
    frame: &WorkerFrame,
) -> Result<WorkerFrameContent, WorkerError> {
    let expected_sequence = sequence.saturating_add(1);
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.sequence != expected_sequence
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "response version, request id, sequence, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedFrame {
            version: frame.version,
            request_id: &frame.request_id,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content_base64: &frame.content_base64,
        },
        &frame.authentication_tag,
        "worker response",
    )?;
    let content = BASE64
        .decode(&frame.content_base64)
        .map_err(|_| WorkerError::Protocol("worker response payload is not base64".into()))?;
    let content = serde_json::from_slice(&content)
        .map_err(|error| WorkerError::Protocol(format!("invalid worker response: {error}")))?;
    *sequence = expected_sequence;
    Ok(content)
}

fn validate_client_frame(
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: &mut u64,
    frame: &WorkerClientFrame,
) -> Result<ClientFrameContent, WorkerError> {
    let expected_sequence = sequence.saturating_add(1);
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.connection_nonce != connection_nonce
        || frame.sequence != expected_sequence
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "client frame version, request, connection, sequence, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedClientFrame {
            version: frame.version,
            request_id: &frame.request_id,
            connection_nonce: &frame.connection_nonce,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content_base64: &frame.content_base64,
        },
        &frame.authentication_tag,
        "worker client frame",
    )?;
    let content = BASE64
        .decode(&frame.content_base64)
        .map_err(|_| WorkerError::Protocol("worker client payload is not base64".into()))?;
    let content = serde_json::from_slice(&content)
        .map_err(|error| WorkerError::Protocol(format!("invalid worker client frame: {error}")))?;
    *sequence = expected_sequence;
    Ok(content)
}

async fn write_signed_frame<S>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    sequence: u64,
    content: WorkerFrameContent,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
{
    let timestamp_ms = now_ms();
    let content =
        serde_json::to_vec(&content).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let content_base64 = BASE64.encode(content);
    let authentication_tag = request_tag(
        key,
        &UnsignedFrame {
            version: PROTOCOL_VERSION,
            request_id,
            sequence,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )?;
    write_message(
        stream,
        &WorkerFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            sequence,
            timestamp_ms,
            content_base64,
            authentication_tag,
        },
        MAX_FRAME_BYTES,
    )
    .await
}

async fn write_signed_client_frame<S>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: u64,
    content: ClientFrameContent,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
{
    let timestamp_ms = now_ms();
    let content =
        serde_json::to_vec(&content).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    if content.len() > MAX_REQUEST_BYTES {
        return Err(WorkerError::Protocol(
            "worker client frame exceeds the 1 MiB limit".into(),
        ));
    }
    let content_base64 = BASE64.encode(content);
    let authentication_tag = request_tag(
        key,
        &UnsignedClientFrame {
            version: PROTOCOL_VERSION,
            request_id,
            connection_nonce,
            sequence,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )?;
    write_message(
        stream,
        &WorkerClientFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            connection_nonce: connection_nonce.into(),
            sequence,
            timestamp_ms,
            content_base64,
            authentication_tag,
        },
        MAX_REQUEST_BYTES,
    )
    .await
}

fn request_tag<T: Serialize>(key: &[u8; 32], value: &T) -> Result<String, WorkerError> {
    let bytes = canonical_authentication_bytes(value)?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn verify_tag<T: Serialize>(
    key: &[u8; 32],
    value: &T,
    tag: &str,
    context: &str,
) -> Result<(), WorkerError> {
    let bytes = canonical_authentication_bytes(value)?;
    let tag = hex::decode(tag)
        .map_err(|_| WorkerError::Protocol("authentication tag is not hexadecimal".into()))?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    mac.verify_slice(&tag)
        .map_err(|_| WorkerError::Protocol(format!("{context} authentication tag mismatch")))
}

fn canonical_authentication_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, WorkerError> {
    let value =
        serde_json::to_value(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let mut bytes = Vec::new();
    write_canonical_json(&value, &mut bytes)?;
    Ok(bytes)
}

fn write_canonical_json(value: &Value, bytes: &mut Vec<u8>) -> Result<(), WorkerError> {
    match value {
        Value::Object(object) => {
            bytes.push(b'{');
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                serde_json::to_writer(&mut *bytes, key)
                    .map_err(|error| WorkerError::Protocol(error.to_string()))?;
                bytes.push(b':');
                write_canonical_json(value, bytes)?;
            }
            bytes.push(b'}');
        }
        Value::Array(array) => {
            bytes.push(b'[');
            for (index, value) in array.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                write_canonical_json(value, bytes)?;
            }
            bytes.push(b']');
        }
        _ => serde_json::to_writer(bytes, value)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?,
    }
    Ok(())
}

async fn write_message<S, T>(stream: &mut S, value: &T, limit: usize) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    if bytes.len() > limit || bytes.len() > u32::MAX as usize {
        return Err(WorkerError::Protocol("IPC message exceeds bound".into()));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_message<S, T>(stream: &mut S, limit: usize) -> Result<T, WorkerError>
where
    S: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > limit {
        return Err(WorkerError::Protocol(
            "IPC message length is empty or exceeds bound".into(),
        ));
    }
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|error| WorkerError::Protocol(error.to_string()))
}

fn now_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

fn bounded_error(message: &str) -> String {
    message.chars().take(4_096).collect()
}

#[cfg(unix)]
mod platform {
    use super::WorkerError;
    use std::{
        fs,
        os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
        path::Path,
    };
    use tokio::net::{UnixListener, UnixStream};

    pub type ClientStream = UnixStream;
    pub type ServerStream = UnixStream;

    pub struct Listener {
        inner: UnixListener,
        endpoint: String,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let path = Path::new(endpoint);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if !metadata.file_type().is_socket() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint exists and is not a socket: {endpoint}"
                    )));
                }
                if UnixStream::connect(path).await.is_ok() {
                    return Err(WorkerError::Protocol(format!(
                        "worker endpoint is already active: {endpoint}"
                    )));
                }
                fs::remove_file(path)?;
            }
            let inner = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(Self {
                inner,
                endpoint: endpoint.into(),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            self.inner
                .accept()
                .await
                .map(|(stream, _)| stream)
                .map_err(Into::into)
        }

        pub fn cleanup(&mut self) {
            let _ = fs::remove_file(&self.endpoint);
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        UnixStream::connect(endpoint).await
    }

    pub fn connection_is_busy(_error: &std::io::Error) -> bool {
        false
    }

    pub fn endpoint_is_trusted(endpoint: &str) -> Result<bool, WorkerError> {
        let path = Path::new(endpoint);
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let Some(parent) = path.parent() else {
            return Err(WorkerError::Protocol(
                "worker endpoint has no parent directory".into(),
            ));
        };
        let parent = fs::metadata(parent)?;
        if !metadata.file_type().is_socket()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != parent.uid()
        {
            return Err(WorkerError::Protocol(
                "worker endpoint is not an owner-only socket in its owning directory".into(),
            ));
        }
        Ok(true)
    }
}

#[cfg(windows)]
mod platform {
    use super::WorkerError;
    use std::{io::ErrorKind, time::Duration};
    use tokio::{
        net::windows::named_pipe::{
            ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
        },
        time::{Instant, sleep},
    };

    pub type ClientStream = NamedPipeClient;
    pub type ServerStream = NamedPipeServer;

    const ERROR_PIPE_BUSY: i32 = 231;

    pub struct Listener {
        endpoint: String,
        next: Option<NamedPipeServer>,
    }

    impl Listener {
        pub async fn bind(endpoint: &str) -> Result<Self, WorkerError> {
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .create(endpoint)?;
            Ok(Self {
                endpoint: endpoint.into(),
                next: Some(next),
            })
        }

        pub async fn accept(&mut self) -> Result<ServerStream, WorkerError> {
            // Keep the pending instance in `self` while awaiting connection. This
            // future is polled inside `select!`; taking the instance first would
            // drop it whenever another branch wins and permanently lose the
            // listener after serving one client.
            self.next
                .as_ref()
                .ok_or_else(|| WorkerError::Protocol("named pipe listener lost instance".into()))?
                .connect()
                .await?;
            let server = self
                .next
                .take()
                .ok_or_else(|| WorkerError::Protocol("named pipe listener lost instance".into()))?;
            self.next = Some(ServerOptions::new().create(&self.endpoint)?);
            Ok(server)
        }

        pub fn cleanup(&mut self) {}
    }

    pub async fn connect(endpoint: &str) -> Result<ClientStream, std::io::Error> {
        let missing_deadline = Instant::now() + Duration::from_secs(2);
        let busy_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(client) => return Ok(client),
                Err(error) if connection_is_busy(&error) && Instant::now() < busy_deadline => {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error)
                    if error.kind() == ErrorKind::NotFound && Instant::now() < missing_deadline =>
                {
                    sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn connection_is_busy(error: &std::io::Error) -> bool {
        error.kind() == ErrorKind::WouldBlock || error.raw_os_error() == Some(ERROR_PIPE_BUSY)
    }

    pub fn endpoint_is_trusted(_endpoint: &str) -> Result<bool, WorkerError> {
        Ok(true)
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("colossus-worker supports only Unix sockets or Windows named pipes");

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_pipe_saturation_is_classified_as_busy() {
        let error = std::io::Error::from_raw_os_error(231);
        assert!(platform::connection_is_busy(&error));
    }

    #[test]
    fn workflow_schedule_operation_round_trips_the_worker_contract() {
        let encoded = serde_json::to_value(WorkerOperation::WorkflowScheduleCreate {
            schedule_id: "nightly".into(),
            name: "smoke".into(),
            version: "1.0.0".into(),
            inputs_source: r#"{"message":"scheduled"}"#.into(),
            cadence_seconds: 3_600,
            misfire_policy: WorkflowScheduleMisfirePolicy::Skip,
            enabled: false,
            starts_at: Some("2026-01-01T12:00:00Z".into()),
        })
        .expect("serialize schedule operation");
        assert_eq!(encoded["operation"], "workflow_schedule_create");
        assert_eq!(encoded["misfire_policy"], "skip");
        let decoded: WorkerOperation =
            serde_json::from_value(encoded).expect("deserialize schedule operation");
        let WorkerOperation::WorkflowScheduleCreate {
            schedule_id,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at,
            ..
        } = decoded
        else {
            panic!("expected schedule creation operation");
        };
        assert_eq!(schedule_id, "nightly");
        assert_eq!(cadence_seconds, 3_600);
        assert_eq!(misfire_policy, WorkflowScheduleMisfirePolicy::Skip);
        assert!(!enabled);
        assert_eq!(starts_at.as_deref(), Some("2026-01-01T12:00:00Z"));
    }

    #[test]
    fn authenticated_frames_cover_exact_serialized_payload_bytes() {
        let key = [6_u8; 32];
        let content = WorkerFrameContent::Complete {
            result: json!({
                "z": {"two": 2, "one": 1},
                "a": [true, null, "value"],
            }),
        };
        let timestamp_ms = now_ms();
        let content_base64 = BASE64.encode(serde_json::to_vec(&content).expect("content JSON"));
        let authentication_tag = request_tag(
            &key,
            &UnsignedFrame {
                version: PROTOCOL_VERSION,
                request_id: "canonical-frame",
                sequence: 1,
                timestamp_ms,
                content_base64: &content_base64,
            },
        )
        .expect("tag");
        let encoded = serde_json::to_vec(&WorkerFrame {
            version: PROTOCOL_VERSION,
            request_id: "canonical-frame".into(),
            sequence: 1,
            timestamp_ms,
            content_base64,
            authentication_tag,
        })
        .expect("frame JSON");
        let decoded: WorkerFrame = serde_json::from_slice(&encoded).expect("decoded frame");
        let mut sequence = 0;
        let decoded =
            validate_frame(&key, "canonical-frame", &mut sequence, &decoded).expect("frame");
        assert!(matches!(decoded, WorkerFrameContent::Complete { .. }));
        assert_eq!(sequence, 1);
    }

    #[test]
    fn authentication_detects_tampering_and_replay() {
        let key = [7_u8; 32];
        let mut request =
            signed_request(&key, WorkerOperation::Ping, "connection-one").expect("request");
        request.operation = WorkerOperation::Echo {
            message: "tampered".into(),
        };
        let replay = Mutex::new(ReplayGuard::default());
        assert!(matches!(
            validate_request(&key, &request, &replay, "connection-one"),
            Err(WorkerError::Protocol(_))
        ));

        let request =
            signed_request(&key, WorkerOperation::Ping, "connection-two").expect("request");
        validate_request(&key, &request, &replay, "connection-two").expect("first request");
        assert!(matches!(
            validate_request(&key, &request, &replay, "connection-two"),
            Err(WorkerError::Protocol(message)) if message.contains("replayed")
        ));
    }

    fn signed_client_frame(
        key: &[u8; 32],
        request_id: &str,
        connection_nonce: &str,
        sequence: u64,
        content: ClientFrameContent,
    ) -> WorkerClientFrame {
        let timestamp_ms = now_ms();
        let content_base64 = BASE64.encode(serde_json::to_vec(&content).expect("content"));
        let authentication_tag = request_tag(
            key,
            &UnsignedClientFrame {
                version: PROTOCOL_VERSION,
                request_id,
                connection_nonce,
                sequence,
                timestamp_ms,
                content_base64: &content_base64,
            },
        )
        .expect("tag");
        WorkerClientFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            connection_nonce: connection_nonce.into(),
            sequence,
            timestamp_ms,
            content_base64,
            authentication_tag,
        }
    }

    #[test]
    fn client_frames_reject_wrong_connection_request_and_replay() {
        let key = [11_u8; 32];
        let frame = signed_client_frame(
            &key,
            "request-one",
            "connection-one",
            1,
            ClientFrameContent::Cancel,
        );
        let mut sequence = 0;
        assert!(matches!(
            validate_client_frame(
                &key,
                "request-one",
                "wrong-connection",
                &mut sequence,
                &frame,
            ),
            Err(WorkerError::Protocol(_))
        ));
        assert!(matches!(
            validate_client_frame(
                &key,
                "wrong-request",
                "connection-one",
                &mut sequence,
                &frame,
            ),
            Err(WorkerError::Protocol(_))
        ));
        validate_client_frame(&key, "request-one", "connection-one", &mut sequence, &frame)
            .expect("first frame");
        assert!(matches!(
            validate_client_frame(&key, "request-one", "connection-one", &mut sequence, &frame,),
            Err(WorkerError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn prompt_ids_are_one_use_and_unknown_ids_fail_closed() {
        let (prompt_tx, _prompt_rx) = tokio::sync::mpsc::channel(1);
        let bridge = InteractiveRunBridge {
            prompts: prompt_tx,
            responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        };
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        bridge
            .responses
            .lock()
            .await
            .insert("prompt-one".into(), response_tx);
        bridge
            .respond("prompt-one", Some("answer".into()))
            .await
            .expect("first response");
        assert_eq!(
            response_rx.await.expect("answer").as_deref(),
            Some("answer")
        );
        assert!(matches!(
            bridge.respond("prompt-one", None).await,
            Err(WorkerError::Protocol(message)) if message.contains("replayed")
        ));
        assert!(matches!(
            bridge.respond("wrong-prompt", None).await,
            Err(WorkerError::Protocol(_))
        ));
    }

    fn test_prompt(id: &str, kind: WorkerPromptKind) -> WorkerPrompt {
        WorkerPrompt {
            prompt_id: id.into(),
            kind,
            title: "Test prompt".into(),
            question: "Continue?".into(),
            choices: vec!["Allow once".into(), "Deny".into()],
            allow_free_form: false,
            details: Value::Null,
        }
    }

    #[tokio::test]
    async fn prompt_bridge_covers_answer_cancel_disconnect_timeout_and_run_cancel() {
        let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(4);
        let bridge = InteractiveRunBridge {
            prompts: prompt_tx,
            responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        };

        let answered_bridge = bridge.clone();
        let answered = tokio::spawn(async move {
            answered_bridge
                .request(test_prompt("answered", WorkerPromptKind::UserInput))
                .await
        });
        let prompt = prompt_rx.recv().await.expect("answered prompt");
        bridge
            .respond(&prompt.prompt_id, Some("Allow once".into()))
            .await
            .expect("answer prompt");
        assert_eq!(
            answered.await.expect("answered task").expect("answer"),
            Some("Allow once".into())
        );

        let cancelled_bridge = bridge.clone();
        let cancelled = tokio::spawn(async move {
            cancelled_bridge
                .request(test_prompt("cancelled", WorkerPromptKind::UserInput))
                .await
        });
        prompt_rx.recv().await.expect("cancelled prompt");
        bridge.cancel_all().await;
        assert_eq!(cancelled.await.expect("cancel task").expect("cancel"), None);

        let timeout_bridge = bridge.clone();
        let timed_out = tokio::spawn(async move {
            timeout_bridge
                .request_with_timeout(
                    test_prompt("timeout", WorkerPromptKind::UserInput),
                    Duration::from_millis(1),
                )
                .await
        });
        prompt_rx.recv().await.expect("timeout prompt");
        assert!(matches!(
            timed_out.await.expect("timeout task"),
            Err(message) if message.contains("timed out")
        ));

        let control = RunControl::default();
        control.cancel();
        assert!(control.is_cancelled());

        let (disconnected_tx, disconnected_rx) = tokio::sync::mpsc::channel(1);
        drop(disconnected_rx);
        let disconnected = InteractiveRunBridge {
            prompts: disconnected_tx,
            responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
        };
        assert!(matches!(
            disconnected
                .request(test_prompt("disconnect", WorkerPromptKind::Approval))
                .await,
            Err(message) if message.contains("disconnected")
        ));
    }

    #[tokio::test]
    async fn interactive_worker_approval_accepts_only_the_exact_allow_choice() {
        let request = colossus_policy::effect_request(
            colossus_policy::system_actor("worker-test"),
            "filesystem.write",
            "note.txt",
            json!({"content": "bounded"}),
        );
        let decision = PolicyDecision {
            decision_id: "decision-test".into(),
            policy_revision: "test-v1".into(),
            outcome: colossus_contracts::DecisionOutcome::RequireApproval,
            reason: "operator must approve".into(),
            obligations: colossus_contracts::PolicyObligations::default(),
        };

        for (answer, expected_approval) in [("Allow once", true), ("Deny", false)] {
            let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(1);
            let bridge = InteractiveRunBridge {
                prompts: prompt_tx,
                responses: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
            };
            let responder_bridge = bridge.clone();
            let answer = answer.to_owned();
            let responder = tokio::spawn(async move {
                let prompt = prompt_rx.recv().await.expect("approval prompt");
                assert_eq!(prompt.kind, WorkerPromptKind::Approval);
                responder_bridge
                    .respond(&prompt.prompt_id, Some(answer))
                    .await
                    .expect("approval response");
            });
            let provider = WorkerInteractiveApproval {
                mode: WorkerApprovalMode::Ask,
            };
            let proof = ACTIVE_INTERACTIVE_RUN
                .scope(
                    bridge,
                    provider.request_approval(&request, "request-hash", &decision),
                )
                .await
                .expect("approval result");
            responder.await.expect("responder");
            assert_eq!(proof.is_some(), expected_approval);
        }
    }

    #[tokio::test]
    async fn protocol_version_mismatch_has_restart_guidance() {
        let key = [13_u8; 32];
        let mut frame =
            signed_client_frame(&key, "request", "connection", 1, ClientFrameContent::Cancel);
        frame.version = PROTOCOL_VERSION - 1;
        let mut sequence = 0;
        assert!(matches!(
            validate_client_frame(&key, "request", "connection", &mut sequence, &frame),
            Err(WorkerError::Protocol(message)) if message.contains("version")
        ));

        let (mut client, mut server) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            write_message(
                &mut client,
                &ClientHello {
                    version: PROTOCOL_VERSION - 1,
                    challenge: "a".repeat(64),
                },
                1024,
            )
            .await
        });
        assert!(matches!(
            server_handshake(&mut server, &key).await,
            Err(WorkerError::Protocol(message)) if message.contains("restart the worker")
        ));
        writer.await.expect("hello writer").expect("hello");
    }

    #[tokio::test]
    async fn oversized_client_prompt_response_is_rejected_before_write() {
        let key = [12_u8; 32];
        let (mut writer, _reader) = tokio::io::duplex(64);
        let result = write_signed_client_frame(
            &mut writer,
            &key,
            "request",
            "connection",
            1,
            ClientFrameContent::PromptResponse {
                prompt_id: "prompt".into(),
                answer: Some("x".repeat(MAX_REQUEST_BYTES + 1)),
            },
        )
        .await;
        assert!(matches!(result, Err(WorkerError::Protocol(message)) if message.contains("1 MiB")));
    }

    #[tokio::test]
    async fn framing_rejects_oversized_lengths_before_allocation() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let task = tokio::spawn(async move {
            writer
                .write_u32((MAX_REQUEST_BYTES + 1) as u32)
                .await
                .expect("length");
        });
        let result = read_message::<_, WorkerRequest>(&mut reader, MAX_REQUEST_BYTES).await;
        assert!(matches!(result, Err(WorkerError::Protocol(_))));
        task.await.expect("writer");
    }

    #[tokio::test]
    async fn client_discloses_no_operation_to_an_unauthenticated_server() {
        let expected_key = [8_u8; 32];
        let fake_key = [9_u8; 32];
        let (mut client, mut server) = tokio::io::duplex(4_096);
        let fake = tokio::spawn(async move {
            let hello: ClientHello = read_message(&mut server, 1024).await.expect("hello");
            let server_nonce = hex::encode([3_u8; 32]);
            let timestamp_ms = now_ms();
            let authentication_tag = request_tag(
                &fake_key,
                &UnsignedServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: &hello.challenge,
                    server_nonce: &server_nonce,
                    timestamp_ms,
                },
            )
            .expect("tag");
            write_message(
                &mut server,
                &ServerHello {
                    version: PROTOCOL_VERSION,
                    challenge: hello.challenge,
                    server_nonce,
                    timestamp_ms,
                    authentication_tag,
                },
                1024,
            )
            .await
            .expect("server hello");
            read_message::<_, WorkerRequest>(&mut server, MAX_REQUEST_BYTES).await
        });
        assert!(matches!(
            client_handshake(&mut client, &expected_key).await,
            Err(WorkerError::Protocol(_))
        ));
        drop(client);
        assert!(matches!(
            fake.await.expect("fake server"),
            Err(WorkerError::Io(_))
        ));
    }
}
