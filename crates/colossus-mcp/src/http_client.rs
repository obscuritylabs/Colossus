use colossus_network::{AdditionalRootCertificates, pinned_reqwest_client};
use colossus_policy::{
    ExecutionError, ExecutionPermit, NetworkDestinationMatch, network_destination_match,
    non_public_network_address,
};
use futures::{StreamExt as _, stream::BoxStream};
use http::{HeaderName, HeaderValue, header::WWW_AUTHENTICATE};
use reqwest::{Method, Response, StatusCode, Url};
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
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
use std::{borrow::Cow, collections::HashMap, sync::Arc};
use thiserror::Error;

#[derive(Clone)]
pub(super) struct HardenedStreamableHttpClient {
    endpoint: Arc<str>,
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl HardenedStreamableHttpClient {
    pub(super) async fn new(
        endpoint: &str,
        permit: &ExecutionPermit,
        tls_roots: &AdditionalRootCertificates,
    ) -> Result<Self, ExecutionError> {
        let url = Url::parse(endpoint).map_err(adapter_failure)?;
        let origin = url.origin().ascii_serialization();
        let matched =
            network_destination_match(&permit.obligations().network_destinations, &origin)
                .map_err(adapter_failure)?
                .ok_or_else(|| adapter_failure("MCP HTTP origin is not permitted"))?;
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("MCP HTTP URL has no host"))?;
        let allow_non_public = matched == NetworkDestinationMatch::Exact
            && (host.eq_ignore_ascii_case("localhost")
                || colossus_network::parse_host_ip(host).is_some_and(non_public_network_address));
        let client = pinned_reqwest_client(
            &url,
            tls_roots,
            permit.obligations().timeout_ms,
            allow_non_public,
        )
        .await
        .map_err(adapter_failure)?;
        let max_response_bytes =
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?;
        Ok(Self {
            endpoint: endpoint.into(),
            client,
            max_response_bytes,
        })
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
            builder = builder.header(name, value);
        }
        Ok(builder)
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
}

impl StreamableHttpClient for HardenedStreamableHttpClient {
    type Error = McpHttpClientError;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request = self
            .request(Method::GET, &uri, auth_header, custom_headers)?
            .header(
                reqwest::header::ACCEPT,
                [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "),
            )
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            if last_event_id.is_empty() || last_event_id.len() > 8 * 1024 {
                return Err(StreamableHttpError::Client(McpHttpClientError::Request));
            }
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        let response = request
            .send()
            .await
            .map_err(|_| StreamableHttpError::Client(McpHttpClientError::Request))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(unexpected_status(response.status()));
        }
        require_content_type(&response, EVENT_STREAM_MIME_TYPE)?;
        Ok(bounded_sse_stream(response, self.max_response_bytes))
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
            .map_err(|_| StreamableHttpError::Client(McpHttpClientError::Request))?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED || response.status().is_success() {
            return Ok(());
        }
        Err(unexpected_status(response.status()))
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let session_was_attached = session_id.is_some();
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
            .map_err(|_| StreamableHttpError::Client(McpHttpClientError::Request))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                sanitized_auth_challenge(&response),
            )));
        }
        if response.status() == StatusCode::FORBIDDEN {
            return Err(StreamableHttpError::InsufficientScope(
                InsufficientScopeError::new(sanitized_auth_challenge(&response), None),
            ));
        }
        let status = response.status();
        if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        if !status.is_success() {
            return Err(unexpected_status(status));
        }
        let one_way = matches!(
            message,
            ClientJsonRpcMessage::Notification(_)
                | ClientJsonRpcMessage::Response(_)
                | ClientJsonRpcMessage::Error(_)
        );
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
                    bounded_sse_stream(response, self.max_response_bytes),
                    session_id,
                ))
            }
            Some(value) if content_type_matches(value, JSON_MIME_TYPE) => {
                let bytes = bounded_body(response, self.max_response_bytes).await?;
                if one_way && is_empty_body(&bytes) {
                    return Ok(StreamableHttpPostResponse::Accepted);
                }
                let message = serde_json::from_slice::<ServerJsonRpcMessage>(&bytes)?;
                Ok(StreamableHttpPostResponse::Json(message, session_id))
            }
            // Chunked and close-delimited responses expose no size hint, so the
            // bounded body is the only way to tell an empty one-way acknowledgement
            // apart from a genuinely malformed payload.
            _ if one_way && declared_length.is_none() => {
                let bytes = bounded_body(response, self.max_response_bytes).await?;
                if is_empty_body(&bytes) {
                    Ok(StreamableHttpPostResponse::Accepted)
                } else {
                    Err(StreamableHttpError::UnexpectedContentType(content_type))
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
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
) -> Result<Vec<u8>, StreamableHttpError<McpHttpClientError>> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| StreamableHttpError::Client(McpHttpClientError::Request))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
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
) -> BoxStream<'static, Result<Sse, SseError>> {
    let bounded = response
        .bytes_stream()
        .scan((0_usize, false), move |state, item| {
            let result = if state.1 {
                None
            } else {
                Some(match item {
                    Ok(chunk) if state.0.saturating_add(chunk.len()) <= limit => {
                        state.0 += chunk.len();
                        Ok(chunk)
                    }
                    Ok(_) => {
                        state.1 = true;
                        Err(McpHttpClientError::ResponseTooLarge)
                    }
                    Err(_) => {
                        state.1 = true;
                        Err(McpHttpClientError::Request)
                    }
                })
            };
            std::future::ready(result)
        });
    SseStream::from_bytes_stream(bounded).boxed()
}

fn adapter_failure(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}
