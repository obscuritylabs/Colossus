use super::*;

fn validate_plan_lifecycle_selection(
    plan: &PlanRecord,
    expected_session_id: &str,
    expected_revision: u64,
) -> Result<(), RuntimeError> {
    if plan.session_id != expected_session_id || plan.revision != expected_revision {
        return Err(RuntimeError::Config(
            "Plan lifecycle action requires the selected same-session revision".into(),
        ));
    }
    Ok(())
}

impl Runtime {
    /// Current task, decision, plan, and goal snapshots.
    pub fn work_repository(&self) -> Arc<dyn WorkRepository> {
        Arc::clone(&self.work)
    }

    pub(super) async fn execute_work_operation(
        &self,
        mutation: WorkOperation,
    ) -> Result<Value, RuntimeError> {
        let action = mutation.action();
        let resource = mutation.resource().to_owned();
        let session_id = match &mutation {
            WorkOperation::TaskCreate { session_id, .. }
            | WorkOperation::TaskList { session_id, .. }
            | WorkOperation::DecisionCreate { session_id, .. }
            | WorkOperation::DecisionList { session_id, .. }
            | WorkOperation::PlanCreate { session_id, .. }
            | WorkOperation::GoalCreate { session_id, .. }
            | WorkOperation::SubagentCreate { session_id, .. }
            | WorkOperation::SubagentList { session_id, .. } => session_id.clone(),
            WorkOperation::TaskUpdate { id, .. } => {
                self.work
                    .get_task(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?
                    .session_id
            }
            WorkOperation::DecisionUpdate { id, .. }
            | WorkOperation::DecisionArchive { id }
            | WorkOperation::DecisionSupersede { id, .. } => {
                self.work
                    .get_decision(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?
                    .session_id
            }
            WorkOperation::PlanUpdate { id, .. }
            | WorkOperation::PlanShow { id }
            | WorkOperation::PlanApprove { id }
            | WorkOperation::PlanApproveAtRevision { id, .. }
            | WorkOperation::PlanDiscard { id, .. }
            | WorkOperation::PlanExecute { id, .. }
            | WorkOperation::PlanExecuteAtRevision { id, .. } => {
                self.work
                    .get_plan(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?
                    .session_id
            }
            WorkOperation::GoalShow { id }
            | WorkOperation::GoalUpdate { id, .. }
            | WorkOperation::GoalIteration { id } => {
                self.work
                    .get_goal(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("goal {id}")))?
                    .session_id
            }
            WorkOperation::SubagentRead { id }
            | WorkOperation::SubagentStart { id }
            | WorkOperation::SubagentComplete { id, .. }
            | WorkOperation::SubagentStop { id, .. }
            | WorkOperation::SubagentRequeue { id } => {
                self.work
                    .get_subagent(id)?
                    .ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))?
                    .session_id
            }
        };
        let mut request = effect_request(
            terminal_actor(),
            action,
            resource,
            serde_json::to_value(&mutation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![action.into()];
        request.context.session_id = Some(session_id);
        match &mutation {
            WorkOperation::GoalCreate { source_plan_id, .. } => {
                request.context.plan_id = source_plan_id.clone();
            }
            WorkOperation::PlanExecute { id, .. }
            | WorkOperation::PlanExecuteAtRevision { id, .. } => {
                request.context.plan_id = Some(id.clone());
            }
            WorkOperation::GoalShow { id }
            | WorkOperation::GoalUpdate { id, .. }
            | WorkOperation::GoalIteration { id } => {
                request.context.goal_id = Some(id.clone());
            }
            WorkOperation::SubagentRead { id }
            | WorkOperation::SubagentStart { id }
            | WorkOperation::SubagentComplete { id, .. }
            | WorkOperation::SubagentStop { id, .. }
            | WorkOperation::SubagentRequeue { id } => {
                request.context.subagent_id = Some(id.clone());
            }
            _ => {}
        }
        let result = self
            .gateway
            .execute(request, self.work_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Create a canonical session-scoped task.
    pub async fn create_task(
        &self,
        session_id: &str,
        title: &str,
        description: &str,
        status: TaskStatus,
    ) -> Result<TaskRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::TaskCreate {
                session_id: session_id.into(),
                title: title.into(),
                description: description.into(),
                status,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Update mutable task fields through a new canonical event.
    pub async fn update_task(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<TaskStatus>,
    ) -> Result<TaskRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::TaskUpdate {
                id: id.into(),
                title: title.map(str::to_owned),
                description: description.map(str::to_owned),
                status,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Reconstruct one canonical task.
    pub fn get_task(&self, id: &str) -> Result<Option<TaskRecord>, RuntimeError> {
        self.work.get_task(id).map_err(Into::into)
    }

    /// List bounded canonical tasks.
    pub fn list_tasks(
        &self,
        session_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        self.work
            .list_tasks(session_id, status, limit)
            .map_err(Into::into)
    }

    /// Create a canonical active key decision.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_decision(
        &self,
        session_id: &str,
        title: &str,
        decision: &str,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
    ) -> Result<KeyDecision, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::DecisionCreate {
                session_id: session_id.into(),
                title: title.into(),
                decision: decision.into(),
                source: DecisionSource::User,
                priority,
                intent: intent.into(),
                applies_when: applies_when.into(),
                rationale: rationale.into(),
                source_excerpt: source_excerpt.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Update mutable key-decision content through a new canonical event.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_decision(
        &self,
        id: &str,
        title: Option<&str>,
        decision: Option<&str>,
        priority: Option<DecisionPriority>,
        intent: Option<&str>,
        applies_when: Option<&str>,
        rationale: Option<&str>,
        source_excerpt: Option<&str>,
    ) -> Result<KeyDecision, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::DecisionUpdate {
                id: id.into(),
                title: title.map(str::to_owned),
                decision: decision.map(str::to_owned),
                priority,
                intent: intent.map(str::to_owned),
                applies_when: applies_when.map(str::to_owned),
                rationale: rationale.map(str::to_owned),
                source_excerpt: source_excerpt.map(str::to_owned),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Reconstruct one canonical key decision.
    pub fn get_decision(&self, id: &str) -> Result<Option<KeyDecision>, RuntimeError> {
        self.work.get_decision(id).map_err(Into::into)
    }

    /// List bounded canonical key decisions.
    pub fn list_decisions(
        &self,
        session_id: Option<&str>,
        status: Option<DecisionStatus>,
        limit: usize,
    ) -> Result<Vec<KeyDecision>, RuntimeError> {
        self.work
            .list_decisions(session_id, status, limit)
            .map_err(Into::into)
    }

    /// Reconstruct one canonical durable plan.
    pub fn get_plan(&self, id: &str) -> Result<Option<PlanRecord>, RuntimeError> {
        self.work.get_plan(id).map_err(Into::into)
    }

    /// List bounded canonical plans.
    pub fn list_plans(
        &self,
        session_id: Option<&str>,
        status: Option<PlanStatus>,
        limit: usize,
    ) -> Result<Vec<PlanRecord>, RuntimeError> {
        self.work
            .list_plans(session_id, status, limit)
            .map_err(Into::into)
    }

    /// Reconstruct one canonical bounded-autonomy goal.
    pub fn get_goal(&self, id: &str) -> Result<Option<GoalRecord>, RuntimeError> {
        self.work.get_goal(id).map_err(Into::into)
    }

    /// List bounded canonical goals.
    pub fn list_goals(
        &self,
        session_id: Option<&str>,
        status: Option<GoalStatus>,
        limit: usize,
    ) -> Result<Vec<GoalRecord>, RuntimeError> {
        self.work
            .list_goals(session_id, status, limit)
            .map_err(Into::into)
    }

    /// Refresh bounded actionable work for one exact durable session.
    pub fn work_state(&self, session_id: &str) -> Result<WorkStateSnapshot, RuntimeError> {
        self.get_session(session_id)?
            .ok_or_else(|| StoreError::NotFound(format!("session {session_id}")))?;
        let tasks = self.work.list_tasks(Some(session_id), None, 1_000)?;
        let open_task_count = tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled))
            .count();
        let active_decisions =
            self.work
                .list_decisions(Some(session_id), Some(DecisionStatus::Active), 1_000)?;
        let actionable_plans = self
            .work
            .list_plans(Some(session_id), None, 1_000)?
            .into_iter()
            .filter(|plan| matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved))
            .collect();
        let current_goals = self
            .work
            .list_goals(Some(session_id), None, 1_000)?
            .into_iter()
            .filter(|goal| goal.status != GoalStatus::Complete)
            .collect();
        let current_subagents = self
            .work
            .list_subagents(Some(session_id), None, 1_000)?
            .into_iter()
            .filter(|job| {
                matches!(
                    job.status,
                    SubagentStatus::Queued | SubagentStatus::Running | SubagentStatus::Interrupted
                )
            })
            .collect();
        Ok(WorkStateSnapshot {
            session_id: session_id.into(),
            tasks,
            open_task_count,
            active_decisions,
            actionable_plans,
            current_goals,
            current_subagents,
        })
    }

    /// Create a durable draft plan through the effect gateway.
    pub async fn create_plan(
        &self,
        session_id: &str,
        prompt: &str,
        content: &str,
        steps: Vec<PlanStep>,
    ) -> Result<PlanRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanCreate {
                session_id: session_id.into(),
                prompt: prompt.into(),
                content: content.into(),
                steps,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Approve one draft plan through the configured approval obligation.
    pub async fn approve_plan(&self, id: &str) -> Result<PlanRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanApprove { id: id.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Approve one exact draft revision through the configured approval obligation.
    pub async fn approve_plan_at_revision(
        &self,
        expected_session_id: &str,
        id: &str,
        revision: u64,
    ) -> Result<PlanRecord, RuntimeError> {
        let plan = self
            .work
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        validate_plan_lifecycle_selection(&plan, expected_session_id, revision)?;
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanApproveAtRevision {
                id: id.into(),
                expected_revision: revision,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Discard one exact draft or approved revision without deleting its history.
    pub async fn discard_plan_at_revision(
        &self,
        expected_session_id: &str,
        id: &str,
        revision: u64,
    ) -> Result<PlanRecord, RuntimeError> {
        let plan = self
            .work
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        validate_plan_lifecycle_selection(&plan, expected_session_id, revision)?;
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanDiscard {
                id: id.into(),
                expected_revision: Some(revision),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Discard the newest draft or approved revision without deleting its history.
    pub async fn discard_plan(&self, id: &str) -> Result<PlanRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::PlanDiscard {
                id: id.into(),
                expected_revision: None,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Archive one active decision while retaining its complete history.
    pub async fn archive_decision(&self, id: &str) -> Result<KeyDecision, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::DecisionArchive { id: id.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Atomically replace one active decision and preserve lineage.
    #[allow(clippy::too_many_arguments)]
    pub async fn supersede_decision(
        &self,
        id: &str,
        title: &str,
        decision: &str,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
    ) -> Result<(KeyDecision, KeyDecision), RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::DecisionSupersede {
                id: id.into(),
                title: title.into(),
                decision: decision.into(),
                source: DecisionSource::User,
                priority,
                intent: intent.into(),
                applies_when: applies_when.into(),
                rationale: rationale.into(),
                source_excerpt: source_excerpt.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}

#[cfg(test)]
mod plan_lifecycle_selection_tests {
    use super::validate_plan_lifecycle_selection;
    use colossus_contracts::{PlanRecord, PlanStatus};

    fn plan() -> PlanRecord {
        PlanRecord {
            id: "plan-1".into(),
            session_id: "session-1".into(),
            prompt: "Plan it".into(),
            status: PlanStatus::Draft,
            revision: 2,
            content: "# Plan".into(),
            steps: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            approved_at: None,
            executed_run_id: None,
        }
    }

    #[test]
    fn lifecycle_actions_require_the_selected_session_and_revision() {
        let selected = plan();
        validate_plan_lifecycle_selection(&selected, "session-1", 2).expect("selected plan");
        assert!(validate_plan_lifecycle_selection(&selected, "session-2", 2).is_err());
        assert!(validate_plan_lifecycle_selection(&selected, "session-1", 1).is_err());
    }
}
