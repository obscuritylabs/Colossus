use super::*;

/// Immutable inputs for queueing one isolated child-agent job.
pub struct CreateSubagentRequest {
    /// Parent session that owns the job.
    pub session_id: String,
    /// Exact parent agent run.
    pub parent_run_id: String,
    /// Exact model tool-call lineage.
    pub parent_call_id: String,
    /// Bounded child task.
    pub task: String,
    /// Child role resolved by the agent runtime.
    pub role: String,
    /// Persisted exact model-tool ceiling, or trusted private-call compatibility.
    pub allowed_tools: Option<Vec<String>>,
}

/// Validated application service shared by CLI, TUI, tools, and embedded callers.
pub struct WorkService {
    repository: Arc<dyn WorkRepository>,
    sessions: Arc<dyn SessionRepository>,
}

impl WorkService {
    /// Compose work operations from canonical repository ports.
    pub fn new(repository: Arc<dyn WorkRepository>, sessions: Arc<dyn SessionRepository>) -> Self {
        Self {
            repository,
            sessions,
        }
    }

    fn require_session(&self, session_id: &str) -> Result<(), StoreError> {
        self.sessions
            .get_session(session_id)?
            .ok_or_else(|| StoreError::NotFound(format!("session {session_id}")))?;
        Ok(())
    }

    /// Create a new task with a generated stable id.
    pub fn create_task(
        &self,
        session_id: &str,
        title: &str,
        description: &str,
        status: TaskStatus,
        actor: Actor,
    ) -> Result<TaskRecord, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        self.repository.create_task(
            TaskRecord {
                id: format!("task-{}", Uuid::now_v7()),
                session_id: session_id.into(),
                title: title.trim().into(),
                description: description.into(),
                status,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            },
            actor,
        )
    }

    /// Update supplied task fields while preserving identity and creation time.
    pub fn update_task(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<TaskStatus>,
        actor: Actor,
    ) -> Result<TaskRecord, StoreError> {
        let mut task = self
            .repository
            .get_task(id)?
            .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?;
        if let Some(title) = title {
            task.title = title.trim().into();
        }
        if let Some(description) = description {
            task.description = description.into();
        }
        if let Some(status) = status {
            task.status = status;
        }
        task.updated_at = now()?;
        self.repository.update_task(task, actor)
    }

    /// Create one active future-facing decision.
    #[allow(clippy::too_many_arguments)]
    pub fn create_decision(
        &self,
        session_id: &str,
        title: &str,
        decision: &str,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
        goal_id: Option<String>,
        plan_id: Option<String>,
        supersedes: Option<String>,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        self.repository.create_decision(
            KeyDecision {
                id: format!("kd_{}", Uuid::now_v7()),
                session_id: session_id.into(),
                goal_id,
                plan_id,
                source,
                status: DecisionStatus::Active,
                priority,
                title: title.trim().into(),
                decision: decision.trim().into(),
                intent: intent.into(),
                applies_when: applies_when.into(),
                rationale: rationale.into(),
                source_excerpt: source_excerpt.into(),
                supersedes,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            },
            actor,
        )
    }

    /// Update mutable decision content while leaving provenance and status intact.
    #[allow(clippy::too_many_arguments)]
    pub fn update_decision(
        &self,
        id: &str,
        title: Option<&str>,
        decision: Option<&str>,
        priority: Option<DecisionPriority>,
        intent: Option<&str>,
        applies_when: Option<&str>,
        rationale: Option<&str>,
        source_excerpt: Option<&str>,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        let mut record = self
            .repository
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        if let Some(value) = title {
            record.title = value.trim().into();
        }
        if let Some(value) = decision {
            record.decision = value.trim().into();
        }
        if let Some(value) = priority {
            record.priority = value;
        }
        if let Some(value) = intent {
            record.intent = value.into();
        }
        if let Some(value) = applies_when {
            record.applies_when = value.into();
        }
        if let Some(value) = rationale {
            record.rationale = value.into();
        }
        if let Some(value) = source_excerpt {
            record.source_excerpt = value.into();
        }
        record.updated_at = now()?;
        self.repository.update_decision(record, actor)
    }

    /// Archive one active decision.
    pub fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError> {
        self.repository.archive_decision(id, actor)
    }

    /// Replace an active decision atomically and preserve lineage.
    #[allow(clippy::too_many_arguments)]
    pub fn supersede_decision(
        &self,
        id: &str,
        title: &str,
        decision: &str,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError> {
        let old = self
            .repository
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        let timestamp = now()?;
        let replacement = KeyDecision {
            id: format!("kd_{}", Uuid::now_v7()),
            session_id: old.session_id.clone(),
            goal_id: old.goal_id.clone(),
            plan_id: old.plan_id.clone(),
            source,
            status: DecisionStatus::Active,
            priority,
            title: title.trim().into(),
            decision: decision.trim().into(),
            intent: intent.into(),
            applies_when: applies_when.into(),
            rationale: rationale.into(),
            source_excerpt: source_excerpt.into(),
            supersedes: Some(old.id.clone()),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        self.repository.supersede_decision(id, replacement, actor)
    }

    /// Create a draft plan with ordered steps in one session.
    pub fn create_plan(
        &self,
        session_id: &str,
        prompt: &str,
        content: &str,
        steps: Vec<PlanStep>,
        actor: Actor,
    ) -> Result<PlanRecord, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        self.repository.create_plan(
            PlanRecord {
                id: format!("plan-{}", Uuid::now_v7()),
                session_id: session_id.into(),
                prompt: prompt.trim().into(),
                status: PlanStatus::Draft,
                content: content.into(),
                steps,
                created_at: timestamp.clone(),
                updated_at: timestamp,
                approved_at: None,
                executed_run_id: None,
            },
            actor,
        )
    }

    /// Replace editable draft content while preserving identity and lineage.
    pub fn update_draft_plan(
        &self,
        id: &str,
        prompt: Option<&str>,
        content: Option<&str>,
        steps: Option<Vec<PlanStep>>,
        actor: Actor,
    ) -> Result<PlanRecord, StoreError> {
        let mut plan = self
            .repository
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        if plan.status != PlanStatus::Draft {
            return Err(StoreError::Adapter("only draft plans can be edited".into()));
        }
        if let Some(prompt) = prompt {
            plan.prompt = prompt.trim().into();
        }
        if let Some(content) = content {
            plan.content = content.into();
        }
        if let Some(steps) = steps {
            plan.steps = steps;
        }
        plan.updated_at = now()?;
        self.repository.update_plan(plan, actor)
    }

    /// Approve one draft exactly once.
    pub fn approve_plan(&self, id: &str, actor: Actor) -> Result<PlanRecord, StoreError> {
        let mut plan = self
            .repository
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        if plan.status != PlanStatus::Draft {
            return Err(StoreError::Adapter(
                "only draft plans can be approved".into(),
            ));
        }
        let timestamp = now()?;
        plan.status = PlanStatus::Approved;
        plan.updated_at = timestamp.clone();
        plan.approved_at = Some(timestamp);
        self.repository.update_plan(plan, actor)
    }

    /// Consume one approved plan for a single execution run.
    pub fn execute_plan(
        &self,
        id: &str,
        run_id: &str,
        actor: Actor,
    ) -> Result<PlanRecord, StoreError> {
        let mut plan = self
            .repository
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        if plan.status != PlanStatus::Approved || !valid_id(run_id) {
            return Err(StoreError::Adapter(
                "plan execution requires one approved plan and a valid run id".into(),
            ));
        }
        plan.status = PlanStatus::Executed;
        plan.updated_at = now()?;
        plan.executed_run_id = Some(run_id.into());
        self.repository.update_plan(plan, actor)
    }

    /// Discard a draft or approved plan without deleting history.
    pub fn discard_plan(&self, id: &str, actor: Actor) -> Result<PlanRecord, StoreError> {
        let mut plan = self
            .repository
            .get_plan(id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {id}")))?;
        if !matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved) {
            return Err(StoreError::Adapter(
                "only draft or approved plans can be discarded".into(),
            ));
        }
        plan.status = PlanStatus::Discarded;
        plan.updated_at = now()?;
        self.repository.update_plan(plan, actor)
    }

    /// Create one active bounded-autonomy goal with optional approved-plan lineage.
    pub fn create_goal(
        &self,
        session_id: &str,
        objective: &str,
        iteration_budget: u16,
        source_plan_id: Option<String>,
        actor: Actor,
    ) -> Result<GoalRecord, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        let goal = GoalRecord {
            id: format!("goal-{}", Uuid::now_v7()),
            session_id: session_id.into(),
            objective: objective.trim().into(),
            source_plan_id: source_plan_id.clone(),
            status: GoalStatus::Active,
            summary: String::new(),
            blocked_reason: String::new(),
            iteration_budget,
            iterations_completed: 0,
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        let Some(plan_id) = source_plan_id else {
            return self.repository.create_goal(goal, actor);
        };
        let mut plan = self
            .repository
            .get_plan(&plan_id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
        if plan.session_id != session_id || plan.status != PlanStatus::Approved {
            return Err(StoreError::Adapter(
                "goal plan lineage requires an approved same-session plan".into(),
            ));
        }
        plan.status = PlanStatus::Executed;
        plan.updated_at = now()?;
        plan.executed_run_id = Some(goal.id.clone());
        self.repository
            .create_goal_from_plan(goal, plan, actor)
            .map(|(goal, _)| goal)
    }

    /// Record one completed iteration without changing a terminal outcome.
    pub fn record_goal_iteration(&self, id: &str, actor: Actor) -> Result<GoalRecord, StoreError> {
        let mut goal = self
            .repository
            .get_goal(id)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {id}")))?;
        if goal.iterations_completed >= goal.iteration_budget {
            return Err(StoreError::Adapter(
                "only a goal with remaining budget can record an iteration".into(),
            ));
        }
        goal.iterations_completed = goal.iterations_completed.saturating_add(1);
        goal.updated_at = now()?;
        self.repository.update_goal(goal, actor)
    }

    /// Mark an active goal complete or blocked with required evidence text.
    pub fn update_goal_status(
        &self,
        id: &str,
        status: GoalStatus,
        summary: &str,
        blocked_reason: &str,
        actor: Actor,
    ) -> Result<GoalRecord, StoreError> {
        let mut goal = self
            .repository
            .get_goal(id)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {id}")))?;
        if goal.status != GoalStatus::Active {
            return Err(StoreError::Adapter(
                "terminal goals cannot be updated".into(),
            ));
        }
        goal.status = status;
        goal.summary = summary.into();
        goal.blocked_reason = if status == GoalStatus::Blocked {
            blocked_reason.into()
        } else {
            String::new()
        };
        goal.updated_at = now()?;
        self.repository.update_goal(goal, actor)
    }

    /// Queue a durable child-agent job and create its isolated child session.
    pub fn create_subagent(
        &self,
        request: CreateSubagentRequest,
        actor: Actor,
    ) -> Result<SubagentJob, StoreError> {
        self.require_session(&request.session_id)?;
        let id = format!("agent-{}", Uuid::now_v7());
        let child_session_id = Uuid::now_v7().to_string();
        self.sessions.create_session(
            &child_session_id,
            Some(&format!("subagent {id}")),
            actor.clone(),
        )?;
        let timestamp = now()?;
        self.repository.create_subagent(
            SubagentJob {
                id,
                session_id: request.session_id,
                parent_run_id: request.parent_run_id,
                parent_call_id: request.parent_call_id,
                task: request.task.trim().into(),
                role: request.role,
                allowed_tools: request.allowed_tools,
                status: SubagentStatus::Queued,
                child_session_id,
                child_run_id: None,
                final_output: String::new(),
                error: String::new(),
                created_at: timestamp.clone(),
                updated_at: timestamp,
                started_at: None,
                completed_at: None,
            },
            actor,
        )
    }

    /// Move a queued job to running.
    pub fn start_subagent(&self, id: &str, actor: Actor) -> Result<SubagentJob, StoreError> {
        let mut job = self.require_subagent(id)?;
        if job.status != SubagentStatus::Queued {
            return Err(StoreError::Adapter(
                "only queued subagents can start".into(),
            ));
        }
        let timestamp = now()?;
        job.status = SubagentStatus::Running;
        job.started_at = Some(timestamp.clone());
        job.updated_at = timestamp;
        self.repository.update_subagent(job, actor)
    }

    /// Store one released child result.
    pub fn complete_subagent(
        &self,
        id: &str,
        child_run_id: &str,
        output: &str,
        actor: Actor,
    ) -> Result<SubagentJob, StoreError> {
        let mut job = self.require_subagent(id)?;
        if job.status != SubagentStatus::Running {
            return Err(StoreError::Adapter(
                "only running subagents can complete".into(),
            ));
        }
        let timestamp = now()?;
        job.status = SubagentStatus::Completed;
        job.child_run_id = Some(child_run_id.into());
        job.final_output = output.into();
        job.error.clear();
        job.completed_at = Some(timestamp.clone());
        job.updated_at = timestamp;
        self.repository.update_subagent(job, actor)
    }

    /// Store a bounded failed, cancelled, or interrupted terminal outcome.
    pub fn stop_subagent(
        &self,
        id: &str,
        status: SubagentStatus,
        error: &str,
        actor: Actor,
    ) -> Result<SubagentJob, StoreError> {
        let mut job = self.require_subagent(id)?;
        let allowed = match status {
            SubagentStatus::Cancelled => {
                matches!(job.status, SubagentStatus::Queued | SubagentStatus::Running)
            }
            SubagentStatus::Failed | SubagentStatus::Interrupted => {
                job.status == SubagentStatus::Running
            }
            _ => false,
        };
        if !allowed {
            return Err(StoreError::Adapter(
                "invalid subagent terminal transition".into(),
            ));
        }
        let timestamp = now()?;
        job.status = status;
        job.error = error.into();
        job.completed_at = Some(timestamp.clone());
        job.updated_at = timestamp;
        self.repository.update_subagent(job, actor)
    }

    /// Requeue a failed, cancelled, or interrupted job without losing lineage.
    pub fn requeue_subagent(&self, id: &str, actor: Actor) -> Result<SubagentJob, StoreError> {
        let mut job = self.require_subagent(id)?;
        if !matches!(
            job.status,
            SubagentStatus::Failed | SubagentStatus::Cancelled | SubagentStatus::Interrupted
        ) {
            return Err(StoreError::Adapter(
                "only failed, cancelled, or interrupted subagents can be requeued".into(),
            ));
        }
        job.status = SubagentStatus::Queued;
        job.child_run_id = None;
        job.final_output.clear();
        job.error.clear();
        job.started_at = None;
        job.completed_at = None;
        job.updated_at = now()?;
        self.repository.update_subagent(job, actor)
    }

    fn require_subagent(&self, id: &str) -> Result<SubagentJob, StoreError> {
        self.repository
            .get_subagent(id)?
            .ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))
    }

    /// Canonical repository for bounded query surfaces.
    pub fn repository(&self) -> Arc<dyn WorkRepository> {
        Arc::clone(&self.repository)
    }
}

#[cfg(test)]
pub(super) fn user_actor() -> Actor {
    Actor {
        actor_type: colossus_contracts::ActorType::User,
        id: "terminal-user".into(),
    }
}
