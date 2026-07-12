//! Canonical event-sourced tasks and future-facing key decisions.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    Actor, DecisionPriority, DecisionSource, DecisionStatus, EventClassification, ExecutionContext,
    GoalRecord, GoalStatus, KeyDecision, NewEvent, PlanRecord, PlanStatus, PlanStep, SubagentJob,
    SubagentStatus, TaskRecord, TaskStatus,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError, WorkRepository};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const TASK_CREATED: &str = "task.created.v1";
const TASK_UPDATED: &str = "task.updated.v1";
const DECISION_CREATED: &str = "decision.created.v1";
const DECISION_UPDATED: &str = "decision.updated.v1";
const DECISION_ARCHIVED: &str = "decision.archived.v1";
const DECISION_SUPERSEDED: &str = "decision.superseded.v1";
const PLAN_CREATED: &str = "plan.created.v1";
const PLAN_UPDATED: &str = "plan.updated.v1";
const PLAN_APPROVED: &str = "plan.approved.v1";
const PLAN_EXECUTED: &str = "plan.executed.v1";
const PLAN_DISCARDED: &str = "plan.discarded.v1";
const GOAL_CREATED: &str = "goal.created.v1";
const GOAL_UPDATED: &str = "goal.updated.v1";
const SUBAGENT_CREATED: &str = "subagent.queued.v1";
const SUBAGENT_UPDATED: &str = "subagent.status_changed.v1";
const MAX_TITLE_BYTES: usize = 512;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_PLAN_BYTES: usize = 256 * 1024;
const MAX_LIST: usize = 1_000;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_task(task: &TaskRecord) -> Result<(), StoreError> {
    if !valid_id(&task.id)
        || !valid_id(&task.session_id)
        || task.title.trim().is_empty()
        || task.title.len() > MAX_TITLE_BYTES
        || task.description.len() > MAX_TEXT_BYTES
        || task.created_at.is_empty()
        || task.updated_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid task identity, title, description, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_decision(decision: &KeyDecision) -> Result<(), StoreError> {
    let bounded = [
        &decision.decision,
        &decision.intent,
        &decision.applies_when,
        &decision.rationale,
        &decision.source_excerpt,
    ];
    if !valid_id(&decision.id)
        || !valid_id(&decision.session_id)
        || decision.title.trim().is_empty()
        || decision.title.len() > MAX_TITLE_BYTES
        || decision.decision.trim().is_empty()
        || bounded.iter().any(|value| value.len() > MAX_TEXT_BYTES)
        || decision.created_at.is_empty()
        || decision.updated_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid key-decision identity, content, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_plan(plan: &PlanRecord) -> Result<(), StoreError> {
    let total_bytes = plan.steps.iter().fold(
        plan.prompt.len().saturating_add(plan.content.len()),
        |total, step| {
            total
                .saturating_add(step.title.len())
                .saturating_add(step.detail.len())
        },
    );
    let ordered = !plan.steps.is_empty()
        && plan.steps.len() <= 100
        && plan.steps.iter().enumerate().all(|(index, step)| {
            step.index == u32::try_from(index + 1).unwrap_or(u32::MAX)
                && !step.title.trim().is_empty()
                && step.title.len() <= MAX_TITLE_BYTES
                && step.detail.len() <= MAX_TEXT_BYTES
        });
    let lifecycle_valid = match plan.status {
        PlanStatus::Draft => plan.approved_at.is_none() && plan.executed_run_id.is_none(),
        PlanStatus::Approved => plan.approved_at.is_some() && plan.executed_run_id.is_none(),
        PlanStatus::Executed => plan.approved_at.is_some() && plan.executed_run_id.is_some(),
        PlanStatus::Discarded => plan.executed_run_id.is_none(),
    };
    if !valid_id(&plan.id)
        || !valid_id(&plan.session_id)
        || plan.prompt.trim().is_empty()
        || plan.prompt.len() > MAX_TEXT_BYTES
        || plan.content.len() > MAX_TEXT_BYTES
        || total_bytes > MAX_PLAN_BYTES
        || plan.created_at.is_empty()
        || plan.updated_at.is_empty()
        || !ordered
        || !lifecycle_valid
    {
        return Err(StoreError::Adapter(
            "invalid plan identity, content, steps, lifecycle, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_goal(goal: &GoalRecord) -> Result<(), StoreError> {
    let terminal_valid = match goal.status {
        GoalStatus::Active => goal.blocked_reason.is_empty(),
        GoalStatus::Complete => !goal.summary.trim().is_empty() && goal.blocked_reason.is_empty(),
        GoalStatus::Blocked => !goal.blocked_reason.trim().is_empty(),
    };
    if !valid_id(&goal.id)
        || !valid_id(&goal.session_id)
        || goal.objective.trim().is_empty()
        || goal.objective.len() > MAX_TEXT_BYTES
        || goal.summary.len() > MAX_TEXT_BYTES
        || goal.blocked_reason.len() > MAX_TEXT_BYTES
        || !(1..=50).contains(&goal.iteration_budget)
        || goal.iterations_completed > goal.iteration_budget
        || goal.created_at.is_empty()
        || goal.updated_at.is_empty()
        || !terminal_valid
    {
        return Err(StoreError::Adapter(
            "invalid goal identity, objective, budget, lifecycle, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_subagent(job: &SubagentJob) -> Result<(), StoreError> {
    let lifecycle_valid = match job.status {
        SubagentStatus::Queued => {
            job.child_run_id.is_none()
                && job.started_at.is_none()
                && job.completed_at.is_none()
                && job.final_output.is_empty()
                && job.error.is_empty()
        }
        SubagentStatus::Running => job.started_at.is_some() && job.completed_at.is_none(),
        SubagentStatus::Completed => {
            job.started_at.is_some()
                && job.completed_at.is_some()
                && job.child_run_id.is_some()
                && job.error.is_empty()
        }
        SubagentStatus::Failed | SubagentStatus::Cancelled | SubagentStatus::Interrupted => {
            job.completed_at.is_some() && !job.error.trim().is_empty()
        }
    };
    if !valid_id(&job.id)
        || !valid_id(&job.session_id)
        || !valid_id(&job.parent_run_id)
        || !valid_id(&job.parent_call_id)
        || !valid_id(&job.child_session_id)
        || job.task.trim().is_empty()
        || job.task.len() > MAX_TEXT_BYTES
        || job.role.trim().is_empty()
        || job.role.len() > 128
        || job.final_output.len() > MAX_TEXT_BYTES
        || job.error.len() > MAX_TEXT_BYTES
        || job.created_at.is_empty()
        || job.updated_at.is_empty()
        || !lifecycle_valid
    {
        return Err(StoreError::Adapter(
            "invalid subagent identity, task, result, lifecycle, or timestamp".into(),
        ));
    }
    Ok(())
}

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

    fn ids(&self, prefix: &str, created_event: &str) -> Result<BTreeSet<String>, StoreError> {
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if event.event_type == created_event
                    && let Some(id) = event.stream_id.strip_prefix(prefix)
                {
                    ids.insert(id.into());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 {
                break;
            }
        }
        Ok(ids)
    }

    fn record<T: serde::de::DeserializeOwned>(
        &self,
        stream_id: &str,
        expected_created_event: &str,
    ) -> Result<Option<T>, StoreError> {
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
        let payload = self.journal.decrypt_payload(last)?;
        serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("work record is absent".into()))?,
        )
        .map(Some)
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
            .ids("task:", TASK_CREATED)?
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
            .ids("decision:", DECISION_CREATED)?
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
        if plan.status != PlanStatus::Draft {
            return Err(StoreError::Adapter("new plans must be drafts".into()));
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

    fn update_plan(&self, plan: PlanRecord, actor: Actor) -> Result<PlanRecord, StoreError> {
        validate_plan(&plan)?;
        let current = self
            .get_plan(&plan.id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {}", plan.id)))?;
        if current.session_id != plan.session_id || current.created_at != plan.created_at {
            return Err(StoreError::Adapter(
                "plan session and creation timestamp are immutable".into(),
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
            && (current.prompt != plan.prompt
                || current.content != plan.content
                || current.steps != plan.steps)
        {
            return Err(StoreError::Adapter(
                "plan content is immutable during lifecycle transitions".into(),
            ));
        }
        let stream = Self::plan_stream(&plan.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
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
            .ids("plan:", PLAN_CREATED)?
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
        executed_plan: PlanRecord,
        actor: Actor,
    ) -> Result<(GoalRecord, PlanRecord), StoreError> {
        validate_goal(&goal)?;
        validate_plan(&executed_plan)?;
        let plan_id = goal
            .source_plan_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("goal plan lineage is absent".into()))?;
        let current = self
            .get_plan(plan_id)?
            .ok_or_else(|| StoreError::NotFound(format!("plan {plan_id}")))?;
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
        let plan_stream = Self::plan_stream(plan_id);
        let expected =
            u64::try_from(self.journal.read_stream(&plan_stream)?.len()).map_err(adapter)?;
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
        let current = self
            .get_goal(&goal.id)?
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
            || goal.iterations_completed < current.iterations_completed
            || goal.iterations_completed > current.iterations_completed.saturating_add(1)
        {
            return Err(StoreError::Adapter(
                "goal provenance, budget, terminal state, and iteration progression are immutable"
                    .into(),
            ));
        }
        let stream = Self::goal_stream(&goal.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
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
            .ids("goal:", GOAL_CREATED)?
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
            .ids("subagent:", SUBAGENT_CREATED)?
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

/// Validated application service shared by CLI, REPL, tools, and embedded callers.
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
        session_id: &str,
        parent_run_id: &str,
        parent_call_id: &str,
        task: &str,
        role: &str,
        actor: Actor,
    ) -> Result<SubagentJob, StoreError> {
        self.require_session(session_id)?;
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
                session_id: session_id.into(),
                parent_run_id: parent_run_id.into(),
                parent_call_id: parent_call_id.into(),
                task: task.trim().into(),
                role: role.into(),
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
fn user_actor() -> Actor {
    Actor {
        actor_type: colossus_contracts::ActorType::User,
        id: "terminal-user".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::{InMemoryEventJournal, assert_work_repository_conformance};

    #[test]
    fn event_sourced_work_repository_passes_shared_conformance() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        assert_work_repository_conformance(|| {
            Box::new(EventSourcedWorkRepository::new(Arc::clone(&journal)))
        });
    }

    fn fixture() -> (Arc<dyn EventJournal>, Arc<dyn WorkRepository>, WorkService) {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        sessions
            .create_session("session-1", Some("work"), user_actor())
            .expect("session");
        let repository: Arc<dyn WorkRepository> =
            Arc::new(EventSourcedWorkRepository::new(Arc::clone(&journal)));
        let service = WorkService::new(Arc::clone(&repository), sessions);
        (journal, repository, service)
    }

    #[test]
    fn tasks_reconstruct_after_updates_and_repository_restart() {
        let (journal, repository, service) = fixture();
        let created = service
            .create_task(
                "session-1",
                "Implement work state",
                "Use immutable events",
                TaskStatus::Pending,
                user_actor(),
            )
            .expect("create");
        let updated = service
            .update_task(
                &created.id,
                None,
                Some("Repository and projections"),
                Some(TaskStatus::InProgress),
                user_actor(),
            )
            .expect("update");
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(
            repository
                .list_tasks(Some("session-1"), None, 10)
                .expect("list")
                .len(),
            1
        );

        let reopened = EventSourcedWorkRepository::new(journal);
        assert_eq!(
            reopened.get_task(&created.id).expect("get").expect("task"),
            updated
        );
    }

    #[test]
    fn active_decisions_update_archive_and_filter_without_deletion() {
        let (_journal, repository, service) = fixture();
        let created = service
            .create_decision(
                "session-1",
                "Audit first",
                "All durable mutations use immutable events.",
                DecisionSource::User,
                DecisionPriority::Critical,
                "Preserve evidence",
                "When changing canonical state",
                "Auditability is foundational",
                "I want auditing from the ground up",
                None,
                None,
                None,
                user_actor(),
            )
            .expect("create");
        let updated = service
            .update_decision(
                &created.id,
                None,
                Some("All state changes use immutable canonical events."),
                Some(DecisionPriority::High),
                None,
                None,
                None,
                None,
                user_actor(),
            )
            .expect("update");
        assert_eq!(updated.priority, DecisionPriority::High);
        assert_eq!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active")
                .len(),
            1
        );
        let archived = service
            .archive_decision(&created.id, user_actor())
            .expect("archive");
        assert_eq!(archived.status, DecisionStatus::Archived);
        assert!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active")
                .is_empty()
        );
        assert_eq!(
            repository
                .get_decision(&created.id)
                .expect("get")
                .expect("decision")
                .status,
            DecisionStatus::Archived
        );
    }

    #[test]
    fn supersession_is_atomic_and_preserves_lineage() {
        let (journal, repository, service) = fixture();
        let old = service
            .create_decision(
                "session-1",
                "Storage",
                "Use SQLite.",
                DecisionSource::User,
                DecisionPriority::Normal,
                "Keep state local",
                "During the Python implementation",
                "Legacy choice",
                "SQLite at first",
                None,
                None,
                None,
                user_actor(),
            )
            .expect("old");
        let (superseded, replacement) = service
            .supersede_decision(
                &old.id,
                "Storage",
                "Use replaceable repository ports with redb as the initial canonical adapter.",
                DecisionSource::User,
                DecisionPriority::Critical,
                "Keep storage replaceable",
                "For all Rust canonical state",
                "Supports redb, PostgreSQL, and indexes",
                "abstract layer like a Repo pattern",
                user_actor(),
            )
            .expect("supersede");
        assert_eq!(superseded.status, DecisionStatus::Superseded);
        assert_eq!(replacement.supersedes.as_deref(), Some(old.id.as_str()));
        assert_eq!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active"),
            vec![replacement.clone()]
        );
        let stream = journal
            .read_stream(&format!("decision:{}", old.id))
            .expect("old stream");
        assert_eq!(stream.last().expect("last").event_type, DECISION_SUPERSEDED);
        assert_eq!(
            journal
                .read_stream(&format!("decision:{}", replacement.id))
                .expect("replacement stream")
                .len(),
            1
        );
    }

    #[test]
    fn plans_reconstruct_and_enforce_single_execution_lifecycle() {
        let (journal, repository, service) = fixture();
        let steps = vec![PlanStep {
            index: 1,
            title: "Implement".into(),
            detail: "Make the scoped Rust change.".into(),
            requires_mutation: true,
        }];
        let draft = service
            .create_plan(
                "session-1",
                "Finish the Rust transition",
                "# Plan",
                steps,
                user_actor(),
            )
            .expect("create");
        let edited = service
            .update_draft_plan(&draft.id, None, Some("# Updated plan"), None, user_actor())
            .expect("edit");
        let approved = service
            .approve_plan(&draft.id, user_actor())
            .expect("approve");
        assert_eq!(approved.status, PlanStatus::Approved);
        assert!(approved.approved_at.is_some());
        assert!(
            service
                .update_draft_plan(&draft.id, Some("changed"), None, None, user_actor())
                .is_err()
        );
        let mut forged = approved.clone();
        forged.status = PlanStatus::Executed;
        forged.approved_at = Some("forged".into());
        forged.executed_run_id = Some("run-forged".into());
        assert!(repository.update_plan(forged, user_actor()).is_err());
        let executed = service
            .execute_plan(&draft.id, "run-1", user_actor())
            .expect("execute");
        assert_eq!(executed.status, PlanStatus::Executed);
        assert_eq!(executed.executed_run_id.as_deref(), Some("run-1"));
        assert!(
            service
                .execute_plan(&draft.id, "run-2", user_actor())
                .is_err()
        );
        assert_eq!(
            repository
                .list_plans(Some("session-1"), Some(PlanStatus::Executed), 10)
                .expect("list"),
            vec![executed.clone()]
        );
        let reopened = EventSourcedWorkRepository::new(journal);
        assert_eq!(reopened.get_plan(&draft.id).expect("get"), Some(executed));
        assert_eq!(edited.content, "# Updated plan");
    }

    #[test]
    fn goals_reconstruct_enforce_budget_and_preserve_terminal_evidence() {
        let (journal, repository, service) = fixture();
        let goal = service
            .create_goal(
                "session-1",
                "Complete the Rust transition",
                2,
                None,
                user_actor(),
            )
            .expect("create");
        let first = service
            .record_goal_iteration(&goal.id, user_actor())
            .expect("iteration");
        assert_eq!(first.iterations_completed, 1);
        let complete = service
            .update_goal_status(
                &goal.id,
                GoalStatus::Complete,
                "Transition verified.",
                "",
                user_actor(),
            )
            .expect("complete");
        let final_goal = service
            .record_goal_iteration(&goal.id, user_actor())
            .expect("terminal iteration");
        assert_eq!(final_goal.status, GoalStatus::Complete);
        assert_eq!(final_goal.iterations_completed, 2);
        assert_eq!(final_goal.summary, "Transition verified.");
        assert!(
            service
                .record_goal_iteration(&goal.id, user_actor())
                .is_err()
        );
        assert!(
            service
                .update_goal_status(&goal.id, GoalStatus::Blocked, "", "late", user_actor(),)
                .is_err()
        );
        assert_eq!(
            repository
                .list_goals(Some("session-1"), Some(GoalStatus::Complete), 10)
                .expect("list"),
            vec![final_goal.clone()]
        );
        let reopened = EventSourcedWorkRepository::new(journal);
        assert_eq!(reopened.get_goal(&goal.id).expect("get"), Some(final_goal));
        assert_eq!(complete.iterations_completed, 1);
    }

    #[test]
    fn approved_plan_is_atomically_consumed_by_only_one_goal() {
        let (journal, repository, service) = fixture();
        let plan = service
            .create_plan(
                "session-1",
                "Ship Rust",
                "# Approved",
                vec![PlanStep {
                    index: 1,
                    title: "Verify".into(),
                    detail: String::new(),
                    requires_mutation: false,
                }],
                user_actor(),
            )
            .expect("plan");
        service
            .approve_plan(&plan.id, user_actor())
            .expect("approve");
        let goal = service
            .create_goal(
                "session-1",
                "Execute approved plan",
                5,
                Some(plan.id.clone()),
                user_actor(),
            )
            .expect("goal");
        let consumed = repository
            .get_plan(&plan.id)
            .expect("plan")
            .expect("record");
        assert_eq!(consumed.status, PlanStatus::Executed);
        assert_eq!(consumed.executed_run_id.as_deref(), Some(goal.id.as_str()));
        assert_eq!(goal.source_plan_id.as_deref(), Some(plan.id.as_str()));
        assert!(
            service
                .create_goal(
                    "session-1",
                    "Duplicate",
                    5,
                    Some(plan.id.clone()),
                    user_actor(),
                )
                .is_err()
        );
        let plan_events = journal
            .read_stream(&format!("plan:{}", plan.id))
            .expect("plan events");
        assert_eq!(plan_events.last().expect("event").event_type, PLAN_EXECUTED);
    }

    #[test]
    fn subagents_reconstruct_and_enforce_terminal_requeue_transitions() {
        let (journal, repository, service) = fixture();
        let queued = service
            .create_subagent(
                "session-1",
                "run-1",
                "call-1",
                "Review the tests",
                "subagent_default",
                user_actor(),
            )
            .expect("queue");
        let running = service
            .start_subagent(&queued.id, user_actor())
            .expect("start");
        assert_eq!(running.status, SubagentStatus::Running);
        let failed = service
            .stop_subagent(
                &queued.id,
                SubagentStatus::Failed,
                "provider failed",
                user_actor(),
            )
            .expect("fail");
        assert_eq!(failed.status, SubagentStatus::Failed);
        let requeued = service
            .requeue_subagent(&queued.id, user_actor())
            .expect("requeue");
        assert_eq!(requeued.status, SubagentStatus::Queued);
        assert!(requeued.error.is_empty());
        service
            .start_subagent(&queued.id, user_actor())
            .expect("restart");
        let completed = service
            .complete_subagent(&queued.id, "child-run", "done", user_actor())
            .expect("complete");
        assert_eq!(completed.status, SubagentStatus::Completed);
        assert!(service.requeue_subagent(&queued.id, user_actor()).is_err());
        assert_eq!(
            repository
                .list_subagents(Some("session-1"), Some(SubagentStatus::Completed), 10)
                .expect("list"),
            vec![completed.clone()]
        );
        let reopened = EventSourcedWorkRepository::new(journal);
        assert_eq!(
            reopened.get_subagent(&queued.id).expect("get"),
            Some(completed)
        );
        let cancellable = service
            .create_subagent(
                "session-1",
                "run-2",
                "call-2",
                "Long child task",
                "subagent_default",
                user_actor(),
            )
            .expect("queue cancellable");
        service
            .start_subagent(&cancellable.id, user_actor())
            .expect("start cancellable");
        let cancelled = service
            .stop_subagent(
                &cancellable.id,
                SubagentStatus::Cancelled,
                "operator cancelled",
                user_actor(),
            )
            .expect("cancel");
        assert_eq!(cancelled.status, SubagentStatus::Cancelled);
        assert!(
            service
                .complete_subagent(&cancellable.id, "late-run", "late output", user_actor())
                .is_err()
        );
    }
}
