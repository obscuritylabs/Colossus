use crate::{status::api_status, system::caller_context};
use colossus_api::{
    AgentRunApi, ApiError, ApiErrorReason, ApprovalRisk as CoreApprovalRisk, CallerContext,
    CancelRunRequest as CoreCancelRunRequest, ContentPart as CoreContentPart,
    CreateRunRequest as CoreCreateRunRequest, GetRunRequest as CoreGetRunRequest, IdempotencyKey,
    Interaction as CoreInteraction, InteractionKind as CoreInteractionKind,
    InteractionResponse as CoreInteractionResponse, InteractionStatus as CoreInteractionStatus,
    ListRunsRequest as CoreListRunsRequest, OutcomeCertainty as CoreOutcomeCertainty,
    RespondInteractionRequest as CoreRespondInteractionRequest, Run as CoreRun,
    RunMode as CoreRunMode, RunStatus as CoreRunStatus, RunUpdate as CoreRunUpdate,
    RunUpdateKind as CoreRunUpdateKind, ToolActivityState as CoreToolActivityState,
    WatchRunRequest as CoreWatchRunRequest, validate_public_approval_display,
};
use colossus_api_proto::v1alpha1::{
    ApprovalInteraction, ApprovalRisk, CancelRunRequest, CancelRunResponse, CreateRunRequest,
    CreateRunResponse, GetRunRequest, GetRunResponse, Interaction, InteractionKind,
    InteractionStatus, ListRunsRequest, ListRunsResponse, OutcomeCertainty, PageResponse,
    PromptChoice, ReasoningSummary, RespondInteractionRequest, RespondInteractionResponse, Run,
    RunCancellation, RunFailed, RunFailure, RunMode, RunNotice, RunResult, RunStateChanged,
    RunStatus, RunUpdate, TokenUsage, ToolActivity, ToolActivityState, UserPromptInteraction,
    VisibleOutputDelta, WatchRunRequest, WatchRunResponse,
    agent_run_service_server::AgentRunService, content_part, interaction, prompt_answer,
    respond_interaction_request, run, run_update,
};
use prost_types::Timestamp;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::{
    Request, Response, Status,
    codegen::tokio_stream::{Stream, StreamExt},
};

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_OPAQUE_TOKEN_BYTES: usize = 512;
pub(crate) const MAX_CREATE_INPUT_PARTS: usize = 128;
pub(crate) const MAX_RUN_STATUS_FILTERS: usize = 9;
/// Hard transport ceiling for active public watch streams.
///
/// The HTTP/2 request-admission pool is deliberately larger than this value so
/// watches cannot consume the slots reserved for cancellation, interaction
/// responses, and system RPCs.
pub const MAX_ACTIVE_WATCH_STREAMS: usize = 64;

struct AdmittedWatchStream {
    inner: Pin<Box<dyn Stream<Item = Result<WatchRunResponse, Status>> + Send + 'static>>,
    _permit: OwnedSemaphorePermit,
}

impl Stream for AdmittedWatchStream {
    type Item = Result<WatchRunResponse, Status>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

/// Authenticated tonic adapter for durable agent-run resources.
///
/// Caller identity is accepted exclusively from request extensions populated by the
/// authentication interceptor. None of the public request messages can select an actor,
/// credential, scope, role ceiling, or tool ceiling.
#[derive(Clone)]
pub struct AgentRunServiceAdapter {
    api: Arc<dyn AgentRunApi>,
    watch_slots: Arc<Semaphore>,
}

impl AgentRunServiceAdapter {
    /// Wrap a transport-neutral run API.
    pub fn new(api: Arc<dyn AgentRunApi>) -> Self {
        Self {
            api,
            watch_slots: Arc::new(Semaphore::new(MAX_ACTIVE_WATCH_STREAMS)),
        }
    }
}

#[tonic::async_trait]
impl AgentRunService for AgentRunServiceAdapter {
    async fn create_run(
        &self,
        request: Request<CreateRunRequest>,
    ) -> Result<Response<CreateRunResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = create_request(&caller, request.into_inner())?;
        let response = self
            .api
            .create_run(&caller, request)
            .await
            .map_err(api_status)?;
        Ok(Response::new(CreateRunResponse {
            run: Some(proto_run(response.run)?),
        }))
    }

    async fn get_run(
        &self,
        request: Request<GetRunRequest>,
    ) -> Result<Response<GetRunResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        validate_identifier(&caller, "run_id", &request.run_id)?;
        let run = self
            .api
            .get_run(
                &caller,
                CoreGetRunRequest {
                    run_id: request.run_id,
                },
            )
            .await
            .map_err(api_status)?;
        let pending_interactions = run
            .pending_interaction
            .as_ref()
            .map(|pending| {
                proto_interaction(pending, &run.id, Some(&run.etag), &caller)
                    .map(|value| vec![value])
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Response::new(GetRunResponse {
            run: Some(proto_run(run)?),
            pending_interactions,
        }))
    }

    async fn list_runs(
        &self,
        request: Request<ListRunsRequest>,
    ) -> Result<Response<ListRunsResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        if let Some(session_id) = request.session_id.as_deref() {
            validate_identifier(&caller, "session_id", session_id)?;
        }
        if request.statuses.len() > MAX_RUN_STATUS_FILTERS {
            return Err(invalid(
                &caller,
                "statuses",
                "statuses must contain at most nine lifecycle states",
            ));
        }
        let statuses = request
            .statuses
            .into_iter()
            .map(|status| core_run_status(&caller, status))
            .collect::<Result<Vec<_>, _>>()?;
        let (page_size, page_token) = request.page.map_or((0, None), |page| {
            let page_token = (!page.page_token.is_empty()).then_some(page.page_token);
            (page.page_size, page_token)
        });
        if let Some(page_token) = page_token.as_deref() {
            validate_opaque(&caller, "page.page_token", page_token)?;
        }
        let response = self
            .api
            .list_runs(
                &caller,
                CoreListRunsRequest {
                    session_id: request.session_id,
                    statuses,
                    page_size,
                    page_token,
                },
            )
            .await
            .map_err(api_status)?;
        let runs = response
            .runs
            .into_iter()
            .map(proto_run)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Response::new(ListRunsResponse {
            runs,
            page: Some(PageResponse {
                next_page_token: response.next_page_token.unwrap_or_default(),
            }),
        }))
    }

    type WatchRunStream =
        Pin<Box<dyn Stream<Item = Result<WatchRunResponse, Status>> + Send + 'static>>;

    async fn watch_run(
        &self,
        request: Request<WatchRunRequest>,
    ) -> Result<Response<Self::WatchRunStream>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        validate_identifier(&caller, "run_id", &request.run_id)?;
        let watch_permit = Arc::clone(&self.watch_slots)
            .try_acquire_owned()
            .map_err(|_| {
                api_status(
                    ApiError::resource_exhausted(
                        ApiErrorReason::CapacityExceeded,
                        "public watch transport capacity reached",
                    )
                    .with_correlation_id(caller.request_id().clone()),
                )
            })?;
        let stream = self
            .api
            .watch_run(
                &caller,
                CoreWatchRunRequest {
                    run_id: request.run_id,
                    after_sequence: request.after_sequence,
                },
            )
            .await
            .map_err(api_status)?;
        let api = Arc::clone(&self.api);
        let mapped = stream.then(move |item| {
            let api = Arc::clone(&api);
            let caller = caller.clone();
            async move {
                let update = item.map_err(api_status)?;
                let update = proto_update(api.as_ref(), &caller, update).await?;
                Ok(WatchRunResponse {
                    update: Some(update),
                })
            }
        });
        Ok(Response::new(Box::pin(AdmittedWatchStream {
            inner: Box::pin(mapped),
            _permit: watch_permit,
        })))
    }

    async fn cancel_run(
        &self,
        request: Request<CancelRunRequest>,
    ) -> Result<Response<CancelRunResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        validate_identifier(&caller, "run_id", &request.run_id)?;
        let idempotency_key = idempotency_key(&caller, request.idempotency_key)?;
        let run = self
            .api
            .cancel_run(
                &caller,
                CoreCancelRunRequest {
                    run_id: request.run_id,
                    idempotency_key,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(CancelRunResponse {
            run: Some(proto_run(run)?),
        }))
    }

    async fn respond_interaction(
        &self,
        request: Request<RespondInteractionRequest>,
    ) -> Result<Response<RespondInteractionResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let request = request.into_inner();
        validate_identifier(&caller, "run_id", &request.run_id)?;
        validate_identifier(&caller, "interaction_id", &request.interaction_id)?;
        validate_opaque(&caller, "etag", &request.etag)?;
        let idempotency_key = idempotency_key(&caller, request.idempotency_key)?;
        let run_id = request.run_id;
        let interaction_id = request.interaction_id;
        let etag = request.etag;
        let response = core_interaction_response(&caller, request.response)?;
        let interaction = self
            .api
            .respond_interaction(
                &caller,
                CoreRespondInteractionRequest {
                    run_id: run_id.clone(),
                    interaction_id,
                    etag: etag.clone(),
                    idempotency_key,
                    response,
                },
            )
            .await
            .map_err(api_status)?;
        Ok(Response::new(RespondInteractionResponse {
            interaction: Some(proto_interaction(&interaction, &run_id, None, &caller)?),
        }))
    }
}

fn create_request(
    caller: &CallerContext,
    request: CreateRunRequest,
) -> Result<CoreCreateRunRequest, Status> {
    if request.input.len() > MAX_CREATE_INPUT_PARTS {
        return Err(invalid(
            caller,
            "input",
            "input must contain at most 128 text parts",
        ));
    }
    if !request.selected_skills.is_empty() {
        return Err(invalid(
            caller,
            "selected_skills",
            "public skill activation is unavailable in v1alpha1",
        ));
    }
    let mode = match RunMode::try_from(request.mode) {
        Ok(RunMode::Execute) => CoreRunMode::Execute,
        Ok(RunMode::Plan) => CoreRunMode::Plan,
        Ok(RunMode::Unspecified) | Err(_) => {
            return Err(invalid(
                caller,
                "mode",
                "mode must be RUN_MODE_EXECUTE or RUN_MODE_PLAN",
            ));
        }
    };
    let input = request
        .input
        .into_iter()
        .map(|part| match part.content {
            Some(content_part::Content::Text(text)) => {
                Ok(CoreContentPart::Text { text: text.text })
            }
            Some(content_part::Content::Artifact(_)) => Err(invalid(
                caller,
                "input.content",
                "v1alpha1 create-run input accepts text content only",
            )),
            None => Err(invalid(
                caller,
                "input.content",
                "each input part must contain text",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let idempotency_key = idempotency_key(caller, request.idempotency_key)?;
    let request = CoreCreateRunRequest {
        input,
        session_id: request.session_id,
        role: (!request.role.is_empty()).then_some(request.role),
        mode,
        skill_ids: request.selected_skills,
        max_turns: request.max_turns,
        idempotency_key,
    };
    request
        .validate()
        .map_err(|error| api_status(with_correlation(error, caller)))?;
    Ok(request)
}

fn core_run_status(caller: &CallerContext, value: i32) -> Result<CoreRunStatus, Status> {
    match RunStatus::try_from(value) {
        Ok(RunStatus::Queued) => Ok(CoreRunStatus::Queued),
        Ok(RunStatus::Running) => Ok(CoreRunStatus::Running),
        Ok(RunStatus::Waiting) => Ok(CoreRunStatus::Waiting),
        Ok(RunStatus::Cancelling) => Ok(CoreRunStatus::Cancelling),
        Ok(RunStatus::Completed) => Ok(CoreRunStatus::Completed),
        Ok(RunStatus::Failed) => Ok(CoreRunStatus::Failed),
        Ok(RunStatus::Cancelled) => Ok(CoreRunStatus::Cancelled),
        Ok(RunStatus::Interrupted) => Ok(CoreRunStatus::Interrupted),
        Ok(RunStatus::OutcomeUnknown) => Ok(CoreRunStatus::OutcomeUnknown),
        Ok(RunStatus::Unspecified) | Err(_) => Err(invalid(
            caller,
            "statuses",
            "statuses must not contain unspecified or unknown values",
        )),
    }
}

fn idempotency_key(caller: &CallerContext, value: String) -> Result<IdempotencyKey, Status> {
    IdempotencyKey::new(value).map_err(|error| api_status(with_correlation(error, caller)))
}

fn validate_identifier(
    caller: &CallerContext,
    field: &'static str,
    value: &str,
) -> Result<(), Status> {
    let supported = !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        });
    if supported {
        Ok(())
    } else {
        Err(invalid(
            caller,
            field,
            "identifier is empty, oversized, or contains unsupported characters",
        ))
    }
}

fn validate_opaque(caller: &CallerContext, field: &'static str, value: &str) -> Result<(), Status> {
    if !value.is_empty()
        && value.len() <= MAX_OPAQUE_TOKEN_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
    {
        Ok(())
    } else {
        Err(invalid(
            caller,
            field,
            "opaque token is empty, oversized, or malformed",
        ))
    }
}

fn proto_run(value: CoreRun) -> Result<Run, Status> {
    let mode = match value.mode {
        CoreRunMode::Execute => RunMode::Execute,
        CoreRunMode::Plan => RunMode::Plan,
    };
    let status = proto_run_status(value.status);
    let terminal = match value.status {
        CoreRunStatus::Completed => {
            let result = value.result.ok_or_else(projection_invariant)?;
            if value.failure.is_some() || value.cancellation.is_some() {
                return Err(projection_invariant());
            }
            Some(run::Terminal::Result(proto_result(result)))
        }
        CoreRunStatus::Failed | CoreRunStatus::Interrupted | CoreRunStatus::OutcomeUnknown => {
            let failure = value.failure.ok_or_else(projection_invariant)?;
            if value.result.is_some() || value.cancellation.is_some() {
                return Err(projection_invariant());
            }
            Some(run::Terminal::Failure(proto_failure(failure)))
        }
        CoreRunStatus::Cancelled => {
            let cancellation = value.cancellation.ok_or_else(projection_invariant)?;
            if value.result.is_some() || value.failure.is_some() {
                return Err(projection_invariant());
            }
            Some(run::Terminal::Cancellation(RunCancellation {
                turn: cancellation.turn,
                message: cancellation.message,
            }))
        }
        _ => {
            if value.result.is_some() || value.failure.is_some() || value.cancellation.is_some() {
                return Err(projection_invariant());
            }
            None
        }
    };
    Ok(Run {
        run_id: value.id,
        session_id: value.session_id,
        title: value.title,
        role: value.role,
        mode: mode as i32,
        status: status as i32,
        created_at: Some(parse_timestamp(&value.created_at)?),
        updated_at: Some(parse_timestamp(&value.updated_at)?),
        started_at: value
            .started_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?,
        finished_at: value
            .finished_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?,
        last_sequence: value.last_sequence,
        pending_interaction_count: u32::from(value.pending_interaction.is_some()),
        etag: value.etag,
        terminal,
        selected_skills: value.skill_ids,
    })
}

fn proto_run_status(value: CoreRunStatus) -> RunStatus {
    match value {
        CoreRunStatus::Queued => RunStatus::Queued,
        CoreRunStatus::Running => RunStatus::Running,
        CoreRunStatus::Waiting => RunStatus::Waiting,
        CoreRunStatus::Cancelling => RunStatus::Cancelling,
        CoreRunStatus::Completed => RunStatus::Completed,
        CoreRunStatus::Failed => RunStatus::Failed,
        CoreRunStatus::Cancelled => RunStatus::Cancelled,
        CoreRunStatus::Interrupted => RunStatus::Interrupted,
        CoreRunStatus::OutcomeUnknown => RunStatus::OutcomeUnknown,
    }
}

fn proto_outcome(value: CoreOutcomeCertainty) -> OutcomeCertainty {
    match value {
        CoreOutcomeCertainty::Known => OutcomeCertainty::Known,
        CoreOutcomeCertainty::Unknown => OutcomeCertainty::Unknown,
    }
}

fn proto_result(value: colossus_api::RunResult) -> RunResult {
    RunResult {
        output: value.output,
        profile: value.profile,
        model: value.model,
        elapsed_seconds: value.elapsed_seconds,
        model_profile: value.model_profile,
        provider_profile: value.provider_profile,
    }
}

fn proto_failure(value: colossus_api::RunFailure) -> RunFailure {
    RunFailure {
        reason: value.code,
        message: value.message,
        outcome_certainty: proto_outcome(value.outcome) as i32,
        recoverable: value.recoverable,
        http_status: value.http_status.map(u32::from),
        retry_after_ms: value.retry_after_ms,
    }
}

fn proto_interaction(
    value: &CoreInteraction,
    run_id: &str,
    etag: Option<&str>,
    caller: &CallerContext,
) -> Result<Interaction, Status> {
    let status = match value.status {
        CoreInteractionStatus::Pending => InteractionStatus::Pending,
        CoreInteractionStatus::Responded => InteractionStatus::Answered,
        CoreInteractionStatus::Expired => InteractionStatus::Expired,
        CoreInteractionStatus::Cancelled => InteractionStatus::Cancelled,
    };
    let (kind, content) = match value.kind {
        CoreInteractionKind::Prompt => (
            InteractionKind::UserPrompt,
            interaction::Content::UserPrompt(UserPromptInteraction {
                question: value.prompt.clone(),
                choices: value
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(index, label)| PromptChoice {
                        choice_id: choice_id(index),
                        label: label.clone(),
                    })
                    .collect(),
                allow_free_form: value.allow_free_form,
            }),
        ),
        CoreInteractionKind::Approval => {
            let request_hash = value
                .request_hash
                .clone()
                .ok_or_else(projection_invariant)?;
            let action = value.action.clone().ok_or_else(projection_invariant)?;
            let resource = value.resource.clone().ok_or_else(projection_invariant)?;
            validate_public_approval_display(&action, &resource)
                .map_err(|_| projection_invariant())?;
            let risk = match value.risk {
                Some(CoreApprovalRisk::Low) => ApprovalRisk::Low,
                Some(CoreApprovalRisk::Medium) => ApprovalRisk::Medium,
                Some(CoreApprovalRisk::High) => ApprovalRisk::High,
                None => ApprovalRisk::Unspecified,
            };
            (
                InteractionKind::Approval,
                interaction::Content::Approval(ApprovalInteraction {
                    reason: value.prompt.clone(),
                    action,
                    resource,
                    risk: risk as i32,
                    request_hash,
                }),
            )
        }
    };
    let response_scope = match value.kind {
        CoreInteractionKind::Prompt => colossus_api::scopes::PROMPTS_RESPOND,
        CoreInteractionKind::Approval => colossus_api::scopes::APPROVALS_RESPOND,
    };
    Ok(Interaction {
        interaction_id: value.id.clone(),
        run_id: run_id.into(),
        kind: kind as i32,
        status: status as i32,
        created_at: Some(parse_timestamp(&value.created_at)?),
        expires_at: Some(parse_timestamp(&value.expires_at)?),
        respondable_by_caller: value.status == CoreInteractionStatus::Pending
            && value.application_id == caller.principal().application_id()
            && caller.principal().has_scope(response_scope)
            && etag.is_some(),
        etag: etag.unwrap_or_default().into(),
        content: Some(content),
    })
}

fn core_interaction_response(
    caller: &CallerContext,
    response: Option<respond_interaction_request::Response>,
) -> Result<CoreInteractionResponse, Status> {
    match response {
        Some(respond_interaction_request::Response::PromptAnswer(answer)) => match answer.answer {
            Some(prompt_answer::Answer::Choice(choice)) => {
                let index = parse_choice_id(caller, &choice.choice_id)?;
                let selected_index = u32::try_from(index).map_err(|_| {
                    invalid(
                        caller,
                        "response.prompt_answer.choice_id",
                        "choice_id is outside the supported range",
                    )
                })?;
                Ok(CoreInteractionResponse::Prompt {
                    answer: choice.label,
                    selected_index: Some(selected_index),
                })
            }
            Some(prompt_answer::Answer::FreeFormText(answer)) => {
                Ok(CoreInteractionResponse::Prompt {
                    answer,
                    selected_index: None,
                })
            }
            None => Err(invalid(
                caller,
                "response.prompt_answer.answer",
                "prompt answer must select a choice or provide free-form text",
            )),
        },
        Some(respond_interaction_request::Response::ApprovalAnswer(answer)) => {
            Ok(CoreInteractionResponse::Approval {
                approved: answer.approved,
                request_hash: answer.request_hash,
            })
        }
        None => Err(invalid(
            caller,
            "response",
            "an interaction response is required",
        )),
    }
}

async fn proto_update(
    api: &dyn AgentRunApi,
    caller: &CallerContext,
    value: CoreRunUpdate,
) -> Result<RunUpdate, Status> {
    let CoreRunUpdate {
        run_id,
        sequence,
        occurred_at,
        kind,
    } = value;
    let update = match kind {
        CoreRunUpdateKind::OutputDelta { text } => {
            run_update::Update::OutputDelta(VisibleOutputDelta { text })
        }
        CoreRunUpdateKind::ReasoningSummary { summary } => {
            run_update::Update::ReasoningSummary(ReasoningSummary { summary })
        }
        CoreRunUpdateKind::ToolActivity { activity } => {
            let state = match activity.state {
                CoreToolActivityState::Requested => ToolActivityState::Requested,
                CoreToolActivityState::WaitingApproval => ToolActivityState::WaitingApproval,
                CoreToolActivityState::Started => ToolActivityState::Started,
                CoreToolActivityState::Completed => ToolActivityState::Completed,
                CoreToolActivityState::Failed => ToolActivityState::Failed,
                CoreToolActivityState::OutcomeUnknown => ToolActivityState::OutcomeUnknown,
            };
            run_update::Update::ToolActivity(ToolActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state: state as i32,
                summary: activity.summary,
            })
        }
        CoreRunUpdateKind::Usage { usage } => run_update::Update::Usage(TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
        CoreRunUpdateKind::Interaction { interaction } => {
            let etag = if interaction.status == CoreInteractionStatus::Pending {
                let current = api
                    .get_run(
                        caller,
                        CoreGetRunRequest {
                            run_id: run_id.clone(),
                        },
                    )
                    .await
                    .map_err(api_status)?;
                current
                    .pending_interaction
                    .as_ref()
                    .filter(|pending| pending.id == interaction.id)
                    .map(|_| current.etag)
            } else {
                None
            };
            run_update::Update::Interaction(proto_interaction(
                &interaction,
                &run_id,
                etag.as_deref(),
                caller,
            )?)
        }
        CoreRunUpdateKind::Message { message } => {
            run_update::Update::Message(proto_released_message(message)?)
        }
        CoreRunUpdateKind::Notice { notice } => run_update::Update::Notice(RunNotice {
            reason: notice.reason,
            message: notice.message,
        }),
        CoreRunUpdateKind::State { status } => run_update::Update::State(RunStateChanged {
            status: proto_run_status(status) as i32,
        }),
        CoreRunUpdateKind::Result { result } => run_update::Update::Result(proto_result(result)),
        CoreRunUpdateKind::Failure { status, failure } => run_update::Update::Failure(RunFailed {
            status: proto_run_status(status) as i32,
            failure: Some(proto_failure(failure)),
        }),
        CoreRunUpdateKind::Cancellation { cancellation } => {
            run_update::Update::Cancellation(RunCancellation {
                turn: cancellation.turn,
                message: cancellation.message,
            })
        }
    };
    Ok(RunUpdate {
        run_id,
        sequence,
        created_at: Some(parse_timestamp(&occurred_at)?),
        update: Some(update),
    })
}

fn proto_released_message(
    value: colossus_api::ReleasedSessionMessage,
) -> Result<colossus_api_proto::v1alpha1::SessionMessage, Status> {
    use colossus_api::{
        ReleasedArtifactPurpose, ReleasedArtifactState, ReleasedContentPart, ReleasedMessageRole,
    };
    use colossus_api_proto::v1alpha1::{
        ArtifactPurpose, ArtifactReference, ArtifactState, ContentPart, MessageRole, TextContent,
        content_part,
    };

    let role = match value.role {
        ReleasedMessageRole::User => MessageRole::User,
        ReleasedMessageRole::Assistant => MessageRole::Assistant,
        ReleasedMessageRole::Tool => MessageRole::Tool,
        ReleasedMessageRole::System => MessageRole::System,
    };
    let content = value
        .content
        .into_iter()
        .map(|part| {
            let content = match part {
                ReleasedContentPart::Text { text } => {
                    content_part::Content::Text(TextContent { text })
                }
                ReleasedContentPart::Artifact { artifact } => {
                    let purpose = match artifact.purpose {
                        ReleasedArtifactPurpose::RunInput => ArtifactPurpose::RunInput,
                        ReleasedArtifactPurpose::RunOutput => ArtifactPurpose::RunOutput,
                        ReleasedArtifactPurpose::Workflow => ArtifactPurpose::Workflow,
                        ReleasedArtifactPurpose::Extension => ArtifactPurpose::Extension,
                        ReleasedArtifactPurpose::Archive => ArtifactPurpose::Archive,
                    };
                    let state = match artifact.state {
                        ReleasedArtifactState::Uploading => ArtifactState::Uploading,
                        ReleasedArtifactState::Quarantined => ArtifactState::Quarantined,
                        ReleasedArtifactState::Available => ArtifactState::Available,
                        ReleasedArtifactState::Rejected => ArtifactState::Rejected,
                        ReleasedArtifactState::Expired => ArtifactState::Expired,
                    };
                    content_part::Content::Artifact(ArtifactReference {
                        artifact_id: artifact.artifact_id,
                        file_name: artifact.file_name,
                        media_type: artifact.media_type,
                        size_bytes: artifact.size_bytes,
                        sha256: artifact.sha256,
                        purpose: purpose as i32,
                        state: state as i32,
                        created_at: Some(parse_timestamp(&artifact.created_at)?),
                    })
                }
            };
            Ok(ContentPart {
                content: Some(content),
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(colossus_api_proto::v1alpha1::SessionMessage {
        session_id: value.session_id,
        run_id: value.run_id,
        sequence: value.sequence,
        role: role as i32,
        content,
        created_at: Some(parse_timestamp(&value.created_at)?),
    })
}

fn choice_id(index: usize) -> String {
    format!("choice:{index}")
}

fn parse_choice_id(caller: &CallerContext, value: &str) -> Result<usize, Status> {
    let digits = value.strip_prefix("choice:").ok_or_else(|| {
        invalid(
            caller,
            "response.prompt_answer.choice_id",
            "choice_id is malformed",
        )
    })?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            caller,
            "response.prompt_answer.choice_id",
            "choice_id is malformed",
        ));
    }
    digits.parse::<usize>().map_err(|_| {
        invalid(
            caller,
            "response.prompt_answer.choice_id",
            "choice_id is outside the supported range",
        )
    })
}

fn parse_timestamp(value: &str) -> Result<Timestamp, Status> {
    let timestamp = value
        .parse::<Timestamp>()
        .map_err(|_| projection_invariant())?;
    // google.protobuf.Timestamp is restricted to 0001-01-01 through
    // 9999-12-31 with normalized nanoseconds.
    if !(-62_135_596_800..=253_402_300_799).contains(&timestamp.seconds)
        || !(0..1_000_000_000).contains(&timestamp.nanos)
    {
        return Err(projection_invariant());
    }
    Ok(timestamp)
}

fn invalid(caller: &CallerContext, field: &'static str, description: &'static str) -> Status {
    api_status(
        ApiError::invalid(ApiErrorReason::InvalidArgument, field, description)
            .with_correlation_id(caller.request_id().clone()),
    )
}

fn with_correlation(mut error: ApiError, caller: &CallerContext) -> ApiError {
    if error.correlation_id.is_none() {
        error = error.with_correlation_id(caller.request_id().clone());
    }
    error
}

fn projection_invariant() -> Status {
    Status::internal("public run projection invariant failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::{
        ApiResult, ApiScope, ApplicationKind, ApplicationPrincipal, CreateRunResponse,
        ListRunsResponse as CoreListRunsResponse, RequestId, RunUpdateStream,
    };

    fn caller() -> CallerContext {
        CallerContext::authenticated(
            ApplicationPrincipal::authenticated(
                "app:test",
                "credential:test",
                ApplicationKind::Enrolled,
                [
                    ApiScope::new(colossus_api::scopes::RUNS_READ).expect("scope"),
                    ApiScope::new(colossus_api::scopes::PROMPTS_RESPOND).expect("scope"),
                ],
                ["assistant".into()],
                Vec::<String>::new(),
            )
            .expect("principal"),
            RequestId::new("request:test").expect("request id"),
        )
    }

    struct NeverApi;

    #[tonic::async_trait]
    impl AgentRunApi for NeverApi {
        async fn create_run(
            &self,
            _caller: &CallerContext,
            _request: CoreCreateRunRequest,
        ) -> ApiResult<CreateRunResponse> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn get_run(
            &self,
            _caller: &CallerContext,
            _request: CoreGetRunRequest,
        ) -> ApiResult<CoreRun> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn list_runs(
            &self,
            _caller: &CallerContext,
            _request: CoreListRunsRequest,
        ) -> ApiResult<CoreListRunsResponse> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn watch_run(
            &self,
            _caller: &CallerContext,
            _request: CoreWatchRunRequest,
        ) -> ApiResult<RunUpdateStream> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn cancel_run(
            &self,
            _caller: &CallerContext,
            _request: CoreCancelRunRequest,
        ) -> ApiResult<CoreRun> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn respond_interaction(
            &self,
            _caller: &CallerContext,
            _request: CoreRespondInteractionRequest,
        ) -> ApiResult<CoreInteraction> {
            panic!("unauthenticated transport must not invoke the API")
        }
    }

    #[tokio::test]
    async fn missing_authenticated_extension_fails_closed() {
        let service = AgentRunServiceAdapter::new(Arc::new(NeverApi));
        let error = AgentRunService::get_run(
            &service,
            Request::new(GetRunRequest {
                run_id: "run-1".into(),
            }),
        )
        .await
        .expect_err("missing caller must fail");
        assert_eq!(error.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn transport_watch_slots_are_bounded_and_release_on_drop() {
        let service = AgentRunServiceAdapter::new(Arc::new(NeverApi));
        let permits = (0..MAX_ACTIVE_WATCH_STREAMS)
            .map(|_| {
                Arc::clone(&service.watch_slots)
                    .try_acquire_owned()
                    .expect("slot within transport ceiling")
            })
            .collect::<Vec<_>>();
        assert!(
            Arc::clone(&service.watch_slots)
                .try_acquire_owned()
                .is_err()
        );
        drop(permits);
        assert!(Arc::clone(&service.watch_slots).try_acquire_owned().is_ok());
    }

    #[test]
    fn unspecified_and_unknown_modes_are_rejected() {
        let caller = caller();
        for mode in [RunMode::Unspecified as i32, 2_147_483_647] {
            let error = create_request(
                &caller,
                CreateRunRequest {
                    input: Vec::new(),
                    session_id: None,
                    role: String::new(),
                    mode,
                    selected_skills: Vec::new(),
                    max_turns: 0,
                    idempotency_key: "key-1".into(),
                },
            )
            .expect_err("mode must fail");
            assert_eq!(error.code(), tonic::Code::InvalidArgument);
        }
    }

    #[test]
    fn create_cardinality_is_rejected_before_transport_projection() {
        let error = create_request(
            &caller(),
            CreateRunRequest {
                input: (0..=MAX_CREATE_INPUT_PARTS)
                    .map(|_| colossus_api_proto::v1alpha1::ContentPart { content: None })
                    .collect(),
                session_id: None,
                role: "assistant".into(),
                mode: RunMode::Execute as i32,
                selected_skills: Vec::new(),
                max_turns: 1,
                idempotency_key: "bounded-input".into(),
            },
        )
        .expect_err("oversized repeated input must fail before projection");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn list_cardinality_is_rejected_before_api_dispatch() {
        let service = AgentRunServiceAdapter::new(Arc::new(NeverApi));
        let mut request = Request::new(ListRunsRequest {
            session_id: None,
            statuses: vec![RunStatus::Queued as i32; MAX_RUN_STATUS_FILTERS + 1],
            page: None,
        });
        request.extensions_mut().insert(caller());
        let error = AgentRunService::list_runs(&service, request)
            .await
            .expect_err("oversized status filters must fail before dispatch");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn emitted_choice_ids_are_canonical_and_round_trip() {
        let caller = caller();
        assert_eq!(choice_id(12), "choice:12");
        assert_eq!(parse_choice_id(&caller, "choice:12").expect("choice"), 12);
        assert!(parse_choice_id(&caller, "choice:012").is_err());
        assert!(parse_choice_id(&caller, "12").is_err());
    }

    #[test]
    fn choice_and_approval_integrity_values_are_forwarded_exactly() {
        let caller = caller();
        let choice = core_interaction_response(
            &caller,
            Some(respond_interaction_request::Response::PromptAnswer(
                colossus_api_proto::v1alpha1::PromptAnswer {
                    answer: Some(prompt_answer::Answer::Choice(
                        colossus_api_proto::v1alpha1::PromptChoiceAnswer {
                            choice_id: "choice:2".into(),
                            label: "Exact displayed label".into(),
                        },
                    )),
                },
            )),
        )
        .expect("choice");
        assert_eq!(
            choice,
            CoreInteractionResponse::Prompt {
                answer: "Exact displayed label".into(),
                selected_index: Some(2),
            }
        );

        let approval = core_interaction_response(
            &caller,
            Some(respond_interaction_request::Response::ApprovalAnswer(
                colossus_api_proto::v1alpha1::ApprovalAnswer {
                    approved: true,
                    request_hash: "ab".repeat(32),
                },
            )),
        )
        .expect("approval");
        assert_eq!(
            approval,
            CoreInteractionResponse::Approval {
                approved: true,
                request_hash: "ab".repeat(32),
            }
        );
    }

    #[test]
    fn projection_rejects_private_persisted_approval_display_without_echoing_it() {
        let private_action = "filesystem.write.customer-secret";
        let private_resource = "/Users/alex/private/customer-secret.txt";
        let value = CoreInteraction {
            id: "interaction-private-approval".into(),
            kind: CoreInteractionKind::Approval,
            status: CoreInteractionStatus::Pending,
            application_id: "app:test-ui".into(),
            created_at: "2026-07-19T12:00:00Z".into(),
            prompt: "An effect requires explicit approval".into(),
            choices: Vec::new(),
            allow_free_form: false,
            request_hash: Some("ab".repeat(32)),
            action: Some(private_action.into()),
            resource: Some(private_resource.into()),
            risk: Some(CoreApprovalRisk::High),
            expires_at: "2026-07-19T12:05:00Z".into(),
            response: None,
            responded_at: None,
        };
        let error = proto_interaction(&value, "run-1", Some("etag-1"), &caller())
            .expect_err("private approval display must not cross the transport");
        assert_eq!(error.code(), tonic::Code::Internal);
        assert_eq!(error.message(), "public run projection invariant failed");
        assert!(!error.message().contains(private_action));
        assert!(!error.message().contains(private_resource));
    }

    #[tokio::test]
    async fn historical_state_update_is_mapped_without_loading_current_run() {
        let update = proto_update(
            &NeverApi,
            &caller(),
            CoreRunUpdate {
                run_id: "run-1".into(),
                sequence: 7,
                occurred_at: "2026-07-19T12:00:00Z".into(),
                kind: CoreRunUpdateKind::State {
                    status: CoreRunStatus::Cancelling,
                },
            },
        )
        .await
        .expect("state update");
        assert!(matches!(
            update.update,
            Some(run_update::Update::State(RunStateChanged {
                status
            })) if status == RunStatus::Cancelling as i32
        ));
    }

    #[test]
    fn malformed_or_out_of_range_timestamps_fail_as_internal_invariants() {
        for timestamp in ["not-a-time", "10000-01-01T00:00:00Z"] {
            let error = parse_timestamp(timestamp).expect_err("timestamp must fail");
            assert_eq!(error.code(), tonic::Code::Internal);
            assert_eq!(
                error.message(),
                "public run projection invariant failed",
                "private parse detail must not leak"
            );
        }
    }
}
