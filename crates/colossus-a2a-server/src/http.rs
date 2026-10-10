use crate::{
    projection,
    service::{PeerProfile, State, credential_digest},
    wire::*,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State as Extract},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use colossus_sdk::{
    AgentTaskSnapshot, AgentTaskStatus, ApiError, ApiErrorCode, GetAgentTaskRequest,
    ListAgentTasksRequest, SubmitAgentTaskMessageRequest,
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::{collections::VecDeque, convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::OwnedSemaphorePermit;

pub(crate) struct RpcError {
    code: i32,
    message: &'static str,
    data: Option<Value>,
}
impl RpcError {
    pub(crate) fn invalid() -> Self {
        Self {
            code: -32602,
            message: "Invalid params",
            data: None,
        }
    }
    pub(crate) fn unsupported() -> Self {
        Self {
            code: -32004,
            message: "Unsupported operation",
            data: None,
        }
    }
    pub(crate) fn media() -> Self {
        Self {
            code: -32005,
            message: "Content type not supported",
            data: None,
        }
    }
    fn task_missing() -> Self {
        Self {
            code: -32001,
            message: "Task not found",
            data: None,
        }
    }
    fn value(self, id: Value) -> Value {
        let mut error = json!({"code": self.code, "message": self.message});
        if let Some(data) = self.data {
            error["data"] = data;
        }
        json!({"jsonrpc": "2.0", "id": id, "error": error})
    }
}
impl From<ApiError> for RpcError {
    fn from(error: ApiError) -> Self {
        match error.code {
            ApiErrorCode::NotFound
            | ApiErrorCode::PermissionDenied
            | ApiErrorCode::Unauthenticated => Self::task_missing(),
            ApiErrorCode::InvalidArgument
            | ApiErrorCode::Conflict
            | ApiErrorCode::AlreadyExists => Self::invalid(),
            ApiErrorCode::FailedPrecondition => Self::unsupported(),
            ApiErrorCode::OutcomeUnknown => Self {
                code: -32000,
                message: "Input outcome is unknown; reconcile before retrying",
                data: Some(json!({"outcomeUnknown": true})),
            },
            ApiErrorCode::ResourceExhausted => Self {
                code: -32000,
                message: "Agent admission limit reached",
                data: None,
            },
            _ => Self {
                code: -32603,
                message: "Agent service unavailable",
                data: None,
            },
        }
    }
}
pub(crate) fn router(state: Arc<State>) -> Router {
    Router::new()
        .route("/.well-known/agent-card.json", get(card))
        .route("/", post(rpc))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}
fn peer(state: &State, headers: &HeaderMap) -> Option<Arc<PeerProfile>> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
        .map(|(_, token)| token)
        .filter(|value| (32..=512).contains(&value.len()));
    token
        .and_then(|token| state.peers.get(&credential_digest(token)))
        .cloned()
}
fn unauthenticated() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Authentication required",
    )
        .into_response()
}
async fn card(Extract(state): Extract<Arc<State>>, headers: HeaderMap) -> Response {
    if peer(&state, &headers).is_none() {
        return unauthenticated();
    }
    Json(json!({"name": "Colossus", "description": "Bounded coding tasks in an independently authorized Colossus runtime", "version": env!("CARGO_PKG_VERSION"),
        "supportedInterfaces": [{"url": format!("{}/", state.public_url), "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}],
        "capabilities": {"streaming": true, "pushNotifications": false, "extendedAgentCard": false},
        "securitySchemes": {"peerBearer": {"httpAuthSecurityScheme": {"scheme": "Bearer"}}}, "securityRequirements": [{"schemes": {"peerBearer": {"list": []}}}],
        "defaultInputModes": ["text/plain"], "defaultOutputModes": ["text/plain"],
        "skills": [{"id": "coding", "name": "Coding tasks", "description": "Execute text tasks under a fixed operator-selected runtime profile", "tags": ["coding"]}]})).into_response()
}
async fn rpc(Extract(state): Extract<Arc<State>>, incoming: Request) -> Response {
    let headers = incoming.headers().clone();
    let profile = match peer(&state, &headers) {
        Some(profile) => profile,
        None => return unauthenticated(),
    };
    if headers
        .get("A2A-Version")
        .and_then(|value| value.to_str().ok())
        != Some("1.0")
    {
        return Json(
            RpcError {
                code: -32009,
                message: "Version not supported; request A2A-Version 1.0",
                data: None,
            }
            .value(Value::Null),
        )
        .into_response();
    }
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| {
            !value
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON content required").into_response();
    }
    let body = match tokio::time::timeout(
        Duration::from_secs(5),
        axum::body::to_bytes(incoming.into_body(), 64 * 1024),
    )
    .await
    {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "Request exceeds its bound").into_response();
        }
        Err(_) => return (StatusCode::REQUEST_TIMEOUT, "Request body timed out").into_response(),
    };
    let request = match serde_json::from_slice::<RpcRequest>(&body) {
        Ok(request)
            if request.jsonrpc == "2.0"
                && (request.id.as_str().is_some_and(|id| id.len() <= 128)
                    || request.id.as_i64().is_some()) =>
        {
            request
        }
        _ => {
            return Json(
                RpcError {
                    code: -32600,
                    message: "Invalid JSON-RPC request",
                    data: None,
                }
                .value(Value::Null),
            )
            .into_response();
        }
    };
    let global = match state.permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return Json(
                RpcError {
                    code: -32000,
                    message: "Agent admission limit reached",
                    data: None,
                }
                .value(request.id),
            )
            .into_response();
        }
    };
    let local = match profile.permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return Json(
                RpcError {
                    code: -32000,
                    message: "Peer admission limit reached",
                    data: None,
                }
                .value(request.id),
            )
            .into_response();
        }
    };
    let id = request.id.clone();
    let response = dispatch(profile, request, (global, local)).await;
    let mut response = match response {
        Ok(response) => response,
        Err(error) => Json(error.value(id)).into_response(),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    response
}
fn success(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}
async fn dispatch(
    profile: Arc<PeerProfile>,
    request: RpcRequest,
    permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
) -> Result<Response, RpcError> {
    match request.method.as_str() {
        "SendMessage" | "SendStreamingMessage" => {
            let params: SendRequest =
                serde_json::from_value(request.params).map_err(|_| RpcError::invalid())?;
            if !params.tenant.is_empty()
                || params.configuration.task_push_notification_config.is_some()
            {
                return Err(RpcError::unsupported());
            }
            if params.configuration.accepted_output_modes.len() > 16
                || params
                    .configuration
                    .accepted_output_modes
                    .iter()
                    .any(|mode| mode != "text/plain")
            {
                return Err(RpcError::media());
            }
            let history = history_limit(params.configuration.history_length)?;
            let text = text(&params.message)?;
            let _untrusted_metadata = (&params.metadata, &params.message.metadata);
            let task = profile
                .runs
                .submit_agent_task_message(SubmitAgentTaskMessageRequest {
                    message_id: params.message.message_id,
                    task_id: params.message.task_id.filter(|id| !id.is_empty()),
                    context_id: params.message.context_id.filter(|id| !id.is_empty()),
                    text,
                    role: profile.role.clone(),
                    max_turns: profile.max_turns,
                })
                .await?;
            if request.method == "SendStreamingMessage" {
                let task = if history > 0 {
                    read_task(&profile, &task.task_id, history).await?
                } else {
                    task
                };
                return stream(profile, request.id, task, history, permits).await;
            }
            let task = if params.configuration.return_immediately
                || task.status.is_terminal()
                || task.status == AgentTaskStatus::Waiting
            {
                if history > 0 {
                    read_task(&profile, &task.task_id, history).await?
                } else {
                    task
                }
            } else {
                blocking(&profile, task, history).await?
            };
            Ok(success(
                request.id,
                json!({"task": projection::task(&task, true)}),
            ))
        }
        "GetTask" | "CancelTask" | "SubscribeToTask" => {
            let params: TaskRequest =
                serde_json::from_value(request.params).map_err(|_| RpcError::invalid())?;
            if !params.tenant.is_empty() || !token(&params.id) {
                return Err(RpcError::invalid());
            }
            let _untrusted_metadata = &params.metadata;
            let history = history_limit(params.history_length)?;
            let task = read_task(&profile, &params.id, history).await?;
            if request.method == "SubscribeToTask" {
                if task.status.is_terminal() {
                    return Err(RpcError::unsupported());
                }
                return stream(profile, request.id, task, history, permits).await;
            }
            if request.method == "GetTask" {
                return Ok(success(request.id, projection::task(&task, true)));
            }
            if task.status == AgentTaskStatus::Cancelled {
                return Ok(success(request.id, projection::task(&task, true)));
            }
            if task.status.is_terminal() {
                return Err(RpcError {
                    code: -32002,
                    message: "Task is not cancelable",
                    data: None,
                });
            }
            profile
                .runs
                .cancel_run(colossus_sdk::CancelRunRequest {
                    run_id: params.id.clone(),
                    idempotency_key: colossus_sdk::IdempotencyKey::new(format!(
                        "a2a-cancel-{}",
                        params.id
                    ))
                    .map_err(|_| RpcError::invalid())?,
                })
                .await?;
            let task = read_task(&profile, &params.id, history).await?;
            let mut value = projection::task(&task, true);
            if task.status == AgentTaskStatus::Cancelling {
                value["metadata"] = json!({"colossusCancellationRequested": true});
            }
            Ok(success(request.id, value))
        }
        "ListTasks" => {
            let params: ListRequest =
                serde_json::from_value(request.params).map_err(|_| RpcError::invalid())?;
            if let Some(timestamp) = &params.status_timestamp_after {
                time::OffsetDateTime::parse(
                    timestamp,
                    &time::format_description::well_known::Rfc3339,
                )
                .map_err(|_| RpcError::invalid())?;
            }
            if !params.tenant.is_empty()
                || params
                    .page_size
                    .is_some_and(|size| !(1..=100).contains(&size))
            {
                return Err(RpcError::invalid());
            }
            let page_size = params.page_size.unwrap_or(50).min(20) as u32;
            let history_length = history_limit(Some(params.history_length.unwrap_or(0)))?;
            if matches!(
                params.status.as_deref(),
                Some("TASK_STATE_AUTH_REQUIRED" | "TASK_STATE_REJECTED")
            ) {
                if params
                    .page_token
                    .as_ref()
                    .is_some_and(|token| !token.is_empty())
                    || params
                        .context_id
                        .as_ref()
                        .is_some_and(|id| !id.is_empty() && !token(id))
                {
                    return Err(RpcError::invalid());
                }
                return Ok(success(
                    request.id,
                    json!({"tasks": [], "nextPageToken": "", "pageSize": page_size, "totalSize": 0}),
                ));
            }
            let tasks = profile
                .runs
                .list_agent_tasks(ListAgentTasksRequest {
                    context_id: params.context_id.filter(|id| !id.is_empty()),
                    statuses: projection::filters(params.status.as_deref())?,
                    status_updated_after: params.status_timestamp_after,
                    page_size,
                    page_token: params.page_token.filter(|token| !token.is_empty()),
                    include_output: params.include_artifacts,
                    history_length,
                })
                .await?;
            Ok(success(
                request.id,
                json!({"tasks": tasks.tasks.iter().map(|task| projection::task(task, params.include_artifacts)).collect::<Vec<_>>(), "nextPageToken": tasks.next_page_token.unwrap_or_default(), "pageSize": page_size, "totalSize": tasks.total_size}),
            ))
        }
        "GetExtendedAgentCard"
        | "CreateTaskPushNotificationConfig"
        | "GetTaskPushNotificationConfig"
        | "ListTaskPushNotificationConfigs"
        | "DeleteTaskPushNotificationConfig" => Err(RpcError::unsupported()),
        _ => Err(RpcError {
            code: -32601,
            message: "Method not found",
            data: None,
        }),
    }
}
async fn read_task(
    profile: &PeerProfile,
    task_id: &str,
    history_length: u32,
) -> Result<AgentTaskSnapshot, RpcError> {
    // Safe reads can wait for the daemon's token bucket. Never retry admissions or effects.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match profile
                .runs
                .get_agent_task(GetAgentTaskRequest {
                    task_id: task_id.into(),
                    history_length,
                })
                .await
            {
                Err(error) if error.code == ApiErrorCode::ResourceExhausted => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                result => return result.map_err(RpcError::from),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        Err(RpcError {
            code: -32000,
            message: "Task read timed out; use GetTask to reconcile",
            data: Some(json!({"taskId": task_id})),
        })
    })
}
async fn blocking(
    profile: &PeerProfile,
    task: AgentTaskSnapshot,
    history: u32,
) -> Result<AgentTaskSnapshot, RpcError> {
    let task_id = task.task_id;
    let result = tokio::time::timeout(Duration::from_secs(300), async {
        let mut updates = profile
            .runs
            .watch_run(colossus_sdk::WatchRunRequest {
                run_id: task_id.clone(),
                after_sequence: task.last_sequence,
            })
            .await?;
        while let Some(update) = updates.next().await {
            let update = update.map_err(RpcError::from)?;
            if !matches!(
                update.update,
                colossus_sdk::RunUpdateKind::Interaction(_)
                    | colossus_sdk::RunUpdateKind::Result(_)
                    | colossus_sdk::RunUpdateKind::Failure { .. }
                    | colossus_sdk::RunUpdateKind::Cancellation(_)
            ) {
                continue;
            }
            let task = read_task(profile, &task_id, history).await?;
            if task.status.is_terminal() || task.status == AgentTaskStatus::Waiting {
                return Ok(task);
            }
        }
        read_task(profile, &task_id, history).await
    })
    .await;
    match result {
        Ok(Ok(task)) if task.status.is_terminal() || task.status == AgentTaskStatus::Waiting => {
            Ok(task)
        }
        Ok(Err(error)) => Err(error),
        Ok(Ok(_)) | Err(_) => Err(RpcError {
            code: -32000,
            message: "Task remains active; use GetTask to reconcile",
            data: Some(json!({"taskId": task_id, "accepted": true})),
        }),
    }
}
async fn stream(
    profile: Arc<PeerProfile>,
    id: Value,
    task: AgentTaskSnapshot,
    history: u32,
    permits: (OwnedSemaphorePermit, OwnedSemaphorePermit),
) -> Result<Response, RpcError> {
    let updates = if task.status.is_terminal() {
        None
    } else {
        Some(
            profile
                .runs
                .watch_run(colossus_sdk::WatchRunRequest {
                    run_id: task.task_id.clone(),
                    after_sequence: task.last_sequence,
                })
                .await?,
        )
    };
    let pending = VecDeque::from([json!({"task": projection::task(&task, true)})]);
    let stream = futures::stream::unfold(
        (
            updates,
            pending,
            profile,
            id,
            task.task_id,
            task.status.is_terminal(),
            permits,
        ),
        move |(mut updates, mut pending, profile, id, task_id, mut done, permits)| async move {
            loop {
                if let Some(result) = pending.pop_front() {
                    let event = Event::default()
                        .data(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string());
                    return Some((
                        Ok::<_, Infallible>(event),
                        (updates, pending, profile, id, task_id, done, permits),
                    ));
                }
                if done {
                    return None;
                }
                let update = match updates.as_mut()?.next().await {
                    Some(update) => update,
                    None => {
                        let error = match read_task(&profile, &task_id, history).await {
                            Ok(task) if task.status.is_terminal() => {
                                queue_task_updates(&task, &mut pending);
                                done = true;
                                continue;
                            }
                            Ok(_) => RpcError {
                                code: -32000,
                                message: "Task remains active; use GetTask to reconcile",
                                data: Some(json!({"taskId": task_id, "accepted": true})),
                            },
                            Err(error) => error,
                        };
                        let event = Event::default().data(error.value(id.clone()).to_string());
                        return Some((
                            Ok(event),
                            (updates, pending, profile, id, task_id, true, permits),
                        ));
                    }
                };
                let error = match update {
                    Ok(update)
                        if matches!(
                            update.update,
                            colossus_sdk::RunUpdateKind::State(_)
                                | colossus_sdk::RunUpdateKind::Interaction(_)
                                | colossus_sdk::RunUpdateKind::Result(_)
                                | colossus_sdk::RunUpdateKind::Failure { .. }
                                | colossus_sdk::RunUpdateKind::Cancellation(_)
                        ) =>
                    {
                        match read_task(&profile, &task_id, history).await {
                            Ok(task) => {
                                queue_task_updates(&task, &mut pending);
                                done = task.status.is_terminal();
                                None
                            }
                            Err(error) => Some(error),
                        }
                    }
                    Ok(_) => None,
                    Err(error) => Some(RpcError::from(error)),
                };
                if let Some(error) = error {
                    let event = Event::default().data(error.value(id.clone()).to_string());
                    return Some((
                        Ok(event),
                        (updates, pending, profile, id, task_id, true, permits),
                    ));
                }
            }
        },
    );
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}
fn queue_task_updates(task: &AgentTaskSnapshot, pending: &mut VecDeque<Value>) {
    if task.status == AgentTaskStatus::Completed
        && let Some(artifact) = projection::artifact(task)
    {
        pending.push_back(json!({"artifactUpdate": {"taskId": task.task_id, "contextId": task.context_id, "artifact": artifact, "lastChunk": true}}));
    }
    pending.push_back(json!({"statusUpdate": {"taskId": task.task_id, "contextId": task.context_id, "status": projection::status(task)}}));
}
