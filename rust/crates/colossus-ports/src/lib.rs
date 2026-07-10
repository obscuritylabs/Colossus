//! Replaceable runtime ports. Adapters depend on these contracts, never the reverse.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ApprovalProof, EffectRequest, EventEnvelope, MemoryRecord, NewEvent, PolicyDecision,
    SignedCheckpoint, WorkflowDefinition, WorkflowRun,
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

    /// Decrypt an event payload after policy has authorized disclosure.
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError>;

    /// Verify encryption, hashes, sequence, secure anchor, and checkpoints.
    fn verify(&self) -> Result<VerificationReport, StoreError>;

    /// Return whether writes are blocked due to failed verification.
    fn is_recovery_mode(&self) -> bool;

    /// Create a signed checkpoint at the current chain head.
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError>;
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

    /// Atomically apply a projection update at a sequence.
    fn apply(&self, projection: &str, sequence: u64, value: Value) -> Result<(), StoreError>;

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

/// Session aggregate repository.
pub trait SessionRepository: AggregateRepository {}
/// Task, decision, plan, and goal repository.
pub trait WorkRepository: AggregateRepository {}
/// Canonical event-sourced memory lifecycle repository.
pub trait MemoryRepository: Send + Sync {
    /// Create a new active canonical record.
    fn create(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError>;

    /// Load one reconstructed canonical record.
    fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>;

    /// List bounded active canonical records before policy filtering.
    fn list_active(&self, limit: usize) -> Result<Vec<MemoryRecord>, StoreError>;

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
/// Research source, claim, and report repository.
pub trait ResearchRepository: AggregateRepository {}
/// Skills, resources, packs, and trust repository.
pub trait ExtensionRepository: AggregateRepository {}

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
