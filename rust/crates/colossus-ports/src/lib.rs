//! Replaceable runtime ports. Adapters depend on these contracts, never the reverse.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ApprovalProof, ContextSnapshot, DecisionStatus, EffectRequest, EventEnvelope,
    ExecutionContext, GoalRecord, GoalStatus, IntegrationConnection, KeyDecision, MemoryRecord,
    ModelMessage, ModelRequest, ModelToolDefinition, NewEvent, PackInstallation, PackStatus,
    PlanRecord, PlanStatus, PolicyDecision, PreparedContext, ProjectionBatch, ProjectionWorkItem,
    ProviderEvent, ProviderRoute, ProviderTurn, PublisherTrust, ResearchClaim, ResearchRun,
    ResearchSource, SessionMessage, SessionSummary, SignedCheckpoint, SkillDuplicate, SkillRecord,
    SubagentJob, SubagentStatus, TaskRecord, TaskStatus, ToolCall, ToolResult, ToolSpec,
    WorkflowDefinition, WorkflowRun,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Journal and repository failure.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The expected stream version did not match the durable version.
    #[error(
        "optimistic concurrency conflict for {stream_id}: expected {expected}, actual {actual}"
    )]
    Conflict {
        /// Aggregate stream identifier.
        stream_id: String,
        /// Caller expectation.
        expected: u64,
        /// Durable version.
        actual: u64,
    },
    /// Journal verification failed and the runtime must recover read-only.
    #[error("journal verification failed: {0}")]
    Verification(String),
    /// Requested record does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The configured key is absent or invalid.
    #[error("key unavailable: {0}")]
    KeyUnavailable(String),
    /// Adapter-specific failure with secrets removed.
    #[error("storage adapter failure: {0}")]
    Adapter(String),
    /// Writes are disabled because startup verification failed.
    #[error("runtime is in read-only recovery mode")]
    RecoveryMode,
}

/// Provider-turn failure classification preserved across the application port.
#[derive(Debug, Error)]
pub enum ModelProviderError {
    /// Profile or request configuration is invalid.
    #[error("provider configuration failed: {0}")]
    Configuration(String),
    /// A bounded correction turn may safely be attempted.
    #[error("recoverable provider failure {code}: {message}")]
    Recoverable {
        /// Stable machine-readable recovery code.
        code: String,
        /// Bounded safe diagnostic.
        message: String,
    },
    /// Provider failed with a known terminal outcome.
    #[error("provider turn failed: {0}")]
    Failed(String),
    /// The external outcome cannot be proven and must not be retried.
    #[error("provider outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Tool lookup, validation, policy, or execution failure.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool name is absent from the active catalog.
    #[error("unknown tool: {0}")]
    Unknown(String),
    /// Arguments failed the strict registered schema.
    #[error("invalid arguments for {tool}: {message}")]
    InvalidArguments {
        /// Requested tool.
        tool: String,
        /// Bounded validation detail.
        message: String,
    },
    /// Policy or approval denied the effect before execution.
    #[error("tool execution denied: {0}")]
    Denied(String),
    /// Adapter reported a known failure.
    #[error("tool execution failed: {0}")]
    Failed(String),
    /// Tool effect may have occurred and cannot be retried implicitly.
    #[error("tool outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Context preparation or snapshot lifecycle failure.
#[derive(Debug, Error)]
pub enum ContextError {
    /// Context configuration or request cannot satisfy the safety contract.
    #[error("context configuration failed: {0}")]
    Configuration(String),
    /// Canonical snapshot persistence or session reconstruction failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Optional model summarization failed before deterministic fallback could run.
    #[error(transparent)]
    Provider(#[from] ModelProviderError),
}

/// Result of full journal verification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Number of verified records.
    pub event_count: u64,
    /// Highest verified global sequence.
    pub last_sequence: u64,
    /// Hash at the verified chain head.
    pub last_hash: String,
    /// Latest verified checkpoint, if present.
    pub checkpoint: Option<SignedCheckpoint>,
}

/// Authoritative immutable event store.
pub trait EventJournal: Send + Sync {
    /// Append one event atomically.
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError>;

    /// Append events in one transaction and in the supplied order.
    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read a stream in ascending version order.
    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read global events from a one-based sequence, bounded by `limit`.
    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read projection outbox items in ascending sequence order.
    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError>;

    /// Return the durable global sequence and record hash at the journal head.
    fn head(&self) -> Result<(u64, String), StoreError>;

    /// Decrypt an event payload after policy has authorized disclosure.
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError>;

    /// Verify encryption, hashes, sequence, secure anchor, and checkpoints.
    fn verify(&self) -> Result<VerificationReport, StoreError>;

    /// Return whether writes are blocked due to failed verification.
    fn is_recovery_mode(&self) -> bool;

    /// Create a signed checkpoint at the current chain head.
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError>;
}

/// Role-routed, policy-bound model provider used by the application loop.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Resolve role metadata without performing an effect.
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError>;

    /// Execute one normalized provider turn through the effect boundary.
    async fn turn(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError>;

    /// Execute one provider turn while observing safe events as they are released.
    async fn turn_stream(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
        observer: &mut dyn ProviderEventObserver,
    ) -> Result<ProviderTurn, ModelProviderError> {
        let turn = self.turn(role, request, context).await?;
        for event in &turn.events {
            observer.observe(event.clone()).await?;
        }
        Ok(turn)
    }
}

/// Application observer for provider events released through policy.
#[async_trait]
pub trait ProviderEventObserver: Send {
    /// Persist or render one safe ordered event.
    async fn observe(&mut self, event: ProviderEvent) -> Result<(), ModelProviderError>;
}

/// Active model-visible tool catalog with strict schema validation.
pub trait ToolRegistry: Send + Sync {
    /// Stable sorted active specifications.
    fn list_specs(&self) -> Vec<ToolSpec>;

    /// Resolve and validate one call before policy evaluation.
    fn validate(&self, call: &ToolCall) -> Result<ToolSpec, ToolError>;
}

/// Execute a previously validated tool call.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute with full run/session provenance.
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError>;
}

/// Supplies journal encryption keys without a plaintext fallback.
pub trait KeyProvider: Send + Sync {
    /// Active key identifier and exactly 32 bytes of key material.
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError>;

    /// Resolve historical key material by identifier.
    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError>;

    /// Persist an independently protected sequence/hash anchor.
    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError>;

    /// Load the last independently protected sequence/hash anchor.
    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError>;
}

/// Signs and verifies immutable chain checkpoints.
pub trait CheckpointSigner: Send + Sync {
    /// Stable public key identifier.
    fn key_id(&self) -> &str;

    /// Sign the canonical checkpoint message.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, StoreError>;

    /// Verify a checkpoint signature.
    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), StoreError>;
}

/// Disposable projection state.
pub trait ProjectionStore: Send + Sync {
    /// Last globally applied sequence for a named projection.
    fn position(&self, projection: &str) -> Result<u64, StoreError>;

    /// Load one projection-local record.
    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError>;

    /// List bounded projection-local records in key order.
    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError>;

    /// Atomically apply mutations and advance an optimistic projection position.
    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError>;

    /// Delete a projection so it can be rebuilt.
    fn reset(&self, projection: &str) -> Result<(), StoreError>;
}

/// Shared behavior for aggregate repositories reconstructed from events.
pub trait AggregateRepository: Send + Sync {
    /// Load the aggregate's current JSON projection.
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError>;

    /// List bounded aggregate projections.
    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError>;
}

/// Canonical event-sourced session and append-only message repository.
pub trait SessionRepository: Send + Sync {
    /// Create an empty durable session with a caller-supplied stable id.
    fn create_session(
        &self,
        id: &str,
        title: Option<&str>,
        actor: Actor,
    ) -> Result<SessionSummary, StoreError>;

    /// Reconstruct one session summary from canonical events.
    fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, StoreError>;

    /// List recent reconstructed sessions, newest first.
    fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError>;

    /// Append one message using optimistic session-stream concurrency.
    fn append_message(
        &self,
        session_id: &str,
        run_id: &str,
        message: colossus_contracts::ModelMessage,
        actor: Actor,
    ) -> Result<SessionMessage, StoreError>;

    /// Reconstruct every append-only message in sequence order.
    fn list_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, StoreError>;
}

/// Canonical immutable context snapshots and explicit activation history.
pub trait ContextRepository: Send + Sync {
    /// Append and activate a new snapshot using session-stream concurrency.
    fn create(
        &self,
        snapshot: ContextSnapshot,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError>;

    /// Reconstruct every snapshot for a session in creation order.
    fn list(&self, session_id: &str) -> Result<Vec<ContextSnapshot>, StoreError>;

    /// Return the explicitly active snapshot, if any.
    fn active(&self, session_id: &str) -> Result<Option<ContextSnapshot>, StoreError>;

    /// Activate an existing snapshot without mutating or deleting later snapshots.
    fn activate(
        &self,
        session_id: &str,
        snapshot_id: &str,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError>;
}

/// Shared context preparation boundary used by every agent provider turn.
#[async_trait]
pub trait ContextPreparer: Send + Sync {
    /// Apply an active snapshot or create one when the configured budget requires it.
    async fn prepare(
        &self,
        session_id: &str,
        instructions: &str,
        messages: Vec<ModelMessage>,
        tools: &[ModelToolDefinition],
        context: ExecutionContext,
        force: bool,
    ) -> Result<PreparedContext, ContextError>;
}
/// Canonical task and key-decision lifecycle repository.
pub trait WorkRepository: Send + Sync {
    /// Create a new session-scoped task.
    fn create_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError>;

    /// Append the complete next task state after validating immutable identity fields.
    fn update_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError>;

    /// Reconstruct one task from canonical events.
    fn get_task(&self, id: &str) -> Result<Option<TaskRecord>, StoreError>;

    /// List bounded tasks with optional session and status filters.
    fn list_tasks(
        &self,
        session_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError>;

    /// Create a new active key decision.
    fn create_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError>;

    /// Append the complete next active key-decision state.
    fn update_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError>;

    /// Reconstruct one key decision from canonical events.
    fn get_decision(&self, id: &str) -> Result<Option<KeyDecision>, StoreError>;

    /// List bounded decisions with optional session and status filters.
    fn list_decisions(
        &self,
        session_id: Option<&str>,
        status: Option<DecisionStatus>,
        limit: usize,
    ) -> Result<Vec<KeyDecision>, StoreError>;

    /// Archive one active decision through a new immutable event.
    fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError>;

    /// Atomically supersede one active decision and create its replacement.
    fn supersede_decision(
        &self,
        id: &str,
        replacement: KeyDecision,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError>;

    /// Create a new draft plan.
    fn create_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError>;

    /// Append a validated draft edit or lifecycle transition.
    fn update_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError>;

    /// Reconstruct one canonical plan.
    fn get_plan(&self, id: &str) -> Result<Option<PlanRecord>, StoreError>;

    /// List bounded plans with optional session and status filters.
    fn list_plans(
        &self,
        session_id: Option<&str>,
        status: Option<PlanStatus>,
        limit: usize,
    ) -> Result<Vec<PlanRecord>, StoreError>;

    /// Create a new active bounded-autonomy goal.
    fn create_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError>;

    /// Atomically consume one approved plan and create its linked active goal.
    fn create_goal_from_plan(
        &self,
        goal: GoalRecord,
        executed_plan: PlanRecord,
        actor: Actor,
    ) -> Result<(GoalRecord, PlanRecord), StoreError>;

    /// Append an iteration or terminal goal state transition.
    fn update_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError>;

    /// Reconstruct one canonical goal.
    fn get_goal(&self, id: &str) -> Result<Option<GoalRecord>, StoreError>;

    /// List bounded goals with optional session and status filters.
    fn list_goals(
        &self,
        session_id: Option<&str>,
        status: Option<GoalStatus>,
        limit: usize,
    ) -> Result<Vec<GoalRecord>, StoreError>;

    /// Create one queued durable child-agent job.
    fn create_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError>;

    /// Append one validated child-agent lifecycle transition.
    fn update_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError>;

    /// Reconstruct one child-agent job.
    fn get_subagent(&self, id: &str) -> Result<Option<SubagentJob>, StoreError>;

    /// List bounded child-agent jobs.
    fn list_subagents(
        &self,
        session_id: Option<&str>,
        status: Option<SubagentStatus>,
        limit: usize,
    ) -> Result<Vec<SubagentJob>, StoreError>;
}
/// Canonical event-sourced memory lifecycle repository.
pub trait MemoryRepository: Send + Sync {
    /// Create a new active canonical record.
    fn create(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// Load one reconstructed canonical record.
    fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>;

    /// Append a new active state for one existing record without changing identity or scope.
    fn update(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// List bounded active canonical records before policy filtering.
    fn list_active(&self, limit: usize) -> Result<Vec<MemoryRecord>, StoreError>;

    /// List bounded canonical records with an optional lifecycle filter.
    fn list_memories(
        &self,
        status: Option<colossus_contracts::MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;

    /// Archive a canonical record using a new lifecycle event.
    fn archive(&self, id: &str, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// Atomically supersede one record and create its replacement.
    fn supersede(
        &self,
        id: &str,
        replacement: MemoryRecord,
        actor: Actor,
    ) -> Result<(MemoryRecord, MemoryRecord), StoreError>;
}
/// Canonical event-sourced research runs, evidence, and claims.
pub trait ResearchRepository: Send + Sync {
    /// Create one running research aggregate.
    fn create_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError>;

    /// Append a validated lifecycle/progress update.
    fn update_run(&self, run: ResearchRun, actor: Actor) -> Result<ResearchRun, StoreError>;

    /// Reconstruct one canonical run.
    fn get_run(&self, id: &str) -> Result<Option<ResearchRun>, StoreError>;

    /// List bounded runs with optional session filtering.
    fn list_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResearchRun>, StoreError>;

    /// Append one canonical evidence source with a stable label.
    fn add_source(
        &self,
        source: ResearchSource,
        actor: Actor,
    ) -> Result<ResearchSource, StoreError>;

    /// List source records in stable label order.
    fn list_sources(&self, run_id: &str) -> Result<Vec<ResearchSource>, StoreError>;

    /// Append one source-backed canonical claim.
    fn add_claim(&self, claim: ResearchClaim, actor: Actor) -> Result<ResearchClaim, StoreError>;

    /// List claims in durable append order.
    fn list_claims(&self, run_id: &str) -> Result<Vec<ResearchClaim>, StoreError>;
}
/// Canonical integration, pack, resource, and publisher-trust repository.
pub trait ExtensionRepository: AggregateRepository {
    /// Reconstruct one integration connection.
    fn get_integration(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError>;

    /// List integration connections in deterministic name order.
    fn list_integrations(&self, limit: usize) -> Result<Vec<IntegrationConnection>, StoreError>;

    /// Append a validated next connection state using optimistic concurrency.
    fn save_integration(
        &self,
        connection: IntegrationConnection,
        actor: Actor,
    ) -> Result<IntegrationConnection, StoreError>;

    /// Append an explicit disconnection event without deleting history.
    fn disconnect_integration(
        &self,
        name: &str,
        actor: Actor,
        updated_at: &str,
    ) -> Result<IntegrationConnection, StoreError>;

    /// Reconstruct one installed pack lifecycle.
    fn get_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError>;

    /// List installed and historically uninstalled packs in deterministic name order.
    fn list_packs(&self, limit: usize) -> Result<Vec<PackInstallation>, StoreError>;

    /// Append one fully verified pack installation event.
    fn install_pack(
        &self,
        installation: PackInstallation,
        actor: Actor,
    ) -> Result<PackInstallation, StoreError>;

    /// Append an enable, disable, or uninstall lifecycle transition.
    fn set_pack_status(
        &self,
        name: &str,
        status: PackStatus,
        actor: Actor,
        updated_at: &str,
    ) -> Result<PackInstallation, StoreError>;

    /// Persist a publisher/key binding as an explicit trust decision.
    fn add_publisher_trust(
        &self,
        trust: PublisherTrust,
        actor: Actor,
    ) -> Result<PublisherTrust, StoreError>;

    /// Resolve one exact publisher/key binding.
    fn get_publisher_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError>;

    /// List bounded publisher/key bindings in deterministic order.
    fn list_publisher_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, StoreError>;
}

/// Deterministic discovery for declarative data-only skills.
pub trait SkillRepository: Send + Sync {
    /// List selected skills in deterministic name order.
    fn list_skills(&self) -> Result<Vec<SkillRecord>, StoreError>;

    /// Load one selected skill.
    fn get_skill(&self, name: &str) -> Result<Option<SkillRecord>, StoreError>;

    /// Report every duplicate and the configured winner.
    fn duplicate_names(&self) -> Result<Vec<SkillDuplicate>, StoreError>;
}

/// Workflow definitions and run projections.
pub trait WorkflowRepository: Send + Sync {
    /// Persist a definition and immutable hash/provenance.
    fn register(
        &self,
        definition: &WorkflowDefinition,
        content_hash: &str,
        provenance: &str,
    ) -> Result<(), StoreError>;

    /// Load an exact definition version.
    fn definition(
        &self,
        name: &str,
        version: &str,
    ) -> Result<Option<(WorkflowDefinition, String)>, StoreError>;

    /// Load a run projection.
    fn run(&self, run_id: &str) -> Result<Option<WorkflowRun>, StoreError>;

    /// List bounded run projections.
    fn runs(&self, limit: usize) -> Result<Vec<WorkflowRun>, StoreError>;
}

/// Disposable search projection for canonical memory identifiers.
#[async_trait]
pub trait MemoryIndex: Send + Sync {
    /// Last durably applied global journal sequence.
    fn position(&self) -> Result<u64, StoreError>;

    /// Persist the last fully applied global journal sequence.
    async fn set_position(&self, position: u64) -> Result<(), StoreError>;

    /// Idempotently add/update an indexed record using the source event id.
    async fn upsert(
        &self,
        event_id: &str,
        memory_id: &str,
        text: &str,
        metadata: &Value,
        embedding: Option<&[f32]>,
    ) -> Result<(), StoreError>;

    /// Idempotently remove an indexed record.
    async fn remove(&self, event_id: &str, memory_id: &str) -> Result<(), StoreError>;

    /// Return candidate identifiers and scores only.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError>;

    /// Return bounded readiness and lag metadata.
    async fn status(&self) -> Result<Value, StoreError>;

    /// Rebuild from canonical records supplied by the caller.
    async fn rebuild(&self, records: &[(String, String, Value)]) -> Result<(), StoreError>;
}

/// Policy-aware relevant-memory retrieval used by context composition.
#[async_trait]
pub trait MemoryRetriever: Send + Sync {
    /// Return canonical active records authorized for this session and query.
    async fn relevant(
        &self,
        query: &str,
        session_id: &str,
        context: ExecutionContext,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;
}

/// Provider for caller-generated embedding vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed bounded input text.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StoreError>;
}

/// Bounded redacted audit export sink.
pub trait AuditExporter: Send + Sync {
    /// Export one immutable envelope; payload disclosure is caller-controlled.
    fn export(&self, event: &EventEnvelope) -> Result<(), StoreError>;
}

/// Policy-decision failures always fail closed at the gateway.
#[derive(Debug, Error)]
pub enum PolicyError {
    /// Input exceeded the configured disclosure limit.
    #[error("policy input exceeds {limit} bytes")]
    InputTooLarge {
        /// Configured byte limit.
        limit: usize,
    },
    /// Transport, readiness, or timeout failure.
    #[error("policy unavailable: {0}")]
    Unavailable(String),
    /// Response failed the strict contract.
    #[error("invalid policy response: {0}")]
    InvalidDecision(String),
}

/// Built-in or OPA policy decision point.
#[async_trait]
pub trait PolicyDecisionPoint: Send + Sync {
    /// Evaluate a fully redacted logical request.
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError>;

    /// Report current readiness and bounded revision metadata.
    async fn doctor(&self) -> Result<Value, PolicyError>;
}

/// Interactive or application-supplied approval handler.
#[async_trait]
pub trait ApprovalProvider: Send + Sync {
    /// Request a proof bound to the canonical request hash and initial decision.
    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError>;
}
