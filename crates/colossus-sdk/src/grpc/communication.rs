use super::*;
use colossus_grpc::communication_wire::{
    message_from_proto, participant_from_proto, validate_token,
};
use proto::agent_communication_service_client::AgentCommunicationServiceClient;

fn client(parent: &GrpcAgentRunClient) -> AgentCommunicationServiceClient<Channel> {
    AgentCommunicationServiceClient::new(parent.channel.clone())
        .max_decoding_message_size(2 * 1024 * 1024)
        .max_encoding_message_size(32 * 1024)
}

pub(super) async fn participants(
    parent: &GrpcAgentRunClient,
    request: crate::ListAgentParticipantsRequest,
) -> ApiResult<Vec<crate::AgentParticipant>> {
    validate_token(&request.root_run_id)?;
    let root = request.root_run_id.clone();
    let response = client(parent)
        .list_agent_participants(
            parent
                .request(proto::ListAgentParticipantsRequest {
                    root_run_id: request.root_run_id,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?
        .into_inner();
    if response.participants.len() > 128 {
        return Err(protocol_error());
    }
    response
        .participants
        .into_iter()
        .map(|value| {
            let participant = participant_from_proto(value)?;
            if participant.root_run_id != root {
                return Err(protocol_error());
            }
            Ok(participant)
        })
        .collect()
}

pub(super) async fn send(
    parent: &GrpcAgentRunClient,
    request: crate::SendAgentMessage,
) -> ApiResult<crate::AgentMessage> {
    validate_token(&request.recipient_id)?;
    validate_token(&request.idempotency_key)?;
    if request.text.is_empty() || request.text.len() > colossus_api::MAX_AGENT_MESSAGE_BYTES {
        return Err(invalid_request(
            "text",
            "agent message must fit its UTF-8 byte bound",
        ));
    }
    let recipient = request.recipient_id.clone();
    let response = client(parent)
        .send_agent_message(
            parent
                .request(proto::SendAgentMessageRequest {
                    recipient_id: request.recipient_id,
                    text: request.text,
                    idempotency_key: request.idempotency_key,
                    reply_to: request.reply_to,
                })
                .await?,
        )
        .await
        .map_err(api_error_from_status)?
        .into_inner();
    let message = message_from_proto(required(response.message)?)?;
    if message.recipient_id != recipient {
        return Err(protocol_error());
    }
    Ok(message)
}

pub(super) async fn get(
    parent: &GrpcAgentRunClient,
    request: crate::GetAgentMessageRequest,
) -> ApiResult<crate::AgentMessage> {
    validate_token(&request.message_id)?;
    let id = request.message_id.clone();
    let response = client(parent)
        .get_agent_message(
            parent
                .request(proto::GetAgentMessageRequest {
                    message_id: request.message_id,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?
        .into_inner();
    let message = message_from_proto(required(response.message)?)?;
    if message.id != id {
        return Err(protocol_error());
    }
    Ok(message)
}

pub(super) async fn messages(
    parent: &GrpcAgentRunClient,
    request: crate::ListAgentMessagesRequest,
) -> ApiResult<crate::AgentMessagePage> {
    validate_token(&request.participant_id)?;
    if !(1..=16).contains(&request.limit) {
        return Err(invalid_request(
            "limit",
            "agent message page limit must be from 1 through 16",
        ));
    }
    let recipient = request.participant_id.clone();
    let after = request.after_sequence;
    let limit = request.limit as usize;
    let response = client(parent)
        .list_agent_messages(
            parent
                .request(proto::ListAgentMessagesRequest {
                    participant_id: request.participant_id,
                    after_sequence: request.after_sequence,
                    limit: request.limit,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?
        .into_inner();
    if response.messages.len() > limit {
        return Err(protocol_error());
    }
    let messages = response
        .messages
        .into_iter()
        .map(message_from_proto)
        .collect::<ApiResult<Vec<_>>>()?;
    let mut cursor = after;
    for message in &messages {
        if message.recipient_id != recipient || message.sequence != cursor + 1 {
            return Err(protocol_error());
        }
        cursor = message.sequence;
    }
    if response.next_sequence != cursor || response.has_more && messages.is_empty() {
        return Err(protocol_error());
    }
    Ok(crate::AgentMessagePage {
        messages,
        next_sequence: response.next_sequence,
        has_more: response.has_more,
    })
}

pub(super) async fn watch(
    parent: &GrpcAgentRunClient,
    request: crate::WatchAgentMessagesRequest,
) -> ApiResult<crate::AgentCommunicationStream> {
    validate_token(&request.root_run_id)?;
    let root = request.root_run_id.clone();
    let cursor = request.after_sequence;
    let response = client(parent)
        .watch_agent_messages(
            parent
                .request(proto::WatchAgentMessagesRequest {
                    root_run_id: request.root_run_id,
                    after_sequence: request.after_sequence,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?;
    let updates = futures::stream::unfold(
        (
            response.into_inner(),
            cursor,
            parent.closed.subscribe(),
            false,
        ),
        move |(mut stream, cursor, mut closed, failed)| {
            let root = root.clone();
            async move {
                if failed || *closed.borrow() {
                    return None;
                }
                let result = tokio::select! { result = stream.message() => result, _ = closed.changed() => return None };
                match result {
                    Ok(Some(update)) => {
                        let value = required(update.message)
                            .and_then(message_from_proto)
                            .and_then(|message| {
                                if message.root_run_id != root || update.sequence != cursor + 1 {
                                    return Err(protocol_error());
                                }
                                Ok(crate::AgentCommunicationUpdate {
                                    sequence: update.sequence,
                                    message,
                                })
                            });
                        let failed = value.is_err();
                        Some((value, (stream, update.sequence, closed, failed)))
                    }
                    Err(status) => Some((
                        Err(read_error_from_status(status)),
                        (stream, cursor, closed, true),
                    )),
                    Ok(None) => None,
                }
            }
        },
    );
    Ok(Box::pin(updates))
}

pub(super) async fn submit_task(
    parent: &GrpcAgentRunClient,
    request: crate::SubmitAgentTaskMessageRequest,
) -> ApiResult<crate::AgentTaskSnapshot> {
    validate_token(&request.message_id)?;
    validate_token(&request.role)?;
    if request.text.is_empty() || request.text.len() > colossus_api::MAX_AGENT_MESSAGE_BYTES {
        return Err(invalid_request("text", "peer input exceeds its bound"));
    }
    let target = request.task_id.clone();
    let response = client(parent)
        .submit_agent_task_message(
            parent
                .request(proto::SubmitAgentTaskMessageRequest {
                    message_id: request.message_id,
                    task_id: request.task_id,
                    context_id: request.context_id,
                    text: request.text,
                    role: request.role,
                    max_turns: request.max_turns,
                })
                .await?,
        )
        .await
        .map_err(api_error_from_status)?
        .into_inner();
    let task = colossus_grpc::agent_task_wire::task_from_proto(required(response.task)?)?;
    if target.as_ref().is_some_and(|id| id != &task.task_id) {
        return Err(protocol_error());
    }
    Ok(task)
}
pub(super) async fn get_task(
    parent: &GrpcAgentRunClient,
    request: crate::GetAgentTaskRequest,
) -> ApiResult<crate::AgentTaskSnapshot> {
    validate_token(&request.task_id)?;
    let id = request.task_id.clone();
    let history_limit = request.history_length as usize;
    let response = client(parent)
        .get_agent_task(
            parent
                .request(proto::GetAgentTaskRequest {
                    task_id: request.task_id,
                    history_length: request.history_length,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?
        .into_inner();
    let task = colossus_grpc::agent_task_wire::task_from_proto(required(response.task)?)?;
    if task.task_id != id || task.history.len() > history_limit {
        return Err(protocol_error());
    }
    Ok(task)
}
pub(super) async fn list_tasks(
    parent: &GrpcAgentRunClient,
    request: crate::ListAgentTasksRequest,
) -> ApiResult<crate::ListAgentTasksResponse> {
    if request.statuses.len() > 9 || !(1..=20).contains(&request.page_size) {
        return Err(invalid_request("query", "invalid task page bounds"));
    }
    let limit = request.page_size as usize;
    let response = client(parent)
        .list_agent_tasks(
            parent
                .request(proto::ListAgentTasksRequest {
                    context_id: request.context_id,
                    statuses: request
                        .statuses
                        .into_iter()
                        .map(colossus_grpc::agent_task_wire::status_to_wire)
                        .map(str::to_owned)
                        .collect(),
                    status_updated_after: request.status_updated_after,
                    page_size: request.page_size,
                    page_token: request.page_token,
                    include_output: request.include_output,
                    history_length: request.history_length,
                })
                .await?,
        )
        .await
        .map_err(read_error_from_status)?
        .into_inner();
    if response.tasks.len() > limit {
        return Err(protocol_error());
    }
    let tasks = response
        .tasks
        .into_iter()
        .map(colossus_grpc::agent_task_wire::task_from_proto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(crate::ListAgentTasksResponse {
        tasks,
        next_page_token: response.next_page_token,
        total_size: response.total_size,
    })
}
