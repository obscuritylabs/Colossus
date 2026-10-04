//! Atomic receipts and canonical control transitions; called under the service writer.
use super::*;

impl WorkflowService {
    pub(super) fn control_mutation(
        &self,
        operation: &WorkflowControlOperation,
        actor: Actor,
        origin: WorkflowOrigin,
    ) -> Result<Value, WorkflowError> {
        let key = match operation {
            WorkflowControlOperation::RegisterDefinition {
                idempotency_key, ..
            }
            | WorkflowControlOperation::CreateSchedule {
                idempotency_key, ..
            }
            | WorkflowControlOperation::StartRun {
                idempotency_key, ..
            } => Some(idempotency_key.as_str()),
            _ => None,
        };
        let fingerprint = hex::encode(Sha256::digest(
            serde_json::to_vec(operation).map_err(control_encoding)?,
        ));
        let receipt_stream = key
            .map(|key| -> Result<String, WorkflowError> {
                token(key)?;
                let identity =
                    serde_json::to_vec(&(&origin.owner, key)).map_err(control_encoding)?;
                Ok(format!(
                    "workflow-control-request:{}",
                    hex::encode(Sha256::digest(identity))
                ))
            })
            .transpose()?;
        if let Some(stream) = &receipt_stream
            && let Some(receipt) = self.journal.read_stream(stream)?.first()
        {
            let payload = self.journal.decrypt_payload(receipt)?;
            if payload.get("fingerprint").and_then(Value::as_str) != Some(&fingerprint) {
                return Err(WorkflowError::Conflict(
                    "idempotency key reused for different intent".into(),
                ));
            }
            let mut result = payload.get("result").cloned().ok_or_else(|| {
                StoreError::Verification("workflow control receipt is incomplete".into())
            })?;
            if result.get("etag").is_some() {
                let id = result
                    .pointer("/record/schedule_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::Verification("schedule receipt identity is absent".into())
                    })?;
                let creation = self
                    .journal
                    .read_stream(&schedule_stream(id))?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        StoreError::Verification("schedule receipt has no allocation".into())
                    })?;
                result["etag"] = json!(creation.record_hash);
            }
            return Ok(result);
        }
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let mut events = Vec::new();
        let mut result = match operation {
            WorkflowControlOperation::RegisterDefinition {
                yaml,
                expected_hash,
                ..
            } => {
                let validated = validate_definition(yaml)?;
                if validated.definition.metadata.scheduled_task {
                    return Err(WorkflowError::InvalidDefinition(
                        "scheduled tasks are allocated through schedules".into(),
                    ));
                }
                let value = metadata(&validated.definition, &validated.content_hash)?;
                if expected_hash != &validated.content_hash {
                    return Err(WorkflowError::Conflict(
                        "reviewed definition bytes changed".into(),
                    ));
                }
                validate_call_graph(self.repository.as_ref(), &validated.definition, false)?;
                match self.repository.definition(&value.name, &value.version)? {
                    Some((_, hash)) if hash != validated.content_hash => {
                        return Err(WorkflowError::Conflict(
                            "this version already exists; import a new workflow version".into(),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        if self.control_streams("workflow-definition:")?.len()
                            >= MAX_WORKFLOW_SCHEDULES
                        {
                            return Err(WorkflowError::InvalidTransition(
                                "workflow catalog limit reached".into(),
                            ));
                        }
                        events.push(control_event(format!("workflow-definition:{}", value.workflow_id), 0, "workflow.definition.registered.v1", &actor, &origin, json!({"definition": validated.definition, "content_hash": validated.content_hash, "provenance": "authenticated-workflow-import", "origin": &origin, "trust_invalidated": false})));
                    }
                }
                serde_json::to_value(value).map_err(control_encoding)?
            }
            WorkflowControlOperation::CreateSchedule {
                schedule_id,
                workflow_id,
                expected_hash,
                inputs,
                cadence_seconds,
                calendar,
                task,
                starts_at,
                misfire_policy,
                enabled,
                ..
            } => {
                if schedule_id.is_empty()
                    || schedule_id.len() > MAX_SCHEDULE_ID_BYTES
                    || !valid_name(schedule_id)
                {
                    return Err(WorkflowError::InvalidDefinition(
                        "schedule IDs use 1..=128 lowercase letters, digits, dots, or hyphens"
                            .into(),
                    ));
                }
                if calendar.is_some() && *cadence_seconds != 0 {
                    return Err(WorkflowError::InvalidDefinition(
                        "calendar and elapsed cadence are exclusive".into(),
                    ));
                }
                if calendar.is_none()
                    && !(MIN_SCHEDULE_CADENCE_SECONDS..=MAX_SCHEDULE_CADENCE_SECONDS)
                        .contains(cadence_seconds)
                {
                    return Err(WorkflowError::InvalidDefinition(
                        "cadence must be 60 seconds through 31 days".into(),
                    ));
                }
                if self.repository.schedule(schedule_id)?.is_some() {
                    return Err(WorkflowError::Conflict("schedule ID already exists".into()));
                }
                if self.control_streams("workflow-schedule:")?.len() >= MAX_WORKFLOW_SCHEDULES {
                    return Err(WorkflowError::InvalidTransition(
                        "schedule limit reached".into(),
                    ));
                }
                let metadata = if let Some(task) = task {
                    if !workflow_id.is_empty() || !expected_hash.is_empty() || inputs != &json!({})
                    {
                        return Err(WorkflowError::InvalidDefinition(
                            "task and existing workflow selection are exclusive".into(),
                        ));
                    }
                    if task.name.trim().is_empty()
                        || task.name.len() > 128
                        || task.instructions.trim().is_empty()
                        || task.instructions.len() > 64 * 1024
                        || task.tools.len() > 128
                        || task
                            .tools
                            .iter()
                            .any(|tool| tool.is_empty() || tool.len() > 128)
                        || task
                            .options
                            .model_profile
                            .as_ref()
                            .is_some_and(|profile| profile.is_empty() || profile.len() > 128)
                    {
                        return Err(WorkflowError::InvalidDefinition(
                            "task name, instructions, or configured model exceeds its bounds"
                                .into(),
                        ));
                    }
                    if self.control_streams("workflow-definition:")?.len() >= MAX_WORKFLOW_SCHEDULES
                    {
                        return Err(WorkflowError::InvalidTransition(
                            "workflow catalog limit reached".into(),
                        ));
                    }
                    let name = format!("scheduled-task-{}", Uuid::now_v7());
                    let validated = crate::validation::validate_task_definition(
                        json!({"apiVersion": "colossus.dev/v1alpha1", "kind": "Workflow", "metadata": {"name": name, "version": "1.0.0", "description": "Scheduled task", "scheduled_task": true}, "inputs": {"type":"object", "additionalProperties":false}, "outputs": {}, "capabilities": task.tools, "maxConcurrency":1, "stepBudget":1, "steps":[{"type":"agent", "id":"task", "prompt":task.instructions, "options":task.options, "idempotency":null}]}),
                    )?;
                    let value = metadata(&validated.definition, &validated.content_hash)?;
                    events.push(control_event(format!("workflow-definition:{}", value.workflow_id), 0, "workflow.definition.registered.v1", &actor, &origin, json!({"definition": validated.definition, "content_hash": validated.content_hash, "provenance": "authenticated-scheduled-task", "origin": &origin, "trust_invalidated": false})));
                    value
                } else {
                    let value = self.control_workflow(workflow_id, Some(expected_hash))?;
                    let (definition, _) = self
                        .repository
                        .definition(&value.name, &value.version)?
                        .ok_or_else(|| WorkflowError::NotFound("registered workflow".into()))?;
                    validate_call_graph(self.repository.as_ref(), &definition, true)?;
                    validate_instance(&definition.inputs, inputs, "schedule input")?;
                    value
                };
                let starts_at =
                    format_schedule_time(parse_schedule_time(starts_at, "schedule start")?)?;
                let record = WorkflowSchedule {
                    schedule_id: schedule_id.clone(),
                    workflow_name: metadata.name,
                    workflow_version: metadata.version,
                    workflow_hash: metadata.workflow_hash,
                    inputs: inputs.clone(),
                    cadence_seconds: *cadence_seconds,
                    calendar: calendar.clone(),
                    task: task.as_deref().cloned(),
                    misfire_policy: *misfire_policy,
                    enabled: *enabled,
                    starts_at: starts_at.clone(),
                    next_fire_at: starts_at,
                    last_scheduled_at: None,
                    last_run_id: None,
                    blocked_reason: None,
                    created_at: now.clone(),
                    updated_at: now,
                };
                events.push(control_event(
                    schedule_stream(schedule_id),
                    0,
                    "workflow.schedule.registered.v1",
                    &actor,
                    &origin,
                    json!({"record": &record, "origin": &origin}),
                ));
                serde_json::to_value(WorkflowScheduleSnapshot {
                    record,
                    origin: Some(origin.clone()),
                    etag: String::new(),
                    controllable: true,
                    last_dispatch: None,
                })
                .map_err(control_encoding)?
            }
            WorkflowControlOperation::SetScheduleEnabled {
                schedule_id,
                enabled,
                etag,
            } => {
                let mut snapshot = self.control_schedule(schedule_id, &origin.owner)?;
                if !snapshot.controllable {
                    return Err(WorkflowError::PermissionDenied);
                }
                if etag != &snapshot.etag {
                    return Err(WorkflowError::Conflict(
                        "schedule changed; refresh before reviewing control".into(),
                    ));
                }
                if *enabled {
                    let id = format!(
                        "{}:{}",
                        snapshot.record.workflow_name, snapshot.record.workflow_version
                    );
                    self.control_workflow(&id, Some(&snapshot.record.workflow_hash))?;
                    let (definition, _) = self
                        .repository
                        .definition(
                            &snapshot.record.workflow_name,
                            &snapshot.record.workflow_version,
                        )?
                        .ok_or_else(|| WorkflowError::NotFound("registered workflow".into()))?;
                    validate_call_graph(self.repository.as_ref(), &definition, true)?;
                    validate_instance(
                        &definition.inputs,
                        &snapshot.record.inputs,
                        "schedule input",
                    )?;
                }
                if snapshot.record.enabled == *enabled {
                    return serde_json::to_value(snapshot).map_err(control_encoding);
                }
                snapshot.record.enabled = *enabled;
                snapshot.record.updated_at = now;
                if *enabled {
                    snapshot.record.blocked_reason = None;
                }
                let version = self.schedule_version(schedule_id)?;
                events.push(control_event(
                    schedule_stream(schedule_id),
                    version,
                    if *enabled {
                        "workflow.schedule.enabled.v1"
                    } else {
                        "workflow.schedule.disabled.v1"
                    },
                    &actor,
                    &origin,
                    json!({"record": &snapshot.record}),
                ));
                snapshot.etag.clear();
                serde_json::to_value(snapshot).map_err(control_encoding)?
            }
            WorkflowControlOperation::StartRun {
                workflow_id,
                expected_hash,
                inputs,
                ..
            } => {
                let metadata = self.control_workflow(workflow_id, Some(expected_hash))?;
                let (definition, _) = self
                    .repository
                    .definition(&metadata.name, &metadata.version)?
                    .ok_or_else(|| WorkflowError::NotFound("registered workflow".into()))?;
                validate_call_graph(self.repository.as_ref(), &definition, true)?;
                validate_instance(&definition.inputs, inputs, "workflow input")?;
                let run_id = Uuid::now_v7().to_string();
                events.push(control_event(format!("workflow-run:{run_id}"), 0, "workflow.run.queued.v1", &actor, &origin, json!({"workflow_name": metadata.name, "workflow_version": metadata.version, "workflow_hash": metadata.workflow_hash, "inputs": inputs, "origin": &origin, "call_depth": 1})));
                json!({"run_id": run_id})
            }
            _ => {
                return Err(WorkflowError::InvalidTransition(
                    "unsupported workflow control mutation".into(),
                ));
            }
        };
        // The opaque etag is derived from the committed envelope, so receipts store an
        // allocation identity rather than an etag that could not exist before commit.
        if let Some(stream) = receipt_stream {
            events.push(control_event(
                stream,
                0,
                "workflow.control.receipt.v1",
                &actor,
                &origin,
                json!({"fingerprint": fingerprint, "result": &result}),
            ));
        }
        let committed = self.journal.append_batch(events)?;
        if result.get("etag").is_some() {
            result["etag"] = json!(
                committed
                    .iter()
                    .find(|event| event.stream_id.starts_with("workflow-schedule:"))
                    .map(|event| event.record_hash.clone())
                    .unwrap_or_default()
            );
        }
        Ok(result)
    }
}

fn control_event(
    stream_id: String,
    expected_stream_version: u64,
    event_type: &str,
    actor: &Actor,
    origin: &WorkflowOrigin,
    payload: Value,
) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id,
        expected_stream_version,
        classification: EventClassification::Workflow,
        event_type: event_type.into(),
        actor: actor.clone(),
        context: ExecutionContext {
            correlation_id: origin
                .run_id
                .clone()
                .unwrap_or_else(|| Uuid::now_v7().to_string()),
            session_id: origin.session_id.clone(),
            run_id: origin.run_id.clone(),
            ..ExecutionContext::default()
        },
        payload,
    }
}
