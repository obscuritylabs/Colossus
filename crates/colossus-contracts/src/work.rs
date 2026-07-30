use super::*;

/// Application-level behavior for one agent run.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "target",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentRunMode {
    /// Execute with the caller's normal tool and effect ceiling.
    #[default]
    Execute,
    /// Produce or refine one durable draft plan without execution authority.
    Plan(PlanDraftTarget),
}

/// Trusted durable write target for one Plan Mode run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanDraftTarget {
    /// Create one new draft plan.
    Create,
    /// Replace one exact optimistic revision of an existing draft plan.
    Update {
        /// Stable plan identifier, supplied by the trusted caller rather than the model.
        plan_id: String,
        /// Expected canonical plan revision.
        revision: u64,
    },
}

/// Operator-selected handoff for one approved plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanExecutionStrategy {
    /// Consume the plan in one ordinary agent run.
    Direct,
    /// Consume the plan into bounded Goal Mode.
    Goal {
        /// Maximum autonomous Goal Mode iterations.
        max_iterations: u16,
    },
}

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
    /// Optimistic lifecycle revision. Legacy records deserialize as revision zero.
    #[serde(default)]
    pub revision: u64,
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
    /// Consumed iteration slots, including started runs that failed or were cancelled.
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

/// Terminal state of one controlled ordinary agent execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlledAgentTerminal {
    /// The agent returned a normal released result.
    Completed {
        /// Durable completed run result.
        result: AgentRunResult,
    },
    /// The operator cooperatively cancelled the run.
    Cancelled {
        /// Durable cancellation evidence.
        result: AgentRunCancellation,
    },
    /// The consumed run failed after its identity became durable.
    Failed {
        /// Durable run identity allocated before Plan consumption.
        run_id: String,
        /// Bounded policy-released failure message.
        message: String,
    },
}

/// Terminal state of one controlled Goal Mode invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalRunOutcome {
    /// The invocation stopped because the goal became terminal or exhausted its budget.
    Completed {
        /// Current goal and iterations completed by this invocation.
        result: GoalRunResult,
    },
    /// The operator stopped at a safe boundary; the active goal remains resumable.
    Cancelled {
        /// Current goal and iterations completed before cancellation.
        result: GoalRunResult,
        /// Cancellation evidence when a concrete agent iteration had started.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancellation: Option<Box<AgentRunCancellation>>,
    },
    /// An iteration failed after the goal became durable; the active goal remains resumable.
    Failed {
        /// Current goal and iterations completed before failure.
        result: GoalRunResult,
        /// Durable run identity allocated for the failed iteration, when it reached run setup.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        /// Bounded policy-released failure message.
        message: String,
    },
}

/// Result of one controlled approved-Plan handoff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "execution", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanExecutionOutcome {
    /// Cancellation won before the approved Plan was consumed.
    CancelledBeforeStart {
        /// Canonical still-approved Plan record.
        plan: PlanRecord,
    },
    /// The Plan was consumed by one ordinary bounded agent run.
    Direct {
        /// Canonical executed Plan record.
        plan: PlanRecord,
        /// Terminal state of the consuming run.
        terminal: ControlledAgentTerminal,
    },
    /// The Plan was atomically consumed into one bounded Goal.
    Goal {
        /// Canonical executed Plan record linked to the Goal id.
        plan: PlanRecord,
        /// Terminal state of this Goal Mode invocation.
        terminal: GoalRunOutcome,
    },
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
    /// Exact inherited model-visible tool ceiling.
    ///
    /// `None` is retained for trusted terminal-created and preview-era jobs. Public
    /// application delegation always stores `Some`, where an empty list denies every
    /// tool. Child runs additionally remove nested delegation.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
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
