use super::*;

fn upsert(key: impl Into<String>, value: Value) -> Vec<ProjectionMutation> {
    vec![ProjectionMutation::Upsert {
        key: key.into(),
        value,
    }]
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn merge_event(
    store: &dyn ProjectionStore,
    projection: &str,
    key: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Value, StoreError> {
    let mut current = store.get(projection, key)?.map(object).unwrap_or_default();
    if let Some(fields) = payload.as_object() {
        current.extend(fields.clone());
    } else {
        current.insert("payload".into(), payload.clone());
    }
    current.insert("id".into(), Value::String(key.into()));
    current.insert("stream_id".into(), Value::String(event.stream_id.clone()));
    current.insert("stream_version".into(), json!(event.stream_version));
    current.insert(
        "last_event_type".into(),
        Value::String(event.event_type.clone()),
    );
    current.insert(
        "updated_at".into(),
        Value::String(event.occurred_at.clone()),
    );
    Ok(Value::Object(current))
}

/// Session aggregate reducer.
pub struct SessionProjection;

impl ProjectionHandler for SessionProjection {
    fn name(&self) -> &'static str {
        "sessions-v1"
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.stream_id.starts_with("session:")
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let Some(id) = event.stream_id.strip_prefix("session:") else {
            return Ok(Vec::new());
        };
        let mut current = store.get(self.name(), id)?.map(object).unwrap_or_default();
        match event.event_type.as_str() {
            "session.created.v1" => {
                current.insert(
                    "title".into(),
                    payload.get("title").cloned().unwrap_or(Value::Null),
                );
                current.insert("created_at".into(), json!(event.occurred_at));
                current.insert("message_count".into(), json!(0));
                current.insert("last_run_id".into(), Value::Null);
                current.insert("last_user_preview".into(), Value::Null);
            }
            "session.message.appended.v1" => {
                let message_count = current
                    .get("message_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_add(1);
                current.insert("message_count".into(), json!(message_count));
                current.insert(
                    "last_run_id".into(),
                    payload.get("run_id").cloned().unwrap_or(Value::Null),
                );
                if payload.pointer("/message/role").and_then(Value::as_str) == Some("user") {
                    let preview = payload
                        .pointer("/message/content")
                        .and_then(Value::as_str)
                        .map(|content| content.chars().take(160).collect::<String>());
                    current.insert(
                        "last_user_preview".into(),
                        preview.map_or(Value::Null, Value::String),
                    );
                }
            }
            _ => {}
        }
        current.insert("id".into(), Value::String(id.into()));
        current.insert("stream_id".into(), Value::String(event.stream_id.clone()));
        current.insert("stream_version".into(), json!(event.stream_version));
        current.insert(
            "last_event_type".into(),
            Value::String(event.event_type.clone()),
        );
        current.insert("updated_at".into(), json!(event.occurred_at));
        Ok(upsert(id, Value::Object(current)))
    }
}

/// Task, decision, plan, and goal reducer.
pub struct WorkProjection;

impl ProjectionHandler for WorkProjection {
    fn name(&self) -> &'static str {
        "work-v1"
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        ["task:", "decision:", "plan:", "goal:"]
            .iter()
            .any(|prefix| event.stream_id.starts_with(prefix))
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        if !["task:", "decision:", "plan:", "goal:"]
            .iter()
            .any(|prefix| event.stream_id.starts_with(prefix))
        {
            return Ok(Vec::new());
        }
        if let Some(record) = payload.get("record") {
            let mut record = object(record.clone());
            record.insert("stream_id".into(), Value::String(event.stream_id.clone()));
            record.insert("stream_version".into(), json!(event.stream_version));
            record.insert(
                "last_event_type".into(),
                Value::String(event.event_type.clone()),
            );
            return Ok(upsert(&event.stream_id, Value::Object(record)));
        }
        Ok(upsert(
            &event.stream_id,
            merge_event(store, self.name(), &event.stream_id, event, payload)?,
        ))
    }
}

/// Canonical memory lifecycle reducer.
pub struct MemoryProjection;

impl ProjectionHandler for MemoryProjection {
    fn name(&self) -> &'static str {
        "memory-v1"
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.stream_id.starts_with("memory:")
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let Some(id) = event.stream_id.strip_prefix("memory:") else {
            return Ok(Vec::new());
        };
        let value = match event.event_type.as_str() {
            "memory.created.v1" | "memory.updated.v1" => payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("memory record is absent".into()))?,
            "memory.archived.v1" | "memory.superseded.v1" => {
                let mut current = store.get(self.name(), id)?.ok_or_else(|| {
                    StoreError::Verification(format!("memory {id} was not created"))
                })?;
                let status = if event.event_type == "memory.archived.v1" {
                    "archived"
                } else {
                    "superseded"
                };
                current["status"] = Value::String(status.into());
                if let Some(updated_at) = payload.get("updated_at") {
                    current["updated_at"] = updated_at.clone();
                }
                if let Some(replacement) = payload.get("replacement_id") {
                    current["superseded_by"] = replacement.clone();
                }
                current
            }
            _ => return Ok(Vec::new()),
        };
        Ok(upsert(id, value))
    }
}

/// Workflow definition and run reducer.
pub struct WorkflowProjection;

impl ProjectionHandler for WorkflowProjection {
    fn name(&self) -> &'static str {
        "workflows-v1"
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.stream_id.starts_with("workflow-definition:")
            || event.stream_id.starts_with("workflow-run:")
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        if let Some(id) = event.stream_id.strip_prefix("workflow-definition:") {
            return Ok(upsert(
                format!("definition:{id}"),
                merge_event(
                    store,
                    self.name(),
                    &format!("definition:{id}"),
                    event,
                    payload,
                )?,
            ));
        }
        let Some(run_id) = event.stream_id.strip_prefix("workflow-run:") else {
            return Ok(Vec::new());
        };
        let key = format!("run:{run_id}");
        let mut run = store.get(self.name(), &key)?.unwrap_or_else(|| json!({}));
        match event.event_type.as_str() {
            "workflow.run.started.v1" => {
                run = json!({
                    "run_id": run_id,
                    "workflow_name": payload.get("workflow_name"),
                    "workflow_version": payload.get("workflow_version"),
                    "workflow_hash": payload.get("workflow_hash"),
                    "inputs": payload.get("inputs"),
                    "outputs": null,
                    "completed_steps": 0,
                    "status": "running",
                });
            }
            "workflow.step.completed.v1" => {
                let completed = payload
                    .get("root_index")
                    .and_then(Value::as_u64)
                    .map_or(0, |index| index.saturating_add(1));
                run["completed_steps"] = json!(completed);
            }
            "workflow.run.waiting.v1" => run["status"] = json!("waiting"),
            "workflow.run.resumed.v1" => run["status"] = json!("running"),
            "workflow.run.completed.v1" => {
                run["status"] = json!("completed");
                run["outputs"] = payload.get("outputs").cloned().unwrap_or(Value::Null);
            }
            "workflow.run.failed.v1" => run["status"] = json!("failed"),
            "workflow.run.cancelled.v1" => run["status"] = json!("cancelled"),
            "workflow.run.interrupted.v1" => run["status"] = json!("interrupted"),
            _ => return Ok(Vec::new()),
        }
        run["stream_version"] = json!(event.stream_version);
        run["updated_at"] = Value::String(event.occurred_at.clone());
        Ok(upsert(key, run))
    }
}

/// Stable projection name for effects that began without a terminal event.
pub const EFFECT_RECOVERY_PROJECTION: &str = "effects-recovery-v1";

/// Minimal envelope metadata needed to close an interrupted effect on startup.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingEffect {
    /// Effect journal stream.
    pub stream_id: String,
    /// Version of the unmatched `effect.started.v1` event.
    pub started_stream_version: u64,
    /// Latest nonterminal stream version observed by the projection.
    pub latest_stream_version: u64,
    /// Identifier of the unmatched start event.
    pub started_event_id: String,
}

/// Reducer tracking only effect streams whose latest lifecycle state is started.
pub struct EffectRecoveryProjection;

impl ProjectionHandler for EffectRecoveryProjection {
    fn name(&self) -> &'static str {
        EFFECT_RECOVERY_PROJECTION
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.stream_id.starts_with("effect:")
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        _payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        match event.event_type.as_str() {
            "effect.started.v1" => Ok(upsert(
                &event.stream_id,
                serde_json::to_value(PendingEffect {
                    stream_id: event.stream_id.clone(),
                    started_stream_version: event.stream_version,
                    latest_stream_version: event.stream_version,
                    started_event_id: event.event_id.clone(),
                })
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
            )),
            "effect.completed.v1" | "effect.failed.v1" | "effect.outcome_unknown.v1" => {
                Ok(vec![ProjectionMutation::Delete {
                    key: event.stream_id.clone(),
                }])
            }
            _ => {
                let Some(value) = store.get(self.name(), &event.stream_id)? else {
                    return Ok(Vec::new());
                };
                let mut effect: PendingEffect = serde_json::from_value(value).map_err(|error| {
                    StoreError::Verification(format!(
                        "pending effect projection record {} is invalid: {error}",
                        event.stream_id
                    ))
                })?;
                effect.latest_stream_version = event.stream_version;
                Ok(upsert(
                    &event.stream_id,
                    serde_json::to_value(effect)
                        .map_err(|error| StoreError::Adapter(error.to_string()))?,
                ))
            }
        }
    }
}

/// Load a bounded, validated snapshot of effects needing startup recovery.
pub fn pending_effects(
    store: &dyn ProjectionStore,
    limit: usize,
) -> Result<Vec<PendingEffect>, StoreError> {
    store
        .list(EFFECT_RECOVERY_PROJECTION, "effect:", limit)?
        .into_iter()
        .map(|(key, value)| {
            let effect: PendingEffect = serde_json::from_value(value).map_err(|error| {
                StoreError::Verification(format!(
                    "pending effect projection record {key} is invalid: {error}"
                ))
            })?;
            if effect.stream_id != key {
                return Err(StoreError::Verification(format!(
                    "pending effect projection key {key} does not match its stream"
                )));
            }
            Ok(effect)
        })
        .collect()
}

/// Built-in projections required for the P0/P1 runtime state.
pub fn default_handlers() -> Vec<Arc<dyn ProjectionHandler>> {
    vec![
        Arc::new(SessionProjection),
        Arc::new(WorkProjection),
        Arc::new(MemoryProjection),
        Arc::new(WorkflowProjection),
        Arc::new(EffectRecoveryProjection),
    ]
}
