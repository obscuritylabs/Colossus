//! Private provider state and safe references; never part of session messages.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Safe provenance bound into a permit before resolving opaque Responses state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderContinuationPlan {
    /// Canonical session owner.
    pub session_id: String,
    /// Hash of provider kind, endpoint, model, instructions and tool schemas.
    pub binding: String,
    /// Hash of the exact workspace decision and retrieved memory messages sent.
    pub context_binding_hash: String,
    /// Canonical prefix sent on this turn.
    pub source_count: usize,
    /// Hash of that canonical prefix.
    pub source_hash: String,
    /// Local snapshot epoch; a restore invalidates a continuation.
    pub snapshot_epoch: u64,
    /// Previously settled continuation selected by context preparation.
    pub selected: Option<ProviderContinuationView>,
}

/// Metadata sufficient for context preparation, with no opaque contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderContinuationView {
    /// Exact immutable state identifier.
    pub id: String,
    /// Canonical messages already represented by the state.
    pub covered_count: usize,
    /// Hash of the dynamic context bindings absorbed by this continuation.
    pub context_binding_hash: String,
    /// Conservative token reservation for the complete retained wire array.
    pub reserved_tokens: u64,
    /// Serialized retained wire-array bytes.
    pub bytes: usize,
}

/// Adapter-owned state. Store only in protected journals or bounded process memory.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderContinuation {
    /// Safe state metadata.
    pub view: ProviderContinuationView,
    /// Provenance of the request producing this state.
    pub plan: ProviderContinuationPlan,
    /// Hash of the canonical prefix after the assistant message was settled.
    pub settled_hash: String,
    /// Last normalized assistant message, for exact settlement and tool pairing.
    pub assistant: crate::ModelMessage,
    /// Exact wire items starting at the newest compaction item. Never display.
    pub hidden_reasoning: Vec<Value>,
}

impl std::fmt::Debug for ProviderContinuation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderContinuation")
            .field("view", &self.view)
            .field("hidden_reasoning", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
