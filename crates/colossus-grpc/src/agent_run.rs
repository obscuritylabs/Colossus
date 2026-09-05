use crate::{status::api_status, system::caller_context};
use colossus_api::{
    AgentRunApi, ApiError, ApiErrorReason, ApprovalRisk as CoreApprovalRisk,
    ArchiveThreadRequest as CoreArchiveThreadRequest, CallerContext,
    CancelRunRequest as CoreCancelRunRequest, ContentPart as CoreContentPart,
    CreateRunRequest as CoreCreateRunRequest, GetRunRequest as CoreGetRunRequest, IdempotencyKey,
    Interaction as CoreInteraction, InteractionKind as CoreInteractionKind,
    InteractionResponse as CoreInteractionResponse, InteractionStatus as CoreInteractionStatus,
    ListRunsRequest as CoreListRunsRequest,
    ListSessionActivityRequest as CoreListSessionActivityRequest,
    OutcomeCertainty as CoreOutcomeCertainty, PlanExecutionStrategy as CorePlanExecutionStrategy,
    PlanRunAction as CorePlanRunAction, PlanStatus as CorePlanStatus,
    ResearchDepth as CoreResearchDepth, ResearchSourceKind as CoreResearchSourceKind,
    RespondInteractionRequest as CoreRespondInteractionRequest,
    RestoreThreadRequest as CoreRestoreThreadRequest, Run as CoreRun, RunBranch as CoreRunBranch,
    RunBranchContextMode as CoreRunBranchContextMode, RunMode as CoreRunMode,
    RunStatus as CoreRunStatus, RunUpdate as CoreRunUpdate, RunUpdateKind as CoreRunUpdateKind,
    SessionActivity as CoreSessionActivity, SessionActivityKind as CoreSessionActivityKind,
    SessionActivityLane as CoreSessionActivityLane,
    SessionActivityStatus as CoreSessionActivityStatus, ToolActivityState as CoreToolActivityState,
    WatchRunRequest as CoreWatchRunRequest, validate_public_approval_display,
};
use colossus_api_proto::v1alpha1::{
    ApprovalInteraction, ApprovalRisk, ArchiveThreadRequest, ArchiveThreadResponse,
    CancelRunRequest, CancelRunResponse, CreateRunRequest, CreateRunResponse, GetRunRequest,
    GetRunResponse, Interaction, InteractionKind, InteractionStatus, ListRunsRequest,
    ListRunsResponse, ListSessionActivityRequest, ListSessionActivityResponse, OutcomeCertainty,
    PageResponse, PlanExecutionStrategy, PlanStatus, PromptChoice, ReasoningSummary, ResearchDepth,
    ResearchSourceKind, RespondInteractionRequest, RespondInteractionResponse,
    RestoreThreadRequest, RestoreThreadResponse, Run, RunCancellation, RunFailed, RunFailure,
    RunMode, RunNotice, RunResult, RunStateChanged, RunStatus, RunUpdate, SessionActivity,
    SessionActivityContent, SessionActivityKind, SessionActivityLane, SessionActivityStatus,
    ThreadLifecycle, TokenUsage, ToolActivity, ToolActivityState, UserPromptInteraction,
    VisibleOutputDelta, WatchRunRequest, WatchRunResponse,
    agent_run_service_server::AgentRunService, content_part, interaction, plan_run_action,
    prompt_answer, respond_interaction_request, run, run_update,
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
use tracing::Instrument as _;

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
    span: tracing::Span,
}

impl Stream for AdmittedWatchStream {
    type Item = Result<WatchRunResponse, Status>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let span = self.span.clone();
        let _entered = span.enter();
        let result = self.inner.as_mut().poll_next(context);
        if matches!(result, Poll::Ready(Some(Err(_)))) {
            span.record("otel.status_code", "ERROR");
            span.record("error.type", "rpc.stream.failed");
        }
        result
    }
}

fn public_rpc_span<T>(
    request: &Request<T>,
    caller: &CallerContext,
    method: &'static str,
) -> tracing::Span {
    let remote = colossus_observability::extract_remote_trace_context(
        request
            .metadata()
            .get("traceparent")
            .and_then(|value| value.to_str().ok()),
        request
            .metadata()
            .get("tracestate")
            .and_then(|value| value.to_str().ok()),
    );
    let span = tracing::info_span!(
        target: "colossus.rpc",
        "grpc.request",
        otel.name = %format_args!("colossus.api.v1alpha1.AgentRunService/{method}"),
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        rpc.system = "grpc",
        rpc.service = "colossus.api.v1alpha1.AgentRunService",
        rpc.method = method,
        colossus.application.id = caller.principal().application_id(),
        error.type = tracing::field::Empty,
    );
    if let Some(remote) = remote.as_ref() {
        let _ = colossus_observability::set_remote_parent(&span, remote);
    }
    span
}

fn record_rpc_result<T>(span: &tracing::Span, result: &Result<T, Status>) {
    if result.is_ok() {
        span.record("otel.status_code", "OK");
    } else {
        span.record("otel.status_code", "ERROR");
        span.record("error.type", "rpc.failed");
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
        let span = public_rpc_span(&request, &caller, "CreateRun");
        let result = async {
            let caller =
                caller.with_remote_trace_context(colossus_observability::current_trace_context());
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
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn get_run(
        &self,
        request: Request<GetRunRequest>,
    ) -> Result<Response<GetRunResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "GetRun");
        let result = async {
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
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn list_runs(
        &self,
        request: Request<ListRunsRequest>,
    ) -> Result<Response<ListRunsResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "ListRuns");
        let result = async {
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
                        include_archived: request.include_archived,
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
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn list_session_activity(
        &self,
        request: Request<ListSessionActivityRequest>,
    ) -> Result<Response<ListSessionActivityResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "ListSessionActivity");
        let result = async {
            let request = request.into_inner();
            validate_identifier(&caller, "source_run_id", &request.source_run_id)?;
            if request.query.len() > 256 || request.query.chars().any(char::is_control) {
                return Err(invalid(
                    &caller,
                    "query",
                    "query must be at most 256 bytes and contain no control characters",
                ));
            }
            let lanes = request
                .lanes
                .into_iter()
                .map(|value| core_activity_lane(&caller, value))
                .collect::<Result<Vec<_>, _>>()?;
            let kinds = request
                .kinds
                .into_iter()
                .map(|value| core_activity_kind(&caller, value))
                .collect::<Result<Vec<_>, _>>()?;
            let statuses = request
                .statuses
                .into_iter()
                .map(|value| core_activity_status(&caller, value))
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
                .list_session_activity(
                    &caller,
                    CoreListSessionActivityRequest {
                        source_run_id: request.source_run_id,
                        query: request.query,
                        lanes,
                        kinds,
                        statuses,
                        page_size,
                        page_token,
                    },
                )
                .await
                .map_err(api_status)?;
            let activities = response
                .activities
                .into_iter()
                .map(proto_activity)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Response::new(ListSessionActivityResponse {
                activities,
                page: Some(PageResponse {
                    next_page_token: response.next_page_token.unwrap_or_default(),
                }),
                head_sequence: response.head_sequence,
                projected_through_sequence: response.projected_through_sequence,
                caught_up: response.caught_up,
            }))
        }
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    type WatchRunStream =
        Pin<Box<dyn Stream<Item = Result<WatchRunResponse, Status>> + Send + 'static>>;

    async fn watch_run(
        &self,
        request: Request<WatchRunRequest>,
    ) -> Result<Response<Self::WatchRunStream>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "WatchRun");
        let result = async {
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
                span: span.clone(),
            }) as Self::WatchRunStream))
        }
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn cancel_run(
        &self,
        request: Request<CancelRunRequest>,
    ) -> Result<Response<CancelRunResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "CancelRun");
        let result = async {
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
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn archive_thread(
        &self,
        request: Request<ArchiveThreadRequest>,
    ) -> Result<Response<ArchiveThreadResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "ArchiveThread");
        let result = async {
            let request = request.into_inner();
            validate_identifier(&caller, "run_id", &request.run_id)?;
            let idempotency_key = idempotency_key(&caller, request.idempotency_key)?;
            let thread = self
                .api
                .archive_thread(
                    &caller,
                    CoreArchiveThreadRequest {
                        run_id: request.run_id,
                        idempotency_key,
                    },
                )
                .await
                .map_err(api_status)?;
            Ok(Response::new(ArchiveThreadResponse {
                thread: Some(ThreadLifecycle {
                    session_id: thread.session_id,
                    archived: thread.archived,
                }),
            }))
        }
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn restore_thread(
        &self,
        request: Request<RestoreThreadRequest>,
    ) -> Result<Response<RestoreThreadResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "RestoreThread");
        let result = async {
            let request = request.into_inner();
            validate_identifier(&caller, "run_id", &request.run_id)?;
            let idempotency_key = idempotency_key(&caller, request.idempotency_key)?;
            let thread = self
                .api
                .restore_thread(
                    &caller,
                    CoreRestoreThreadRequest {
                        run_id: request.run_id,
                        idempotency_key,
                    },
                )
                .await
                .map_err(api_status)?;
            Ok(Response::new(RestoreThreadResponse {
                thread: Some(ThreadLifecycle {
                    session_id: thread.session_id,
                    archived: thread.archived,
                }),
            }))
        }
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
    }

    async fn respond_interaction(
        &self,
        request: Request<RespondInteractionRequest>,
    ) -> Result<Response<RespondInteractionResponse>, Status> {
        let caller = caller_context(&request)?.clone();
        let span = public_rpc_span(&request, &caller, "RespondInteraction");
        let result = async {
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
        .instrument(span.clone())
        .await;
        record_rpc_result(&span, &result);
        result
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
            "input must contain at most 128 content parts",
        ));
    }
    let mode = match RunMode::try_from(request.mode) {
        Ok(RunMode::Execute) => CoreRunMode::Execute,
        Ok(RunMode::Plan) => CoreRunMode::Plan,
        Ok(RunMode::Research) => CoreRunMode::Research,
        Ok(RunMode::Unspecified) | Err(_) => {
            return Err(invalid(
                caller,
                "mode",
                "mode must be RUN_MODE_EXECUTE, RUN_MODE_PLAN, or RUN_MODE_RESEARCH",
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
            Some(content_part::Content::Artifact(artifact)) => {
                validate_identifier(caller, "input.artifact.artifact_id", &artifact.artifact_id)?;
                Ok(CoreContentPart::Artifact {
                    artifact_id: artifact.artifact_id,
                })
            }
            None => Err(invalid(
                caller,
                "input.content",
                "each input part must contain text or an artifact reference",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let idempotency_key = idempotency_key(caller, request.idempotency_key)?;
    let plan_action = request
        .plan_action
        .map(|action| {
            validate_identifier(caller, "plan_action.source_run_id", &action.source_run_id)?;
            let action_kind = action.action.ok_or_else(|| {
                invalid(
                    caller,
                    "plan_action.action",
                    "Plan action must select revise or execute",
                )
            })?;
            match action_kind {
                plan_run_action::Action::Revise(_) => Ok(CorePlanRunAction::Revise {
                    source_run_id: action.source_run_id,
                    expected_revision: action.expected_revision,
                }),
                plan_run_action::Action::Execute(execute) => {
                    let strategy = match PlanExecutionStrategy::try_from(execute.strategy) {
                        Ok(PlanExecutionStrategy::Direct) => CorePlanExecutionStrategy::Direct,
                        Ok(PlanExecutionStrategy::Goal) => CorePlanExecutionStrategy::Goal {
                            max_iterations: u16::try_from(execute.max_goal_iterations).map_err(
                                |_| {
                                    invalid(
                                        caller,
                                        "plan_action.execute.max_goal_iterations",
                                        "Goal iterations must be in 1..=50",
                                    )
                                },
                            )?,
                        },
                        Ok(PlanExecutionStrategy::Unspecified) | Err(_) => {
                            return Err(invalid(
                                caller,
                                "plan_action.execute.strategy",
                                "Plan execution strategy must be direct or Goal",
                            ));
                        }
                    };
                    Ok(CorePlanRunAction::Execute {
                        source_run_id: action.source_run_id,
                        expected_revision: action.expected_revision,
                        strategy,
                    })
                }
            }
        })
        .transpose()?;
    let request = CoreCreateRunRequest {
        input,
        session_id: request.session_id,
        end_user_id: request.end_user_id,
        role: (!request.role.is_empty()).then_some(request.role),
        mode,
        research_depth: match ResearchDepth::try_from(request.research_depth) {
            Ok(ResearchDepth::Quick) => Some(CoreResearchDepth::Quick),
            Ok(ResearchDepth::Standard) => Some(CoreResearchDepth::Standard),
            Ok(ResearchDepth::Deep) => Some(CoreResearchDepth::Deep),
            Ok(ResearchDepth::Unspecified) => None,
            Err(_) => return Err(invalid(caller, "research_depth", "unknown Research depth")),
        },
        research_sources: request.research_sources.into_iter().map(|source| {
            match ResearchSourceKind::try_from(source) {
                Ok(ResearchSourceKind::Repo) => Ok(CoreResearchSourceKind::Repo),
                Ok(ResearchSourceKind::Web) => Ok(CoreResearchSourceKind::Web),
                Ok(ResearchSourceKind::Mcp) => Ok(CoreResearchSourceKind::Mcp),
                Ok(ResearchSourceKind::Unspecified) | Err(_) => Err(invalid(
                    caller,
                    "research_sources",
                    "Research evidence lanes must not be unspecified or unknown",
                )),
            }
        }).collect::<Result<Vec<_>, _>>()?,
        skill_ids: request.plugin_skill_ids,
        plan_action,
        branch: request
            .branch
            .map(|branch| {
                let context_mode =
                    match colossus_api_proto::v1alpha1::RunBranchContextMode::try_from(
                        branch.context_mode,
                    ) {
                        Ok(
                            colossus_api_proto::v1alpha1::RunBranchContextMode::Unspecified
                            | colossus_api_proto::v1alpha1::RunBranchContextMode::Exact,
                        ) => CoreRunBranchContextMode::Exact,
                        Ok(colossus_api_proto::v1alpha1::RunBranchContextMode::Conversation) => {
                            CoreRunBranchContextMode::Conversation
                        }
                        Ok(
                            colossus_api_proto::v1alpha1::RunBranchContextMode::SourceRunConversation,
                        ) => CoreRunBranchContextMode::SourceRunConversation,
                        Err(_) => {
                            return Err(invalid(
                                caller,
                                "branch.context_mode",
                                "unknown branch context mode",
                            ));
                        }
                    };
                Ok(CoreRunBranch {
                    source_run_id: branch.source_run_id,
                    source_message_count: branch.source_message_count,
                    context_mode,
                })
            })
            .transpose()?,
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

fn core_activity_lane(
    caller: &CallerContext,
    value: i32,
) -> Result<CoreSessionActivityLane, Status> {
    match SessionActivityLane::try_from(value) {
        Ok(SessionActivityLane::Agent) => Ok(CoreSessionActivityLane::Agent),
        Ok(SessionActivityLane::Tools) => Ok(CoreSessionActivityLane::Tools),
        Ok(SessionActivityLane::System) => Ok(CoreSessionActivityLane::System),
        Ok(SessionActivityLane::Unspecified) | Err(_) => Err(invalid(
            caller,
            "lanes",
            "lanes must not contain unspecified or unknown values",
        )),
    }
}

fn core_activity_kind(
    caller: &CallerContext,
    value: i32,
) -> Result<CoreSessionActivityKind, Status> {
    match SessionActivityKind::try_from(value) {
        Ok(SessionActivityKind::User) => Ok(CoreSessionActivityKind::User),
        Ok(SessionActivityKind::Assistant) => Ok(CoreSessionActivityKind::Assistant),
        Ok(SessionActivityKind::Tool) => Ok(CoreSessionActivityKind::Tool),
        Ok(SessionActivityKind::System) => Ok(CoreSessionActivityKind::System),
        Ok(SessionActivityKind::Unspecified) | Err(_) => Err(invalid(
            caller,
            "kinds",
            "kinds must not contain unspecified or unknown values",
        )),
    }
}

fn core_activity_status(
    caller: &CallerContext,
    value: i32,
) -> Result<CoreSessionActivityStatus, Status> {
    match SessionActivityStatus::try_from(value) {
        Ok(SessionActivityStatus::Requested) => Ok(CoreSessionActivityStatus::Requested),
        Ok(SessionActivityStatus::Running) => Ok(CoreSessionActivityStatus::Running),
        Ok(SessionActivityStatus::Waiting) => Ok(CoreSessionActivityStatus::Waiting),
        Ok(SessionActivityStatus::Completed) => Ok(CoreSessionActivityStatus::Completed),
        Ok(SessionActivityStatus::Failed) => Ok(CoreSessionActivityStatus::Failed),
        Ok(SessionActivityStatus::Cancelled) => Ok(CoreSessionActivityStatus::Cancelled),
        Ok(SessionActivityStatus::OutcomeUnknown) => Ok(CoreSessionActivityStatus::OutcomeUnknown),
        Ok(SessionActivityStatus::Unspecified) | Err(_) => Err(invalid(
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
        CoreRunMode::Research => RunMode::Research,
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
            Some(run::Terminal::Cancellation(proto_cancellation(
                cancellation,
            )))
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
        plugin_skill_ids: value.skill_ids,
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
        archived: value.archived,
    })
}

fn proto_activity(value: CoreSessionActivity) -> Result<SessionActivity, Status> {
    Ok(SessionActivity {
        activity_id: value.activity_id,
        run_id: value.run_id,
        turn: value.turn,
        lane: match value.lane {
            CoreSessionActivityLane::Agent => SessionActivityLane::Agent as i32,
            CoreSessionActivityLane::Tools => SessionActivityLane::Tools as i32,
            CoreSessionActivityLane::System => SessionActivityLane::System as i32,
        },
        kind: match value.kind {
            CoreSessionActivityKind::User => SessionActivityKind::User as i32,
            CoreSessionActivityKind::Assistant => SessionActivityKind::Assistant as i32,
            CoreSessionActivityKind::Tool => SessionActivityKind::Tool as i32,
            CoreSessionActivityKind::System => SessionActivityKind::System as i32,
        },
        title: value.title,
        summary: value.summary,
        actor: value.actor,
        status: value.status.map(|status| match status {
            CoreSessionActivityStatus::Requested => SessionActivityStatus::Requested as i32,
            CoreSessionActivityStatus::Running => SessionActivityStatus::Running as i32,
            CoreSessionActivityStatus::Waiting => SessionActivityStatus::Waiting as i32,
            CoreSessionActivityStatus::Completed => SessionActivityStatus::Completed as i32,
            CoreSessionActivityStatus::Failed => SessionActivityStatus::Failed as i32,
            CoreSessionActivityStatus::Cancelled => SessionActivityStatus::Cancelled as i32,
            CoreSessionActivityStatus::OutcomeUnknown => {
                SessionActivityStatus::OutcomeUnknown as i32
            }
        }),
        started_at: Some(parse_timestamp(&value.started_at)?),
        completed_at: value
            .completed_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?,
        duration_ms: value.duration_ms,
        input: value.input.map(|content| SessionActivityContent {
            format: content.format,
            value: content.value,
        }),
        result: value.result.map(|content| SessionActivityContent {
            format: content.format,
            value: content.value,
        }),
        attributes: value.attributes.into_iter().collect(),
        source_event_types: value.source_event_types,
        first_sequence: value.first_sequence,
        last_sequence: value.last_sequence,
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
        plan_id: value.plan_id,
        plan_revision: value.plan_revision,
        plan_status: value
            .plan_status
            .map_or(PlanStatus::Unspecified, proto_plan_status) as i32,
        goal_id: value.goal_id,
        profile: value.profile,
        model: value.model,
        elapsed_seconds: value.elapsed_seconds,
        model_profile: value.model_profile,
        provider_profile: value.provider_profile,
    }
}

fn proto_cancellation(value: colossus_api::RunCancellation) -> RunCancellation {
    RunCancellation {
        turn: value.turn,
        message: value.message,
        plan_id: value.plan_id,
        plan_revision: value.plan_revision,
        plan_status: value
            .plan_status
            .map_or(PlanStatus::Unspecified, proto_plan_status) as i32,
        goal_id: value.goal_id,
    }
}

fn proto_plan_status(value: CorePlanStatus) -> PlanStatus {
    match value {
        CorePlanStatus::Draft => PlanStatus::Draft,
        CorePlanStatus::Approved => PlanStatus::Approved,
        CorePlanStatus::Executed => PlanStatus::Executed,
        CorePlanStatus::Discarded => PlanStatus::Discarded,
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
                CoreToolActivityState::Cancelled => ToolActivityState::Cancelled,
                CoreToolActivityState::Failed => ToolActivityState::Failed,
                CoreToolActivityState::OutcomeUnknown => ToolActivityState::OutcomeUnknown,
            };
            run_update::Update::ToolActivity(ToolActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state: state as i32,
                summary: activity.summary,
                preview: activity.preview,
                input: activity.input,
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
            run_update::Update::Cancellation(proto_cancellation(cancellation))
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

        async fn archive_thread(
            &self,
            _caller: &CallerContext,
            _request: CoreArchiveThreadRequest,
        ) -> ApiResult<colossus_api::ThreadLifecycle> {
            panic!("unauthenticated transport must not invoke the API")
        }

        async fn restore_thread(
            &self,
            _caller: &CallerContext,
            _request: CoreRestoreThreadRequest,
        ) -> ApiResult<colossus_api::ThreadLifecycle> {
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
                    plugin_skill_ids: Vec::new(),
                    session_id: None,
                    end_user_id: None,
                    role: String::new(),
                    mode,
                    plan_action: None,
                    branch: None,
                    research_depth: ResearchDepth::Unspecified as i32,
                    research_sources: Vec::new(),
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
                plugin_skill_ids: Vec::new(),
                session_id: None,
                end_user_id: None,
                role: "assistant".into(),
                mode: RunMode::Execute as i32,
                plan_action: None,
                branch: None,
                research_depth: ResearchDepth::Unspecified as i32,
                research_sources: Vec::new(),
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
            include_archived: false,
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

    #[test]
    fn result_projection_preserves_optional_plan_identity() {
        let result = proto_result(colossus_api::RunResult {
            output: "Plan saved".into(),
            plan_id: Some("plan-1".into()),
            plan_revision: Some(3),
            plan_status: Some(CorePlanStatus::Draft),
            goal_id: None,
            profile: "default".into(),
            model_profile: "default".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 1.0,
        });

        assert_eq!(result.plan_id.as_deref(), Some("plan-1"));
    }

    #[test]
    fn cancellation_projection_preserves_optional_plan_identity() {
        let cancellation = proto_cancellation(colossus_api::RunCancellation {
            turn: 2,
            message: "cancelled after persistence".into(),
            plan_id: Some("plan-1".into()),
            plan_revision: Some(2),
            plan_status: Some(CorePlanStatus::Draft),
            goal_id: None,
        });

        assert_eq!(cancellation.plan_id.as_deref(), Some("plan-1"));
    }
}
