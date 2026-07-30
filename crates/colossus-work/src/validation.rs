use super::*;

pub(super) const TASK_CREATED: &str = "task.created.v1";
pub(super) const TASK_UPDATED: &str = "task.updated.v1";
pub(super) const DECISION_CREATED: &str = "decision.created.v1";
pub(super) const DECISION_UPDATED: &str = "decision.updated.v1";
pub(super) const DECISION_ARCHIVED: &str = "decision.archived.v1";
pub(super) const DECISION_SUPERSEDED: &str = "decision.superseded.v1";
pub(super) const PLAN_CREATED: &str = "plan.created.v1";
pub(super) const PLAN_UPDATED: &str = "plan.updated.v1";
pub(super) const PLAN_APPROVED: &str = "plan.approved.v1";
pub(super) const PLAN_EXECUTED: &str = "plan.executed.v1";
pub(super) const PLAN_DISCARDED: &str = "plan.discarded.v1";
pub(super) const GOAL_CREATED: &str = "goal.created.v1";
pub(super) const GOAL_UPDATED: &str = "goal.updated.v1";
pub(super) const SUBAGENT_CREATED: &str = "subagent.queued.v1";
pub(super) const SUBAGENT_UPDATED: &str = "subagent.status_changed.v1";
pub(super) const MAX_TITLE_BYTES: usize = 512;
pub(super) const MAX_TEXT_BYTES: usize = 64 * 1024;
pub(super) const MAX_PLAN_BYTES: usize = 256 * 1024;
pub(super) const MAX_LIST: usize = 1_000;

pub(super) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(super) fn require_plan_revision(
    plan: &PlanRecord,
    expected_revision: u64,
) -> Result<(), StoreError> {
    if plan.revision != expected_revision {
        return Err(StoreError::Conflict {
            stream_id: format!("plan:{}", plan.id),
            expected: expected_revision,
            actual: plan.revision,
        });
    }
    Ok(())
}

pub(super) fn validate_task(task: &TaskRecord) -> Result<(), StoreError> {
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

pub(super) fn validate_decision(decision: &KeyDecision) -> Result<(), StoreError> {
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

pub(super) fn validate_plan(plan: &PlanRecord) -> Result<(), StoreError> {
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

pub(super) fn validate_goal(goal: &GoalRecord) -> Result<(), StoreError> {
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

pub(super) fn validate_subagent(job: &SubagentJob) -> Result<(), StoreError> {
    let tools_valid = job.allowed_tools.as_ref().is_none_or(|tools| {
        let mut unique = BTreeSet::new();
        tools.len() <= 512
            && tools.iter().all(|tool| {
                !tool.is_empty()
                    && tool.len() <= 128
                    && tool.trim() == tool
                    && !tool.chars().any(char::is_control)
                    && unique.insert(tool)
            })
    });
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
        || !tools_valid
        || !lifecycle_valid
    {
        return Err(StoreError::Adapter(
            "invalid subagent identity, task, result, lifecycle, or timestamp".into(),
        ));
    }
    Ok(())
}
