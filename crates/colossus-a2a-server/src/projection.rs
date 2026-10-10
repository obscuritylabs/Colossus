use colossus_sdk::{AgentTaskSnapshot, AgentTaskStatus};
use serde_json::{Value, json};

pub(crate) fn state(status: AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Queued => "TASK_STATE_SUBMITTED",
        AgentTaskStatus::Running | AgentTaskStatus::Cancelling => "TASK_STATE_WORKING",
        AgentTaskStatus::Waiting => "TASK_STATE_INPUT_REQUIRED",
        AgentTaskStatus::Completed => "TASK_STATE_COMPLETED",
        AgentTaskStatus::Cancelled => "TASK_STATE_CANCELED",
        AgentTaskStatus::Failed
        | AgentTaskStatus::Interrupted
        | AgentTaskStatus::OutcomeUnknown => "TASK_STATE_FAILED",
    }
}
pub(crate) fn filters(state: Option<&str>) -> Result<Vec<AgentTaskStatus>, crate::http::RpcError> {
    Ok(match state {
        None | Some("TASK_STATE_UNSPECIFIED") => Vec::new(),
        Some("TASK_STATE_SUBMITTED") => vec![AgentTaskStatus::Queued],
        Some("TASK_STATE_WORKING") => vec![AgentTaskStatus::Running, AgentTaskStatus::Cancelling],
        Some("TASK_STATE_INPUT_REQUIRED") => vec![AgentTaskStatus::Waiting],
        Some("TASK_STATE_COMPLETED") => vec![AgentTaskStatus::Completed],
        Some("TASK_STATE_CANCELED") => vec![AgentTaskStatus::Cancelled],
        Some("TASK_STATE_FAILED") => vec![
            AgentTaskStatus::Failed,
            AgentTaskStatus::Interrupted,
            AgentTaskStatus::OutcomeUnknown,
        ],
        Some("TASK_STATE_AUTH_REQUIRED" | "TASK_STATE_REJECTED") => {
            return Err(crate::http::RpcError::unsupported());
        }
        _ => return Err(crate::http::RpcError::invalid()),
    })
}
pub(crate) fn artifact(task: &AgentTaskSnapshot) -> Option<Value> {
    task.output.as_ref().map(|output| json!({"artifactId": format!("{}:result", task.task_id), "name": "Result", "parts": [{"text": output, "mediaType": "text/plain"}]}))
}
pub(crate) fn status(task: &AgentTaskSnapshot) -> Value {
    let mut value = json!({"state": state(task.status), "timestamp": task.status_updated_at});
    let text = task
        .failure
        .as_ref()
        .map(|failure| failure.message.clone())
        .or_else(|| {
            task.waiting_kind.as_ref().map(|kind| {
                if kind == "approval" {
                    "This task needs local approval. Peer messages cannot approve effects.".into()
                } else {
                    "This task needs an authorized local prompt response.".into()
                }
            })
        });
    if let Some(text) = text {
        value["message"] = json!({"messageId": format!("{}:status:{}", task.task_id, task.last_sequence), "taskId": task.task_id, "contextId": task.context_id, "role": "ROLE_AGENT", "parts": [{"text": text}]});
    }
    value
}
pub(crate) fn task(task: &AgentTaskSnapshot, include_artifacts: bool) -> Value {
    let mut value =
        json!({"id": task.task_id, "contextId": task.context_id, "status": status(task)});
    if include_artifacts && let Some(artifact) = artifact(task) {
        value["artifacts"] = json!([artifact]);
    }
    if !task.history.is_empty() {
        value["history"] = json!(task.history.iter().map(|input| json!({"messageId": input.message_id, "taskId": input.task_id, "contextId": input.context_id, "role": "ROLE_USER", "parts": [{"text": input.text}]})).collect::<Vec<_>>());
    }
    if task
        .failure
        .as_ref()
        .is_some_and(|failure| failure.outcome_unknown)
    {
        value["metadata"] = json!({"colossusOutcomeUnknown": true});
    }
    value
}
