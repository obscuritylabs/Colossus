use crate::admission::{ListAdmission, WatchAdmission};
use async_trait::async_trait;
use colossus_api::*;
use colossus_communication::CommunicationError;
use colossus_contracts::ExecutionContext;
use colossus_runtime::Runtime;
use std::sync::Arc;

pub(crate) struct RuntimeCommunicationApi {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) repository: Arc<dyn RunRepository>,
    pub(crate) lists: Arc<ListAdmission>,
    watches: Arc<WatchAdmission>,
    pub(crate) runs: crate::RuntimeAgentRunApi,
}

impl RuntimeCommunicationApi {
    pub(crate) fn new(
        runtime: Arc<Runtime>,
        repository: Arc<dyn RunRepository>,
        lists: Arc<ListAdmission>,
        watches: Arc<WatchAdmission>,
        runs: crate::RuntimeAgentRunApi,
    ) -> Self {
        Self {
            runtime,
            repository,
            lists,
            watches,
            runs,
        }
    }

    fn read_authority(caller: &CallerContext) -> ApiResult<()> {
        caller.require_scope(scopes::AGENT_MESSAGES_READ)?;
        caller.require_scope(scopes::RUNS_READ)
    }
}

#[async_trait]
impl AgentCommunicationApi for RuntimeCommunicationApi {
    async fn submit_task_message(
        &self,
        caller: &CallerContext,
        request: SubmitAgentTaskMessageRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        self.submit_task_input(caller, request).await
    }
    async fn get_task(
        &self,
        caller: &CallerContext,
        request: GetAgentTaskRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        self.task(caller, request.clone())?;
        self.runs
            .get_run(
                caller,
                GetRunRequest {
                    run_id: request.task_id.clone(),
                },
            )
            .await?;
        self.task(caller, request)
    }
    async fn list_tasks(
        &self,
        caller: &CallerContext,
        request: ListAgentTasksRequest,
    ) -> ApiResult<ListAgentTasksResponse> {
        self.tasks(caller, request)
    }

    async fn list_participants(
        &self,
        caller: &CallerContext,
        request: ListAgentParticipantsRequest,
    ) -> ApiResult<Vec<AgentParticipant>> {
        Self::read_authority(caller)?;
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        let run = self
            .repository
            .get_run(caller, &request.root_run_id)?
            .ok_or_else(unavailable)?;
        match self.runtime.agent_participants(&caller.actor(), &run.id) {
            Ok(participants) => Ok(participants),
            Err(CommunicationError::Store(colossus_ports::StoreError::NotFound(_)))
                if run.status == RunStatus::Queued =>
            {
                Ok(Vec::new())
            }
            Err(error) => Err(map_error(error, caller)),
        }
    }

    async fn send_message(
        &self,
        caller: &CallerContext,
        request: SendAgentMessage,
    ) -> ApiResult<AgentMessage> {
        caller.require_scope(scopes::AGENT_MESSAGES_SEND)?;
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        self.runtime
            .communication()
            .list_messages(&caller.actor(), &request.recipient_id, 0, 1)
            .map_err(|error| map_error(error, caller))?;
        let context = ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            ..ExecutionContext::default()
        };
        self.runtime
            .send_agent_message(caller.actor(), context, request)
            .await
            .map_err(|error| {
                if error.outcome_unknown() {
                    return ApiError::from_store(
                        &colossus_ports::StoreError::OutcomeUnknown("communication".into()),
                        caller.request_id(),
                    );
                }
                match error {
                    colossus_runtime::RuntimeError::Store(error) => {
                        ApiError::from_store(&error, caller.request_id())
                    }
                    colossus_runtime::RuntimeError::Communication(error) => {
                        map_error(error, caller)
                    }
                    _ => ApiError::permission_denied(
                        ApiErrorReason::ToolDenied,
                        "agent message was not admitted",
                    ),
                }
            })
    }

    async fn get_message(
        &self,
        caller: &CallerContext,
        request: GetAgentMessageRequest,
    ) -> ApiResult<AgentMessage> {
        Self::read_authority(caller)?;
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        self.runtime
            .communication()
            .get_message(&caller.actor(), &request.message_id)
            .map_err(|error| map_error(error, caller))
    }

    async fn list_messages(
        &self,
        caller: &CallerContext,
        request: ListAgentMessagesRequest,
    ) -> ApiResult<AgentMessagePage> {
        Self::read_authority(caller)?;
        if !(1..=16).contains(&request.limit) {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "limit",
                "message page limit must be from 1 through 16",
            ));
        }
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        self.runtime
            .communication()
            .list_messages(
                &caller.actor(),
                &request.participant_id,
                request.after_sequence,
                request.limit as usize,
            )
            .map_err(|error| map_error(error, caller))
    }

    async fn watch_messages(
        &self,
        caller: &CallerContext,
        request: WatchAgentMessagesRequest,
    ) -> ApiResult<AgentCommunicationStream> {
        Self::read_authority(caller)?;
        let service = self.runtime.communication();
        service
            .list_participants(&caller.actor(), &request.root_run_id)
            .map_err(|error| map_error(error, caller))?;
        // Register before reading the durable cursor; notifications are hints, not payloads.
        let mut changed = service.subscribe();
        service
            .updates(
                &caller.actor(),
                &request.root_run_id,
                request.after_sequence,
            )
            .map_err(|error| map_error(error, caller))?;
        let permit = self
            .watches
            .acquire(caller.principal().application_id())
            .map_err(|_| capacity())?;
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let caller = caller.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let mut cursor = request.after_sequence;
            loop {
                let updates = match service.updates(&caller.actor(), &request.root_run_id, cursor) {
                    Ok(updates) => updates,
                    Err(error) => {
                        let _ = sender.send(Err(map_error(error, &caller))).await;
                        break;
                    }
                };
                let had_updates = !updates.is_empty();
                for update in updates {
                    cursor = update.sequence;
                    if sender.send(Ok(update)).await.is_err() {
                        return;
                    }
                }
                if had_updates {
                    continue;
                }
                match service.list_participants(&caller.actor(), &request.root_run_id) {
                    Ok(participants)
                        if participants.iter().all(|participant| !participant.open) =>
                    {
                        match service.updates(&caller.actor(), &request.root_run_id, cursor) {
                            Ok(updates) if updates.is_empty() => break,
                            Ok(_) => continue,
                            Err(error) => {
                                let _ = sender.send(Err(map_error(error, &caller))).await;
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(map_error(error, &caller))).await;
                        break;
                    }
                    _ => {}
                }
                tokio::select! { _ = sender.closed() => break, result = changed.changed() => if result.is_err() { break; } }
            }
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }
}

fn unavailable() -> ApiError {
    ApiError::not_found(
        ApiErrorReason::RunNotFound,
        "agent communication resource was not found",
    )
}
fn capacity() -> ApiError {
    ApiError::resource_exhausted(
        ApiErrorReason::CapacityExceeded,
        "agent communication admission limit reached",
    )
}

pub(crate) fn map_error(error: CommunicationError, caller: &CallerContext) -> ApiError {
    match error {
        CommunicationError::Forbidden
        | CommunicationError::Store(colossus_ports::StoreError::NotFound(_)) => unavailable(),
        CommunicationError::Invalid => ApiError::invalid(
            ApiErrorReason::InvalidArgument,
            "request",
            "invalid agent communication request",
        ),
        CommunicationError::Closed => ApiError::failed_precondition(
            ApiErrorReason::InvalidRunTransition,
            "agent recipient is closed",
        ),
        CommunicationError::Full => capacity(),
        CommunicationError::IdempotencyConflict => ApiError::conflict(
            ApiErrorReason::IdempotencyKeyReused,
            "agent message identity was reused for different input",
        ),
        CommunicationError::Store(error) => ApiError::from_store(&error, caller.request_id()),
    }
}
