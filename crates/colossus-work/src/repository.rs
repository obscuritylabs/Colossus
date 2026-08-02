use super::*;

/// Immutable-journal implementation of the work lifecycle port.
pub struct EventSourcedWorkRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedWorkRepository {
    /// Bind canonical task and decision streams to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn task_stream(id: &str) -> String {
        format!("task:{id}")
    }

    fn decision_stream(id: &str) -> String {
        format!("decision:{id}")
    }

    fn plan_stream(id: &str) -> String {
        format!("plan:{id}")
    }

    fn goal_stream(id: &str) -> String {
        format!("goal:{id}")
    }

    fn subagent_stream(id: &str) -> String {
        format!("subagent:{id}")
    }

    fn event(
        stream_id: String,
        expected_stream_version: u64,
        event_type: &str,
        actor: Actor,
        session_id: &str,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: stream_id.clone(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: stream_id,
                session_id: Some(session_id.into()),
                ..ExecutionContext::default()
            },
            payload,
        }
    }

    fn ids(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        collect_stream_ids(self.journal.as_ref(), prefix)?
            .into_iter()
            .map(|stream_id| {
                stream_id
                    .strip_prefix(prefix)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "indexed stream {stream_id} does not match prefix {prefix}"
                        ))
                    })
            })
            .collect()
    }

    fn record<T: serde::de::DeserializeOwned>(
        &self,
        stream_id: &str,
        expected_created_event: &str,
    ) -> Result<Option<T>, StoreError> {
        Ok(self
            .record_with_version(stream_id, expected_created_event)?
            .map(|(record, _)| record))
    }

    fn record_with_version<T: serde::de::DeserializeOwned>(
        &self,
        stream_id: &str,
        expected_created_event: &str,
    ) -> Result<Option<(T, u64)>, StoreError> {
        let events = self.journal.read_stream(stream_id)?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if first.event_type != expected_created_event {
            return Err(StoreError::Verification(format!(
                "work stream {stream_id} has no valid creation event"
            )));
        }
        let last = events
            .last()
            .ok_or_else(|| StoreError::Verification("work stream disappeared".into()))?;
        let stream_version = u64::try_from(events.len()).map_err(adapter)?;
        let payload = self.journal.decrypt_payload(last)?;
        serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("work record is absent".into()))?,
        )
        .map(|record| Some((record, stream_version)))
        .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

impl WorkRepository for EventSourcedWorkRepository {
    fn create_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError> {
        validate_task(&task)?;
        self.journal.append(Self::event(
            Self::task_stream(&task.id),
            0,
            TASK_CREATED,
            actor,
            &task.session_id,
            json!({"record": &task}),
        ))?;
        Ok(task)
    }

    fn update_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError> {
        validate_task(&task)?;
        let current = self
            .get_task(&task.id)?
            .ok_or_else(|| StoreError::NotFound(format!("task {}", task.id)))?;
        if current.session_id != task.session_id || current.created_at != task.created_at {
            return Err(StoreError::Adapter(
                "task session and creation timestamp are immutable".into(),
            ));
        }
        let stream = Self::task_stream(&task.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            TASK_UPDATED,
            actor,
            &task.session_id,
            json!({"record": &task}),
        ))?;
        Ok(task)
    }

    fn get_task(&self, id: &str) -> Result<Option<TaskRecord>, StoreError> {
        self.record(&Self::task_stream(id), TASK_CREATED)
    }

    fn list_tasks(
        &self,
        session_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let mut records = self
            .ids("task:")?
            .into_iter()
            .filter_map(|id| self.get_task(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn create_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        validate_decision(&decision)?;
        if decision.status != DecisionStatus::Active {
            return Err(StoreError::Adapter(
                "new key decisions must be active".into(),
            ));
        }
        self.journal.append(Self::event(
            Self::decision_stream(&decision.id),
            0,
            DECISION_CREATED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn update_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        validate_decision(&decision)?;
        let current = self
            .get_decision(&decision.id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {}", decision.id)))?;
        if current.status != DecisionStatus::Active
            || decision.status != DecisionStatus::Active
            || current.session_id != decision.session_id
            || current.created_at != decision.created_at
            || current.source != decision.source
            || current.supersedes != decision.supersedes
        {
            return Err(StoreError::Adapter(
                "only active decision content may be updated; provenance is immutable".into(),
            ));
        }
        let stream = Self::decision_stream(&decision.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            DECISION_UPDATED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn get_decision(&self, id: &str) -> Result<Option<KeyDecision>, StoreError> {
        self.record(&Self::decision_stream(id), DECISION_CREATED)
    }

    fn list_decisions(
        &self,
        session_id: Option<&str>,
        status: Option<DecisionStatus>,
        limit: usize,
    ) -> Result<Vec<KeyDecision>, StoreError> {
        let mut records = self
            .ids("decision:")?
            .into_iter()
            .filter_map(|id| self.get_decision(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError> {
        let mut decision = self
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        if decision.status != DecisionStatus::Active {
            return Err(StoreError::Adapter(
                "only active decisions can be archived".into(),
            ));
        }
        decision.status = DecisionStatus::Archived;
        decision.updated_at = now()?;
        let stream = Self::decision_stream(id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            DECISION_ARCHIVED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn supersede_decision(
        &self,
        id: &str,
        replacement: KeyDecision,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError> {
        let mut old = self
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        validate_decision(&replacement)?;
        if old.status != DecisionStatus::Active
            || replacement.status != DecisionStatus::Active
            || replacement.id == old.id
            || replacement.session_id != old.session_id
            || replacement.supersedes.as_deref() != Some(id)
            || self.get_decision(&replacement.id)?.is_some()
        {
            return Err(StoreError::Adapter(
                "decision supersession requires an active same-session replacement linked to the old id"
                    .into(),
            ));
        }
        old.status = DecisionStatus::Superseded;
        old.updated_at = now()?;
        let old_stream = Self::decision_stream(id);
        let expected =
            u64::try_from(self.journal.read_stream(&old_stream)?.len()).map_err(adapter)?;
        self.journal.append_batch(vec![
            Self::event(
                old_stream,
                expected,
                DECISION_SUPERSEDED,
                actor.clone(),
                &old.session_id,
                json!({"record": &old, "replacement_id": replacement.id}),
            ),
            Self::event(
                Self::decision_stream(&replacement.id),
                0,
                DECISION_CREATED,
                actor,
                &replacement.session_id,
                json!({"record": &replacement}),
            ),
        ])?;
        Ok((old, replacement))
    }

    fn create_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError> {
        validate_plan(&plan)?;
        if plan.status != PlanStatus::Draft || plan.revision != 1 {
            return Err(StoreError::Adapter(
                "new plans must be drafts at revision one".into(),
            ));
        }
        self.journal.append(Self::event(
            Self::plan_stream(&plan.id),
            0,
            PLAN_CREATED,
            actor,
            &plan.session_id,
            json!({"record": &plan}),
        ))?;
        Ok(plan)
    }

    fn update_plan(&self, mut plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError> {
        validate_plan(&plan)?;
        let stream = Self::plan_stream(&plan.id);
        let (current, expected) = self
            .record_with_version(&stream, PLAN_CREATED)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {}", plan.id)))?;
        require_plan_revision(&current, plan.revision)?;
        if current.session_id != plan.session_id
            || current.created_at != plan.created_at
            || current.prompt != plan.prompt
        {
            return Err(StoreError::Adapter(
                "plan session, objective, and creation timestamp are immutable".into(),
            ));
        }
        let transition = (current.status, plan.status);
        let event_type = match transition {
            (PlanStatus::Draft, PlanStatus::Draft) => PLAN_UPDATED,
            (PlanStatus::Draft, PlanStatus::Approved) => PLAN_APPROVED,
            (PlanStatus::Approved, PlanStatus::Executed) => PLAN_EXECUTED,
            (PlanStatus::Draft | PlanStatus::Approved, PlanStatus::Discarded) => PLAN_DISCARDED,
            _ => {
                return Err(StoreError::Adapter(format!(
                    "invalid plan transition from {:?} to {:?}",
                    current.status, plan.status
                )));
            }
        };
        let approval_lineage_valid = match transition {
            (PlanStatus::Draft, PlanStatus::Draft | PlanStatus::Discarded) => {
                plan.approved_at.is_none()
            }
            (PlanStatus::Draft, PlanStatus::Approved) => plan.approved_at.is_some(),
            (PlanStatus::Approved, PlanStatus::Executed | PlanStatus::Discarded) => {
                plan.approved_at == current.approved_at
            }
            _ => false,
        };
        if !approval_lineage_valid {
            return Err(StoreError::Adapter(
                "plan approval lineage is immutable".into(),
            ));
        }
        if (current.status != PlanStatus::Draft || plan.status != PlanStatus::Draft)
            && (current.content != plan.content || current.steps != plan.steps)
        {
            return Err(StoreError::Adapter(
                "plan content is immutable during lifecycle transitions".into(),
            ));
        }
        plan.revision = plan
            .revision
            .checked_add(1)
            .ok_or_else(|| StoreError::Adapter("plan revision overflow".into()))?;
        self.journal.append(Self::event(
            stream,
            expected,
            event_type,
            actor,
            &plan.session_id,
            json!({"record": &plan}),
        ))?;
        Ok(plan)
    }

    fn get_plan(&self, id: &str) -> Result<Option<PlanRecord>, StoreError> {
        self.record(&Self::plan_stream(id), PLAN_CREATED)
    }

    fn list_plans(
        &self,
        session_id: Option<&str>,
        status: Option<PlanStatus>,
        limit: usize,
    ) -> Result<Vec<PlanRecord>, StoreError> {
        let mut records = self
            .ids("plan:")?
            .into_iter()
            .filter_map(|id| self.get_plan(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn create_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError> {
        validate_goal(&goal)?;
        if goal.status != GoalStatus::Active || goal.iterations_completed != 0 {
            return Err(StoreError::Adapter(
                "new goals must be active with zero completed iterations".into(),
            ));
        }
        self.journal.append(Self::event(
            Self::goal_stream(&goal.id),
            0,
            GOAL_CREATED,
            actor,
            &goal.session_id,
            json!({"record": &goal}),
        ))?;
        Ok(goal)
    }

    fn create_goal_from_plan(
        &self,
        goal: GoalRecord,
        mut executed_plan: PlanRecord,
        actor: Actor,
    ) -> Result<(GoalRecord, PlanRecord), StoreError> {
        validate_goal(&goal)?;
        validate_plan(&executed_plan)?;
        let plan_id = goal
            .source_plan_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("goal plan lineage is absent".into()))?;
        let plan_stream = Self::plan_stream(plan_id);
        let (current, expected) = self
            .record_with_version(&plan_stream, PLAN_CREATED)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
        require_plan_revision(&current, executed_plan.revision)?;
        if goal.status != GoalStatus::Active
            || goal.iterations_completed != 0
            || current.status != PlanStatus::Approved
            || executed_plan.id != current.id
            || executed_plan.session_id != goal.session_id
            || executed_plan.status != PlanStatus::Executed
            || executed_plan.prompt != current.prompt
            || executed_plan.content != current.content
            || executed_plan.steps != current.steps
            || executed_plan.created_at != current.created_at
            || executed_plan.approved_at != current.approved_at
            || executed_plan.executed_run_id.as_deref() != Some(goal.id.as_str())
        {
            return Err(StoreError::Adapter(
                "goal creation requires one unchanged approved same-session plan consumed by the goal id"
                    .into(),
            ));
        }
        if self.get_goal(&goal.id)?.is_some() {
            return Err(StoreError::Conflict {
                stream_id: Self::goal_stream(&goal.id),
                expected: 0,
                actual: 1,
            });
        }
        executed_plan.revision = executed_plan
            .revision
            .checked_add(1)
            .ok_or_else(|| StoreError::Adapter("plan revision overflow".into()))?;
        self.journal.append_batch(vec![
            Self::event(
                plan_stream,
                expected,
                PLAN_EXECUTED,
                actor.clone(),
                &executed_plan.session_id,
                json!({"record": &executed_plan}),
            ),
            Self::event(
                Self::goal_stream(&goal.id),
                0,
                GOAL_CREATED,
                actor,
                &goal.session_id,
                json!({"record": &goal}),
            ),
        ])?;
        Ok((goal, executed_plan))
    }

    fn update_goal(&self, goal: GoalRecord, actor: Actor) -> Result<GoalRecord, StoreError> {
        validate_goal(&goal)?;
        let stream = Self::goal_stream(&goal.id);
        let (current, expected) = self
            .record_with_version::<GoalRecord>(&stream, GOAL_CREATED)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {}", goal.id)))?;
        let terminal_iteration_only = current.status != GoalStatus::Active
            && (goal.status != current.status
                || goal.summary != current.summary
                || goal.blocked_reason != current.blocked_reason);
        if terminal_iteration_only
            || current.session_id != goal.session_id
            || current.objective != goal.objective
            || current.source_plan_id != goal.source_plan_id
            || current.iteration_budget != goal.iteration_budget
            || current.created_at != goal.created_at
            || goal.iterations_completed != current.iterations_completed
        {
            return Err(StoreError::Adapter(
                "goal provenance, budget, terminal state, and iteration count are immutable during status updates"
                    .into(),
            ));
        }
        self.journal.append(Self::event(
            stream,
            expected,
            GOAL_UPDATED,
            actor,
            &goal.session_id,
            json!({"record": &goal}),
        ))?;
        Ok(goal)
    }

    fn record_goal_iteration(
        &self,
        goal: GoalRecord,
        expected_iterations_completed: u16,
        actor: Actor,
    ) -> Result<GoalRecord, StoreError> {
        validate_goal(&goal)?;
        let stream = Self::goal_stream(&goal.id);
        let (current, expected_stream_version) = self
            .record_with_version::<GoalRecord>(&stream, GOAL_CREATED)?
            .ok_or_else(|| StoreError::NotFound(format!("goal {}", goal.id)))?;
        if current.iterations_completed != expected_iterations_completed {
            return Err(StoreError::Conflict {
                stream_id: stream,
                expected: u64::from(expected_iterations_completed),
                actual: u64::from(current.iterations_completed),
            });
        }
        let next_iterations_completed = expected_iterations_completed
            .checked_add(1)
            .ok_or_else(|| StoreError::Adapter("goal iteration count overflow".into()))?;
        if expected_iterations_completed >= current.iteration_budget
            || goal.iterations_completed != next_iterations_completed
            || current.session_id != goal.session_id
            || current.objective != goal.objective
            || current.source_plan_id != goal.source_plan_id
            || current.status != goal.status
            || current.summary != goal.summary
            || current.blocked_reason != goal.blocked_reason
            || current.iteration_budget != goal.iteration_budget
            || current.created_at != goal.created_at
        {
            return Err(StoreError::Adapter(
                "goal iteration reservation requires one unchanged canonical goal with remaining budget"
                    .into(),
            ));
        }
        self.journal.append(Self::event(
            stream,
            expected_stream_version,
            GOAL_UPDATED,
            actor,
            &goal.session_id,
            json!({"record": &goal}),
        ))?;
        Ok(goal)
    }

    fn get_goal(&self, id: &str) -> Result<Option<GoalRecord>, StoreError> {
        self.record(&Self::goal_stream(id), GOAL_CREATED)
    }

    fn list_goals(
        &self,
        session_id: Option<&str>,
        status: Option<GoalStatus>,
        limit: usize,
    ) -> Result<Vec<GoalRecord>, StoreError> {
        let mut records = self
            .ids("goal:")?
            .into_iter()
            .filter_map(|id| self.get_goal(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn create_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError> {
        validate_subagent(&job)?;
        if job.status != SubagentStatus::Queued {
            return Err(StoreError::Adapter("new subagents must be queued".into()));
        }
        self.journal.append(Self::event(
            Self::subagent_stream(&job.id),
            0,
            SUBAGENT_CREATED,
            actor,
            &job.session_id,
            json!({"record": &job}),
        ))?;
        Ok(job)
    }

    fn update_subagent(&self, job: SubagentJob, actor: Actor) -> Result<SubagentJob, StoreError> {
        validate_subagent(&job)?;
        let current = self
            .get_subagent(&job.id)?
            .ok_or_else(|| StoreError::NotFound(format!("subagent {}", job.id)))?;
        let transition_valid = matches!(
            (current.status, job.status),
            (
                SubagentStatus::Queued,
                SubagentStatus::Running | SubagentStatus::Cancelled
            ) | (
                SubagentStatus::Running,
                SubagentStatus::Completed
                    | SubagentStatus::Failed
                    | SubagentStatus::Cancelled
                    | SubagentStatus::Interrupted
            ) | (
                SubagentStatus::Failed | SubagentStatus::Cancelled | SubagentStatus::Interrupted,
                SubagentStatus::Queued
            )
        );
        if !transition_valid
            || current.session_id != job.session_id
            || current.parent_run_id != job.parent_run_id
            || current.parent_call_id != job.parent_call_id
            || current.task != job.task
            || current.role != job.role
            || current.allowed_tools != job.allowed_tools
            || current.child_session_id != job.child_session_id
            || current.created_at != job.created_at
        {
            return Err(StoreError::Adapter(
                "invalid subagent transition or changed immutable provenance".into(),
            ));
        }
        let stream = Self::subagent_stream(&job.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            SUBAGENT_UPDATED,
            actor,
            &job.session_id,
            json!({"record": &job}),
        ))?;
        Ok(job)
    }

    fn get_subagent(&self, id: &str) -> Result<Option<SubagentJob>, StoreError> {
        self.record(&Self::subagent_stream(id), SUBAGENT_CREATED)
    }

    fn list_subagents(
        &self,
        session_id: Option<&str>,
        status: Option<SubagentStatus>,
        limit: usize,
    ) -> Result<Vec<SubagentJob>, StoreError> {
        let mut records = self
            .ids("subagent:")?
            .into_iter()
            .filter_map(|id| self.get_subagent(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }
}
