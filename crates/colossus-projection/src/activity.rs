use super::*;
use colossus_contracts::{ActorType, EventClassification};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// Stable rebuildable projection containing policy-released session activity.
pub const SESSION_ACTIVITY_PROJECTION: &str = "session-activity-v4";

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
            "subagent.queued.v1" | "subagent.status_changed.v1" => {
                project_subagent_lifecycle(store, session_id, event, payload)
            }
            "plan.written.v1" => project_plan(session_id, event, payload),
            "run.started.v1" | "run.completed.v1" | "run.cancelled.v1" | "run.max_turns.v1"
            | "error.v1" => project_run_lifecycle(store, session_id, event, payload),
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
                project_policy_activity(store, session_id, event, payload)
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
            let run_id = event.context.run_id.as_deref().unwrap_or_default();
            let turn = payload_turn(kind);
            update_logical(
                store,
                session_id,
                event,
                &format!("usage:{run_id}:{}", turn.unwrap_or_default()),
                |record| {
                    record.run_id = event.context.run_id.clone();
                    record.turn = turn;
                    record.lane = "system".into();
                    record.kind = "system".into();
                    record.title = "Token usage".into();
                    record.summary = format!("{total} tokens used");
                    record.actor = "System".into();
                    record.status = Some("completed".into());
                    record.completed_at = None;
                    record.duration_ms = None;
                    record.attributes = attributes;
                },
            )
        }
        "interaction" => project_interaction(session_id, event, kind),
        "notice" => project_notice(session_id, event, kind),
        "state" => project_released_state(store, session_id, event, kind),
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
            set_assistant_actor(record, event);
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
            record.run_id = event.context.run_id.clone();
            record.lane = "agent".into();
            record.kind = "assistant".into();
            set_assistant_actor(record, event);
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
            assistant_actor(event),
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
    store: &dyn ProjectionStore,
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
    let mut mutations = update_logical(store, session_id, event, &logical_id, |record| {
        record.run_id = event.context.run_id.clone();
        record.turn = turn;
        record.lane = "agent".into();
        record.kind = "assistant".into();
        record.title = "Assistant turn".into();
        record.summary = "Model request prepared".into();
        set_assistant_actor(record, event);
        record.status = Some("running".into());
        record.attributes = attributes;
    })?;
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
            set_assistant_actor(record, event);
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

fn project_subagent_lifecycle(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(record_value) = payload.get("record") else {
        return Ok(Vec::new());
    };
    let Some(subagent_id) = record_value.get("id").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    let role = record_value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("delegated agent");
    let status = record_value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("queued");
    let activity_status = match status {
        "queued" => "requested",
        "interrupted" => "failed",
        value => value,
    };
    let parent_run_id = record_value
        .get("parent_run_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let child_run_id = record_value.get("child_run_id").and_then(Value::as_str);
    let logical_id = format!("subagent:{subagent_id}");
    update_logical(store, session_id, event, &logical_id, |activity| {
        activity.run_id = child_run_id
            .or((!parent_run_id.is_empty()).then_some(parent_run_id))
            .map(str::to_owned);
        activity.lane = "agent".into();
        activity.kind = "assistant".into();
        activity.title = format!("Subagent · {}", bounded(role, MAX_ATTRIBUTE_BYTES));
        activity.summary = format!("Delegated agent is {}", status.replace('_', " "));
        activity.actor = format!("Subagent · {}", bounded(role, MAX_ATTRIBUTE_BYTES));
        activity.status = Some(activity_status.into());
        insert_attribute(&mut activity.attributes, "run_role", "subagent");
        insert_attribute(&mut activity.attributes, "subagent_id", subagent_id);
        insert_attribute(&mut activity.attributes, "subagent_role", role);
        insert_attribute(&mut activity.attributes, "subagent_status", status);
        if !parent_run_id.is_empty() {
            insert_attribute(&mut activity.attributes, "parent_run_id", parent_run_id);
        }
        if let Some(child_run_id) = child_run_id {
            insert_attribute(&mut activity.attributes, "child_run_id", child_run_id);
        }
        if let Some(started_at) = record_value.get("started_at").and_then(Value::as_str) {
            activity.started_at = bounded(started_at, MAX_ATTRIBUTE_BYTES);
        }
        if terminal_status(activity_status)
            && let Some(completed_at) = record_value.get("completed_at").and_then(Value::as_str)
        {
            finish_record(activity, completed_at);
        }
    })
}

fn project_run_lifecycle(
    store: &dyn ProjectionStore,
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
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    update_logical(
        store,
        session_id,
        event,
        &format!("run-lifecycle:{run_id}"),
        |record| {
            record.run_id = event.context.run_id.clone();
            record.turn = payload_turn_value(payload).or(record.turn);
            record.lane = "system".into();
            record.kind = "system".into();
            record.title = title.into();
            record.summary = title.into();
            record.actor = "System".into();
            record.status = Some(status.into());
            for (key, value) in attributes {
                insert_attribute(&mut record.attributes, &key, &value);
            }
            if terminal_status(status) {
                finish_record(record, &event.occurred_at);
            }
        },
    )
}

fn project_effect_lifecycle(
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let Some(call_id) = tool_call_id(&event.actor.id) else {
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
    payload: &Value,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let mut details = policy_attributes(payload);
    if !details.contains_key("reason_code") {
        let reason_code = match event.event_type.as_str() {
            "approval.requested.v1" => Some("approval_requested"),
            "approval.granted.v1" => Some("approval_granted"),
            "approval.denied.v1" => Some("approval_denied"),
            "approval.error.v1" => Some("approval_provider_failed"),
            "policy.error.v1" => Some("policy_evaluation_failed"),
            _ => None,
        };
        if let Some(reason_code) = reason_code {
            insert_attribute(&mut details, "reason_code", reason_code);
        }
    }
    if let Some(call_id) = tool_call_id(&event.actor.id) {
        let run_id = event.context.run_id.as_deref().unwrap_or_default();
        return update_logical(
            store,
            session_id,
            event,
            &format!("tool:{run_id}:{call_id}"),
            |record| {
                for (key, value) in &details {
                    insert_attribute(&mut record.attributes, key, value);
                }
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

    let outcome = details.get("policy_outcome").map(String::as_str);
    if event.event_type == "policy.decided.v1" && outcome == Some("allow") {
        // Routine unbound allow decisions are audit evidence, not useful session
        // activity. Tool-bound decisions were attached above; retain standalone
        // rows only when a decision requires attention.
        return Ok(Vec::new());
    }
    let (title, summary, status) = match (event.event_type.as_str(), outcome) {
        ("policy.decided.v1", Some("deny")) => (
            "Policy denied".into(),
            policy_summary(&details, "Policy denied this operation"),
            Some("failed"),
        ),
        ("policy.decided.v1", Some("require_approval")) => (
            "Approval required".into(),
            policy_summary(&details, "Policy requires approval for this operation"),
            Some("waiting"),
        ),
        _ => (
            human_event_type(&event.event_type),
            "Policy or approval state changed".into(),
            event_status(&event.event_type),
        ),
    };
    upsert_new(
        session_id,
        event,
        format!("system:{}", event.event_id),
        "system",
        "system",
        &title,
        &summary,
        "System",
        status,
        None,
        None,
        None,
        details,
    )
}

fn tool_call_id(actor_id: &str) -> Option<&str> {
    actor_id
        .strip_prefix("tool-call:")
        .or_else(|| {
            actor_id
                .rsplit_once(":tool-call:")
                .map(|(_, call_id)| call_id)
        })
        .filter(|call_id| !call_id.is_empty())
}

fn policy_attributes(payload: &Value) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    for (source, target) in [
        ("decision_id", "decision_id"),
        ("policy_revision", "policy_revision"),
        ("outcome", "policy_outcome"),
        ("action", "action"),
        ("phase", "effect_phase"),
        ("sandbox_backend", "sandbox_boundary"),
        ("require_post_effect", "post_effect_review"),
        ("resource_authority", "resource_authority"),
        ("error_kind", "reason_code"),
    ] {
        if let Some(value) = payload.get(source).and_then(scalar) {
            insert_attribute(&mut attributes, target, &value);
        }
    }
    if let Some(action) = attributes.get("action").cloned()
        && let Some((category, _)) = action.split_once('.')
    {
        insert_attribute(&mut attributes, "action_category", category);
    }
    if !attributes.contains_key("reason_code") {
        let reason_code = match attributes.get("policy_outcome").map(String::as_str) {
            Some("deny") => Some("policy_denied"),
            Some("require_approval") => Some("approval_required"),
            Some("allow") => Some("policy_allowed"),
            _ => None,
        };
        if let Some(reason_code) = reason_code {
            insert_attribute(&mut attributes, "reason_code", reason_code);
        }
    }
    attributes
}

fn policy_summary(attributes: &BTreeMap<String, String>, fallback: &str) -> String {
    let mut details = Vec::new();
    if let Some(action) = attributes.get("action") {
        details.push(action.clone());
    }
    if let Some(boundary) = attributes.get("sandbox_boundary") {
        details.push(format!("{boundary} sandbox"));
    }
    if let Some(authority) = attributes.get("resource_authority") {
        details.push(format!("{authority} authority"));
    }
    if let Some(revision) = attributes.get("policy_revision") {
        details.push(format!("policy {revision}"));
    }
    if details.is_empty() {
        fallback.into()
    } else {
        details.join(" · ")
    }
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
    if reason.starts_with("run.phase.") || reason == "model.final_output" {
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
    store: &dyn ProjectionStore,
    session_id: &str,
    event: &EventEnvelope,
    kind: &Map<String, Value>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    let status = kind
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("running");
    let activity_status = match status {
        "queued" => "requested",
        "cancelling" => "running",
        "interrupted" => "failed",
        value => value,
    };
    let run_id = event.context.run_id.as_deref().unwrap_or_default();
    update_logical(
        store,
        session_id,
        event,
        &format!("run-lifecycle:{run_id}"),
        |record| {
            let title = match status {
                "running" => "Run started".into(),
                "completed" => "Run completed".into(),
                "cancelled" => "Run cancelled".into(),
                "failed" | "interrupted" | "outcome_unknown" => "Run failed".into(),
                value => format!("Run {}", value.replace('_', " ")),
            };
            record.run_id = event.context.run_id.clone();
            record.lane = "system".into();
            record.kind = "system".into();
            record.title = title;
            record.summary = "Run lifecycle changed".into();
            record.actor = "System".into();
            record.status = Some(activity_status.into());
            if terminal_status(activity_status) {
                finish_record(record, &event.occurred_at);
            }
        },
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
            record.run_id = event.context.run_id.clone();
            record.lane = "agent".into();
            record.kind = "assistant".into();
            set_assistant_actor(record, event);
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
        assistant_actor(event),
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
    mut attributes: BTreeMap<String, String>,
) -> Result<Vec<ProjectionMutation>, StoreError> {
    insert_lineage_attributes(&mut attributes, event);
    let record = ProjectedSessionActivity {
        activity_id: logical_id.clone(),
        session_id: session_id.into(),
        run_id: event.context.run_id.clone(),
        turn,
        lane: lane.into(),
        kind: kind.into(),
        title: bounded(title, MAX_ATTRIBUTE_BYTES),
        summary: bounded(summary, MAX_SUMMARY_BYTES),
        actor: activity_actor(event, actor).into(),
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
    insert_lineage_attributes(&mut record.attributes, event);
    if record.actor == "Assistant" && is_subagent_event(event) {
        record.actor = "Subagent".into();
    }
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

fn assistant_actor(event: &EventEnvelope) -> &'static str {
    if is_subagent_event(event) {
        "Subagent"
    } else {
        "Assistant"
    }
}

fn set_assistant_actor(record: &mut ProjectedSessionActivity, event: &EventEnvelope) {
    record.actor = if is_subagent_event(event)
        || record.attributes.get("run_role").map(String::as_str) == Some("subagent")
    {
        "Subagent".into()
    } else {
        "Assistant".into()
    };
}

fn activity_actor<'a>(event: &EventEnvelope, fallback: &'a str) -> &'a str {
    if fallback == "Assistant" && is_subagent_event(event) {
        "Subagent"
    } else {
        fallback
    }
}

fn is_subagent_event(event: &EventEnvelope) -> bool {
    event.context.subagent_id.is_some() || event.actor.actor_type == ActorType::Subagent
}

fn insert_lineage_attributes(attributes: &mut BTreeMap<String, String>, event: &EventEnvelope) {
    if let Some(subagent_id) = event.context.subagent_id.as_deref() {
        insert_attribute(attributes, "run_role", "subagent");
        insert_attribute(attributes, "subagent_id", subagent_id);
    } else if event.actor.actor_type == ActorType::Subagent {
        insert_attribute(attributes, "run_role", "subagent");
    } else if event.context.workflow_id.is_some() || event.actor.actor_type == ActorType::Workflow {
        insert_attribute(attributes, "run_role", "workflow");
    } else if event.context.run_id.is_some() && !attributes.contains_key("run_role") {
        insert_attribute(attributes, "run_role", "primary");
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
    fn coalesces_subagent_policy_and_effect_events_into_the_tool_activity() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        apply(
            &store,
            &mut position,
            &event(1, "tool.call.requested.v1", "run-a"),
            json!({
                "turn": 1,
                "call_id": "call-a",
                "name": "filesystem.read"
            }),
        );

        let mut policy_event = event(2, "policy.decided.v1", "run-a");
        policy_event.classification = EventClassification::Policy;
        policy_event.actor = Actor {
            actor_type: ActorType::Subagent,
            id: "subagent:agent-a:tool-call:call-a".into(),
        };
        apply(
            &store,
            &mut position,
            &policy_event,
            json!({
                "decision_id": "decision-a",
                "policy_revision": "builtin-v1",
                "outcome": "allow",
                "resource_authority": "declared",
                "reason": "private path /Users/example/.ssh/id_rsa",
                "audit_labels": {"private": "must-not-project"}
            }),
        );

        let mut effect_event = event(3, "effect.started.v1", "run-a");
        effect_event.classification = EventClassification::Effect;
        effect_event.actor = Actor {
            actor_type: ActorType::Subagent,
            id: "subagent:agent-a:tool-call:call-a".into(),
        };
        apply(&store, &mut position, &effect_event, json!({}));

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        assert_eq!(page.records.len(), 1);
        let activity = &page.records[0].1;
        assert_eq!(activity.kind, "tool");
        assert_eq!(
            activity
                .attributes
                .get("policy_outcome")
                .map(String::as_str),
            Some("allow")
        );
        assert_eq!(
            activity
                .attributes
                .get("resource_authority")
                .map(String::as_str),
            Some("declared")
        );
        assert!(
            activity
                .source_event_types
                .contains(&"effect.started.v1".into())
        );
        let encoded = serde_json::to_string(activity).expect("encode");
        assert!(!encoded.contains("/Users/example"));
        assert!(!encoded.contains("must-not-project"));
    }

    #[test]
    fn standalone_policy_decisions_expose_only_allowlisted_details() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        let mut policy_event = event(1, "policy.decided.v1", "run-a");
        policy_event.classification = EventClassification::Policy;
        apply(
            &store,
            &mut position,
            &policy_event,
            json!({
                "decision_id": "decision-a",
                "policy_revision": "builtin-v1",
                "outcome": "require_approval",
                "action": "process.spawn",
                "phase": "pre_effect",
                "sandbox_backend": "native",
                "require_post_effect": true,
                "resource_authority": "ambient",
                "reason": "secret policy explanation",
                "audit_labels": {"credential": "must-not-project"}
            }),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        let activity = &page.records[0].1;
        assert_eq!(activity.title, "Approval required");
        assert_eq!(activity.status.as_deref(), Some("waiting"));
        assert_eq!(
            activity
                .attributes
                .get("policy_revision")
                .map(String::as_str),
            Some("builtin-v1")
        );
        assert_eq!(
            activity
                .attributes
                .get("resource_authority")
                .map(String::as_str),
            Some("ambient")
        );
        assert_eq!(
            activity
                .attributes
                .get("action_category")
                .map(String::as_str),
            Some("process")
        );
        assert_eq!(
            activity
                .attributes
                .get("sandbox_boundary")
                .map(String::as_str),
            Some("native")
        );
        assert_eq!(
            activity.attributes.get("reason_code").map(String::as_str),
            Some("approval_required")
        );
        let encoded = serde_json::to_string(activity).expect("encode");
        assert!(!encoded.contains("secret policy explanation"));
        assert!(!encoded.contains("must-not-project"));
    }

    #[test]
    fn routine_unbound_policy_allows_stay_in_audit_instead_of_the_activity_feed() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        let mut policy_event = event(1, "policy.decided.v1", "run-a");
        policy_event.classification = EventClassification::Policy;
        apply(
            &store,
            &mut position,
            &policy_event,
            json!({
                "decision_id": "decision-a",
                "policy_revision": "builtin-v1",
                "outcome": "allow",
                "resource_authority": "declared"
            }),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        assert!(
            reader
                .list_page("session-a", None, 10)
                .expect("page")
                .records
                .is_empty()
        );
    }

    #[test]
    fn subagent_lineage_is_released_without_task_or_output_content() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        apply(
            &store,
            &mut position,
            &event(1, "subagent.status_changed.v1", "parent-run"),
            json!({
                "record": {
                    "id": "agent-a",
                    "session_id": "session-a",
                    "parent_run_id": "parent-run",
                    "parent_call_id": "call-a",
                    "task": "inspect /private/secret",
                    "role": "security-reviewer",
                    "status": "completed",
                    "child_session_id": "child-session",
                    "child_run_id": "child-run",
                    "final_output": "private child output",
                    "error": "",
                    "created_at": "2026-08-21T09:13:58Z",
                    "updated_at": "2026-08-21T09:14:01Z",
                    "started_at": "2026-08-21T09:14:00Z",
                    "completed_at": "2026-08-21T09:14:01Z"
                }
            }),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        let activity = &page.records[0].1;
        assert_eq!(activity.run_id.as_deref(), Some("child-run"));
        assert_eq!(activity.actor, "Subagent · security-reviewer");
        assert_eq!(activity.status.as_deref(), Some("completed"));
        assert_eq!(
            activity.attributes.get("parent_run_id").map(String::as_str),
            Some("parent-run")
        );
        assert_eq!(activity.duration_ms, Some(1_000));
        let encoded = serde_json::to_string(activity).expect("encode");
        assert!(!encoded.contains("/private/secret"));
        assert!(!encoded.contains("private child output"));
        assert!(!encoded.contains("child-session"));
    }

    #[test]
    fn child_model_results_keep_assistant_kind_and_subagent_actor() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        let mut request = event(1, "model.request.prepared.v1", "child-run");
        request.context.subagent_id = Some("agent-a".into());
        request.actor.actor_type = ActorType::Subagent;
        apply(&store, &mut position, &request, json!({"turn": 1}));

        // Public run updates can be emitted by the API system actor; the model
        // activity must retain lineage established by canonical child events.
        let result = event(2, "api.run.update.v1", "child-run");
        apply(
            &store,
            &mut position,
            &result,
            json!({"kind": {"type": "result", "result": {"output": "released"}}}),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        assert_eq!(page.records.len(), 1);
        let activity = &page.records[0].1;
        assert_eq!(activity.kind, "assistant");
        assert_eq!(activity.lane, "agent");
        assert_eq!(activity.actor, "Subagent");
        assert_eq!(
            activity.attributes.get("run_role").map(String::as_str),
            Some("subagent")
        );
    }

    #[test]
    fn run_state_usage_and_final_output_notice_are_coalesced() {
        let store = InMemoryProjectionStore::default();
        let mut position = 0;
        apply(
            &store,
            &mut position,
            &event(1, "run.started.v1", "run-a"),
            json!({}),
        );
        apply(
            &store,
            &mut position,
            &event(2, "api.run.update.v1", "run-a"),
            json!({"kind": {"type": "state", "status": "running"}}),
        );
        for (sequence, total) in [(3, 10), (4, 25)] {
            apply(
                &store,
                &mut position,
                &event(sequence, "api.run.update.v1", "run-a"),
                json!({
                    "kind": {
                        "type": "usage",
                        "usage": {"input_tokens": total, "output_tokens": 0, "total_tokens": total}
                    }
                }),
            );
        }
        apply(
            &store,
            &mut position,
            &event(5, "api.run.update.v1", "run-a"),
            json!({
                "kind": {
                    "type": "notice",
                    "notice": {
                        "reason": "model.final_output",
                        "message": "the final visible output is available in the run result"
                    }
                }
            }),
        );

        let reader = ProjectedSessionActivityReader::new(Arc::new(store));
        let page = reader.list_page("session-a", None, 10).expect("page");
        assert_eq!(page.records.len(), 2);
        assert_eq!(
            page.records
                .iter()
                .filter(|(_, activity)| activity.title == "Token usage")
                .count(),
            1
        );
        assert!(
            page.records
                .iter()
                .any(|(_, activity)| activity.summary == "25 tokens used")
        );
        assert!(
            page.records
                .iter()
                .all(|(_, activity)| activity.title != "Run notice")
        );
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
