//! Shared typed communication translation for server and Rust SDK transport.
use colossus_api::{
    AgentMessage, AgentMessageFailure, AgentMessageReceipt, AgentMessageSender, AgentParticipant,
    ApiError, ApiErrorReason, ApiResult,
};
use colossus_api_proto::v1alpha1 as proto;

fn invalid() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InternalInvariant,
        "invalid agent communication response",
    )
}

/// Encode a categorical failure without private diagnostics.
pub fn failure_to_wire(reason: AgentMessageFailure) -> &'static str {
    match reason {
        AgentMessageFailure::Completed => "completed",
        AgentMessageFailure::Cancelled => "cancelled",
        AgentMessageFailure::Failed => "failed",
        AgentMessageFailure::Interrupted => "interrupted",
        AgentMessageFailure::BudgetExhausted => "budget_exhausted",
        AgentMessageFailure::Superseded => "superseded",
    }
}
/// Decode only known categorical failure values.
pub fn failure_from_wire(value: &str) -> ApiResult<AgentMessageFailure> {
    match value {
        "completed" => Ok(AgentMessageFailure::Completed),
        "cancelled" => Ok(AgentMessageFailure::Cancelled),
        "failed" => Ok(AgentMessageFailure::Failed),
        "interrupted" => Ok(AgentMessageFailure::Interrupted),
        "budget_exhausted" => Ok(AgentMessageFailure::BudgetExhausted),
        "superseded" => Ok(AgentMessageFailure::Superseded),
        _ => Err(invalid()),
    }
}

/// Encode one owner-scoped participant.
pub fn participant_to_proto(value: AgentParticipant) -> proto::AgentParticipant {
    proto::AgentParticipant {
        id: value.id,
        root_run_id: value.root_run_id,
        owner_application_id: value.owner.id,
        session_id: value.session_id,
        run_id: value.run_id,
        parent_id: value.parent_id,
        subagent_id: value.subagent_id,
        generation: value.generation,
        open: value.open,
        closed_reason: value
            .closed_reason
            .map(|reason| failure_to_wire(reason).into()),
        pending_messages: value.pending_messages as u32,
        pending_bytes: value.pending_bytes as u32,
        created_at: value.created_at,
    }
}
/// Decode a bounded participant from an authenticated response.
pub fn participant_from_proto(value: proto::AgentParticipant) -> ApiResult<AgentParticipant> {
    for id in [
        &value.id,
        &value.root_run_id,
        &value.owner_application_id,
        &value.session_id,
    ] {
        validate_token(id)?;
    }
    for id in [&value.run_id, &value.parent_id, &value.subagent_id]
        .into_iter()
        .flatten()
    {
        validate_token(id)?;
    }
    if value.generation == 0
        || value.pending_messages > 64
        || value.pending_bytes > 256 * 1024
        || value.created_at.len() > 64
        || value.open == value.closed_reason.is_some()
    {
        return Err(invalid());
    }
    Ok(AgentParticipant {
        id: value.id,
        root_run_id: value.root_run_id,
        owner: colossus_contracts::Actor {
            actor_type: colossus_contracts::ActorType::Application,
            id: value.owner_application_id,
        },
        session_id: value.session_id,
        run_id: value.run_id,
        parent_id: value.parent_id,
        subagent_id: value.subagent_id,
        generation: value.generation,
        open: value.open,
        closed_reason: value
            .closed_reason
            .as_deref()
            .map(failure_from_wire)
            .transpose()?,
        pending_messages: value.pending_messages as usize,
        pending_bytes: value.pending_bytes as usize,
        created_at: value.created_at,
    })
}

/// Encode one message and the exact receipt at a journal cursor.
pub fn message_to_proto(value: AgentMessage) -> proto::AgentMessage {
    let receipt = match value.receipt {
        AgentMessageReceipt::Accepted => proto::AgentMessageReceipt {
            state: "accepted".into(),
            ..Default::default()
        },
        AgentMessageReceipt::IncludedInTurn {
            run_id,
            turn,
            request_hash,
        } => proto::AgentMessageReceipt {
            state: "included_in_turn".into(),
            run_id: Some(run_id),
            turn: Some(u32::from(turn)),
            request_hash: Some(request_hash),
            reason: None,
        },
        AgentMessageReceipt::NotDelivered { reason } => proto::AgentMessageReceipt {
            state: "not_delivered".into(),
            reason: Some(failure_to_wire(reason).into()),
            ..Default::default()
        },
    };
    proto::AgentMessage {
        id: value.id,
        root_run_id: value.root_run_id,
        sender: Some(match value.sender {
            AgentMessageSender::Participant { participant_id } => {
                proto::agent_message::Sender::SenderParticipantId(participant_id)
            }
            AgentMessageSender::Application { application_id } => {
                proto::agent_message::Sender::SenderApplicationId(application_id)
            }
        }),
        recipient_id: value.recipient_id,
        sequence: value.sequence,
        text: value.text,
        reply_to: value.reply_to,
        accepted_at: value.accepted_at,
        receipt: Some(receipt),
    }
}

/// Decode a bounded message without accepting malformed receipt or provenance fields.
pub fn message_from_proto(value: proto::AgentMessage) -> ApiResult<AgentMessage> {
    for id in [&value.id, &value.root_run_id, &value.recipient_id] {
        validate_token(id)?;
    }
    if let Some(id) = &value.reply_to {
        validate_token(id)?;
    }
    if value.sequence == 0
        || value.text.is_empty()
        || value.text.len() > colossus_contracts::MAX_AGENT_MESSAGE_BYTES
        || value.accepted_at.len() > 64
    {
        return Err(invalid());
    }
    let sender = match value.sender.ok_or_else(invalid)? {
        proto::agent_message::Sender::SenderParticipantId(participant_id) => {
            validate_token(&participant_id)?;
            AgentMessageSender::Participant { participant_id }
        }
        proto::agent_message::Sender::SenderApplicationId(application_id) => {
            validate_token(&application_id)?;
            AgentMessageSender::Application { application_id }
        }
    };
    let receipt = value.receipt.ok_or_else(invalid)?;
    let receipt = match receipt.state.as_str() {
        "accepted"
            if receipt.run_id.is_none()
                && receipt.turn.is_none()
                && receipt.request_hash.is_none()
                && receipt.reason.is_none() =>
        {
            AgentMessageReceipt::Accepted
        }
        "included_in_turn" if receipt.reason.is_none() => {
            let run_id = receipt.run_id.ok_or_else(invalid)?;
            validate_token(&run_id)?;
            let turn = receipt
                .turn
                .and_then(|turn| u16::try_from(turn).ok())
                .filter(|turn| *turn > 0)
                .ok_or_else(invalid)?;
            let request_hash = receipt.request_hash.ok_or_else(invalid)?;
            if request_hash.len() != 64
                || !request_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(invalid());
            }
            AgentMessageReceipt::IncludedInTurn {
                run_id,
                turn,
                request_hash,
            }
        }
        "not_delivered"
            if receipt.run_id.is_none()
                && receipt.turn.is_none()
                && receipt.request_hash.is_none() =>
        {
            AgentMessageReceipt::NotDelivered {
                reason: failure_from_wire(&receipt.reason.ok_or_else(invalid)?)?,
            }
        }
        _ => return Err(invalid()),
    };
    Ok(AgentMessage {
        id: value.id,
        root_run_id: value.root_run_id,
        sender,
        recipient_id: value.recipient_id,
        sequence: value.sequence,
        text: value.text,
        reply_to: value.reply_to,
        accepted_at: value.accepted_at,
        receipt,
    })
}

/// Validate communication identifiers before decoding/application work.
pub fn validate_token(value: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_: .".contains(&byte))
        || value.contains(' ')
    {
        return Err(invalid());
    }
    Ok(())
}
