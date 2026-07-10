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
