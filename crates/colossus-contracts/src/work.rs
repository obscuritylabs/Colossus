use super::*;

/// Durable task lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Accepted but not started.
    Pending,
    /// Currently being worked.
    InProgress,
    /// Finished successfully.
    Completed,
    /// Cannot progress without an external change or input.
    Blocked,
    /// Explicitly abandoned.
    Cancelled,
}

/// Canonical session-scoped task state reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    /// Stable task identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Bounded human-readable title.
    pub title: String,
    /// Bounded supporting detail.
    pub description: String,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// Durable plan lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Proposed work that may still be edited or discarded.
    Draft,
    /// Explicitly approved for one execution or goal handoff.
    Approved,
    /// Consumed by an execution run.
    Executed,
    /// Retained for audit but intentionally abandoned.
    Discarded,
}

/// One ordered, bounded plan step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanStep {
    /// One-based stable order within the plan.
    pub index: u32,
    /// Short human-readable action label.
    pub title: String,
    /// Supporting implementation or verification detail.
    pub detail: String,
    /// Whether executing this step may mutate external state.
    pub requires_mutation: bool,
}

/// Canonical session-scoped plan reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRecord {
    /// Stable plan identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Original objective that produced the plan.
    pub prompt: String,
    /// Current lifecycle status.
    pub status: PlanStatus,
    /// Optional bounded Markdown overview.
    pub content: String,
    /// Ordered executable intent without inline code semantics.
    pub steps: Vec<PlanStep>,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// Approval timestamp when approved.
    pub approved_at: Option<String>,
    /// Run that consumed the approved plan.
    pub executed_run_id: Option<String>,
}

/// Durable bounded-autonomy goal status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Further bounded iterations may run.
    Active,
    /// Objective was genuinely achieved.
    Complete,
    /// Progress requires user input or an external state change.
    Blocked,
}

/// Canonical goal state reconstructed from immutable events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRecord {
    /// Stable goal identifier.
    pub id: String,
    /// Owning session identifier.
    pub session_id: String,
    /// Bounded objective preserved across iterations.
    pub objective: String,
    /// Optional approved plan that originated this goal.
    pub source_plan_id: Option<String>,
    /// Current terminal or active state.
    pub status: GoalStatus,
    /// Concise completion or progress summary.
    pub summary: String,
    /// Required explanation when blocked.
    pub blocked_reason: String,
    /// Maximum autonomous iterations.
    pub iteration_budget: u16,
    /// Completed iterations, never greater than the budget.
    pub iterations_completed: u16,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One completed bounded Goal Mode iteration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalIterationResult {
    /// One-based iteration number.
    pub iteration: u16,
    /// Normal agent run identifier.
    pub run_id: String,
    /// Visible final output for this iteration.
    pub output: String,
    /// Durable run event count.
    pub event_count: u64,
    /// Iteration wall time.
    pub elapsed_seconds: f64,
}

/// Result of a bounded Goal Mode loop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRunResult {
    /// Final reconstructed goal state.
    pub goal: GoalRecord,
    /// Completed normal agent runs.
    pub iterations: Vec<GoalIterationResult>,
    /// True when the budget ended while the goal remained active.
    pub iteration_budget_exhausted: bool,
    /// Total loop wall time.
    pub elapsed_seconds: f64,
}

/// Durable subagent job status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    /// Waiting for scheduler capacity.
    Queued,
    /// Child agent run is in progress.
    Running,
    /// Child run returned a released final result.
    Completed,
    /// Child run failed with a bounded redacted error.
    Failed,
    /// Operator cancelled the job.
    Cancelled,
    /// Process loss left a previously running job unfinished.
    Interrupted,
}

/// Canonical durable child-agent job.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentJob {
    /// Stable job identifier.
    pub id: String,
    /// Parent session identifier.
    pub session_id: String,
    /// Run that requested delegation.
    pub parent_run_id: String,
    /// Tool call that requested delegation.
    pub parent_call_id: String,
    /// Bounded child objective.
    pub task: String,
    /// Configured model role.
    pub role: String,
    /// Current lifecycle state.
    pub status: SubagentStatus,
    /// Isolated durable child session.
    pub child_session_id: String,
    /// Completed child run identifier.
    pub child_run_id: Option<String>,
    /// Bounded released child output.
    pub final_output: String,
    /// Bounded redacted terminal error.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC start timestamp.
    pub started_at: Option<String>,
    /// UTC terminal timestamp.
    pub completed_at: Option<String>,
}

/// Bounded scheduler status snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubagentQueueStatus {
    /// Total matching jobs.
    pub total: usize,
    /// Jobs awaiting capacity.
    pub queued: usize,
    /// Jobs currently executing.
    pub running: usize,
    /// Successfully completed jobs.
    pub completed: usize,
    /// Failed jobs.
    pub failed: usize,
    /// Cancelled jobs.
    pub cancelled: usize,
    /// Recovery-interrupted jobs.
    pub interrupted: usize,
    /// Configured scheduler ceiling.
    pub max_concurrent: usize,
    /// Currently available local slots.
    pub available_slots: usize,
}

/// Bounded session-scoped work state for interactive refresh and application clients.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkStateSnapshot {
    /// Exact owning session.
    pub session_id: String,
    /// All bounded task records, including terminal history.
    pub tasks: Vec<TaskRecord>,
    /// Number of tasks not completed or cancelled.
    pub open_task_count: usize,
    /// Binding active decisions.
    pub active_decisions: Vec<KeyDecision>,
    /// Draft or approved plans that remain actionable.
    pub actionable_plans: Vec<PlanRecord>,
    /// Active or blocked goals that remain relevant.
    pub current_goals: Vec<GoalRecord>,
    /// Queued, running, or interrupted child jobs.
    pub current_subagents: Vec<SubagentJob>,
}
