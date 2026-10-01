//! Public caller-bound managed shell operations.
pub use colossus_contracts::{
    Actor as ProcessOwner, ActorType as ProcessOwnerType, ProcessLifetime, ProcessOutputChunk,
    ProcessSessionPage, ProcessSessionSnapshot, ProcessSessionStatus, ProcessSessionSummary,
};
use serde::{Deserialize, Serialize};

/// Discover caller-owned shells within the selected workspace runtime.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListProcessSessionsRequest {
    /// Exclusive opaque session identifier from the previous page.
    pub after: Option<String>,
}
/// Read a bounded released-output page, optionally waiting for a change.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadProcessSessionRequest {
    /// Opaque process session identifier.
    pub session_id: String,
    /// Exclusive released-output sequence, zero initially.
    pub after_sequence: u64,
    /// Yield interval, from zero to thirty seconds. Never resets the deadline.
    pub wait_ms: u64,
    /// Output page ceiling, from 16 KiB to 64 KiB.
    pub max_output_bytes: u32,
}
/// Stop is idempotent for an exact caller-owned process identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopProcessSessionRequest {
    /// Opaque process session identifier, never an operating-system PID.
    pub session_id: String,
}
