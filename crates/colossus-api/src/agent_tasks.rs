//! Protocol-neutral, caller-owned text task submission and released history.
use crate::RunStatus;
use serde::{Deserialize, Serialize};

/// One globally idempotent input beneath an operator-selected execution profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitAgentTaskMessageRequest {
    /// Sender-selected identity, scoped by authenticated application ownership.
    pub message_id: String,
    /// Existing live task, absent for new work.
    pub task_id: Option<String>,
    /// Existing context, absent when allocating a new conversation.
    pub context_id: Option<String>,
    /// Bounded peer text; no approval response or executable attachment.
    pub text: String,
    /// Operator-selected role within the caller's existing grant.
    pub role: String,
    /// Operator-selected initial turn limit; follow-ups never extend it.
    pub max_turns: u32,
}

/// Released protocol input history retained in the runtime journal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskInput {
    /// Original peer message identity.
    pub message_id: String,
    /// Fixed public execution.
    pub task_id: String,
    /// Fixed public conversation.
    pub context_id: String,
    /// Policy-released peer input.
    pub text: String,
    /// Canonical admission time.
    pub accepted_at: String,
    /// Durable inbox message for follow-ups; initial work uses create-run admission.
    pub inbox_message_id: Option<String>,
}

/// Read a task without disclosing private session or tool transcripts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAgentTaskRequest {
    /// Caller-owned public execution.
    pub task_id: String,
    /// Most recent released peer inputs, from zero through sixteen.
    pub history_length: u32,
}

/// Curated task state; authority remains in the runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskSnapshot {
    /// Fixed public execution identity.
    pub task_id: String,
    /// Fixed public conversation identity.
    pub context_id: String,
    /// Current canonical execution state.
    pub status: RunStatus,
    /// Last lifecycle update time, independent of output/tool activity.
    pub status_updated_at: String,
    /// Durable released run cursor.
    pub last_sequence: u64,
    /// Released complete final output; intermediate transcripts are excluded.
    pub output: Option<String>,
    /// Safe terminal failure evidence, including uncertainty.
    pub failure: Option<AgentTaskFailure>,
    /// Current prompt or approval category, without an interaction response grant.
    pub waiting_kind: Option<String>,
    /// Bounded released input history.
    pub history: Vec<AgentTaskInput>,
}

/// Stable snapshot query ordered by descending last lifecycle update time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAgentTasksRequest {
    /// Optional fixed public conversation filter.
    pub context_id: Option<String>,
    /// Optional canonical lifecycle filter.
    pub statuses: Vec<RunStatus>,
    /// Inclusive UTC RFC3339 last-status-update lower bound.
    pub status_updated_after: Option<String>,
    /// From one through twenty results.
    pub page_size: u32,
    /// Owner/filter-bound canonical snapshot cursor.
    pub page_token: Option<String>,
    /// Include complete released final output when true.
    pub include_output: bool,
    /// Recent released peer inputs per task, from zero through sixteen.
    pub history_length: u32,
}

/// Stable page with no private artifacts or transcript data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAgentTasksResponse {
    /// Descending last-status-update order, then execution identity.
    pub tasks: Vec<AgentTaskSnapshot>,
    /// None on the final page.
    pub next_page_token: Option<String>,
    /// Matching task count before pagination at the canonical snapshot.
    pub total_size: u32,
}

/// Safe failure evidence without provider or effect diagnostics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskFailure {
    /// Stable released category.
    pub code: String,
    /// Safe released detail.
    pub message: String,
    /// Whether external work may have happened despite an uncertain terminal state.
    pub outcome_unknown: bool,
}
