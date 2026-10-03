//! Caller-bound scheduling, with all preconditions checked under the tick writer lock.
use super::*;
use colossus_contracts::{
    RegisteredWorkflow, WorkflowControlOperation, WorkflowOrigin, WorkflowPage,
    WorkflowRunSnapshot, WorkflowScheduleSnapshot,
};

mod mutations;
#[cfg(test)]
mod tests;

const MAX_CONTROL_BYTES: usize = 256 * 1024;
const MAX_CONTROL_PAGE: usize = 100;

/// Normalize reviewed control intent before policy hashing or durable retry comparison.
pub fn normalize_control_operation(
    operation: &WorkflowControlOperation,
) -> Result<WorkflowControlOperation, WorkflowError> {
    let mut normalized = operation.clone();
    if let WorkflowControlOperation::CreateSchedule { starts_at, .. } = &mut normalized {
        if starts_at.len() > 64 {
            return Err(WorkflowError::InvalidDefinition(
                "schedule start exceeds timestamp bounds".into(),
            ));
        }
        let timestamp = OffsetDateTime::parse(starts_at, &Rfc3339)
            .map_err(|_| WorkflowError::InvalidDefinition("invalid schedule start".into()))?;
        *starts_at = format_schedule_time(timestamp.to_offset(UtcOffset::UTC))?;
    }
    if serde_json::to_vec(&normalized)
        .map_err(control_encoding)?
        .len()
        > MAX_CONTROL_BYTES
    {
        return Err(WorkflowError::InvalidDefinition(
            "workflow intent exceeds its bounds".into(),
        ));
    }
    Ok(normalized)
}

impl WorkflowService {
    /// Execute a bounded host-authorized intent. Origin must be constructed by the host.
    /// Mutations, their idempotency receipt, and reviewed preconditions share one commit.
    pub fn control(
        &self,
        operation: &WorkflowControlOperation,
        actor: Actor,
        origin: WorkflowOrigin,
    ) -> Result<Value, WorkflowError> {
        let normalized = normalize_control_operation(operation)?;
        let operation = &normalized;
        let bytes = serde_json::to_vec(operation).map_err(control_encoding)?;
        if bytes.len() > MAX_CONTROL_BYTES
            || origin.owner.id.is_empty()
            || origin.owner.id.len() > 256
            || origin.session_id.as_ref().is_some_and(|id| id.len() > 256)
            || origin.run_id.as_ref().is_some_and(|id| id.len() > 256)
        {
            return Err(WorkflowError::InvalidDefinition(
                "workflow control request exceeds its bounds".into(),
            ));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|_| StoreError::Adapter("workflow writer unavailable".into()))?;
        match operation {
            WorkflowControlOperation::ListWorkflows { after, limit } => {
                let limit = page_limit(*limit)?;
                if let Some(after) = after {
                    workflow_identity(after)?;
                }
                let ids = self.control_streams("workflow-definition:")?;
                if ids.len() > MAX_WORKFLOW_SCHEDULES {
                    return Err(WorkflowError::InvalidTransition(
                        "workflow catalog limit exceeded".into(),
                    ));
                }
                let mut items = Vec::new();
                for stream in ids {
                    let id = stream.trim_start_matches("workflow-definition:");
                    if after.as_ref().is_some_and(|after| id <= after.as_str()) {
                        continue;
                    }
                    let mut metadata = self.control_workflow(id, None)?;
                    metadata.input_schema = Value::Null;
                    items.push(metadata);
                    if items.len() > limit {
                        break;
                    }
                }
                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|item| item.workflow_id.clone())
                } else {
                    None
                };
                serde_json::to_value(WorkflowPage { items, next_cursor }).map_err(control_encoding)
            }
            WorkflowControlOperation::GetWorkflow { workflow_id } => {
                serde_json::to_value(self.control_workflow(workflow_id, None)?)
                    .map_err(control_encoding)
            }
            WorkflowControlOperation::ValidateDefinition { yaml } => {
                let validated = validate_definition(yaml)?;
                serde_json::to_value(metadata(&validated.definition, &validated.content_hash)?)
                    .map_err(control_encoding)
            }
            WorkflowControlOperation::ListSchedules { after, limit } => {
                let limit = page_limit(*limit)?;
                if let Some(after) = after {
                    token(after)?;
                }
                let mut items = Vec::new();
                let ids = self.control_streams("workflow-schedule:")?;
                if ids.len() > MAX_WORKFLOW_SCHEDULES {
                    return Err(WorkflowError::InvalidTransition(
                        "schedule catalog limit exceeded".into(),
                    ));
                }
                for stream in ids {
                    let id = stream.trim_start_matches("workflow-schedule:");
                    if after.as_ref().is_some_and(|after| id <= after.as_str()) {
                        continue;
                    }
                    let mut snapshot = match self.control_schedule(id, &origin.owner) {
                        Ok(snapshot) => snapshot,
                        Err(WorkflowError::PermissionDenied) => continue,
                        Err(error) => return Err(error),
                    };
                    snapshot.record.inputs = Value::Null;
                    items.push(snapshot);
                    if items.len() > limit {
                        break;
                    }
                }
                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|item| item.record.schedule_id.clone())
                } else {
                    None
                };
                serde_json::to_value(WorkflowPage { items, next_cursor }).map_err(control_encoding)
            }
            WorkflowControlOperation::GetSchedule { schedule_id } => {
                serde_json::to_value(self.control_schedule(schedule_id, &origin.owner)?)
                    .map_err(control_encoding)
            }
            WorkflowControlOperation::GetRun { run_id } => {
                token(run_id)?;
                let run = self.get_run(run_id)?;
                let first = self
                    .journal
                    .read_stream(&format!("workflow-run:{run_id}"))?
                    .into_iter()
                    .next()
                    .ok_or(WorkflowError::PermissionDenied)?;
                let payload = self.journal.decrypt_payload(&first)?;
                let owner = match payload.get("origin") {
                    Some(value) => Some(
                        serde_json::from_value::<WorkflowOrigin>(value.clone())
                            .map_err(control_encoding)?
                            .owner,
                    ),
                    None if run.trigger_kind == Some(WorkflowTriggerKind::Schedule) => {
                        let id = run
                            .trigger_id
                            .as_deref()
                            .ok_or(WorkflowError::PermissionDenied)?;
                        self.control_schedule(id, &origin.owner)?
                            .origin
                            .map(|origin| origin.owner)
                    }
                    None => None,
                };
                if owner.as_ref() != Some(&origin.owner) {
                    return Err(WorkflowError::PermissionDenied);
                }
                let events = self
                    .journal
                    .read_stream(&format!("workflow-run:{run_id}"))?;
                let last = events.last().ok_or(WorkflowError::PermissionDenied)?;
                serde_json::to_value(WorkflowRunSnapshot {
                    run_id: run.run_id,
                    workflow_id: format!("{}:{}", run.workflow_name, run.workflow_version),
                    workflow_hash: run.workflow_hash,
                    status: run.status,
                    created_at: first.occurred_at,
                    updated_at: last.occurred_at.clone(),
                    last_sequence: last.stream_version,
                    failure_reason: run
                        .failure_reason
                        .map(|_| "Workflow failed; inspect authorized runtime evidence.".into()),
                    waiting_reason: run
                        .waiting_reason
                        .map(|_| "Workflow is waiting for operator input or a dependency.".into()),
                })
                .map_err(control_encoding)
            }
            WorkflowControlOperation::ActiveWork => {
                // Retain the runtime if a bounded lifecycle inspection cannot prove
                // inactivity. Old waiting runs must not disappear behind a page cap.
                let ids = self.control_streams("workflow-run:")?;
                let active = ids.len() > MAX_WORKFLOW_SCHEDULES
                    || ids
                        .iter()
                        .map(|id| self.get_run(id.trim_start_matches("workflow-run:")))
                        .collect::<Result<Vec<_>, _>>()?
                        .iter()
                        .any(|run| {
                            matches!(
                                run.status,
                                WorkflowStatus::Queued
                                    | WorkflowStatus::Running
                                    | WorkflowStatus::Waiting
                            )
                        });
                Ok(json!({"active": active}))
            }
            _ => self.control_mutation(operation, actor, origin),
        }
    }

    fn control_streams(&self, prefix: &str) -> Result<Vec<String>, WorkflowError> {
        let mut ids = Vec::new();
        let mut after = None::<String>;
        while ids.len() <= MAX_WORKFLOW_SCHEDULES {
            let limit =
                colossus_ports::MAX_STREAM_LIST_BATCH.min(MAX_WORKFLOW_SCHEDULES + 1 - ids.len());
            let page = self
                .journal
                .list_stream_ids(prefix, after.as_deref(), limit)?;
            if page.len() > limit
                || page.iter().any(|id| !id.starts_with(prefix))
                || page.windows(2).any(|pair| pair[0] >= pair[1])
                || page
                    .first()
                    .is_some_and(|id| after.as_ref().is_some_and(|after| id <= after))
            {
                return Err(
                    StoreError::Verification("invalid workflow discovery page".into()).into(),
                );
            }
            if page.is_empty() {
                break;
            }
            after = page.last().cloned();
            ids.extend(page);
        }
        Ok(ids)
    }

    fn control_workflow(
        &self,
        id: &str,
        expected_hash: Option<&str>,
    ) -> Result<RegisteredWorkflow, WorkflowError> {
        let (name, version) = workflow_identity(id)?;
        let (definition, hash) = self
            .repository
            .definition(name, version)?
            .ok_or_else(|| WorkflowError::NotFound("registered workflow".into()))?;
        if expected_hash.is_some_and(|expected| expected != hash) {
            return Err(WorkflowError::Conflict(
                "reviewed workflow hash changed".into(),
            ));
        }
        let mut value = metadata(&definition, &hash)?;
        if validate_call_graph(self.repository.as_ref(), &definition, true).is_err() {
            value.scheduling_eligible = false;
            value.unavailable_reason = Some("Pinned definition or dependency trust is unavailable; restore the exact definition or register a new version.".into());
        }
        Ok(value)
    }

    fn control_schedule(
        &self,
        id: &str,
        owner: &Actor,
    ) -> Result<WorkflowScheduleSnapshot, WorkflowError> {
        token(id)?;
        let events = self.journal.read_stream(&schedule_stream(id))?;
        let first = events
            .first()
            .ok_or_else(|| WorkflowError::NotFound("schedule".into()))?;
        let last = events
            .last()
            .ok_or_else(|| WorkflowError::NotFound("schedule".into()))?;
        let payload = self.journal.decrypt_payload(first)?;
        let origin: Option<WorkflowOrigin> = payload
            .get("origin")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(control_encoding)?;
        if origin.as_ref().is_some_and(|origin| origin.owner != *owner) {
            return Err(WorkflowError::PermissionDenied);
        }
        let mut record = self.get_schedule(id)?;
        let controllable = origin.is_some();
        if !controllable {
            record.inputs = Value::Null;
            record.last_run_id = None;
        }
        let last_dispatch = events
            .iter()
            .rev()
            .find_map(|event| match event.event_type.as_str() {
                "workflow.schedule.fired.v1" => Some(WorkflowScheduleDispatchStatus::Queued),
                "workflow.schedule.skipped.v1" => Some(WorkflowScheduleDispatchStatus::Skipped),
                "workflow.schedule.blocked.v1" => Some(WorkflowScheduleDispatchStatus::Blocked),
                _ => None,
            });
        Ok(WorkflowScheduleSnapshot {
            record,
            origin,
            etag: last.record_hash.clone(),
            controllable,
            last_dispatch,
        })
    }
}

pub(super) fn control_encoding(error: serde_json::Error) -> WorkflowError {
    WorkflowError::Store(StoreError::Verification(error.to_string()))
}
fn page_limit(limit: usize) -> Result<usize, WorkflowError> {
    if (1..=MAX_CONTROL_PAGE).contains(&limit) {
        Ok(limit)
    } else {
        Err(WorkflowError::InvalidDefinition(
            "page size must be 1..=100".into(),
        ))
    }
}
fn token(id: &str) -> Result<(), WorkflowError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        Err(WorkflowError::InvalidDefinition(
            "invalid workflow control identifier".into(),
        ))
    } else {
        Ok(())
    }
}
fn workflow_identity(id: &str) -> Result<(&str, &str), WorkflowError> {
    token(id)?;
    let (name, version) = id.split_once(':').ok_or_else(|| {
        WorkflowError::InvalidDefinition("workflow identity must be name:version".into())
    })?;
    if name.is_empty() || version.is_empty() || !valid_name(name) || version.contains(':') {
        return Err(WorkflowError::InvalidDefinition(
            "invalid workflow identity".into(),
        ));
    }
    Ok((name, version))
}
fn metadata(
    definition: &WorkflowDefinition,
    hash: &str,
) -> Result<RegisteredWorkflow, WorkflowError> {
    let workflow_id = format!(
        "{}:{}",
        definition.metadata.name, definition.metadata.version
    );
    workflow_identity(&workflow_id)?;
    if definition.metadata.description.len() > 4096
        || serde_json::to_vec(&definition.inputs)
            .map_err(control_encoding)?
            .len()
            > 64 * 1024
    {
        return Err(WorkflowError::InvalidDefinition(
            "workflow metadata exceeds public bounds".into(),
        ));
    }
    Ok(RegisteredWorkflow {
        workflow_id,
        name: definition.metadata.name.clone(),
        version: definition.metadata.version.clone(),
        workflow_hash: hash.into(),
        description: definition.metadata.description.clone(),
        input_schema: definition.inputs.clone(),
        scheduling_eligible: true,
        unavailable_reason: None,
    })
}
