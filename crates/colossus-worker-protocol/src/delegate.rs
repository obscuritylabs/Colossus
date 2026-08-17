use serde::{Deserialize, Serialize};

/// Released lifecycle state for one child-agent job.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDelegateStatus {
    /// Waiting for worker scheduler capacity.
    Queued,
    /// Child execution is in progress.
    Running,
    /// Child execution released a final result.
    Completed,
    /// Child execution released a terminal failure.
    Failed,
    /// The operator cancelled the child job.
    Cancelled,
    /// Process loss interrupted the child job.
    Interrupted,
}

/// Released lifecycle state for one bounded child tool activity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerDelegateActivityState {
    /// Permit-bound execution began.
    Started,
    /// A successful result was released.
    Completed,
    /// Execution was cancelled before it began.
    Cancelled,
    /// Execution reached a known failure.
    Failed,
}

/// One bounded tool activity released from a lineage-validated child run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerDelegateActivity {
    /// Provider call identifier.
    pub call_id: String,
    /// Registered model-visible tool name.
    pub tool_name: String,
    /// Released lifecycle state.
    pub state: WorkerDelegateActivityState,
    /// Bounded user-safe lifecycle summary.
    pub summary: String,
    /// Validated input released only after execution began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// Successful post-policy output preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// UTC start timestamp.
    pub started_at: String,
    /// UTC terminal timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Renderer-safe child-agent inspection returned only through native control.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerThreadDelegateInspection {
    /// Exact durable child-agent job identifier.
    pub job_id: String,
    /// Exact public parent run that requested delegation.
    pub parent_run_id: String,
    /// Isolated durable child session.
    pub child_session_id: String,
    /// Internal child execution identifier when one was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_run_id: Option<String>,
    /// Bounded delegated objective.
    pub task: String,
    /// Configured model role.
    pub role: String,
    /// Current child-agent lifecycle.
    pub status: WorkerDelegateStatus,
    /// Bounded released final output.
    pub final_output: String,
    /// Bounded released terminal error.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC start timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// UTC terminal timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Bounded released child tool activity in execution order.
    pub activities: Vec<WorkerDelegateActivity>,
}
