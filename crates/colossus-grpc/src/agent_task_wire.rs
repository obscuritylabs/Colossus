//! Typed curated task translation shared by the server and Rust SDK.
use crate::communication_wire::validate_token;
use colossus_api::*;
use colossus_api_proto::v1alpha1 as proto;

/// Stable lowercase lifecycle label.
pub fn status_to_wire(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Waiting => "waiting",
        RunStatus::Cancelling => "cancelling",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
        RunStatus::OutcomeUnknown => "outcome_unknown",
    }
}
/// Decode known lifecycle values only.
pub fn status_from_wire(value: &str) -> ApiResult<RunStatus> {
    match value {
        "queued" => Ok(RunStatus::Queued),
        "running" => Ok(RunStatus::Running),
        "waiting" => Ok(RunStatus::Waiting),
        "cancelling" => Ok(RunStatus::Cancelling),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "cancelled" => Ok(RunStatus::Cancelled),
        "interrupted" => Ok(RunStatus::Interrupted),
        "outcome_unknown" => Ok(RunStatus::OutcomeUnknown),
        _ => Err(invalid()),
    }
}
/// Encode a released task without private transcript data.
pub fn task_to_proto(value: AgentTaskSnapshot) -> proto::AgentTaskSnapshot {
    proto::AgentTaskSnapshot {
        task_id: value.task_id,
        context_id: value.context_id,
        status: status_to_wire(value.status).into(),
        status_updated_at: value.status_updated_at,
        last_sequence: value.last_sequence,
        output: value.output,
        failure: value.failure.map(|failure| proto::AgentTaskFailure {
            code: failure.code,
            message: failure.message,
            outcome_unknown: failure.outcome_unknown,
        }),
        waiting_kind: value.waiting_kind,
        history: value
            .history
            .into_iter()
            .map(|input| proto::AgentTaskInput {
                message_id: input.message_id,
                task_id: input.task_id,
                context_id: input.context_id,
                text: input.text,
                accepted_at: input.accepted_at,
                inbox_message_id: input.inbox_message_id,
            })
            .collect(),
    }
}
/// Decode a bounded task while checking identity and curated history coherence.
pub fn task_from_proto(value: proto::AgentTaskSnapshot) -> ApiResult<AgentTaskSnapshot> {
    validate_token(&value.task_id)?;
    validate_token(&value.context_id)?;
    let status = status_from_wire(&value.status)?;
    if value.history.len() > 16
        || value.last_sequence == 0
        || value.status_updated_at.len() > 64
        || value
            .output
            .as_ref()
            .is_some_and(|output| output.len() > 1024 * 1024)
        || value
            .waiting_kind
            .as_deref()
            .is_some_and(|kind| !matches!(kind, "approval" | "user_prompt"))
    {
        return Err(invalid());
    }
    let history = value
        .history
        .into_iter()
        .map(|input| {
            validate_token(&input.message_id)?;
            if let Some(id) = &input.inbox_message_id {
                validate_token(id)?;
            }
            if input.task_id != value.task_id
                || input.context_id != value.context_id
                || input.text.is_empty()
                || input.text.len() > MAX_AGENT_MESSAGE_BYTES
                || input.accepted_at.len() > 64
            {
                return Err(invalid());
            }
            Ok(AgentTaskInput {
                message_id: input.message_id,
                task_id: input.task_id,
                context_id: input.context_id,
                text: input.text,
                accepted_at: input.accepted_at,
                inbox_message_id: input.inbox_message_id,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let failure = value
        .failure
        .map(|failure| {
            if failure.code.len() > 128 || failure.message.len() > 4096 {
                return Err(invalid());
            }
            Ok(AgentTaskFailure {
                code: failure.code,
                message: failure.message,
                outcome_unknown: failure.outcome_unknown,
            })
        })
        .transpose()?;
    Ok(AgentTaskSnapshot {
        task_id: value.task_id,
        context_id: value.context_id,
        status,
        status_updated_at: value.status_updated_at,
        last_sequence: value.last_sequence,
        output: value.output,
        failure,
        waiting_kind: value.waiting_kind,
        history,
    })
}
fn invalid() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InternalInvariant,
        "invalid agent task response",
    )
}
