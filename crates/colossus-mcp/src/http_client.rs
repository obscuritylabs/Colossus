use crate::{
    McpOperation,
    diagnostics::{McpDiagnosticCapture, request_failure},
};
use colossus_contracts::{McpDiagnosticCode, McpDiagnosticStage};
use colossus_network::{AdditionalRootCertificates, PinnedHttpClientError, pinned_reqwest_client};
use colossus_policy::{
    ExecutionError, ExecutionPermit, NetworkDestinationMatch, http_transport_authority_match,
    non_public_network_address,
};
use futures::{StreamExt as _, stream::BoxStream};
use http::{HeaderName, HeaderValue, header::WWW_AUTHENTICATE};
use reqwest::{Method, Response, StatusCode, Url};
use rmcp::{
    model::{
        ClientJsonRpcMessage, ClientRequest, ErrorData, GetMeta, ProtocolVersion,
        ServerJsonRpcMessage,
    },
    transport::{
        common::http_header::{
            EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
        },
        streamable_http_client::{
            AuthRequiredError, InsufficientScopeError, StreamableHttpClient, StreamableHttpError,
            StreamableHttpPostResponse,
        },
    },
};
use sse_stream::{Error as SseError, Sse, SseStream};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use thiserror::Error;

#[derive(Clone)]
pub(super) struct HardenedStreamableHttpClient {
    endpoint: Arc<str>,
    client: reqwest::Client,
    max_response_bytes: usize,
    received_bytes: Arc<AtomicUsize>,
    call_headers: Result<HashMap<HeaderName, HeaderValue>, String>,
    diagnostics: McpDiagnosticCapture,
}

impl HardenedStreamableHttpClient {
    pub(super) async fn new(
        endpoint: &str,
        permit: &ExecutionPermit,
        tls_roots: &AdditionalRootCertificates,
        operation: &McpOperation,
        timeout_ms: u64,
        max_output_bytes: u64,
    ) -> Result<Self, ExecutionError> {
        let diagnostics = McpDiagnosticCapture::current();
        diagnostics.stage(McpDiagnosticStage::ClientSetup);
        let url = Url::parse(endpoint).map_err(adapter_failure)?;
        let matched = http_transport_authority_match(permit.obligations(), endpoint)
            .map_err(adapter_failure)?
            .ok_or_else(|| adapter_failure("MCP HTTP origin is not permitted"))?;
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("MCP HTTP URL has no host"))?;
        let allow_non_public = matched == NetworkDestinationMatch::Ambient
            || (matched == NetworkDestinationMatch::Exact
                && (host.eq_ignore_ascii_case("localhost")
                    || colossus_network::parse_host_ip(host)
                        .is_some_and(non_public_network_address)));
        let client = pinned_reqwest_client(
            &url,
            tls_roots,
            timeout_ms.min(permit.obligations().timeout_ms),
            allow_non_public,
        )
        .await
        .map_err(|error| {
            diagnostics.fail(
                match &error {
                    PinnedHttpClientError::Resolution(_) => McpDiagnosticCode::Dns,
                    _ => McpDiagnosticCode::Configuration,
                },
                None,
            );
            adapter_failure(error)
        })?;
        let max_response_bytes =
            usize::try_from(max_output_bytes.min(permit.obligations().max_output_bytes))
                .map_err(adapter_failure)?;
        Ok(Self {
            endpoint: endpoint.into(),
            client,
            max_response_bytes,
            received_bytes: Arc::new(AtomicUsize::new(0)),
            call_headers: Ok(HashMap::new()),
            diagnostics,
        }
        .with_call_headers(operation))
    }

    pub(super) fn with_call_headers(mut self, operation: &McpOperation) -> Self {
        // Legacy protocols do not interpret x-mcp-header. Retain any validation
        // failure until a modern call actually needs the parameter headers.
        self.call_headers = crate::param_headers::call_headers(operation);
        self
    }

    #[cfg(test)]
    pub(super) fn for_test(
        endpoint: String,
        client: reqwest::Client,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            client,
            max_response_bytes,
            received_bytes: Arc::new(AtomicUsize::new(0)),
            call_headers: Ok(HashMap::new()),
            diagnostics: McpDiagnosticCapture::current(),
        }
    }

    fn request(
        &self,
        method: Method,
        uri: &str,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<reqwest::RequestBuilder, StreamableHttpError<McpHttpClientError>> {
        if uri != self.endpoint.as_ref() {
            return Err(StreamableHttpError::Client(
                McpHttpClientError::EndpointMismatch,
            ));
        }
        let mut builder = self.client.request(method, uri);
        if let Some(token) = auth_header {
            builder = builder.bearer_auth(token);
        }
        for (name, value) in custom_headers {
            if matches!(name.as_str(), "traceparent" | "tracestate" | "baggage") {
                continue;
            }
            builder = builder.header(name, value);
        }
        for (name, value) in colossus_observability::current_trace_headers() {
            builder = builder.header(name, value);
        }
        Ok(builder)
    }

    fn request_error(&self, error: reqwest::Error) -> StreamableHttpError<McpHttpClientError> {
        self.diagnostics.fail(request_failure(&error), None);
        StreamableHttpError::Client(McpHttpClientError::Request)
    }

    fn status_error(&self, status: StatusCode) -> StreamableHttpError<McpHttpClientError> {
        self.diagnostics
            .fail(McpDiagnosticCode::HttpStatus, Some(status.as_u16()));
        unexpected_status(status)
    }
}

#[derive(Debug, Error)]
pub(super) enum McpHttpClientError {
    #[error("HTTP request failed")]
    Request,
    #[error("HTTP response exceeded its permitted bound")]
    ResponseTooLarge,
    #[error("MCP transport attempted an unconfigured endpoint")]
    EndpointMismatch,
    #[error("MCP parameter header annotations are invalid")]
    InvalidParameterHeaders,
}

impl StreamableHttpClient for HardenedStreamableHttpClient {
    type Error = McpHttpClientError;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request = self
            .request(Method::GET, &uri, auth_header, custom_headers)?
            .header(
                reqwest::header::ACCEPT,
                [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
            );
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        if let Some(last_event_id) = last_event_id {
            if last_event_id.is_empty() || last_event_id.len() > 8 * 1024 {
                return Err(StreamableHttpError::Client(McpHttpClientError::Request));
            }
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        let response = request
            .send()
            .await
            .map_err(|error| self.request_error(error))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(self.status_error(response.status()));
        }
        require_content_type(&response, EVENT_STREAM_MIME_TYPE)?;
        Ok(bounded_sse_stream(
            response,
            self.max_response_bytes,
            self.received_bytes.clone(),
            self.diagnostics.clone(),
        ))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let response = self
            .request(Method::DELETE, &uri, auth_header, custom_headers)?
            .header(HEADER_SESSION_ID, session_id.as_ref())
            .send()
            .await
            .map_err(|error| self.request_error(error))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED || response.status().is_success() {
            return Ok(());
        }
        Err(self.status_error(response.status()))
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        mut custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        if let ClientJsonRpcMessage::Request(request) = &message
            && matches!(request.request, ClientRequest::CallToolRequest(_))
            && request
                .request
                .get_meta()
                .protocol_version()
                .is_some_and(|version| version >= ProtocolVersion::STANDARD_HEADERS)
        {
            let headers = self.call_headers.as_ref().map_err(|_| {
                StreamableHttpError::Client(McpHttpClientError::InvalidParameterHeaders)
            })?;
            custom_headers.extend(headers.clone());
        }
        let session_was_attached = session_id.is_some();
        let one_way = matches!(
            message,
            ClientJsonRpcMessage::Notification(_)
                | ClientJsonRpcMessage::Response(_)
                | ClientJsonRpcMessage::Error(_)
        );
        let mut request = self
            .request(Method::POST, &uri, auth_header, custom_headers)?
            .header(
                reqwest::header::ACCEPT,
                [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
            )
            .json(&message);
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = request
            .send()
            .await
            .map_err(|error| self.request_error(error))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.diagnostics
                .fail(McpDiagnosticCode::HttpStatus, Some(401));
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                sanitized_auth_challenge(&response),
            )));
        }
        if response.status() == StatusCode::FORBIDDEN {
            self.diagnostics
                .fail(McpDiagnosticCode::HttpStatus, Some(403));
            return Err(StreamableHttpError::InsufficientScope(
                InsufficientScopeError::new(sanitized_auth_challenge(&response), None),
            ));
        }
        let status = response.status();
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return if one_way {
                Ok(StreamableHttpPostResponse::Accepted)
            } else {
                Err(self.status_error(status))
            };
        }
        if status == StatusCode::NOT_FOUND && session_was_attached {
            self.diagnostics
                .fail(McpDiagnosticCode::HttpStatus, Some(404));
            return Err(StreamableHttpError::SessionExpired);
        }
        if !status.is_success() {
            // Classify a bounded discovery rejection in the lifecycle layer.
            // Authentication and expired sessions retain their dedicated paths.
            if !session_was_attached
                && status.is_client_error()
                && let ClientJsonRpcMessage::Request(request) = &message
                && matches!(request.request, ClientRequest::DiscoverRequest(_))
            {
                let bytes = bounded_body(
                    response,
                    self.max_response_bytes,
                    &self.received_bytes,
                    &self.diagnostics,
                )
                .await?;
                let error = match serde_json::from_slice::<ServerJsonRpcMessage>(&bytes) {
                    Ok(ServerJsonRpcMessage::Error(error)) => error.error,
                    _ => ErrorData::invalid_request("MCP discovery is unavailable", None),
                };
                return Ok(StreamableHttpPostResponse::Json(
                    ServerJsonRpcMessage::error(error, Some(request.id.clone())),
                    None,
                ));
            }
            return Err(self.status_error(status));
        }
        let declared_length = response.content_length();
        if one_way && declared_length == Some(0) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty() && value.len() <= 8 * 1024)
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        match content_type.as_deref() {
            Some(value) if content_type_matches(value, EVENT_STREAM_MIME_TYPE) => {
                Ok(StreamableHttpPostResponse::Sse(
                    bounded_sse_stream(
                        response,
                        self.max_response_bytes,
                        self.received_bytes.clone(),
                        self.diagnostics.clone(),
                    ),
                    session_id,
                ))
            }
            Some(value) if content_type_matches(value, JSON_MIME_TYPE) => {
                let bytes = bounded_body(
                    response,
                    self.max_response_bytes,
                    &self.received_bytes,
                    &self.diagnostics,
                )
                .await?;
                if one_way && is_empty_body(&bytes) {
                    return Ok(StreamableHttpPostResponse::Accepted);
                }
                let mut value = serde_json::from_slice(&bytes)?;
                crate::wire::filter_unsupported_task_tools(&mut value);
                let message = serde_json::from_value::<ServerJsonRpcMessage>(value)?;
                Ok(StreamableHttpPostResponse::Json(message, session_id))
            }
            // Chunked and close-delimited responses expose no size hint, so the
            // bounded body is the only way to tell an empty one-way acknowledgement
            // apart from a genuinely malformed payload.
            _ if one_way && declared_length.is_none() => {
                let bytes = bounded_body(
                    response,
                    self.max_response_bytes,
                    &self.received_bytes,
                    &self.diagnostics,
                )
                .await?;
                if is_empty_body(&bytes) {
                    Ok(StreamableHttpPostResponse::Accepted)
                } else {
                    Err(StreamableHttpError::UnexpectedContentType(content_type))
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut bounded = self.clone();
        // Bounding the entire raw response also bounds every unparsed SSE event.
        bounded.max_response_bytes = self.max_response_bytes.min(max_sse_event_size);
        bounded
            .post_message(uri, message, session_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut bounded = self.clone();
        bounded.max_response_bytes = self.max_response_bytes.min(max_sse_event_size);
        bounded
            .get_stream(uri, session_id, last_event_id, auth_header, custom_headers)
            .await
    }
}

fn require_content_type(
    response: &Response,
    required: &str,
) -> Result<(), StreamableHttpError<McpHttpClientError>> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if content_type.is_some_and(|value| content_type_matches(value, required)) {
        Ok(())
    } else {
        Err(StreamableHttpError::UnexpectedContentType(
            content_type.map(str::to_owned),
        ))
    }
}

/// A body carrying no JSON-RPC frame, allowing only insignificant HTTP whitespace.
fn is_empty_body(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace)
}

pub(super) fn content_type_matches(value: &str, required: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case(required))
}

fn sanitized_auth_challenge(response: &Response) -> String {
    if response.headers().contains_key(WWW_AUTHENTICATE) {
        "Bearer".into()
    } else {
        String::new()
    }
}

fn unexpected_status(status: StatusCode) -> StreamableHttpError<McpHttpClientError> {
    StreamableHttpError::UnexpectedServerResponse(Cow::Owned(format!(
        "MCP HTTP server returned {status}"
    )))
}

async fn bounded_body(
    response: Response,
    limit: usize,
    received: &AtomicUsize,
    diagnostics: &McpDiagnosticCapture,
) -> Result<Vec<u8>, StreamableHttpError<McpHttpClientError>> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            diagnostics.fail(request_failure(&error), None);
            StreamableHttpError::Client(McpHttpClientError::Request)
        })?;
        if received
            .fetch_add(chunk.len(), Ordering::Relaxed)
            .saturating_add(chunk.len())
            > limit
        {
            diagnostics.fail(McpDiagnosticCode::ResponseTooLarge, None);
            return Err(StreamableHttpError::Client(
                McpHttpClientError::ResponseTooLarge,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn bounded_sse_stream(
    response: Response,
    limit: usize,
    received: Arc<AtomicUsize>,
    diagnostics: McpDiagnosticCapture,
) -> BoxStream<'static, Result<Sse, SseError>> {
    let bounded = response
        .bytes_stream()
        .scan((0_usize, false), move |state, item| {
            let result = if state.1 {
                None
            } else {
                Some(match item {
                    Ok(chunk)
                        if received
                            .fetch_add(chunk.len(), Ordering::Relaxed)
                            .saturating_add(chunk.len())
                            <= limit =>
                    {
                        state.0 += chunk.len();
                        Ok(chunk)
                    }
                    Ok(_) => {
                        diagnostics.fail(McpDiagnosticCode::ResponseTooLarge, None);
                        state.1 = true;
                        Err(McpHttpClientError::ResponseTooLarge)
                    }
                    Err(error) => {
                        diagnostics.fail(request_failure(&error), None);
                        state.1 = true;
                        Err(McpHttpClientError::Request)
                    }
                })
            };
            std::future::ready(result)
        });
    SseStream::from_bytes_stream(bounded)
        .map(|event| {
            event.map(|mut event| {
                if let Some(data) = &event.data
                    && let Ok(mut value) = serde_json::from_str(data)
                {
                    crate::wire::filter_unsupported_task_tools(&mut value);
                    event.data = Some(value.to_string());
                }
                event
            })
        })
        .boxed()
}

fn adapter_failure(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}
