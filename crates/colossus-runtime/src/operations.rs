use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PresentationOperation {
    Save { preferences: TerminalPreferences },
    AppendHistory { entry: String },
}

impl PresentationOperation {
    pub(super) const fn action(&self) -> &'static str {
        match self {
            Self::Save { .. } => "presentation.preferences.update",
            Self::AppendHistory { .. } => "presentation.history.append",
        }
    }

    pub(super) const fn resource(&self) -> &'static str {
        match self {
            Self::Save { .. } => "presentation:repl",
            Self::AppendHistory { .. } => "presentation:history",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GoalCreationResult {
    pub(super) goal: GoalRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) consumed_plan: Option<PlanRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WorkOperation {
    TaskCreate {
        session_id: String,
        title: String,
        description: String,
        status: TaskStatus,
    },
    TaskUpdate {
        id: String,
        title: Option<String>,
        description: Option<String>,
        status: Option<TaskStatus>,
    },
    TaskList {
        session_id: String,
        status: Option<TaskStatus>,
        limit: usize,
    },
    DecisionCreate {
        session_id: String,
        title: String,
        decision: String,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: String,
        applies_when: String,
        rationale: String,
        source_excerpt: String,
    },
    DecisionUpdate {
        id: String,
        title: Option<String>,
        decision: Option<String>,
        priority: Option<DecisionPriority>,
        intent: Option<String>,
        applies_when: Option<String>,
        rationale: Option<String>,
        source_excerpt: Option<String>,
    },
    DecisionArchive {
        id: String,
    },
    DecisionSupersede {
        id: String,
        title: String,
        decision: String,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: String,
        applies_when: String,
        rationale: String,
        source_excerpt: String,
    },
    DecisionList {
        session_id: String,
        status: Option<DecisionStatus>,
        limit: usize,
    },
    PlanCreate {
        session_id: String,
        prompt: String,
        content: String,
        steps: Vec<PlanStep>,
    },
    PlanUpdate {
        id: String,
        expected_revision: u64,
        content: String,
        steps: Vec<PlanStep>,
    },
    PlanShow {
        id: String,
    },
    PlanApprove {
        id: String,
    },
    PlanApproveAtRevision {
        id: String,
        expected_revision: u64,
    },
    PlanDiscard {
        id: String,
        expected_revision: Option<u64>,
    },
    PlanExecute {
        id: String,
        run_id: String,
    },
    PlanExecuteAtRevision {
        id: String,
        expected_revision: u64,
        run_id: String,
    },
    GoalCreate {
        session_id: String,
        objective: String,
        iteration_budget: u16,
        source_plan_id: Option<String>,
        source_plan_revision: Option<u64>,
    },
    GoalShow {
        id: String,
    },
    GoalUpdate {
        id: String,
        status: GoalStatus,
        summary: String,
        blocked_reason: String,
    },
    GoalIteration {
        id: String,
    },
    SubagentCreate {
        session_id: String,
        parent_run_id: String,
        parent_call_id: String,
        task: String,
        role: String,
        allowed_tools: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instruction_snapshot_id: Option<String>,
    },
    SubagentRead {
        id: String,
    },
    SubagentList {
        session_id: String,
        status: Option<SubagentStatus>,
        limit: usize,
    },
    SubagentStart {
        id: String,
    },
    SubagentComplete {
        id: String,
        child_run_id: String,
        output: String,
    },
    SubagentStop {
        id: String,
        status: SubagentStatus,
        error: String,
    },
    SubagentRequeue {
        id: String,
    },
}

impl WorkOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::TaskCreate { .. } => "task.create",
            Self::TaskUpdate { .. } => "task.update",
            Self::TaskList { .. } => "task.list",
            Self::DecisionCreate { .. } => "decision.create",
            Self::DecisionUpdate { .. } => "decision.update",
            Self::DecisionArchive { .. } => "decision.archive",
            Self::DecisionSupersede { .. } => "decision.supersede",
            Self::DecisionList { .. } => "decision.list",
            Self::PlanCreate { .. } => "plan.create",
            Self::PlanUpdate { .. } => "plan.update",
            Self::PlanShow { .. } => "plan.show",
            Self::PlanApprove { .. } | Self::PlanApproveAtRevision { .. } => "plan.approve_request",
            Self::PlanDiscard { .. } => "plan.discard",
            Self::PlanExecute { .. } | Self::PlanExecuteAtRevision { .. } => "plan.execute",
            Self::GoalCreate {
                source_plan_id: Some(_),
                ..
            } => "plan.execute",
            Self::GoalCreate { .. } => "goal.create",
            Self::GoalShow { .. } => "goal.show",
            Self::GoalUpdate { .. } => "goal.update",
            Self::GoalIteration { .. } => "goal.iteration.record",
            Self::SubagentCreate { .. } => "subagent.create",
            Self::SubagentRead { .. } => "subagent.read",
            Self::SubagentList { .. } => "subagent.list",
            Self::SubagentStart { .. } => "subagent.start",
            Self::SubagentComplete { .. } => "subagent.complete",
            Self::SubagentStop { status, .. } => match status {
                SubagentStatus::Cancelled => "subagent.cancel",
                SubagentStatus::Interrupted => "subagent.interrupt",
                _ => "subagent.fail",
            },
            Self::SubagentRequeue { .. } => "subagent.requeue",
        }
    }

    pub(super) fn resource(&self) -> &str {
        match self {
            Self::TaskCreate { session_id, .. }
            | Self::TaskList { session_id, .. }
            | Self::DecisionCreate { session_id, .. }
            | Self::DecisionList { session_id, .. }
            | Self::PlanCreate { session_id, .. }
            | Self::GoalCreate {
                session_id,
                source_plan_id: None,
                ..
            }
            | Self::SubagentCreate { session_id, .. }
            | Self::SubagentList { session_id, .. } => session_id,
            Self::GoalCreate {
                source_plan_id: Some(plan_id),
                ..
            } => plan_id,
            Self::TaskUpdate { id, .. }
            | Self::DecisionUpdate { id, .. }
            | Self::DecisionArchive { id }
            | Self::DecisionSupersede { id, .. }
            | Self::PlanUpdate { id, .. }
            | Self::PlanShow { id }
            | Self::PlanApprove { id }
            | Self::PlanApproveAtRevision { id, .. }
            | Self::PlanDiscard { id, .. }
            | Self::PlanExecute { id, .. }
            | Self::PlanExecuteAtRevision { id, .. }
            | Self::GoalShow { id }
            | Self::GoalUpdate { id, .. }
            | Self::GoalIteration { id }
            | Self::SubagentRead { id }
            | Self::SubagentStart { id }
            | Self::SubagentComplete { id, .. }
            | Self::SubagentStop { id, .. }
            | Self::SubagentRequeue { id } => id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkOperation;

    #[test]
    fn approved_plan_goal_handoff_uses_execution_authority_and_plan_resource() {
        let handoff = WorkOperation::GoalCreate {
            session_id: "session-1".into(),
            objective: "Execute the plan".into(),
            iteration_budget: 5,
            source_plan_id: Some("plan-1".into()),
            source_plan_revision: Some(3),
        };
        assert_eq!(handoff.action(), "plan.execute");
        assert_eq!(handoff.resource(), "plan-1");

        let standalone = WorkOperation::GoalCreate {
            session_id: "session-1".into(),
            objective: "Independent goal".into(),
            iteration_budget: 5,
            source_plan_id: None,
            source_plan_revision: None,
        };
        assert_eq!(standalone.action(), "goal.create");
        assert_eq!(standalone.resource(), "session-1");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MemoryOperation {
    Create {
        scope: MemoryScope,
        kind: String,
        confidence: f32,
        text: String,
        rationale: String,
        expires_at: Option<String>,
    },
    Update {
        id: String,
        text: Option<String>,
        rationale: Option<String>,
        confidence: Option<f32>,
    },
    Archive {
        id: String,
    },
    Supersede {
        id: String,
        text: String,
        rationale: String,
    },
    Read {
        id: String,
    },
    List {
        status: Option<MemoryStatus>,
        limit: usize,
        session_id: Option<String>,
        repository_id: Option<String>,
    },
    Search {
        query: String,
        session_id: Option<String>,
        repository_id: Option<String>,
        limit: usize,
    },
    IndexStatus,
    IndexSync,
    IndexRebuild,
}

impl MemoryOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Create { .. } => "memory.create",
            Self::Update { .. } => "memory.update",
            Self::Archive { .. } => "memory.archive",
            Self::Supersede { .. } => "memory.supersede",
            Self::Read { .. } => "memory.read",
            Self::List { .. } => "memory.list",
            Self::Search { .. } => "memory.search",
            Self::IndexStatus => "memory.index.status",
            Self::IndexSync => "memory.index.sync",
            Self::IndexRebuild => "memory.index.rebuild",
        }
    }

    pub(super) fn resource(&self) -> String {
        match self {
            Self::Create { scope, .. } => format!("memory-scope:{scope:?}"),
            Self::Update { id, .. }
            | Self::Archive { id }
            | Self::Supersede { id, .. }
            | Self::Read { id } => id.clone(),
            Self::List { .. } => "memory:*".into(),
            Self::Search { session_id, .. } => session_id
                .as_ref()
                .map_or_else(|| "memory:search".into(), |id| format!("session:{id}")),
            Self::IndexStatus | Self::IndexSync | Self::IndexRebuild => "memory-index".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ResearchOperation {
    Run {
        session_id: String,
        question: String,
        depth: ResearchDepth,
        source_kinds: Vec<ResearchSourceKind>,
    },
}

impl ResearchOperation {
    pub(super) fn action(&self) -> &'static str {
        "research.run"
    }

    pub(super) fn session_id(&self) -> &str {
        match self {
            Self::Run { session_id, .. } => session_id,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SkillOperation {
    Scaffold {
        name: String,
        description: String,
        instructions: String,
        resource_dirs: Vec<String>,
    },
    Inspect {
        name: String,
    },
    ReadFile {
        name: String,
        path: String,
    },
    WriteFile {
        name: String,
        path: String,
        content: String,
        expected_sha256: Option<String>,
    },
    ValidateInstalled {
        name: String,
    },
    ValidateLocal {
        path: String,
    },
    InstallLocal {
        path: String,
    },
    ListResources {
        skill_name: String,
        active_skills: Vec<String>,
    },
    ReadResource {
        skill_name: String,
        path: String,
        active_skills: Vec<String>,
    },
}

impl SkillOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Scaffold { .. } => "skill.scaffold",
            Self::Inspect { .. } => "skill.inspect",
            Self::ReadFile { .. } => "skill.read",
            Self::WriteFile { .. } => "skill.write",
            Self::ValidateInstalled { .. } | Self::ValidateLocal { .. } => "skill.validate",
            Self::InstallLocal { .. } => "skill.install",
            Self::ListResources { .. } => "skill.resource.list",
            Self::ReadResource { .. } => "skill.resource.read",
        }
    }

    pub(super) fn resource(&self) -> String {
        match self {
            Self::Scaffold { name, .. }
            | Self::Inspect { name }
            | Self::ReadFile { name, .. }
            | Self::WriteFile { name, .. }
            | Self::ValidateInstalled { name }
            | Self::ListResources {
                skill_name: name, ..
            }
            | Self::ReadResource {
                skill_name: name, ..
            } => format!("skill:{name}"),
            Self::ValidateLocal { path } | Self::InstallLocal { path } => {
                format!("workspace-skill:{path}")
            }
        }
    }
}
