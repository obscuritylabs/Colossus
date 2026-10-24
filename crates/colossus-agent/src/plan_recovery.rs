use super::*;

const MAX_RECOVERY_RUN_EVENTS: usize = 65_536;

pub(super) struct RecoveredPlan {
    pub(super) target: PlanDraftTarget,
    pub(super) written: Option<PlanRecord>,
}

impl AgentService {
    /// Recover application-owned Plan progress independently of private provider replay.
    pub(super) fn recover_unfinished_plan(
        &self,
        session_id: &str,
        requested: &AgentRunMode,
    ) -> Result<Option<RecoveredPlan>, AgentError> {
        let records = self.sessions.list_messages(session_id)?;
        let Some(assistant) = records
            .iter()
            .rev()
            .find(|record| record.message.role == ModelMessageRole::Assistant)
            .filter(|record| !record.message.tool_calls.is_empty())
        else {
            return Ok(None);
        };
        let stream_id = format!("run:{}", assistant.run_id);
        let Some(start) = self.journal.read_stream_from(&stream_id, 0, 1)?.pop() else {
            return Ok(None);
        };
        if start.event_type != "run.started.v1"
            || start.context.session_id.as_deref() != Some(session_id)
            || start.context.run_id.as_deref() != Some(assistant.run_id.as_str())
        {
            return Err(recovery_error());
        }
        let started = self.journal.decrypt_payload(&start)?;
        let Some(mode) = started.get("mode") else {
            return Ok(None);
        };
        let source_mode =
            serde_json::from_value::<AgentRunMode>(mode.clone()).map_err(|_| recovery_error())?;
        let AgentRunMode::Plan(target) = source_mode else {
            return Ok(None);
        };
        validate_model_transcript(
            &records
                .iter()
                .map(|record| record.message.clone())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| recovery_error())?;
        if self.sessions.pending_tool_turn(session_id)?.is_some() {
            return Err(recovery_error());
        }

        let mut written = started
            .get("recovered_plan")
            .filter(|value| !value.is_null())
            .cloned();
        let mut after = start.stream_version;
        let mut inspected = 1_usize;
        loop {
            let page = self.journal.read_stream_from(&stream_id, after, 128)?;
            if page.is_empty() {
                break;
            }
            inspected = inspected.saturating_add(page.len());
            if inspected > MAX_RECOVERY_RUN_EVENTS {
                return Err(recovery_error());
            }
            for event in page {
                after = event.stream_version;
                if event.context.session_id.as_deref() != Some(session_id)
                    || event.context.run_id.as_deref() != Some(assistant.run_id.as_str())
                {
                    return Err(recovery_error());
                }
                if event.event_type == "plan.written.v1" {
                    if written.is_some() {
                        return Err(recovery_error());
                    }
                    written = Some(self.journal.decrypt_payload(&event)?);
                }
            }
        }

        let written = written
            .map(|identity| -> Result<PlanRecord, AgentError> {
                let id = identity["plan_id"].as_str().ok_or_else(recovery_error)?;
                let revision = identity["revision"].as_u64().ok_or_else(recovery_error)?;
                let valid_target = match &target {
                    PlanDraftTarget::Create => revision == 1,
                    PlanDraftTarget::Update {
                        plan_id,
                        revision: before,
                    } => id == plan_id && before.checked_add(1) == Some(revision),
                };
                if !valid_target {
                    return Err(recovery_error());
                }
                let repository = self.work.as_ref().ok_or_else(recovery_error)?;
                let plan = repository.get_plan(id)?.ok_or_else(recovery_error)?;
                if plan.session_id != session_id
                    || plan.revision != revision
                    || plan.status != colossus_contracts::PlanStatus::Draft
                {
                    return Err(recovery_error());
                }
                Ok(plan)
            })
            .transpose()?;
        let compatible = match requested {
            AgentRunMode::Plan(requested) if requested == &target => true,
            AgentRunMode::Plan(PlanDraftTarget::Update { plan_id, revision }) => written
                .as_ref()
                .is_some_and(|plan| plan.id == *plan_id && plan.revision == *revision),
            _ => false,
        };
        if !compatible {
            return Err(AgentError::Configuration(
                "finish the interrupted Plan exchange before selecting another plan or run mode"
                    .into(),
            ));
        }
        Ok(Some(RecoveredPlan { target, written }))
    }
}

fn recovery_error() -> AgentError {
    AgentError::Configuration(
        "interrupted Plan progress no longer matches its canonical draft and committed tool transcript".into(),
    )
}
