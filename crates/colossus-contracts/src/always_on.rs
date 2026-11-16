//! Released state for local, owner-bound scheduled chat wakes.

use serde::{Deserialize, Serialize};

/// One workspace's opt-in standing chat and its bounded scheduling state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlwaysOnProfile {
    /// Conversation continued by each wake.
    pub session_id: String,
    /// Whether future manual and scheduled wakes may be claimed.
    pub enabled: bool,
    /// Exact journal revision for an operator mutation.
    pub revision: u64,
    /// Next UTC RFC3339 scheduled boundary.
    pub next_wake_at: String,
    /// The single queued or claimed occurrence, when unresolved.
    pub pending: Option<AlwaysOnWake>,
    /// Most recent settled occurrence.
    pub last_wake: Option<AlwaysOnWakeResult>,
}

/// Bounded released identity of a single wake.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlwaysOnWake {
    /// Stable identity and idempotency key for this occurrence.
    pub id: String,
    /// UTC RFC3339 request time or scheduled boundary.
    pub requested_at: String,
    /// Human request or elapsed schedule.
    pub trigger: AlwaysOnTrigger,
}

impl AlwaysOnWake {
    /// Key of the same wake's managed application run allocation.
    pub fn idempotency_key(&self) -> String {
        format!("always-on-{}", self.id)
    }
}

/// Origin of a standing chat wake.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlwaysOnTrigger {
    /// The worker observed a due interval.
    Scheduled,
    /// The local owner explicitly requested one wake.
    Manual,
}

/// Durable terminal result without private input or provider diagnostic details.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlwaysOnWakeResult {
    /// Identity of the settled occurrence.
    pub id: String,
    /// UTC RFC3339 settlement timestamp.
    pub completed_at: String,
    /// A bounded terminal disposition.
    pub status: AlwaysOnWakeStatus,
}

/// Outcome of one background or manually requested chat wake.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlwaysOnWakeStatus {
    /// The run reached a confirmed successful result.
    Completed,
    /// The run reached a confirmed failure.
    Failed,
    /// The wake was interrupted and will not be retried automatically.
    Interrupted,
    /// External execution could have started. Future wakes pause until review.
    OutcomeUnknown,
    /// The session or authority was no longer eligible.
    Blocked,
}
