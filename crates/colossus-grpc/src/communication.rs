use crate::{api_status, communication_wire::*, system::caller_context};
use colossus_api::{
    AgentCommunicationApi, GetAgentMessageRequest, ListAgentMessagesRequest,
    ListAgentParticipantsRequest, SendAgentMessage, WatchAgentMessagesRequest,
};
use colossus_api_proto::v1alpha1::{
    self as proto, agent_communication_service_server::AgentCommunicationService,
};
use futures::{Stream, StreamExt as _};
use std::{pin::Pin, sync::Arc};
use tokio::sync::Semaphore;
use tonic::{Request, Response, Status};

#[derive(Clone)]
pub(crate) struct CommunicationServiceAdapter {
    api: Arc<dyn AgentCommunicationApi>,
    watches: Arc<Semaphore>,
}

impl CommunicationServiceAdapter {
    pub(crate) fn new(api: Arc<dyn AgentCommunicationApi>, watches: Arc<Semaphore>) -> Self {
        Self { api, watches }
    }
}

#[tonic::async_trait]
impl AgentCommunicationService for CommunicationServiceAdapter {
    async fn submit_agent_task_message(
        &self,
        request: Request<proto::SubmitAgentTaskMessageRequest>,
    ) -> Result<Response<proto::SubmitAgentTaskMessageResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let value = self
            .api
            .submit_task_message(
                &caller,
                colossus_api::SubmitAgentTaskMessageRequest {
                    message_id: request.message_id,
                    task_id: request.task_id,
                    context_id: request.context_id,
                    text: request.text,
                    role: request.role,
                    max_turns: request.max_turns,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::SubmitAgentTaskMessageResponse {
            task: Some(crate::agent_task_wire::task_to_proto(value)),
        }))
    }
    async fn get_agent_task(
        &self,
        request: Request<proto::GetAgentTaskRequest>,
    ) -> Result<Response<proto::GetAgentTaskResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let value = self
            .api
            .get_task(
                &caller,
                colossus_api::GetAgentTaskRequest {
                    task_id: request.task_id,
                    history_length: request.history_length,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::GetAgentTaskResponse {
            task: Some(crate::agent_task_wire::task_to_proto(value)),
        }))
    }
    async fn list_agent_tasks(
        &self,
        request: Request<proto::ListAgentTasksRequest>,
    ) -> Result<Response<proto::ListAgentTasksResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        if request.statuses.len() > 9 {
            return Err(Status::invalid_argument(
                "too many agent task status filters",
            ));
        }
        let statuses = request
            .statuses
            .iter()
            .map(|status| crate::agent_task_wire::status_from_wire(status).map_err(api_status))
            .collect::<Result<Vec<_>, _>>()?;
        let value = self
            .api
            .list_tasks(
                &caller,
                colossus_api::ListAgentTasksRequest {
                    context_id: request.context_id,
                    statuses,
                    status_updated_after: request.status_updated_after,
                    page_size: request.page_size,
                    page_token: request.page_token,
                    include_output: request.include_output,
                    history_length: request.history_length,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::ListAgentTasksResponse {
            tasks: value
                .tasks
                .into_iter()
                .map(crate::agent_task_wire::task_to_proto)
                .collect(),
            next_page_token: value.next_page_token,
            total_size: value.total_size,
        }))
    }

    async fn list_agent_participants(
        &self,
        request: Request<proto::ListAgentParticipantsRequest>,
    ) -> Result<Response<proto::ListAgentParticipantsResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let participants = self
            .api
            .list_participants(
                &caller,
                ListAgentParticipantsRequest {
                    root_run_id: request.root_run_id,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::ListAgentParticipantsResponse {
            participants: participants.into_iter().map(participant_to_proto).collect(),
        }))
    }
    async fn send_agent_message(
        &self,
        request: Request<proto::SendAgentMessageRequest>,
    ) -> Result<Response<proto::SendAgentMessageResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let message = self
            .api
            .send_message(
                &caller,
                SendAgentMessage {
                    recipient_id: request.recipient_id,
                    text: request.text,
                    idempotency_key: request.idempotency_key,
                    reply_to: request.reply_to,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::SendAgentMessageResponse {
            message: Some(message_to_proto(message)),
        }))
    }
    async fn get_agent_message(
        &self,
        request: Request<proto::GetAgentMessageRequest>,
    ) -> Result<Response<proto::GetAgentMessageResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let message = self
            .api
            .get_message(
                &caller,
                GetAgentMessageRequest {
                    message_id: request.message_id,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::GetAgentMessageResponse {
            message: Some(message_to_proto(message)),
        }))
    }
    async fn list_agent_messages(
        &self,
        request: Request<proto::ListAgentMessagesRequest>,
    ) -> Result<Response<proto::ListAgentMessagesResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let page = self
            .api
            .list_messages(
                &caller,
                ListAgentMessagesRequest {
                    participant_id: request.participant_id,
                    after_sequence: request.after_sequence,
                    limit: request.limit,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(proto::ListAgentMessagesResponse {
            messages: page.messages.into_iter().map(message_to_proto).collect(),
            next_sequence: page.next_sequence,
            has_more: page.has_more,
        }))
    }
    type WatchAgentMessagesStream =
        Pin<Box<dyn Stream<Item = Result<proto::WatchAgentMessagesResponse, Status>> + Send>>;
    async fn watch_agent_messages(
        &self,
        request: Request<proto::WatchAgentMessagesRequest>,
    ) -> Result<Response<Self::WatchAgentMessagesStream>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        let permit = self
            .watches
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("watch admission limit reached"))?;
        let updates = self
            .api
            .watch_messages(
                &caller,
                WatchAgentMessagesRequest {
                    root_run_id: request.root_run_id,
                    after_sequence: request.after_sequence,
                },
            )
            .await
            .map_err(api_status)?;
        // The guard lives in the stream and is released on disconnect or terminal completion.
        let stream =
            futures::stream::unfold((updates, permit), |(mut updates, permit)| async move {
                updates.next().await.map(|update| {
                    (
                        update
                            .map(|update| proto::WatchAgentMessagesResponse {
                                sequence: update.sequence,
                                message: Some(message_to_proto(update.message)),
                            })
                            .map_err(api_status),
                        (updates, permit),
                    )
                })
            });
        Ok(Response::new(Box::pin(stream)))
    }
}
