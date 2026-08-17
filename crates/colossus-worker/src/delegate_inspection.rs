use super::*;
use colossus_worker_protocol::{
    WorkerDelegateActivity, WorkerDelegateActivityState, WorkerDelegateStatus,
    WorkerThreadDelegateInspection,
};

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_ACTIVITY_TEXT_BYTES: usize = 64 * 1024;
const MAX_DELEGATE_ACTIVITIES: usize = 24;
const TRUNCATION_MARKER: &str = "\n… truncated by Colossus Desktop";

#[derive(Default)]
struct MutableActivity {
    call_id: String,
    tool_name: String,
    state: Option<WorkerDelegateActivityState>,
    summary: String,
    input: Option<String>,
    preview: Option<String>,
    started_at: String,
    completed_at: Option<String>,
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let content_limit = limit.saturating_sub(TRUNCATION_MARKER.len());
    let mut end = content_limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = String::with_capacity(limit);
    bounded.push_str(&value[..end]);
    bounded.push_str(TRUNCATION_MARKER);
    bounded
}

fn required_payload_string(payload: &Value, field: &str) -> Option<String> {
    let value = payload.get(field)?.as_str()?;
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES {
        return None;
    }
    Some(value.to_owned())
}

fn payload_turn(payload: &Value) -> u64 {
    payload.get("turn").and_then(Value::as_u64).unwrap_or(0)
}

fn delegate_status(status: SubagentStatus) -> WorkerDelegateStatus {
    match status {
        SubagentStatus::Queued => WorkerDelegateStatus::Queued,
        SubagentStatus::Running => WorkerDelegateStatus::Running,
        SubagentStatus::Completed => WorkerDelegateStatus::Completed,
        SubagentStatus::Failed => WorkerDelegateStatus::Failed,
        SubagentStatus::Cancelled => WorkerDelegateStatus::Cancelled,
        SubagentStatus::Interrupted => WorkerDelegateStatus::Interrupted,
    }
}

fn activity_index(
    positions: &mut BTreeMap<String, usize>,
    activities: &mut Vec<MutableActivity>,
    call_id: &str,
) -> usize {
    if let Some(index) = positions.get(call_id) {
        return *index;
    }
    let index = activities.len();
    positions.insert(call_id.to_owned(), index);
    activities.push(MutableActivity {
        call_id: call_id.to_owned(),
        ..MutableActivity::default()
    });
    index
}

fn released_activities(
    journal: &dyn colossus_ports::EventJournal,
    job_id: &str,
    child_session_id: &str,
    child_run_id: &str,
) -> Result<Vec<WorkerDelegateActivity>, WorkerError> {
    let stream_id = format!("run:{child_run_id}");
    let events = journal.read_stream(&stream_id)?;
    let mut requested_inputs = BTreeMap::<String, String>::new();
    let mut positions = BTreeMap::<String, usize>::new();
    let mut activities = Vec::<MutableActivity>::new();

    for event in events {
        if event.context.run_id.as_deref() != Some(child_run_id)
            || event.context.session_id.as_deref() != Some(child_session_id)
            || event.context.subagent_id.as_deref() != Some(job_id)
        {
            continue;
        }
        if !matches!(
            event.event_type.as_str(),
            "tool.call.requested.v1"
                | "tool.call.started.v1"
                | "tool.call.completed.v1"
                | "tool.call.cancelled.v1"
        ) {
            continue;
        }
        let payload = journal.decrypt_payload(&event)?;
        let Some(call_id) = required_payload_string(&payload, "call_id") else {
            continue;
        };
        match event.event_type.as_str() {
            "tool.call.requested.v1" => {
                if let Some(arguments) = payload.get("arguments") {
                    requested_inputs.insert(
                        call_id,
                        bounded_text(&arguments.to_string(), MAX_ACTIVITY_TEXT_BYTES),
                    );
                }
            }
            "tool.call.started.v1" => {
                let Some(tool_name) = required_payload_string(&payload, "name") else {
                    continue;
                };
                if tool_name.len() > MAX_TOOL_NAME_BYTES {
                    continue;
                }
                let index = activity_index(&mut positions, &mut activities, &call_id);
                let activity = &mut activities[index];
                activity.tool_name = tool_name;
                activity.state = Some(WorkerDelegateActivityState::Started);
                activity.summary =
                    format!("tool execution started at turn {}", payload_turn(&payload));
                activity.input = requested_inputs.remove(&call_id);
                activity.started_at = event.occurred_at;
            }
            "tool.call.completed.v1" => {
                let Some(tool_name) = required_payload_string(&payload, "name") else {
                    continue;
                };
                if tool_name.len() > MAX_TOOL_NAME_BYTES {
                    continue;
                }
                let exit_code = payload
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                let index = activity_index(&mut positions, &mut activities, &call_id);
                let activity = &mut activities[index];
                activity.tool_name = tool_name;
                activity.state = Some(if exit_code == 0 {
                    WorkerDelegateActivityState::Completed
                } else {
                    WorkerDelegateActivityState::Failed
                });
                activity.summary = if exit_code == 0 {
                    "tool execution completed".into()
                } else {
                    "tool execution failed".into()
                };
                if activity.started_at.is_empty() {
                    activity.started_at = event.occurred_at.clone();
                }
                activity.completed_at = Some(event.occurred_at);
                if exit_code == 0 {
                    activity.preview = payload
                        .get("output")
                        .and_then(Value::as_str)
                        .map(|output| bounded_text(output, MAX_ACTIVITY_TEXT_BYTES));
                }
            }
            "tool.call.cancelled.v1" => {
                let Some(tool_name) = required_payload_string(&payload, "name") else {
                    continue;
                };
                if tool_name.len() > MAX_TOOL_NAME_BYTES {
                    continue;
                }
                let index = activity_index(&mut positions, &mut activities, &call_id);
                let activity = &mut activities[index];
                activity.tool_name = tool_name;
                activity.state = Some(WorkerDelegateActivityState::Cancelled);
                activity.summary = format!(
                    "tool execution was cancelled before start at turn {}",
                    payload_turn(&payload)
                );
                activity.started_at = event.occurred_at.clone();
                activity.completed_at = Some(event.occurred_at);
            }
            _ => {}
        }
    }

    Ok(activities
        .into_iter()
        .filter_map(|activity| {
            Some(WorkerDelegateActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state: activity.state?,
                summary: bounded_text(&activity.summary, MAX_ACTIVITY_TEXT_BYTES),
                input: activity.input,
                preview: activity.preview,
                started_at: activity.started_at,
                completed_at: activity.completed_at,
            })
        })
        .rev()
        .take(MAX_DELEGATE_ACTIVITIES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

pub(super) fn inspect_thread_delegate(
    runtime: &Runtime,
    parent_run_id: &str,
    job_id: &str,
) -> Result<WorkerThreadDelegateInspection, WorkerError> {
    if parent_run_id.is_empty()
        || parent_run_id.len() > MAX_IDENTIFIER_BYTES
        || job_id.is_empty()
        || job_id.len() > MAX_IDENTIFIER_BYTES
    {
        return Err(WorkerError::Protocol(
            "delegate inspection identifiers are invalid".into(),
        ));
    }
    let job = runtime
        .get_subagent(job_id)?
        .ok_or_else(|| WorkerError::Protocol("delegated agent was not found".into()))?;
    if job.parent_run_id != parent_run_id {
        return Err(WorkerError::Protocol(
            "delegated agent does not belong to the selected parent run".into(),
        ));
    }
    let activities = match job.child_run_id.as_deref() {
        Some(child_run_id) => released_activities(
            runtime.journal().as_ref(),
            &job.id,
            &job.child_session_id,
            child_run_id,
        )?,
        None => Vec::new(),
    };
    Ok(WorkerThreadDelegateInspection {
        job_id: job.id,
        parent_run_id: job.parent_run_id,
        child_session_id: job.child_session_id,
        child_run_id: job.child_run_id,
        task: bounded_text(&job.task, MAX_ACTIVITY_TEXT_BYTES),
        role: bounded_text(&job.role, MAX_IDENTIFIER_BYTES),
        status: delegate_status(job.status),
        final_output: bounded_text(&job.final_output, MAX_ACTIVITY_TEXT_BYTES),
        error: bounded_text(&job.error, MAX_ACTIVITY_TEXT_BYTES),
        created_at: job.created_at,
        updated_at: job.updated_at,
        started_at: job.started_at,
        completed_at: job.completed_at,
        activities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
    use colossus_ports::EventJournal as _;
    use colossus_testkit::InMemoryEventJournal;

    fn append(
        journal: &InMemoryEventJournal,
        version: u64,
        event_type: &str,
        payload: Value,
        subagent_id: &str,
    ) {
        journal
            .append(NewEvent {
                event_version: 1,
                stream_id: "run:run-child".into(),
                expected_stream_version: version,
                classification: EventClassification::Domain,
                event_type: event_type.into(),
                actor: Actor {
                    actor_type: ActorType::System,
                    id: "worker-test".into(),
                },
                context: ExecutionContext {
                    correlation_id: "run-child".into(),
                    session_id: Some("session-child".into()),
                    run_id: Some("run-child".into()),
                    subagent_id: Some(subagent_id.into()),
                    ..ExecutionContext::default()
                },
                payload,
            })
            .expect("append child event");
    }

    #[test]
    fn child_activity_projection_is_lineage_bound_and_omits_model_events() {
        let journal = InMemoryEventJournal::default();
        append(
            &journal,
            0,
            "tool.call.requested.v1",
            json!({
                "call_id": "call-shell",
                "name": "shell.run",
                "arguments": {"command": "pwd"},
            }),
            "agent-child",
        );
        append(
            &journal,
            1,
            "tool.call.started.v1",
            json!({"turn": 1, "call_id": "call-shell", "name": "shell.run"}),
            "agent-child",
        );
        append(
            &journal,
            2,
            "reasoning.summary.v1",
            json!({"summary": "private child reasoning"}),
            "agent-child",
        );
        append(
            &journal,
            3,
            "tool.call.completed.v1",
            json!({
                "call_id": "call-shell",
                "name": "shell.run",
                "output": "/workspace",
                "exit_code": 0,
            }),
            "agent-child",
        );
        append(
            &journal,
            4,
            "tool.call.completed.v1",
            json!({
                "call_id": "call-cross-lineage",
                "name": "filesystem.read",
                "output": "must not be released",
                "exit_code": 0,
            }),
            "agent-other",
        );

        let activities = released_activities(&journal, "agent-child", "session-child", "run-child")
            .expect("released activities");
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].call_id, "call-shell");
        assert_eq!(activities[0].tool_name, "shell.run");
        assert_eq!(activities[0].input.as_deref(), Some(r#"{"command":"pwd"}"#));
        assert_eq!(activities[0].preview.as_deref(), Some("/workspace"));
        assert!(!format!("{activities:?}").contains("private child reasoning"));
        assert!(!format!("{activities:?}").contains("must not be released"));
    }
}
