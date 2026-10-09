use crate::communication::{RuntimeCommunicationApi, map_error};
use colossus_api::*;
use colossus_contracts::{EventClassification, ExecutionContext, NewEvent};
use colossus_ports::{EventJournal, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) struct InitialTaskInput {
    pub(crate) request: SubmitAgentTaskMessageRequest,
    pub(crate) fingerprint: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputClaim {
    fingerprint: String,
    role: String,
    max_turns: u32,
    pub(crate) input: AgentTaskInput,
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub(crate) fn owner_key(caller: &CallerContext) -> String {
    digest(caller.principal().application_id().as_bytes())
}
pub(crate) fn index_stream(caller: &CallerContext) -> String {
    format!("agent-task-index:{}", owner_key(caller))
}
pub(crate) fn history_stream(task: &str) -> String {
    format!("agent-task-inputs:{task}")
}
fn claim_stream(caller: &CallerContext, message: &str) -> String {
    format!(
        "agent-task-message:{}:{}",
        owner_key(caller),
        digest(message.as_bytes())
    )
}
pub(crate) fn tail_version(journal: &dyn EventJournal, stream: &str) -> Result<u64, StoreError> {
    Ok(journal
        .read_stream_backwards(stream, None, 1)?
        .first()
        .map_or(0, |event| event.stream_version))
}

fn claim_event(
    caller: &CallerContext,
    stream: String,
    expected: u64,
    claim: &InputClaim,
) -> Result<NewEvent, StoreError> {
    Ok(NewEvent {
        event_version: 1,
        stream_id: stream,
        expected_stream_version: expected,
        classification: EventClassification::Domain,
        event_type: "api.agent_task.input.v1".into(),
        actor: caller.actor(),
        context: ExecutionContext {
            correlation_id: caller.request_id().as_str().into(),
            run_id: Some(claim.input.task_id.clone()),
            session_id: Some(claim.input.context_id.clone()),
            ..Default::default()
        },
        payload: serde_json::to_value(claim)
            .map_err(|_| StoreError::Adapter("cannot encode peer input".into()))?,
    })
}

pub(crate) fn stage_initial(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    run: &NewRun,
    input: InitialTaskInput,
) -> ApiResult<Vec<NewEvent>> {
    let version = tail_version(journal, &index_stream(caller))
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
    let claim = InputClaim {
        fingerprint: input.fingerprint,
        role: input.request.role,
        max_turns: input.request.max_turns,
        input: AgentTaskInput {
            message_id: input.request.message_id,
            task_id: run.id().into(),
            context_id: run.session_id().into(),
            text: input.request.text,
            accepted_at: timestamp()?,
            inbox_message_id: None,
        },
    };
    [
        claim_event(
            caller,
            claim_stream(caller, &claim.input.message_id),
            0,
            &claim,
        ),
        claim_event(caller, history_stream(run.id()), 0, &claim),
        claim_event(caller, index_stream(caller), version, &claim),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|error| ApiError::from_store(&error, caller.request_id()))
}

pub(crate) fn read_claim(
    journal: &dyn EventJournal,
    caller: &CallerContext,
    stream: &str,
) -> ApiResult<Option<InputClaim>> {
    let events = journal
        .read_stream_backwards(stream, None, 1)
        .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
    let Some(event) = events.first() else {
        return Ok(None);
    };
    if event.actor != caller.actor() {
        return Err(missing());
    }
    decode_claim(journal, event, caller).map(Some)
}
pub(crate) fn decode_claim(
    journal: &dyn EventJournal,
    event: &colossus_contracts::EventEnvelope,
    caller: &CallerContext,
) -> ApiResult<InputClaim> {
    if event.actor != caller.actor()
        || event.event_type != "api.agent_task.input.v1"
        || event.event_version != 1
    {
        return Err(missing());
    }
    serde_json::from_value(
        journal
            .decrypt_payload(event)
            .map_err(|error| ApiError::from_store(&error, caller.request_id()))?,
    )
    .map_err(|_| {
        ApiError::failed_precondition(
            ApiErrorReason::InternalInvariant,
            "invalid durable peer input",
        )
    })
}

impl RuntimeCommunicationApi {
    pub(crate) async fn submit_task_input(
        &self,
        caller: &CallerContext,
        mut request: SubmitAgentTaskMessageRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        caller.require_scope(scopes::AGENT_MESSAGES_SEND)?;
        caller.require_scope(scopes::AGENT_MESSAGES_READ)?;
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        caller.require_scope(scopes::RUNS_READ)?;
        caller.require_role(&request.role)?;
        if !token(&request.message_id)
            || !token(&request.role)
            || request.task_id.as_ref().is_some_and(|id| !token(id))
            || request.context_id.as_ref().is_some_and(|id| !token(id))
            || request.text.is_empty()
            || request.text.len() > MAX_AGENT_MESSAGE_BYTES
            || !(1..=100).contains(&request.max_turns)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "message",
                "invalid bounded peer task input",
            ));
        }
        let fingerprint = digest(
            &serde_json::to_vec(&(&request.text, &request.role, request.max_turns)).map_err(
                |_| {
                    ApiError::failed_precondition(
                        ApiErrorReason::InternalInvariant,
                        "cannot fingerprint peer input",
                    )
                },
            )?,
        );
        let journal = self.runtime.journal();
        if let Some(claim) = read_claim(
            journal.as_ref(),
            caller,
            &claim_stream(caller, &request.message_id),
        )? {
            if claim.fingerprint != fingerprint
                || request
                    .task_id
                    .as_ref()
                    .is_some_and(|id| id != &claim.input.task_id)
                || request
                    .context_id
                    .as_ref()
                    .is_some_and(|id| id != &claim.input.context_id)
            {
                return Err(conflict());
            }
            return self.task(
                caller,
                GetAgentTaskRequest {
                    task_id: claim.input.task_id,
                    history_length: 0,
                },
            );
        }
        if let Some(task_id) = request.task_id.clone() {
            let run = self
                .repository
                .get_run(caller, &task_id)?
                .ok_or_else(missing)?;
            let initial = journal
                .read_stream_from(&history_stream(&task_id), 0, 1)
                .map_err(|error| ApiError::from_store(&error, caller.request_id()))?;
            let initial = initial
                .first()
                .ok_or_else(missing)
                .and_then(|event| decode_claim(journal.as_ref(), event, caller))?;
            if request
                .context_id
                .as_ref()
                .is_some_and(|id| id != &run.session_id)
            {
                return Err(missing());
            }
            if request.role != initial.role || request.max_turns != initial.max_turns {
                return Err(conflict());
            }
            if run.status.is_terminal() || run.status == RunStatus::Cancelling {
                return Err(ApiError::failed_precondition(
                    ApiErrorReason::InvalidRunTransition,
                    "task cannot accept another input",
                ));
            }
            let participants = self
                .runtime
                .agent_participants(&caller.actor(), &task_id)
                .map_err(|error| map_error(error, caller))?;
            let recipient = participants
                .into_iter()
                .find(|participant| participant.parent_id.is_none())
                .ok_or_else(missing)?;
            request.text = self
                .runtime
                .release_agent_task_text(
                    caller.actor(),
                    ExecutionContext {
                        correlation_id: caller.request_id().as_str().into(),
                        ..Default::default()
                    },
                    request.text,
                    Some(&recipient.id),
                )
                .await
                .map_err(|error| runtime_error(error, caller))?;
            let send = SendAgentMessage {
                recipient_id: recipient.id,
                text: request.text.clone(),
                idempotency_key: format!("peer-{}", digest(request.message_id.as_bytes())),
                reply_to: None,
            };
            self.runtime
                .communication()
                .send_from_application_with_events(&caller.actor(), send, &|message| {
                    let version = tail_version(journal.as_ref(), &history_stream(&task_id))?;
                    if version >= 4096 {
                        return Err(StoreError::Adapter(
                            "peer input history limit reached".into(),
                        ));
                    }
                    let claim = InputClaim {
                        fingerprint: fingerprint.clone(),
                        role: request.role.clone(),
                        max_turns: request.max_turns,
                        input: AgentTaskInput {
                            message_id: request.message_id.clone(),
                            task_id: task_id.clone(),
                            context_id: run.session_id.clone(),
                            text: message.text.clone(),
                            accepted_at: message.accepted_at.clone(),
                            inbox_message_id: Some(message.id.clone()),
                        },
                    };
                    Ok(vec![
                        claim_event(caller, claim_stream(caller, &request.message_id), 0, &claim)?,
                        claim_event(caller, history_stream(&task_id), version, &claim)?,
                    ])
                })
                .map_err(|error| map_error(error, caller))?;
            self.task(
                caller,
                GetAgentTaskRequest {
                    task_id,
                    history_length: 0,
                },
            )
        } else {
            request.text = self
                .runtime
                .release_agent_task_text(
                    caller.actor(),
                    ExecutionContext {
                        correlation_id: caller.request_id().as_str().into(),
                        ..Default::default()
                    },
                    request.text,
                    None,
                )
                .await
                .map_err(|error| runtime_error(error, caller))?;
            let create = CreateRunRequest {
                input: vec![ContentPart::Text {
                    text: request.text.clone(),
                }],
                session_id: request.context_id.clone(),
                end_user_id: None,
                role: Some(request.role.clone()),
                mode: RunMode::Execute,
                research_depth: None,
                research_sources: Vec::new(),
                skill_ids: Vec::new(),
                plan_action: None,
                branch: None,
                max_turns: request.max_turns,
                idempotency_key: IdempotencyKey::new(format!(
                    "peer-{}",
                    digest(request.message_id.as_bytes())
                ))?,
            };
            let response = self
                .runs
                .allocate_run(
                    caller,
                    create,
                    Some(InitialTaskInput {
                        request,
                        fingerprint,
                    }),
                )
                .await?;
            self.task(
                caller,
                GetAgentTaskRequest {
                    task_id: response.run.id,
                    history_length: 0,
                },
            )
        }
    }
}

pub(crate) fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_: .".contains(&byte))
        && !value.contains(' ')
}
pub(crate) fn timestamp() -> ApiResult<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "cannot format peer input time",
            )
        })
}
pub(crate) fn missing() -> ApiError {
    ApiError::not_found(ApiErrorReason::RunNotFound, "agent task was not found")
}
pub(crate) fn capacity() -> ApiError {
    ApiError::bounded_resource_exhausted(
        ApiErrorReason::CapacityExceeded,
        "agent task query or history bound exceeded",
    )
}
fn conflict() -> ApiError {
    ApiError::conflict(
        ApiErrorReason::IdempotencyKeyReused,
        "peer message identity or profile changed",
    )
}
fn runtime_error(error: colossus_runtime::RuntimeError, caller: &CallerContext) -> ApiError {
    if error.outcome_unknown() {
        ApiError::from_store(
            &StoreError::OutcomeUnknown("communication".into()),
            caller.request_id(),
        )
    } else {
        ApiError::permission_denied(ApiErrorReason::ToolDenied, "peer input was not admitted")
    }
}
