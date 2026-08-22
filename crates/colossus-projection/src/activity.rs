use super::*;
use colossus_contracts::{ActorType, EventClassification};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// Stable rebuildable projection containing policy-released session activity.
pub const SESSION_ACTIVITY_PROJECTION: &str = "session-activity-v1";

const MAX_TEXT_BYTES: usize = 65_536;
const MAX_SUMMARY_BYTES: usize = 2_048;
const MAX_ATTRIBUTE_BYTES: usize = 512;
const MAX_SOURCE_EVENTS: usize = 24;

/// One released text or JSON value available to an activity inspector.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedActivityContent {
    /// Rendering hint for the released value.
    pub format: String,
    /// Bounded released value.
    pub value: String,
}

/// One curated logical activity reconstructed from canonical events.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSessionActivity {
    /// Stable logical activity identifier.
    pub activity_id: String,
    /// Owning canonical session.
    pub session_id: String,
    /// Owning run when available.
    pub run_id: Option<String>,
    /// One-based model turn when available.
    pub turn: Option<u32>,
    /// Timeline lane: `agent`, `tools`, or `system`.
    pub lane: String,
    /// Display kind: `user`, `assistant`, `tool`, or `system`.
    pub kind: String,
    /// Bounded display title.
    pub title: String,
    /// Bounded released summary.
    pub summary: String,
    /// Coarse actor label without internal actor identifiers.
    pub actor: String,
    /// Released lifecycle state when applicable.
    pub status: Option<String>,
    /// UTC start or occurrence time.
    pub started_at: String,
    /// UTC completion time when a trustworthy terminal boundary exists.
    pub completed_at: Option<String>,
    /// Millisecond duration only when both boundaries exist.
    pub duration_ms: Option<u64>,
    /// Policy-released input.
    pub input: Option<ProjectedActivityContent>,
    /// Policy-released result.
    pub result: Option<ProjectedActivityContent>,
    /// Small allowlisted metadata values.
    pub attributes: BTreeMap<String, String>,
    /// Canonical event types contributing to this logical activity.
    pub source_event_types: Vec<String>,
    /// First contributing global journal sequence.
    pub first_sequence: u64,
    /// Latest contributing global journal sequence.
    pub last_sequence: u64,
}

/// One internal cursor page from the activity projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedSessionActivityPage {
    /// Newest-first logical records.
    pub records: Vec<(String, ProjectedSessionActivity)>,
    /// Exclusive projection key for the next page.
    pub next_key: Option<String>,
    /// Whether another projection page exists.
    pub has_more: bool,
    /// Latest globally applied projection sequence.
    pub projected_through_sequence: u64,
}

/// Deterministic activity reducer over canonical journal events.
pub struct SessionActivityProjection;

impl ProjectionHandler for SessionActivityProjection {
    fn name(&self) -> &'static str {
        SESSION_ACTIVITY_PROJECTION
    }

    fn applies_to(&self, event: &EventEnvelope) -> bool {
        event.context.session_id.is_some()
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let Some(session_id) = event.context.session_id.as_deref() else {
            return Ok(Vec::new());
        };
        match event.event_type.as_str() {
            "api.run.update.v1" => project_released_update(store, session_id, event, payload),
            "context.prepared.v1" => project_context(session_id, event, payload),
            "model.request.prepared.v1" => project_model_request(store, session_id, event, payload),
            "final.output.v1" => finish_current_model(store, session_id, event),
            "tool.call.requested.v1"
            | "tool.call.started.v1"
            | "tool.call.completed.v1"
            | "tool.call.cancelled.v1" => project_tool_lifecycle(store, session_id, event, payload),
            "plan.written.v1" => project_plan(session_id, event, payload),
            "run.started.v1" | "run.completed.v1" | "run.cancelled.v1" | "run.max_turns.v1"
            | "error.v1" => project_run_lifecycle(session_id, event, payload),
            "effect.started.v1"
            | "effect.completed.v1"
            | "effect.failed.v1"
            | "effect.outcome_unknown.v1"
            | "effect.denied.v1"
            | "effect.release_denied.v1" => project_effect_lifecycle(store, session_id, event),
            _ if matches!(
                event.classification,
                EventClassification::Policy | EventClassification::Approval
            ) =>
            {
                project_policy_activity(store, session_id, event)
            }
            // Streaming content, raw session messages, raw provider output, routine indexing,
            // and unrelated domain records are intentionally absent from this released view.
            _ => Ok(Vec::new()),
        }
    }
}

/// Read bounded newest-first pages without rescanning the canonical journal.
pub struct ProjectedSessionActivityReader {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedSessionActivityReader {
    /// Bind the reader to a disposable projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }

    /// Read one bounded internal page after an exclusive projection key.
    pub fn list_page(
        &self,
        session_id: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<ProjectedSessionActivityPage, StoreError> {
        let limit = limit.clamp(1, 1_000);
        let prefix = record_prefix(session_id);
        if after_key.is_some_and(|key| !key.starts_with(&prefix)) {
            return Err(StoreError::Adapter(
                "session activity cursor does not match the session".into(),
            ));
        }
        let mut values =
            self.store
                .list_after(SESSION_ACTIVITY_PROJECTION, &prefix, after_key, limit + 1)?;
        let has_more = values.len() > limit;
        if has_more {
            values.pop();
        }
        let next_key = has_more
            .then(|| values.last().map(|(key, _)| key.clone()))
            .flatten();
        let records = values
            .into_iter()
            .map(|(key, value)| {
                let record = serde_json::from_value(value).map_err(|error| {
                    StoreError::Verification(format!(
                        "session activity projection record {key} is invalid: {error}"
                    ))
                })?;
                Ok((key, record))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ProjectedSessionActivityPage {
            records,
            next_key,
            has_more,
            projected_through_sequence: self.store.position(SESSION_ACTIVITY_PROJECTION)?,
        })
    }
}

fn project_released_update(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(kind) = payload.get("kind").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let Some(update_type) = kind.get("type").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    match update_type {
        "output_delta" => Ok(Vec::new()),
        "tool_activity" => project_released_tool(store, session_id, event, kind),
        "message" => project_released_message(store, session_id, event, kind),
        "reasoning_summary" => {
            let summary = kind
                .get("summary")
                .and_then(Value::as_str)
                .map_or_else(String::new, |value| bounded(value, MAX_TEXT_BYTES));
            Ok(upsert_new(
                session_id,
                event,
                format!("reasoning:{}", event.event_id),
                "agent",
                "assistant",
                "Reasoning summary",
                &summary,
                "Assistant",
                Some("completed"),
                payload_turn(kind),
                None,
                Some(content("text", summary.clone())),
                BTreeMap::new(),
            )?)
        }
        "usage" => {
            let usage = kind.get("usage").and_then(Value::as_object);
            let total = usage
                .and_then(|value| value.get("total_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let mut attributes = BTreeMap::new();
            for field in [
                "input_tokens",
                "output_tokens",
                "total_tokens",
                "cached_input_tokens",
                "reasoning_tokens",
            ] {
                if let Some(value) = usage.and_then(|item| item.get(field)).and_then(scalar) {
                    insert_attribute(&mut attributes, field, &value);
                }
            }
            Ok(upsert_new(
                session_id,
                event,
                format!("usage:{}", event.event_id),
                "system",
                "system",
                "Token usage",
                &format!("{total} tokens used"),
                "System",
                Some("completed"),
                None,
                None,
                None,
                attributes,
            )?)
        }
        "interaction" => project_interaction(session_id, event, kind),
        "notice" => project_notice(session_id, event, kind),
        "state" => project_released_state(session_id, event, kind),
        "result" => project_released_result(store, session_id, event, kind),
        "failure" | "cancellation" => project_released_terminal(session_id, event, kind),
        _ => Ok(Vec::new()),
    }
}

fn project_released_tool(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(activity) = kind.get("activity").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let Some(call_id) = activity.get("call_id").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    update_logical(
        store,
        session_id,
        event,
        &format!("tool:{run_id}:{call_id}"),
        |record| {
            record.run_id = event.context.run_id.clone();
            record.lane = "tools".into();
            record.kind = "tool".into();
            record.actor = "Assistant".into();
            if let Some(name) = activity.get("tool_name").and_then(Value::as_str) {
                record.title = bounded(name, MAX_ATTRIBUTE_BYTES);
            }
            if let Some(summary) = activity.get("summary").and_then(Value::as_str) {
                record.summary = bounded(summary, MAX_SUMMARY_BYTES);
            }
            if let Some(state) = activity.get("state").and_then(Value::as_str) {
                record.status = Some(bounded(state, MAX_ATTRIBUTE_BYTES));
                if terminal_status(state) {
                    finish_record(record, &event.occurred_at);
                } else if state == "started" && record.started_at.is_empty() {
                    record.started_at.clone_from(&event.occurred_at);
                }
            }
            if let Some(input) = activity.get("input").and_then(Value::as_str) {
                record.input = Some(content("json", bounded(input, MAX_TEXT_BYTES)));
            }
            if let Some(preview) = activity.get("preview").and_then(Value::as_str) {
                record.result = Some(content("text", bounded(preview, MAX_TEXT_BYTES)));
            }
        },
    )
}

fn project_released_message(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(message) = kind.get("message").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("system");
    let text = released_message_text(message);
    if role == "tool" {
        return Ok(Vec::new());
    }
    if role == "assistant"
        && let Some(run_id) = event.context.run_id.as_deref()
        && let Some(logical_id) = lookup_value(store, &model_lookup(session_id, run_id))?
    {
        return update_logical(store, session_id, event, &logical_id, |record| {
            record.title = "Assistant response".into();
            record.summary = preview(&text);
            record.result = Some(content("text", text.clone()));
            record.status = Some("completed".into());
            finish_record(record, &event.occurred_at);
        });
    }
    let (lane, kind_name, title, actor, input, result) = match role {
        "user" => (
            "agent",
            "user",
            "User message",
            "User",
            Some(content("text", text.clone())),
            None,
        ),
        "assistant" => (
            "agent",
            "assistant",
            "Assistant response",
            "Assistant",
            None,
            Some(content("text", text.clone())),
        ),
        _ => (
            "system",
            "system",
            "System message",
            "System",
            None,
            Some(content("text", text.clone())),
        ),
    };
    upsert_new(
        session_id,
        event,
        format!("message:{}", event.event_id),
        lane,
        kind_name,
        title,
        &preview(&text),
        actor,
        Some("completed"),
        None,
        input,
        result,
        BTreeMap::new(),
    )
}

fn project_context(
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let mut attributes = BTreeMap::new();
    for field in [
        "original_token_estimate",
        "token_estimate",
        "context_window_tokens",
        "message_count",
        "compacted",
        "snapshot_created",
        "strategy",
    ] {
        if let Some(value) = payload.get(field).and_then(scalar) {
            insert_attribute(&mut attributes, field, &value);
        }
    }
    let message_count = payload
        .get("message_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    upsert_new(
        session_id,
        event,
        format!("context:{}", event.event_id),
        "system",
        "system",
        "Context prepared",
        &format!("Prepared {message_count} messages from session state"),
        "System",
        Some("completed"),
        payload_turn_value(payload),
        None,
        None,
        attributes,
    )
}

fn project_model_request(
    _store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let turn = payload_turn_value(payload);
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    let logical_id = format!("model:{run_id}:{}", turn.unwrap_or_default());
    let mut attributes = BTreeMap::new();
    for field in [
        "role",
        "model_profile",
        "provider_profile",
        "model",
        "message_count",
        "tool_count",
    ] {
        if let Some(value) = payload.get(field).and_then(scalar) {
            insert_attribute(&mut attributes, field, &value);
        }
    }
    let mut mutations = upsert_new(
        session_id,
        event,
        logical_id.clone(),
        "agent",
        "assistant",
        "Assistant turn",
        "Model request prepared",
        "Assistant",
        Some("running"),
        turn,
        None,
        None,
        attributes,
    )?;
    if !run_id.is_empty() {
        mutations.push(ProjectionMutation::Upsert {
            key: model_lookup(session_id, run_id),
            value: Value::String(logical_id),
        });
    }
    Ok(mutations)
}

fn finish_current_model(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(run_id) = event.context.run_id.as_deref() else {
        return Ok(Vec::new());
    };
    let Some(logical_id) = lookup_value(store, &model_lookup(session_id, run_id))? else {
        return Ok(Vec::new());
    };
    update_logical(store, session_id, event, &logical_id, |record| {
        record.status = Some("completed".into());
        finish_record(record, &event.occurred_at);
    })
}

fn project_tool_lifecycle(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    update_logical(
        store,
        session_id,
        event,
        &format!("tool:{run_id}:{call_id}"),
        |record| {
            record.run_id = event.context.run_id.clone();
            record.turn = payload_turn_value(payload).or(record.turn);
            record.lane = "tools".into();
            record.kind = "tool".into();
            record.actor = "Assistant".into();
            if let Some(name) = payload.get("name").and_then(Value::as_str) {
                record.title = bounded(name, MAX_ATTRIBUTE_BYTES);
            }
            match event.event_type.as_str() {
                "tool.call.requested.v1" => {
                    record.status = Some("requested".into());
                    record.summary = format!("{} requested", record.title);
                }
                "tool.call.started.v1" => {
                    record.status = Some("started".into());
                    record.summary = format!("{} started", record.title);
                    record.started_at.clone_from(&event.occurred_at);
                    if let Some(fields) = payload.get("argument_fields").and_then(Value::as_array) {
                        let names = fields
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ");
                        if !names.is_empty() {
                            insert_attribute(&mut record.attributes, "argument_fields", &names);
                        }
                    }
                }
                "tool.call.completed.v1" => {
                    let status = match payload.get("outcome_certainty").and_then(Value::as_str) {
                        Some("unknown") => "outcome_unknown",
                        _ if payload
                            .get("exit_code")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            != 0 =>
                        {
                            "failed"
                        }
                        _ => "completed",
                    };
                    record.status = Some(status.into());
                    record.summary = format!("{} {status}", record.title);
                    finish_record(record, &event.occurred_at);
                }
                "tool.call.cancelled.v1" => {
                    record.status = Some("cancelled".into());
                    record.summary = format!("{} cancelled", record.title);
                    finish_record(record, &event.occurred_at);
                }
                _ => {}
            }
        },
    )
}

fn project_plan(
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let mut attributes = BTreeMap::new();
    for field in ["plan_id", "revision"] {
        if let Some(value) = payload.get(field).and_then(scalar) {
            insert_attribute(&mut attributes, field, &value);
        }
    }
    upsert_new(
        session_id,
        event,
        format!("plan:{}", event.event_id),
        "system",
        "system",
        "Plan written",
        "A canonical Plan revision was saved",
        "System",
        Some("completed"),
        payload_turn_value(payload),
        None,
        None,
        attributes,
    )
}

fn project_run_lifecycle(
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let (title, status) = match event.event_type.as_str() {
        "run.started.v1" => ("Run started", "running"),
        "run.completed.v1" => ("Run completed", "completed"),
        "run.cancelled.v1" => ("Run cancelled", "cancelled"),
        "run.max_turns.v1" => ("Turn limit reached", "failed"),
        _ => ("Run failed", "failed"),
    };
    let mut attributes = BTreeMap::new();
    if let Some(turn) = payload.get("turn").and_then(scalar) {
        insert_attribute(&mut attributes, "turn", &turn);
    }
    upsert_new(
        session_id,
        event,
        format!("run:{}", event.event_id),
        "system",
        "system",
        title,
        title,
        "System",
        Some(status),
        payload_turn_value(payload),
        None,
        None,
        attributes,
    )
}

fn project_effect_lifecycle(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(call_id) = event.actor.id.strip_prefix("tool-call:") else {
        if matches!(
            event.event_type.as_str(),
            "effect.failed.v1"
                | "effect.outcome_unknown.v1"
                | "effect.denied.v1"
                | "effect.release_denied.v1"
        ) {
            return system_event(
                session_id,
                event,
                human_event_type(&event.event_type),
                "Effect lifecycle",
            );
        }
        return Ok(Vec::new());
    };
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    update_logical(
        store,
        session_id,
        event,
        &format!("tool:{run_id}:{call_id}"),
        |record| match event.event_type.as_str() {
            "effect.failed.v1" | "effect.denied.v1" | "effect.release_denied.v1" => {
                record.status = Some("failed".into());
                finish_record(record, &event.occurred_at);
            }
            "effect.outcome_unknown.v1" => {
                record.status = Some("outcome_unknown".into());
                finish_record(record, &event.occurred_at);
            }
            _ => {}
        },
    )
}

fn project_policy_activity(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    if let Some(call_id) = event.actor.id.strip_prefix("tool-call:") {
        let run_id = event.context.run_id.as_deref().unwrap_or_default();
        return update_logical(
            store,
            session_id,
            event,
            &format!("tool:{run_id}:{call_id}"),
            |record| {
                if matches!(
                    event.event_type.as_str(),
                    "approval.denied.v1" | "approval.error.v1" | "policy.error.v1"
                ) {
                    record.status = Some("failed".into());
                    finish_record(record, &event.occurred_at);
                } else if event.event_type == "approval.granted.v1" {
                    insert_attribute(&mut record.attributes, "approval", "granted");
                }
            },
        );
    }
    system_event(
        session_id,
        event,
        human_event_type(&event.event_type),
        "Policy activity",
    )
}

fn project_interaction(
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let interaction = kind.get("interaction").and_then(Value::as_object);
    let interaction_kind = interaction
        .and_then(|value| value.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("prompt");
    let status = interaction
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("pending");
    let prompt = interaction
        .and_then(|value| value.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    upsert_new(
        session_id,
        event,
        format!("interaction:{}", event.event_id),
        "system",
        "system",
        if interaction_kind == "approval" {
            "Approval requested"
        } else {
            "Input requested"
        },
        &bounded(prompt, MAX_SUMMARY_BYTES),
        "System",
        Some(status),
        None,
        None,
        None,
        BTreeMap::new(),
    )
}

fn project_notice(
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let reason = kind
        .get("notice")
        .and_then(Value::as_object)
        .and_then(|notice| notice.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if reason.starts_with("run.phase.") {
        return Ok(Vec::new());
    }
    let message = kind
        .get("notice")
        .and_then(Value::as_object)
        .and_then(|notice| notice.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    upsert_new(
        session_id,
        event,
        format!("notice:{}", event.event_id),
        "system",
        "system",
        "Run notice",
        &bounded(message, MAX_SUMMARY_BYTES),
        "System",
        Some("completed"),
        None,
        None,
        None,
        BTreeMap::new(),
    )
}

fn project_released_state(
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let status = kind
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    upsert_new(
        session_id,
        event,
        format!("state:{}", event.event_id),
        "system",
        "system",
        &format!("Run {}", status.replace('_', " ")),
        "Public run state changed",
        "System",
        Some(status),
        None,
        None,
        None,
        BTreeMap::new(),
    )
}

fn project_released_result(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let output = kind
        .get("result")
        .and_then(Value::as_object)
        .and_then(|result| result.get("output"))
        .and_then(Value::as_str)
        .map_or_else(String::new, |value| bounded(value, MAX_TEXT_BYTES));
    if let Some(run_id) = event.context.run_id.as_deref()
        && let Some(logical_id) = lookup_value(store, &model_lookup(session_id, run_id))?
    {
        return update_logical(store, session_id, event, &logical_id, |record| {
            record.title = "Assistant response".into();
            record.summary = preview(&output);
            record.result = Some(content("text", output.clone()));
            record.status = Some("completed".into());
            finish_record(record, &event.occurred_at);
        });
    }
    upsert_new(
        session_id,
        event,
        format!("result:{}", event.event_id),
        "agent",
        "assistant",
        "Assistant response",
        &preview(&output),
        "Assistant",
        Some("completed"),
        None,
        None,
        Some(content("text", output)),
        BTreeMap::new(),
    )
}

fn project_released_terminal(
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let update_type = kind
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("failure");
    let (title, status, detail) = if update_type == "cancellation" {
        let detail = kind
            .get("cancellation")
            .and_then(Value::as_object)
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Run cancelled");
        ("Run cancelled", "cancelled", detail)
    } else {
        let detail = kind
            .get("failure")
            .and_then(Value::as_object)
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Run failed");
        ("Run failed", "failed", detail)
    };
    upsert_new(
        session_id,
        event,
        format!("terminal:{}", event.event_id),
        "system",
        "system",
        title,
        &bounded(detail, MAX_SUMMARY_BYTES),
        "System",
        Some(status),
        None,
        None,
        Some(content("text", bounded(detail, MAX_TEXT_BYTES))),
        BTreeMap::new(),
    )
}

fn system_event(
    session_id: &str,
    event: &EventEnvelope,
    title: String,
    summary: &str,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    upsert_new(
        session_id,
        event,
        format!("system:{}", event.event_id),
        "system",
        "system",
        &title,
        summary,
        "System",
        event_status(&event.event_type),
        None,
        None,
        None,
        BTreeMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn upsert_new(
    session_id: &str,
    event: &EventEnvelope,
    logical_id: String,
    lane: &str,
    kind: &str,
    title: &str,
    summary: &str,
    actor: &str,
    status: Option<&str>,
    turn: Option<u32>,
    input: Option<ProjectedActivityContent>,
    result: Option<ProjectedActivityContent>,
    attributes: BTreeMap<String, String>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let record = ProjectedSessionActivity {
        activity_id: logical_id.clone(),
        session_id: session_id.into(),
        run_id: event.context.run_id.clone(),
        turn,
        lane: lane.into(),
        kind: kind.into(),
        title: bounded(title, MAX_ATTRIBUTE_BYTES),
        summary: bounded(summary, MAX_SUMMARY_BYTES),
        actor: actor.into(),
        status: status.map(str::to_owned),
        started_at: event.occurred_at.clone(),
        completed_at: status
            .filter(|value| terminal_status(value))
            .map(|_| event.occurred_at.clone()),
        // Instant records have an occurrence time, not a paired start/end boundary.
        duration_ms: None,
        input,
        result,
        attributes,
        source_event_types: vec![event.event_type.clone()],
        first_sequence: event.global_sequence,
        last_sequence: event.global_sequence,
    };
    Ok(vec![ProjectionMutation::Upsert {
        key: record_key(session_id, event.global_sequence, &logical_id),
        value: serde_json::to_value(record)
            .map_err(|error| StoreError::Adapter(error.to_string()))?,
    }])
}

fn update_logical(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    logical_id: &str,
    update: impl FnOnce(&mut ProjectedSessionActivity),
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let lookup = logical_lookup(session_id, logical_id);
    let existing_key = lookup_value(store, &lookup)?;
    let key = existing_key
        .clone()
        .unwrap_or_else(|| record_key(session_id, event.global_sequence, logical_id));
    let mut record = match store.get(SESSION_ACTIVITY_PROJECTION, &key)? {
        Some(value) => serde_json::from_value(value).map_err(|error| {
            StoreError::Verification(format!("session activity record {key} is invalid: {error}"))
        })?,
        None => ProjectedSessionActivity {
            activity_id: logical_id.into(),
            session_id: session_id.into(),
            run_id: event.context.run_id.clone(),
            turn: None,
            lane: "system".into(),
            kind: "system".into(),
            title: "Activity".into(),
            summary: String::new(),
            actor: actor_label(event.actor.actor_type).into(),
            status: None,
            started_at: event.occurred_at.clone(),
            completed_at: None,
            duration_ms: None,
            input: None,
            result: None,
            attributes: BTreeMap::new(),
            source_event_types: Vec::new(),
            first_sequence: event.global_sequence,
            last_sequence: event.global_sequence,
        },
    };
    update(&mut record);
    record.last_sequence = event.global_sequence;
    push_source_event(&mut record, &event.event_type);
    if record.started_at.is_empty() {
        record.started_at.clone_from(&event.occurred_at);
    }
    let mut mutations = vec![ProjectionMutation::Upsert {
        key: key.clone(),
        value: serde_json::to_value(record)
            .map_err(|error| StoreError::Adapter(error.to_string()))?,
    }];
    if existing_key.is_none() {
        mutations.push(ProjectionMutation::Upsert {
            key: lookup,
            value: Value::String(key),
        });
    }
    Ok(mutations)
}

fn finish_record(record: &mut ProjectedSessionActivity, completed_at: &str) {
    record.completed_at = Some(completed_at.into());
    record.duration_ms = duration_ms(&record.started_at, completed_at);
}

fn duration_ms(started_at: &str, completed_at: &str) -> Option<u64> {
    let started = OffsetDateTime::parse(started_at, &Rfc3339).ok()?;
    let completed = OffsetDateTime::parse(completed_at, &Rfc3339).ok()?;
    let milliseconds = (completed - started).whole_milliseconds();
    u64::try_from(milliseconds).ok()
}

fn record_prefix(session_id: &str) -> String {
    format!("record:{}:", hash_key(session_id))
}

fn record_key(session_id: &str, sequence: u64, logical_id: &str) -> String {
    format!(
        "{}{inverse:020}:{}",
        record_prefix(session_id),
        hash_key(logical_id),
        inverse = u64::MAX - sequence,
    )
}

fn logical_lookup(session_id: &str, logical_id: &str) -> String {
    format!("lookup:{}:{}", hash_key(session_id), hash_key(logical_id))
}

fn model_lookup(session_id: &str, run_id: &str) -> String {
    format!("model:{}:{}", hash_key(session_id), hash_key(run_id))
}

fn lookup_value(store: &dyn ProjectionStore, key: &str) -> Result<Option<String>, StoreError> {
    Ok(store
        .get(SESSION_ACTIVITY_PROJECTION, key)?
        .and_then(|value| value.as_str().map(str::to_owned)))
}

fn hash_key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn content(format: &str, value: String) -> ProjectedActivityContent {
    ProjectedActivityContent {
        format: format.into(),
        value,
    }
}

fn bounded(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut bounded = value[..end].to_owned();
    bounded.push('…');
    bounded
}

fn preview(value: &str) -> String {
    bounded(
        &value.split_whitespace().collect::<Vec<_>>().join(" "),
        MAX_SUMMARY_BYTES,
    )
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn insert_attribute(attributes: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if attributes.len() < 24 {
        attributes.insert(
            bounded(key, MAX_ATTRIBUTE_BYTES),
            bounded(value, MAX_ATTRIBUTE_BYTES),
        );
    }
}

fn payload_turn(payload: &Map<String, Value>) -> Option<u32> {
    payload
        .get("turn")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn payload_turn_value(payload: &Value) -> Option<u32> {
    payload
        .get("turn")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn released_message_text(message: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    for part in message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    parts.push(text.to_owned());
                }
            }
            Some("artifact") => {
                let label = part
                    .pointer("/artifact/file_name")
                    .and_then(Value::as_str)
                    .unwrap_or("released artifact");
                parts.push(format!("[Attachment: {label}]"));
            }
            _ => {}
        }
    }
    bounded(&parts.join("\n"), MAX_TEXT_BYTES)
}

fn terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "expired" | "responded" | "outcome_unknown"
    )
}

fn event_status(event_type: &str) -> Option<&'static str> {
    if event_type.contains("denied")
        || event_type.contains("error")
        || event_type.contains("failed")
    {
        Some("failed")
    } else if event_type.contains("requested") {
        Some("requested")
    } else if event_type.contains("granted") || event_type.contains("completed") {
        Some("completed")
    } else {
        None
    }
}

fn actor_label(actor: ActorType) -> &'static str {
    match actor {
        ActorType::User => "User",
        ActorType::Model | ActorType::Subagent => "Assistant",
        ActorType::Application => "Application",
        ActorType::Workflow => "Workflow",
        ActorType::System => "System",
    }
}

fn human_event_type(event_type: &str) -> String {
    let value = event_type.strip_suffix(".v1").unwrap_or(event_type);
    let mut words = value.replace(['.', '_'], " ");
    if let Some(first) = words.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    words
}

fn push_source_event(record: &mut ProjectedSessionActivity, event_type: &str) {
    if record
        .source_event_types
        .iter()
        .any(|value| value == event_type)
    {
        return;
    }
    if record.source_event_types.len() == MAX_SOURCE_EVENTS {
        record.source_event_types.remove(0);
    }
    record.source_event_types.push(event_type.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{Actor, EncryptedPayload, EventClassification, ExecutionContext};
    use colossus_testkit::InMemoryProjectionStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingProjectionStore {
        inner: InMemoryProjectionStore,
        list_after_calls: AtomicUsize,
        last_list_after_limit: AtomicUsize,
    }

    impl ProjectionStore for CountingProjectionStore {
        fn position(&self, projection: &str) -> Result<u64, StoreError> {
            self.inner.position(projection)
        }

        fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
            self.inner.get(projection, key)
        }

        fn list(
            &self,
            projection: &str,
            key_prefix: &str,
            limit: usize,
        ) -> Result<Vec<(String, Value)>, StoreError> {
            self.inner.list(projection, key_prefix, limit)
        }

        fn list_after(
            &self,
            projection: &str,
            key_prefix: &str,
            after_key: Option<&str>,
            limit: usize,
        ) -> Result<Vec<(String, Value)>, StoreError> {
            self.list_after_calls.fetch_add(1, Ordering::Relaxed);
            self.last_list_after_limit.store(limit, Ordering::Relaxed);
            self.inner
                .list_after(projection, key_prefix, after_key, limit)
        }

        fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
            self.inner.apply(batch)
        }

        fn reset(&self, projection: &str) -> Result<(), StoreError> {
            self.inner.reset(projection)
        }
    }

    fn event(sequence: u64, event_type: &str, run_id: &str) -> EventEnvelope {
        EventEnvelope {
            schema_version: 1,
            event_version: 1,
            event_id: format!("event-{sequence}"),
            global_sequence: sequence,
            stream_id: format!("run:{run_id}"),
            stream_version: sequence,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "system".into(),
            },
            context: ExecutionContext {
                correlation_id: "correlation".into(),
                session_id: Some("session-a".into()),
                run_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            occurred_at: format!("2026-08-21T09:14:{sequence:02}Z"),
            payload: EncryptedPayload {
                key_id: "test".into(),
                algorithm: "test".into(),
                nonce: String::new(),
                ciphertext: String::new(),
                plaintext_hash: "hash".into(),
            },
            previous_hash: "previous".into(),
            record_hash: "record".into(),
        }
    }

    fn apply(
        store: &InMemoryProjectionStore,
        position: &mut u64,
        event: &EventEnvelope,
        payload: Value,
    ) {
        let projection = SessionActivityProjection;
        let mutations = projection.project(store, event, &payload).expect("project");
        store
            .apply(ProjectionBatch {
                projection: SESSION_ACTIVITY_PROJECTION.into(),
                expected_position: *position,
                through_sequence: event.global_sequence,
                mutations,
            })
            .expect("apply");
        *position = event.global_sequence;
    }

    #[test]
    fn coalesces_tool_lifecycle_without_retaining_raw_arguments_or_output() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        apply(
            &store,
            &mut position,
            &event(1, "tool.call.requested.v1", "run-a"),
            json!({
                "turn": 2,
                "call_id": "call-a",
                "name": "filesystem.read",
                "arguments": {"secret": "must-not-project"}
            }),
        );
        apply(
            &store,
            &mut position,
            &event(2, "tool.call.started.v1", "run-a"),
            json!({
                "turn": 2,
                "call_id": "call-a",
                "name": "filesystem.read",
                "argument_fields": ["path"]
            }),
        );
        apply(
            &store,
            &mut position,
            &event(3, "tool.call.completed.v1", "run-a"),
            json!({
                "call_id": "call-a",
                "name": "filesystem.read",
                "output": "must-not-project",
                "exit_code": 0
            }),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        assert_eq!(page.records.len(), 1);
        let activity = &page.records[0].1;
        assert_eq!(activity.turn, Some(2));
        assert_eq!(activity.status.as_deref(), Some("completed"));
        let encoded = serde_json::to_string(activity).expect("encode");
        assert!(!encoded.contains("must-not-project"));
        assert_eq!(activity.duration_ms, Some(1_000));
    }

    #[test]
    fn projection_pages_are_newest_first_and_session_scoped() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        apply(
            &store,
            &mut position,
            &event(1, "context.prepared.v1", "run-a"),
            json!({"turn": 1, "message_count": 2}),
        );
        apply(
            &store,
            &mut position,
            &event(2, "context.prepared.v1", "run-a"),
            json!({"turn": 2, "message_count": 3}),
        );
        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let first = reader.list_page("session-a", None, 1).expect("first");
        assert_eq!(first.records[0].1.first_sequence, 2);
        assert!(first.has_more);
        let second = reader
            .list_page("session-a", first.next_key.as_deref(), 1)
            .expect("second");
        assert_eq!(second.records[0].1.first_sequence, 1);
        assert!(
            reader
                .list_page("session-b", None, 10)
                .expect("other")
                .records
                .is_empty()
        );
    }

    #[test]
    fn projection_page_reads_remain_bounded_as_session_history_grows() {
        let store = Arc::new(CountingProjectionStore::default());
        let mut position = 0;
        for sequence in 1..=2_500 {
            apply(
                &store.inner,
                &mut position,
                &event(sequence, "context.prepared.v1", "run-a"),
                json!({"turn": sequence, "message_count": sequence}),
            );
        }

        let reader = ProjectedSessionActivityReader::new(store.clone());
        let page = reader.list_page("session-a", None, 100).expect("page");

        assert_eq!(page.records.len(), 100);
        assert!(page.has_more);
        assert_eq!(store.list_after_calls.load(Ordering::Relaxed), 1);
        assert_eq!(store.last_list_after_limit.load(Ordering::Relaxed), 101);
    }
}
