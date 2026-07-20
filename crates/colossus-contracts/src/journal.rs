use super::*;

/// Serializable actor provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    /// A human operator.
    User,
    /// An authenticated external application or SDK client.
    Application,
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

/// Bounded model-assisted risk level. This is advisory input to policy, never authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// No material hazard was identified in the proposed effect.
    Low,
    /// The effect has meaningful consequences that warrant operator review.
    Medium,
    /// The effect can cause broad, destructive, or difficult-to-reverse consequences.
    High,
}

/// Strict recommendation returned by a model-assisted risk evaluator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskRecommendation {
    /// The evaluator found no reason to block the effect.
    Allow,
    /// The evaluator recommends that the effect not execute.
    Deny,
    /// The evaluator requires an explicit operator decision.
    RequireApproval,
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

/// Redacted immutable journal evidence suitable for external audit sinks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvidence {
    /// Evidence schema version.
    pub schema_version: u16,
    /// Source event schema version.
    pub event_version: u16,
    /// Source event identifier.
    pub event_id: String,
    /// Source global sequence.
    pub global_sequence: u64,
    /// Aggregate stream identifier.
    pub stream_id: String,
    /// Aggregate stream version.
    pub stream_version: u64,
    /// Security and product classification.
    pub classification: EventClassification,
    /// Versioned event name.
    pub event_type: String,
    /// Actor responsible for the source event.
    pub actor: Actor,
    /// Correlation metadata for the source event.
    pub context: ExecutionContext,
    /// Source UTC RFC3339 timestamp.
    pub occurred_at: String,
    /// Encryption key identifier, never key material.
    pub payload_key_id: String,
    /// Authenticated payload encryption algorithm.
    pub payload_algorithm: String,
    /// Hash of canonical plaintext payload bytes.
    pub payload_plaintext_hash: String,
    /// Previous journal record hash.
    pub previous_hash: String,
    /// Source journal record hash.
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

/// Durable retry and readiness state for one external-work consumer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalWorkRetryState {
    /// Stable versioned consumer identity.
    pub consumer: String,
    /// Global sequence whose processing failed.
    pub global_sequence: u64,
    /// Event identifier at that sequence, when it was available.
    pub event_id: Option<String>,
    /// Consecutive failures for this sequence.
    pub attempts: u32,
    /// Whether automatic retry is permitted.
    pub retryable: bool,
    /// UTC RFC3339 timestamp of the first consecutive failure.
    pub first_failed_at: String,
    /// UTC RFC3339 timestamp of the latest failure.
    pub last_failed_at: String,
    /// UTC RFC3339 time before which automatic retry is suppressed.
    pub next_retry_at: Option<String>,
    /// Stable bounded error category with no sensitive values.
    pub error_code: String,
    /// Bounded redacted diagnostic.
    pub error: String,
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

/// Strict model-assisted assessment returned to the effect gateway.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessment {
    /// Bounded advisory risk level.
    pub risk_level: RiskLevel,
    /// Advisory recommendation interpreted by the gateway and policy.
    pub recommended_decision: RiskRecommendation,
    /// Short human-readable explanation with no secret material.
    pub reason: String,
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
