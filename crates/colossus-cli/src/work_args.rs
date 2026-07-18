use super::*;

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum TaskStatusArg {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

impl From<TaskStatusArg> for TaskStatus {
    fn from(value: TaskStatusArg) -> Self {
        match value {
            TaskStatusArg::Pending => Self::Pending,
            TaskStatusArg::InProgress => Self::InProgress,
            TaskStatusArg::Completed => Self::Completed,
            TaskStatusArg::Blocked => Self::Blocked,
            TaskStatusArg::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Args)]
pub(super) struct TasksCommand {
    #[command(subcommand)]
    pub(super) command: TasksAction,
}

#[derive(Subcommand)]
pub(super) enum TasksAction {
    /// List bounded canonical tasks.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        status: Option<TaskStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact task.
    Show { task_id: String },
    /// Create a session-scoped task.
    Create {
        session_id: String,
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, value_enum, default_value = "pending")]
        status: TaskStatusArg,
    },
    /// Update supplied fields on one task.
    Update {
        task_id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<TaskStatusArg>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum DecisionPriorityArg {
    Critical,
    High,
    Normal,
}

impl From<DecisionPriorityArg> for DecisionPriority {
    fn from(value: DecisionPriorityArg) -> Self {
        match value {
            DecisionPriorityArg::Critical => Self::Critical,
            DecisionPriorityArg::High => Self::High,
            DecisionPriorityArg::Normal => Self::Normal,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum DecisionStatusArg {
    Active,
    Archived,
    Superseded,
}

impl From<DecisionStatusArg> for DecisionStatus {
    fn from(value: DecisionStatusArg) -> Self {
        match value {
            DecisionStatusArg::Active => Self::Active,
            DecisionStatusArg::Archived => Self::Archived,
            DecisionStatusArg::Superseded => Self::Superseded,
        }
    }
}

#[derive(Args)]
pub(super) struct DecisionsCommand {
    #[command(subcommand)]
    pub(super) command: DecisionsAction,
}

#[derive(Subcommand)]
pub(super) enum DecisionsAction {
    /// List bounded canonical decisions.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum, default_value = "active")]
        status: DecisionStatusArg,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact decision.
    Show { decision_id: String },
    /// Create one active future-facing commitment.
    Create {
        session_id: String,
        title: String,
        decision: String,
        #[arg(long, value_enum, default_value = "normal")]
        priority: DecisionPriorityArg,
        #[arg(long, default_value = "")]
        intent: String,
        #[arg(long, default_value = "")]
        applies_when: String,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long, default_value = "")]
        source_excerpt: String,
    },
    /// Update mutable content on an active decision.
    Update {
        decision_id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        decision: Option<String>,
        #[arg(long)]
        priority: Option<DecisionPriorityArg>,
        #[arg(long)]
        intent: Option<String>,
        #[arg(long)]
        applies_when: Option<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        source_excerpt: Option<String>,
    },
    /// Archive an active decision without deleting it.
    Archive { decision_id: String },
    /// Atomically replace an active decision and preserve lineage.
    Supersede {
        decision_id: String,
        title: String,
        decision: String,
        #[arg(long, value_enum, default_value = "normal")]
        priority: DecisionPriorityArg,
        #[arg(long, default_value = "")]
        intent: String,
        #[arg(long, default_value = "")]
        applies_when: String,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long, default_value = "")]
        source_excerpt: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum PlanStatusArg {
    Draft,
    Approved,
    Executed,
    Discarded,
}

impl From<PlanStatusArg> for PlanStatus {
    fn from(value: PlanStatusArg) -> Self {
        match value {
            PlanStatusArg::Draft => Self::Draft,
            PlanStatusArg::Approved => Self::Approved,
            PlanStatusArg::Executed => Self::Executed,
            PlanStatusArg::Discarded => Self::Discarded,
        }
    }
}

#[derive(Args)]
pub(super) struct PlansCommand {
    #[command(subcommand)]
    pub(super) command: PlansAction,
}

#[derive(Subcommand)]
pub(super) enum PlansAction {
    /// List bounded canonical plans.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<PlanStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact plan.
    Show { plan_id: String },
    /// Create a draft plan with ordered title-only steps.
    Create {
        session_id: String,
        prompt: String,
        #[arg(long, default_value = "")]
        content: String,
        #[arg(long = "step", required = true)]
        steps: Vec<String>,
    },
    /// Request operator approval for one draft plan.
    Approve { plan_id: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum GoalStatusArg {
    Active,
    Complete,
    Blocked,
}

impl From<GoalStatusArg> for GoalStatus {
    fn from(value: GoalStatusArg) -> Self {
        match value {
            GoalStatusArg::Active => Self::Active,
            GoalStatusArg::Complete => Self::Complete,
            GoalStatusArg::Blocked => Self::Blocked,
        }
    }
}

#[derive(Args)]
pub(super) struct GoalsCommand {
    #[command(subcommand)]
    pub(super) command: GoalsAction,
}

#[derive(Subcommand)]
pub(super) enum GoalsAction {
    /// List bounded canonical goals.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<GoalStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact goal.
    Show { goal_id: String },
    /// Start a bounded Goal Mode loop in an existing session.
    Run {
        objective: String,
        #[arg(long)]
        session: String,
        #[arg(long, default_value = "primary")]
        role: String,
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u16).range(1..=50))]
        max_iterations: u16,
        #[arg(long)]
        source_plan: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum SubagentStatusArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl From<SubagentStatusArg> for SubagentStatus {
    fn from(value: SubagentStatusArg) -> Self {
        match value {
            SubagentStatusArg::Queued => Self::Queued,
            SubagentStatusArg::Running => Self::Running,
            SubagentStatusArg::Completed => Self::Completed,
            SubagentStatusArg::Failed => Self::Failed,
            SubagentStatusArg::Cancelled => Self::Cancelled,
            SubagentStatusArg::Interrupted => Self::Interrupted,
        }
    }
}

#[derive(Args)]
pub(super) struct AgentsCommand {
    #[command(subcommand)]
    pub(super) command: AgentsAction,
}

#[derive(Subcommand)]
pub(super) enum AgentsAction {
    /// Queue one durable child-agent job from the terminal.
    Queue {
        session_id: String,
        task: String,
        #[arg(long, default_value = "subagent_default")]
        role: String,
    },
    /// List bounded durable child-agent jobs.
    List {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, value_enum)]
        status: Option<SubagentStatusArg>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Show one exact child-agent job and bounded result.
    Show { job_id: String },
    /// Show queue counts and available scheduler slots.
    Status {
        #[arg(long)]
        session: Option<String>,
    },
    /// Execute queued jobs up to configured concurrency until empty.
    Drain,
    /// Cancel one queued or running job.
    Cancel { job_id: String },
    /// Requeue one failed, cancelled, or interrupted job.
    Requeue { job_id: String },
}
