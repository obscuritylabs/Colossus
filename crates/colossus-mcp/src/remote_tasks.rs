//! Send approved modern Tasks requests through the hardened HTTP client.

use crate::http_client::{HardenedStreamableHttpClient, McpHttpClientError};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::StreamExt as _;
use http::{HeaderName, HeaderValue};
use rmcp::{
    model::{
        CallToolRequest, CallToolResult, CancelTaskParams, CancelTaskRequest, ClientJsonRpcMessage,
        ClientRequest, ErrorCode, ErrorData, GetMeta as _, GetTaskParams, GetTaskRequest,
        Implementation, JsonRpcRequest, ProtocolVersion, RequestId, RequestMetaObject,
        ServerJsonRpcMessage, ServerResult, TaskPayload, TaskStatus,
    },
    transport::{
        common::http_header::{BASE64_HEADER_PREFIX, BASE64_HEADER_SUFFIX},
        streamable_http_client::{
            StreamableHttpClient as _, StreamableHttpError, StreamableHttpPostResponse,
        },
    },
};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    fmt,
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};
use zeroize::Zeroizing;

#[cfg(test)]
mod tests;

const MAX_TASK_ID_BYTES: usize = 2048;
static REQUEST_IDS: AtomicI64 = AtomicI64::new(1);

/// Opaque task identifier intended for the configured endpoint's wire only.
#[derive(Clone)]
pub(super) struct TaskHandle(Zeroizing<String>);

impl TaskHandle {
    fn new(task_id: String) -> Result<Self, TaskRequestError> {
        if task_id.is_empty() || task_id.len() > MAX_TASK_ID_BYTES {
            return Err(TaskRequestError::Protocol);
        }
        Ok(Self(Zeroizing::new(task_id)))
    }

    /// Bind the handle to private evidence with a caller-generated random nonce.
    pub(super) fn binding(&self, nonce: &[u8; 32]) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"colossus:mcp-task-handle:v1\0");
        hash.update(nonce);
        hash.update(self.0.as_bytes());
        hash.finalize().into()
    }

    /// Prevent the final tool result from releasing an echoed handle.
    pub(super) fn protect_output(&self, secrets: &mut Vec<String>) {
        secrets.push(self.0.as_str().to_owned());
        if let Some(encoded) = encoded_routing_name(&self.0) {
            secrets.push(encoded);
        }
    }
}

impl fmt::Debug for TaskHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TaskHandle([redacted])")
    }
}

pub(super) struct TaskStart {
    pub(super) handle: TaskHandle,
    pub(super) status: TaskStatus,
    pub(super) poll_interval: Option<Duration>,
    pub(super) ttl: Option<Duration>,
}

pub(super) enum TaskCallResult {
    Complete(CallToolResult),
    Started(TaskStart),
    InputRequired,
}

pub(super) struct TaskObservation {
    pub(super) state: TaskState,
    pub(super) poll_interval: Option<Duration>,
    pub(super) ttl: Option<Duration>,
}

pub(super) enum TaskState {
    Working,
    InputRequired,
    Completed(CallToolResult),
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum TaskRequestError {
    #[error("MCP task request lost its transport")]
    Transport,
    #[error("MCP task is unavailable or expired")]
    Unavailable,
    #[error("MCP task request was refused by the endpoint")]
    Refused,
    #[error("MCP task request or response violated the protocol or its bounds")]
    Protocol,
    #[error("MCP tool response was a JSON-RPC error ({0})")]
    CallRpc(i32),
}

/// Send the initial approved tool request once with scoped capability metadata.
pub(super) async fn call_tool(
    http: &HardenedStreamableHttpClient,
    endpoint: &str,
    headers: HashMap<HeaderName, HeaderValue>,
    request: CallToolRequest,
) -> Result<TaskCallResult, TaskRequestError> {
    let result = send(http, endpoint, headers, request.into()).await?;
    match result {
        ServerResult::CallToolResult(result) => Ok(TaskCallResult::Complete(result)),
        ServerResult::CreateTaskResult(result) => Ok(TaskCallResult::Started(TaskStart {
            handle: TaskHandle::new(result.task.task_id)?,
            status: result.task.status,
            poll_interval: result.task.poll_interval_ms.map(Duration::from_millis),
            ttl: result.task.ttl_ms.map(Duration::from_millis),
        })),
        ServerResult::InputRequiredResult(_) => Ok(TaskCallResult::InputRequired),
        _ => Err(TaskRequestError::Protocol),
    }
}

/// Observe one task after the caller has authorized the follow-up.
pub(super) async fn get_task(
    http: &HardenedStreamableHttpClient,
    endpoint: &str,
    headers: HashMap<HeaderName, HeaderValue>,
    handle: &TaskHandle,
) -> Result<TaskObservation, TaskRequestError> {
    let request = GetTaskRequest::new(GetTaskParams::new(handle.0.as_str()));
    let result = send(http, endpoint, headers, request.into()).await?;
    observation(result, handle)
}

fn observation(
    result: ServerResult,
    handle: &TaskHandle,
) -> Result<TaskObservation, TaskRequestError> {
    let ServerResult::GetTaskResult(result) = result else {
        return Err(TaskRequestError::Protocol);
    };
    let task = result.task;
    if !result.result_type.is_complete() || task.task.task_id != handle.0.as_str() {
        return Err(TaskRequestError::Protocol);
    }
    let state = match task.payload {
        TaskPayload::Working => TaskState::Working,
        TaskPayload::InputRequired { .. } => TaskState::InputRequired,
        TaskPayload::Cancelled => TaskState::Cancelled,
        TaskPayload::Completed { result } => {
            // The output adapter protects string values. Keys may be schema-owned
            // and cannot safely be rewritten there.
            let encoded = encoded_routing_name(&handle.0);
            if contains_handle_key(&result, &handle.0, encoded.as_deref()) {
                return Err(TaskRequestError::Protocol);
            }
            TaskState::Completed(
                serde_json::from_value(result.into()).map_err(|_| TaskRequestError::Protocol)?,
            )
        }
        TaskPayload::Failed { error } => {
            // Check the error shape before treating failed as a confirmed terminal state.
            serde_json::from_value::<ErrorData>(error.into())
                .map_err(|_| TaskRequestError::Protocol)?;
            TaskState::Failed
        }
        _ => return Err(TaskRequestError::Protocol),
    };
    Ok(TaskObservation {
        state,
        poll_interval: task.task.poll_interval_ms.map(Duration::from_millis),
        ttl: task.task.ttl_ms.map(Duration::from_millis),
    })
}

fn contains_handle_key(object: &Map<String, Value>, handle: &str, encoded: Option<&str>) -> bool {
    object.iter().any(|(key, value)| {
        key.contains(handle)
            || encoded.is_some_and(|encoded| key.contains(encoded))
            || nested_handle_key(value, handle, encoded)
    })
}

fn nested_handle_key(value: &Value, handle: &str, encoded: Option<&str>) -> bool {
    match value {
        Value::Object(object) => contains_handle_key(object, handle, encoded),
        Value::Array(values) => values
            .iter()
            .any(|value| nested_handle_key(value, handle, encoded)),
        _ => false,
    }
}

/// Request cooperative cancellation. The acknowledgement carries no task state.
pub(super) async fn cancel_task(
    http: &HardenedStreamableHttpClient,
    endpoint: &str,
    headers: HashMap<HeaderName, HeaderValue>,
    handle: &TaskHandle,
) -> Result<(), TaskRequestError> {
    let request = CancelTaskRequest::new(CancelTaskParams::new(handle.0.as_str()));
    match send(http, endpoint, headers, request.into()).await? {
        ServerResult::TaskAckResult(_) => Ok(()),
        _ => Err(TaskRequestError::Protocol),
    }
}

async fn send(
    http: &HardenedStreamableHttpClient,
    endpoint: &str,
    mut headers: HashMap<HeaderName, HeaderValue>,
    mut request: ClientRequest,
) -> Result<ServerResult, TaskRequestError> {
    let (method, name) = match &request {
        ClientRequest::CallToolRequest(request) => ("tools/call", request.params.name.as_ref()),
        ClientRequest::GetTaskRequest(request) => ("tasks/get", request.params.task_id.as_str()),
        ClientRequest::CancelTaskRequest(request) => {
            ("tasks/cancel", request.params.task_id.as_str())
        }
        _ => return Err(TaskRequestError::Protocol),
    };
    headers.insert(HeaderName::from_static("mcp-name"), routing_name(name)?);
    headers.insert(
        HeaderName::from_static("mcp-method"),
        HeaderValue::from_static(method),
    );
    headers.insert(
        HeaderName::from_static("mcp-protocol-version"),
        HeaderValue::from_static("2026-07-28"),
    );
    let mut capabilities = rmcp::model::ClientCapabilities::default();
    capabilities
        .extensions
        .get_or_insert_with(Default::default)
        .insert(rmcp::model::TASKS_EXTENSION_ID.into(), Map::new());
    request
        .get_meta_mut()
        .extend(RequestMetaObject::with_client_context(
            ProtocolVersion::V_2026_07_28,
            Implementation::new("colossus", env!("CARGO_PKG_VERSION")),
            capabilities,
        ));
    let id = RequestId::Number(REQUEST_IDS.fetch_add(1, Ordering::Relaxed));
    let message = ClientJsonRpcMessage::Request(JsonRpcRequest::new(id.clone(), request));
    // The SDK worker traces whole protocol events. Use its typed messages with the
    // hardened transport directly so task handles stay out of that worker's logs.
    let response = http
        .post_message(endpoint.into(), message, None, None, headers)
        .await
        .map_err(classify_http_error)?;
    receive(response, &id, method).await
}

fn routing_name(name: &str) -> Result<HeaderValue, TaskRequestError> {
    let encoded = encoded_routing_name(name);
    let mut value = HeaderValue::from_str(encoded.as_deref().unwrap_or(name))
        .map_err(|_| TaskRequestError::Protocol)?;
    value.set_sensitive(true);
    Ok(value)
}

fn encoded_routing_name(name: &str) -> Option<String> {
    let needs_encoding = name.starts_with([' ', '\t'])
        || name.ends_with([' ', '\t'])
        || name.bytes().any(|byte| !(0x20..=0x7e).contains(&byte))
        || (name.starts_with(BASE64_HEADER_PREFIX) && name.ends_with(BASE64_HEADER_SUFFIX));
    needs_encoding.then(|| {
        format!(
            "{BASE64_HEADER_PREFIX}{}{BASE64_HEADER_SUFFIX}",
            STANDARD.encode(name)
        )
    })
}

fn classify_http_error(error: StreamableHttpError<McpHttpClientError>) -> TaskRequestError {
    match error {
        StreamableHttpError::Client(McpHttpClientError::Request) => TaskRequestError::Transport,
        StreamableHttpError::AuthRequired(_) | StreamableHttpError::InsufficientScope(_) => {
            TaskRequestError::Refused
        }
        _ => TaskRequestError::Protocol,
    }
}

async fn receive(
    response: StreamableHttpPostResponse,
    request_id: &RequestId,
    method: &str,
) -> Result<ServerResult, TaskRequestError> {
    match response {
        StreamableHttpPostResponse::Json(message, None) => {
            message_result(message, request_id, method)
        }
        StreamableHttpPostResponse::Sse(mut stream, None) => {
            while let Some(event) = stream.next().await {
                let event = event.map_err(|error| match error {
                    sse_stream::Error::Body(source)
                        if matches!(
                            source.downcast_ref::<McpHttpClientError>(),
                            Some(McpHttpClientError::Request)
                        ) =>
                    {
                        TaskRequestError::Transport
                    }
                    _ => TaskRequestError::Protocol,
                })?;
                let Some(data) = event.data.filter(|data| !data.is_empty()) else {
                    continue;
                };
                let message =
                    serde_json::from_str(&data).map_err(|_| TaskRequestError::Protocol)?;
                if !matches!(message, ServerJsonRpcMessage::Notification(_)) {
                    return message_result(message, request_id, method);
                }
            }
            Err(TaskRequestError::Transport)
        }
        _ => Err(TaskRequestError::Protocol),
    }
}

fn message_result(
    message: ServerJsonRpcMessage,
    request_id: &RequestId,
    method: &str,
) -> Result<ServerResult, TaskRequestError> {
    match message {
        ServerJsonRpcMessage::Response(response) if response.id == *request_id => {
            Ok(response.result)
        }
        ServerJsonRpcMessage::Error(error) if error.id.as_ref() == Some(request_id) => {
            let code = error.error.code;
            Err(if method == "tools/call" {
                TaskRequestError::CallRpc(code.0)
            } else if code == ErrorCode::INVALID_PARAMS {
                TaskRequestError::Unavailable
            } else {
                TaskRequestError::Refused
            })
        }
        _ => Err(TaskRequestError::Protocol),
    }
}
