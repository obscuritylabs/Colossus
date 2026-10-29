use super::*;
use colossus_presentation::{ToolActivityPhase, tool_activity_presentation};
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

fn activity_summary(activity: &MutableActivity, state: WorkerDelegateActivityState) -> String {
    let phase = match state {
        WorkerDelegateActivityState::Started => ToolActivityPhase::Started,
        WorkerDelegateActivityState::Completed => ToolActivityPhase::Completed,
        WorkerDelegateActivityState::Cancelled => ToolActivityPhase::Cancelled,
        WorkerDelegateActivityState::Failed => ToolActivityPhase::Failed,
    };
    // These fields have already passed this inspection's existing release gates:
    // validated input after start, and successful post-policy output only.
    let input = activity
        .input
        .as_deref()
        .filter(|input| input.len() <= colossus_presentation::TOOL_ACTIVITY_DETAIL_BYTES)
        .and_then(|input| serde_json::from_str(input).ok());
    let output = activity
        .preview
        .as_deref()
        .filter(|_| state == WorkerDelegateActivityState::Completed);
    tool_activity_presentation(&activity.tool_name, phase, input.as_ref(), output).summary()
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
                activity.started_at = event.occurred_at.clone();
                activity.completed_at = Some(event.occurred_at);
            }
            _ => {}
        }
    }

    Ok(activities
        .into_iter()
        .filter_map(|activity| {
            let state = activity.state?;
            let summary = activity_summary(&activity, state);
            Some(WorkerDelegateActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state,
                summary,
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

    #[test]
    fn inspected_repository_map_uses_shared_summary_from_released_fields() {
        let journal = InMemoryEventJournal::default();
        let input = json!({"root": "src"});
        append(
            &journal,
            0,
            "tool.call.requested.v1",
            json!({"call_id": "call-map", "name": "repo.map", "arguments": input}),
            "agent-child",
        );
        append(
            &journal,
            1,
            "tool.call.started.v1",
            json!({"turn": 1, "call_id": "call-map", "name": "repo.map"}),
            "agent-child",
        );
        let started = released_activities(&journal, "agent-child", "session-child", "run-child")
            .expect("started repository map");
        assert_eq!(started.len(), 1);
        assert_eq!(
            started[0].summary,
            tool_activity_presentation("repo.map", ToolActivityPhase::Started, Some(&input), None)
                .summary()
        );
        assert_eq!(started[0].summary, "Map repository · src · running");

        let output = json!({"file_count": 3, "files": ["body is not a row label"]}).to_string();
        append(
            &journal,
            2,
            "tool.call.completed.v1",
            json!({
                "call_id": "call-map", "name": "repo.map", "output": output, "exit_code": 0,
            }),
            "agent-child",
        );
        let completed = released_activities(&journal, "agent-child", "session-child", "run-child")
            .expect("completed repository map");
        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].summary,
            tool_activity_presentation(
                "repo.map",
                ToolActivityPhase::Completed,
                Some(&input),
                Some(&output),
            )
            .summary()
        );
        assert_eq!(
            completed[0].summary,
            "Map repository · src · Files: 3 · completed"
        );
        assert_eq!(completed[0].preview.as_deref(), Some(output.as_str()));
        assert!(!completed[0].summary.contains("body is not a row label"));
    }

    #[test]
    fn inspected_summaries_never_use_failed_output_or_unstarted_input() {
        let journal = InMemoryEventJournal::default();
        let events = [
            (
                "tool.call.requested.v1",
                json!({
                    "call_id": "call-failed", "name": "repo.map", "arguments": {"root": "src"},
                }),
            ),
            (
                "tool.call.started.v1",
                json!({"call_id": "call-failed", "name": "repo.map"}),
            ),
            (
                "tool.call.completed.v1",
                json!({
                    "call_id": "call-failed", "name": "repo.map", "exit_code": 1,
                    "output": json!({
                        "root": "WITHHELD_OUTPUT_ROOT", "file_count": 9001,
                        "error": {"message": "WITHHELD_ERROR"},
                    }).to_string(),
                }),
            ),
            (
                "tool.call.requested.v1",
                json!({
                    "call_id": "call-cancelled", "name": "repo.map",
                    "arguments": {"root": "WITHHELD_UNSTARTED_INPUT"},
                }),
            ),
            (
                "tool.call.cancelled.v1",
                json!({
                    "call_id": "call-cancelled", "name": "repo.map",
                    "output": "WITHHELD_CANCELLED_OUTPUT",
                }),
            ),
        ];
        for (version, (event_type, payload)) in events.into_iter().enumerate() {
            append(&journal, version as u64, event_type, payload, "agent-child");
        }

        let activities = released_activities(&journal, "agent-child", "session-child", "run-child")
            .expect("released activities");
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0].state, WorkerDelegateActivityState::Failed);
        assert_eq!(
            activities[0].summary,
            "Map repository · src · Reason: Tool reported failure · failed"
        );
        assert_eq!(activities[0].preview, None);
        assert_eq!(activities[1].state, WorkerDelegateActivityState::Cancelled);
        assert_eq!(activities[1].summary, "Map repository · cancelled");
        assert_eq!(activities[1].input, None);
        assert_eq!(activities[1].preview, None);
        let released = format!("{activities:?}");
        assert!(!released.contains("WITHHELD_"));
        assert!(!released.contains("9001"));
    }
}
