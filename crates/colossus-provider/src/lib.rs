//! Permit-bound model-provider adapters and normalized provider events.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, EffectRequest, ModelMessage, ModelMessageRole, ModelRequest,
    ModelToolCall, ModelToolDefinition, ProviderEvent, ProviderModelInfo, ProviderReadiness,
    ProviderReadinessCheck, ProviderStreamItem, ProviderTurn, ProviderUsage,
    QuarantinedEffectResult,
};
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, QuarantinedEffectObserver,
    StreamingEffectExecutor,
};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::net::lookup_host;

const MAX_PROVIDER_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_PROVIDER_ADDRESSES: usize = 16;

/// Supported first-party provider adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Deterministic credential-free smoke provider.
    Echo,
    /// OpenAI Responses API.
    OpenAiResponses,
    /// OpenAI-compatible Chat Completions API.
    OpenAiCompatible,
}

impl ProviderKind {
    /// Stable adapter label used in normalized results.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "echo",
            Self::OpenAiResponses => "openai_responses",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Exact effect action for a generation request.
    pub fn generation_action(self) -> &'static str {
        match self {
            Self::Echo => "provider.echo",
            Self::OpenAiResponses => "provider.openai.responses",
            Self::OpenAiCompatible => "provider.openai.chat",
        }
    }
}

/// Strict normalized provider profile composed by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderProfile {
    /// Stable profile name.
    pub name: String,
    /// Adapter kind.
    pub kind: ProviderKind,
    /// Default model identifier.
    pub model: String,
    /// Base URL ending at the API version prefix, without a trailing slash.
    pub base_url: Option<String>,
    /// Credential reference such as `env:OPENAI_API_KEY`.
    pub credential_reference: Option<String>,
    /// Adapter transport timeout.
    pub timeout_ms: u64,
}

impl ProviderProfile {
    /// Validate and construct a profile without resolving credentials.
    pub fn new(
        name: impl Into<String>,
        kind: ProviderKind,
        model: impl Into<String>,
        base_url: Option<String>,
        credential_reference: Option<String>,
        timeout_ms: u64,
    ) -> Result<Self, ProviderError> {
        let name = name.into();
        let model = model.into();
        if name.is_empty() || model.is_empty() || timeout_ms == 0 {
            return Err(ProviderError::Configuration(
                "provider name, model, and timeout must be nonempty/nonzero".into(),
            ));
        }
        if let Some(reference) = credential_reference.as_deref()
            && !valid_credential_reference(reference)
        {
            return Err(ProviderError::Configuration(
                "provider credentials must use an env:VARIABLE reference".into(),
            ));
        }
        let base_url = match kind {
            ProviderKind::Echo => {
                if base_url.is_some() || credential_reference.is_some() {
                    return Err(ProviderError::Configuration(
                        "echo profiles cannot configure network or credentials".into(),
                    ));
                }
                None
            }
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCompatible => {
                let raw = base_url.ok_or_else(|| {
                    ProviderError::Configuration("network providers require baseUrl".into())
                })?;
                Some(normalize_base_url(&raw)?)
            }
        };
        if kind == ProviderKind::OpenAiResponses && credential_reference.is_none() {
            return Err(ProviderError::Configuration(
                "OpenAI Responses profiles require a credential reference".into(),
            ));
        }
        Ok(Self {
            name,
            kind,
            model,
            base_url,
            credential_reference,
            timeout_ms,
        })
    }

    /// Exact endpoint for generation.
    pub fn generation_endpoint(&self) -> Result<String, ProviderError> {
        match self.kind {
            ProviderKind::Echo => Ok(format!("provider:{}", self.name)),
            ProviderKind::OpenAiResponses => self.endpoint("responses"),
            ProviderKind::OpenAiCompatible => self.endpoint("chat/completions"),
        }
    }

    /// Exact endpoint for model catalog diagnostics.
    pub fn models_endpoint(&self) -> Result<Option<String>, ProviderError> {
        match self.kind {
            ProviderKind::Echo => Ok(None),
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCompatible => {
                self.endpoint("models").map(Some)
            }
        }
    }

    /// Canonical network origin for policy obligations.
    pub fn network_origin(&self) -> Result<Option<String>, ProviderError> {
        self.base_url
            .as_ref()
            .map(|base| {
                Url::parse(base)
                    .map(|url| url.origin().ascii_serialization())
                    .map_err(ProviderError::from)
            })
            .transpose()
    }

    fn endpoint(&self, suffix: &str) -> Result<String, ProviderError> {
        self.base_url
            .as_ref()
            .map(|base| format!("{base}/{suffix}"))
            .ok_or_else(|| ProviderError::Configuration("provider has no base URL".into()))
    }
}

/// Strict content placed inside a provider effect request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderEffectInput {
    /// Profile selected by routing.
    pub profile: String,
    /// Full logical model request. Absent only for model-catalog diagnostics.
    pub request: Option<ModelRequest>,
}

/// Provider configuration, transport, credential, or normalization failure.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// Strict profile configuration failed.
    #[error("provider configuration error: {0}")]
    Configuration(String),
    /// Credential reference could not be resolved.
    #[error("provider credential unavailable: {0}")]
    Credential(String),
    /// Endpoint was unreachable or timed out.
    #[error("provider transport failure: {0}")]
    Transport(String),
    /// Endpoint returned a non-success status.
    #[error("provider endpoint returned HTTP {status}")]
    Status {
        /// HTTP status code only; response bodies are never included.
        status: u16,
    },
    /// Provider response failed the normalized contract.
    #[error("malformed provider output: {0}")]
    Malformed(String),
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error.to_string())
    }
}

impl From<url::ParseError> for ProviderError {
    fn from(error: url::ParseError) -> Self {
        Self::Configuration(error.to_string())
    }
}

/// Resolves a credential only after the gateway has supplied a permit.
pub trait CredentialResolver: Send + Sync {
    /// Resolve a configured reference. Implementations must not log the returned value.
    fn resolve(&self, reference: &str) -> Result<String, ProviderError>;
}

/// Environment-only credential resolver for the first Rust provider milestone.
#[derive(Default)]
pub struct EnvironmentCredentialResolver;

impl CredentialResolver for EnvironmentCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, ProviderError> {
        let variable = reference.strip_prefix("env:").ok_or_else(|| {
            ProviderError::Credential("credential reference is not environment-backed".into())
        })?;
        std::env::var(variable).map_err(|_| {
            ProviderError::Credential(format!("environment variable {variable} is unset"))
        })
    }
}

/// One permit-bound provider adapter instance.
pub struct ProviderExecutor {
    profile: ProviderProfile,
    credentials: Arc<dyn CredentialResolver>,
}

impl ProviderExecutor {
    /// Construct an adapter using environment credential references.
    pub fn new(profile: ProviderProfile) -> Self {
        Self::with_credentials(profile, Arc::new(EnvironmentCredentialResolver))
    }

    /// Construct an adapter with an injected credential resolver.
    pub fn with_credentials(
        profile: ProviderProfile,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            profile,
            credentials,
        }
    }

    /// Profile metadata without credentials.
    pub fn profile(&self) -> &ProviderProfile {
        &self.profile
    }

    /// Credential reference suitable for policy input.
    pub fn credential_reference(&self) -> Option<CredentialReference> {
        self.profile
            .credential_reference
            .as_ref()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
    }

    /// Static capability/readiness shape without making an effectful call.
    pub fn static_readiness(&self) -> ProviderReadiness {
        let echo = self.profile.kind == ProviderKind::Echo;
        ProviderReadiness {
            profile: self.profile.name.clone(),
            provider: self.profile.kind.as_str().into(),
            ready: echo,
            tool_calls: !echo,
            streaming: true,
            checks: vec![ProviderReadinessCheck {
                name: if echo { "offline" } else { "models_endpoint" }.into(),
                status: if echo { "pass" } else { "not_checked" }.into(),
                detail: if echo {
                    "Credential-free deterministic provider is ready.".into()
                } else {
                    "Run provider doctor through the effect gateway.".into()
                },
            }],
        }
    }
}

#[async_trait]
impl EffectExecutor for ProviderExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.execute_permitted(request, &permit)
            .await
            .map_err(provider_execution_error)
    }
}

fn provider_execution_error(error: ProviderError) -> ExecutionError {
    match error {
        ProviderError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
            "provider transport failed after execution began; outcome is unknown: {message}"
        )),
        ProviderError::Malformed(message) if invalid_tool_argument_message(&message) => {
            ExecutionError::Recoverable {
                code: "provider.invalid_tool_arguments".into(),
                message,
            }
        }
        error => ExecutionError::Failed(error.to_string()),
    }
}

#[async_trait]
impl StreamingEffectExecutor for ProviderExecutor {
    async fn execute_stream(
        &self,
        effect: &EffectRequest,
        permit: ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let input: ProviderEffectInput =
            serde_json::from_value(effect.content.clone()).map_err(|error| {
                provider_execution_error(ProviderError::Malformed(error.to_string()))
            })?;
        if input.profile != self.profile.name {
            return Err(provider_execution_error(ProviderError::Configuration(
                "provider effect profile does not match its adapter".into(),
            )));
        }
        validate_credential_disclosure(effect, &self.profile).map_err(provider_execution_error)?;
        if effect.action != self.profile.kind.generation_action() {
            return Err(provider_execution_error(ProviderError::Configuration(
                "streaming provider adapter received an unsupported action".into(),
            )));
        }
        let model_request = input.request.ok_or_else(|| {
            provider_execution_error(ProviderError::Configuration(
                "provider generation request is absent".into(),
            ))
        })?;
        validate_model_request(&model_request, &self.profile).map_err(provider_execution_error)?;
        let endpoint = self
            .profile
            .generation_endpoint()
            .map_err(provider_execution_error)?;
        if self.profile.kind == ProviderKind::Echo {
            if effect.resource != endpoint {
                return Err(provider_execution_error(ProviderError::Configuration(
                    "echo resource does not match the selected profile".into(),
                )));
            }
            let text = model_request
                .messages
                .last()
                .map(|message| message.content.clone())
                .ok_or_else(|| {
                    provider_execution_error(ProviderError::Malformed(
                        "echo request has no message".into(),
                    ))
                })?;
            emit_stream_item(
                ProviderStreamItem::Event {
                    event: ProviderEvent::ModelDelta { text: text.clone() },
                },
                &permit,
                observer,
            )
            .await?;
            emit_stream_item(
                ProviderStreamItem::Event {
                    event: ProviderEvent::FinalOutput { text },
                },
                &permit,
                observer,
            )
            .await?;
            return emit_stream_item(
                ProviderStreamItem::Completed {
                    profile: self.profile.name.clone(),
                    provider: self.profile.kind.as_str().into(),
                    model: self.profile.model.clone(),
                    response_id: None,
                },
                &permit,
                observer,
            )
            .await;
        }
        self.validate_resource(effect, &endpoint, &permit)
            .map_err(provider_execution_error)?;
        let payload = match self.profile.kind {
            ProviderKind::OpenAiResponses => responses_payload(&model_request, true),
            ProviderKind::OpenAiCompatible => chat_payload(&model_request, true),
            ProviderKind::Echo => unreachable!("handled above"),
        }
        .map_err(provider_execution_error)?;
        self.stream_generation(&endpoint, payload, &permit, observer)
            .await
    }
}

async fn emit_stream_item(
    item: ProviderStreamItem,
    permit: &ExecutionPermit,
    observer: &mut dyn QuarantinedEffectObserver,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let bytes =
        serde_json::to_vec(&item).map_err(|error| ExecutionError::Failed(error.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > permit.obligations().max_output_bytes {
        return Err(ExecutionError::Failed(
            "normalized provider stream item exceeds the permitted bound".into(),
        ));
    }
    let result = QuarantinedEffectResult {
        media_type: "application/vnd.colossus.provider-stream+json".into(),
        bytes,
        effect_succeeded: true,
    };
    observer.observe(result.clone()).await?;
    Ok(result)
}

impl ProviderExecutor {
    async fn execute_permitted(
        &self,
        effect: &EffectRequest,
        permit: &ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ProviderError> {
        let input: ProviderEffectInput = serde_json::from_value(effect.content.clone())
            .map_err(|error| ProviderError::Malformed(error.to_string()))?;
        if input.profile != self.profile.name {
            return Err(ProviderError::Configuration(
                "provider effect profile does not match its adapter".into(),
            ));
        }
        validate_credential_disclosure(effect, &self.profile)?;
        if effect.action == "provider.models" {
            if input.request.is_some() || self.profile.kind == ProviderKind::Echo {
                return Err(ProviderError::Configuration(
                    "model catalog effect is invalid for this provider".into(),
                ));
            }
            let endpoint = self
                .profile
                .models_endpoint()?
                .ok_or_else(|| ProviderError::Configuration("provider has no catalog".into()))?;
            self.validate_resource(effect, &endpoint, permit)?;
            let bytes = self.request_json(&endpoint, None, permit).await?;
            let models = normalize_models(&bytes)?;
            return bounded_result(&models, permit);
        }
        if effect.action != self.profile.kind.generation_action() {
            return Err(ProviderError::Configuration(
                "provider adapter received an unsupported action".into(),
            ));
        }
        let model_request = input.request.ok_or_else(|| {
            ProviderError::Configuration("provider generation request is absent".into())
        })?;
        validate_model_request(&model_request, &self.profile)?;
        let endpoint = self.profile.generation_endpoint()?;
        if self.profile.kind == ProviderKind::Echo {
            if effect.resource != endpoint {
                return Err(ProviderError::Configuration(
                    "echo resource does not match the selected profile".into(),
                ));
            }
            let text = model_request
                .messages
                .last()
                .map(|message| message.content.clone())
                .ok_or_else(|| ProviderError::Malformed("echo request has no message".into()))?;
            return bounded_result(
                &ProviderTurn {
                    profile: self.profile.name.clone(),
                    provider: self.profile.kind.as_str().into(),
                    model: self.profile.model.clone(),
                    response_id: None,
                    events: vec![
                        ProviderEvent::ModelDelta { text: text.clone() },
                        ProviderEvent::FinalOutput { text },
                    ],
                },
                permit,
            );
        }
        self.validate_resource(effect, &endpoint, permit)?;
        let payload = match self.profile.kind {
            ProviderKind::OpenAiResponses => responses_payload(&model_request, false),
            ProviderKind::OpenAiCompatible => chat_payload(&model_request, false),
            ProviderKind::Echo => unreachable!("handled above"),
        }?;
        let bytes = self.request_json(&endpoint, Some(payload), permit).await?;
        let turn = match self.profile.kind {
            ProviderKind::OpenAiResponses => normalize_responses(&self.profile, &bytes),
            ProviderKind::OpenAiCompatible => normalize_chat(&self.profile, &bytes),
            ProviderKind::Echo => unreachable!("handled above"),
        }?;
        bounded_result(&turn, permit)
    }

    fn validate_resource(
        &self,
        effect: &EffectRequest,
        endpoint: &str,
        permit: &ExecutionPermit,
    ) -> Result<(), ProviderError> {
        if effect.resource != endpoint {
            return Err(ProviderError::Configuration(
                "provider effect endpoint does not match its configured profile".into(),
            ));
        }
        let origin = self
            .profile
            .network_origin()?
            .ok_or_else(|| ProviderError::Configuration("network provider has no origin".into()))?;
        if !permit.obligations().network_destinations.contains(&origin) {
            return Err(ProviderError::Configuration(
                "provider origin is absent from permit obligations".into(),
            ));
        }
        Ok(())
    }

    async fn request_json(
        &self,
        endpoint: &str,
        payload: Option<Value>,
        permit: &ExecutionPermit,
    ) -> Result<Vec<u8>, ProviderError> {
        let (response, secret) = self.send_request(endpoint, payload, permit).await?;
        let limit = usize::try_from(permit.obligations().max_output_bytes)
            .map_err(|error| ProviderError::Configuration(error.to_string()))?;
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(ProviderError::Malformed(
                    "provider response exceeds the permitted output bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        redact_exact_bytes(&mut bytes, secret.as_deref());
        Ok(bytes)
    }

    async fn send_request(
        &self,
        endpoint: &str,
        payload: Option<Value>,
        permit: &ExecutionPermit,
    ) -> Result<(reqwest::Response, Option<String>), ProviderError> {
        let url = Url::parse(endpoint)?;
        let host = url
            .host_str()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no port".into()))?;
        let addresses = resolve_provider_addresses(host, port).await?;
        let timeout_ms = self.profile.timeout_ms.min(permit.obligations().timeout_ms);
        let client = Client::builder()
            .no_proxy()
            .redirect(RedirectPolicy::none())
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_millis(timeout_ms))
            .build()?;
        let mut builder = payload
            .as_ref()
            .map_or_else(|| client.get(url.clone()), |_| client.post(url.clone()));
        let secret = if let Some(reference) = self.profile.credential_reference.as_deref() {
            let secret = self.credentials.resolve(reference)?;
            if secret.is_empty() {
                return Err(ProviderError::Credential(
                    "resolved provider credential is empty".into(),
                ));
            }
            builder = builder.bearer_auth(&secret);
            Some(secret)
        } else {
            None
        };
        if let Some(payload) = payload {
            let body = serde_json::to_vec(&payload)
                .map_err(|error| ProviderError::Malformed(error.to_string()))?;
            if body.len() > MAX_PROVIDER_REQUEST_BYTES {
                return Err(ProviderError::Configuration(
                    "serialized provider request exceeds 1 MiB".into(),
                ));
            }
            builder = builder
                .header("content-type", "application/json")
                .body(body);
        }
        let response = builder.send().await?;
        if !response.status().is_success() {
            return Err(ProviderError::Status {
                status: response.status().as_u16(),
            });
        }
        Ok((response, secret))
    }

    async fn stream_generation(
        &self,
        endpoint: &str,
        payload: Value,
        permit: &ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let (response, secret) = self
            .send_request(endpoint, Some(payload), permit)
            .await
            .map_err(provider_execution_error)?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
        {
            return Err(provider_execution_error(ProviderError::Malformed(
                "streaming provider response is not text/event-stream".into(),
            )));
        }
        let limit = usize::try_from(permit.obligations().max_output_bytes)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let mut decoder = SseDecoder::default();
        let mut state = ProviderStreamState::new(self.profile.kind);
        let mut raw_bytes = 0_usize;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                provider_execution_error(ProviderError::Transport(error.to_string()))
            })?;
            raw_bytes = raw_bytes.saturating_add(chunk.len());
            if raw_bytes > limit {
                return Err(provider_execution_error(ProviderError::Malformed(
                    "raw provider event stream exceeds the permitted output bound".into(),
                )));
            }
            for data in decoder.feed(&chunk).map_err(provider_execution_error)? {
                let mut data = data;
                redact_exact_bytes(&mut data, secret.as_deref());
                if data == b"[DONE]" {
                    state.mark_done();
                    continue;
                }
                let mut value: Value = serde_json::from_slice(&data).map_err(|error| {
                    provider_execution_error(ProviderError::Malformed(format!(
                        "provider SSE data is not valid JSON: {error}"
                    )))
                })?;
                redact_value_exact(&mut value, secret.as_deref());
                for event in state.ingest(value).map_err(provider_execution_error)? {
                    emit_stream_item(ProviderStreamItem::Event { event }, permit, observer).await?;
                }
            }
        }
        decoder.finish().map_err(provider_execution_error)?;
        for event in state.finish().map_err(provider_execution_error)? {
            emit_stream_item(ProviderStreamItem::Event { event }, permit, observer).await?;
        }
        let response_id = state.response_id().map(str::to_owned);
        emit_stream_item(
            ProviderStreamItem::Completed {
                profile: self.profile.name.clone(),
                provider: self.profile.kind.as_str().into(),
                model: self.profile.model.clone(),
                response_id,
            },
            permit,
            observer,
        )
        .await
    }
}

fn redact_exact_bytes(bytes: &mut Vec<u8>, secret: Option<&str>) {
    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return;
    };
    let needle = secret.as_bytes();
    let replacement = b"[REDACTED]";
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(needle) {
            output.extend_from_slice(replacement);
            cursor = cursor.saturating_add(needle.len());
        } else {
            output.push(bytes[cursor]);
            cursor = cursor.saturating_add(1);
        }
    }
    *bytes = output;
}

fn redact_value_exact(value: &mut Value, secret: Option<&str>) {
    let Some(secret) = secret.filter(|secret| !secret.is_empty()) else {
        return;
    };
    match value {
        Value::String(text) => {
            if text.contains(secret) {
                *text = text.replace(secret, "[REDACTED]");
            }
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_value_exact(value, Some(secret))),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| redact_value_exact(value, Some(secret))),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<Vec<u8>>,
}

impl SseDecoder {
    fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, ProviderError> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_PROVIDER_REQUEST_BYTES {
            return Err(ProviderError::Malformed(
                "provider SSE frame exceeds 1 MiB".into(),
            ));
        }
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=end).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(&line, &mut events)?;
        }
        Ok(events)
    }

    fn process_line(
        &mut self,
        line: &[u8],
        events: &mut Vec<Vec<u8>>,
    ) -> Result<(), ProviderError> {
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                let size = self
                    .data_lines
                    .iter()
                    .map(Vec::len)
                    .sum::<usize>()
                    .saturating_add(self.data_lines.len().saturating_sub(1));
                if size > MAX_PROVIDER_REQUEST_BYTES {
                    return Err(ProviderError::Malformed(
                        "provider SSE data exceeds 1 MiB".into(),
                    ));
                }
                let mut data = Vec::with_capacity(size);
                for (index, line) in self.data_lines.drain(..).enumerate() {
                    if index > 0 {
                        data.push(b'\n');
                    }
                    data.extend_from_slice(&line);
                }
                events.push(data);
            }
            return Ok(());
        }
        if line.starts_with(b":") {
            return Ok(());
        }
        let (field, mut value) =
            line.iter()
                .position(|byte| *byte == b':')
                .map_or((line, &[][..]), |index| {
                    let (field, value) = line.split_at(index);
                    (field, &value[1..])
                });
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        if field == b"data" {
            self.data_lines.push(value.to_vec());
        }
        Ok(())
    }

    fn finish(self) -> Result<(), ProviderError> {
        if self.buffer.iter().any(|byte| !byte.is_ascii_whitespace()) || !self.data_lines.is_empty()
        {
            return Err(ProviderError::Transport(
                "provider event stream ended inside an SSE frame".into(),
            ));
        }
        Ok(())
    }
}

enum ProviderStreamState {
    Responses(ResponsesStreamState),
    Chat(ChatStreamState),
}

impl ProviderStreamState {
    fn new(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::OpenAiResponses => Self::Responses(ResponsesStreamState::default()),
            ProviderKind::OpenAiCompatible => Self::Chat(ChatStreamState::default()),
            ProviderKind::Echo => unreachable!("echo streaming is handled without SSE"),
        }
    }

    fn mark_done(&mut self) {
        match self {
            Self::Responses(state) => state.done_marker = true,
            Self::Chat(state) => state.done_marker = true,
        }
    }

    fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        match self {
            Self::Responses(state) => state.ingest(value),
            Self::Chat(state) => state.ingest(value),
        }
    }

    fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        match self {
            Self::Responses(state) => state.finish(),
            Self::Chat(state) => state.finish(),
        }
    }

    fn response_id(&self) -> Option<&str> {
        match self {
            Self::Responses(state) => state.response_id.as_deref(),
            Self::Chat(state) => state.response_id.as_deref(),
        }
    }
}

#[derive(Default)]
struct ResponsesStreamState {
    response_id: Option<String>,
    text: String,
    tool_call_ids: BTreeSet<String>,
    completed: bool,
    done_marker: bool,
}

impl ResponsesStreamState {
    fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        let object = value.as_object().ok_or_else(|| {
            ProviderError::Malformed("Responses stream event is not an object".into())
        })?;
        let event_type = required_string(object, "type")?;
        match event_type.as_str() {
            "response.created" | "response.in_progress" => {
                if let Some(response) = object.get("response").and_then(Value::as_object) {
                    self.capture_response_id(response.get("id"))?;
                }
                Ok(Vec::new())
            }
            "response.output_text.delta" => {
                let delta = required_string(object, "delta")?;
                self.text.push_str(&delta);
                Ok(vec![ProviderEvent::ModelDelta { text: delta }])
            }
            "response.reasoning_summary_text.done" => {
                let summary = required_string(object, "text")?;
                Ok(vec![ProviderEvent::ReasoningSummary { summary }])
            }
            "response.output_item.done" => {
                let Some(item) = object.get("item").and_then(Value::as_object) else {
                    return Err(ProviderError::Malformed(
                        "Responses output_item.done has no item object".into(),
                    ));
                };
                self.tool_event(item)
                    .map(|event| event.into_iter().collect())
            }
            "response.completed" => self.complete(object),
            "response.failed" | "response.incomplete" | "error" => Err(ProviderError::Malformed(
                format!("provider stream terminated with {event_type}"),
            )),
            _ => Ok(Vec::new()),
        }
    }

    fn capture_response_id(&mut self, value: Option<&Value>) -> Result<(), ProviderError> {
        let Some(id) = value.and_then(Value::as_str).filter(|id| !id.is_empty()) else {
            return Ok(());
        };
        if self
            .response_id
            .as_deref()
            .is_some_and(|current| current != id)
        {
            return Err(ProviderError::Malformed(
                "provider stream changed response id".into(),
            ));
        }
        self.response_id = Some(id.into());
        Ok(())
    }

    fn tool_event(
        &mut self,
        item: &Map<String, Value>,
    ) -> Result<Option<ProviderEvent>, ProviderError> {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let event = function_call_event(
                    item.get("call_id"),
                    item.get("name"),
                    item.get("arguments"),
                )?;
                let ProviderEvent::ToolCallRequested { call_id, .. } = &event else {
                    unreachable!("function call normalization returned another event")
                };
                if self.tool_call_ids.insert(call_id.clone()) {
                    Ok(Some(event))
                } else {
                    Ok(None)
                }
            }
            Some("custom_tool_call") => {
                let call_id = required_string(item, "call_id")?;
                if !self.tool_call_ids.insert(call_id.clone()) {
                    return Ok(None);
                }
                Ok(Some(ProviderEvent::ToolCallRequested {
                    call_id,
                    name: required_string(item, "name")?,
                    arguments: json!({
                        "input": item.get("input").and_then(Value::as_str).unwrap_or_default()
                    }),
                }))
            }
            _ => Ok(None),
        }
    }

    fn complete(
        &mut self,
        event: &Map<String, Value>,
    ) -> Result<Vec<ProviderEvent>, ProviderError> {
        if self.completed {
            return Err(ProviderError::Malformed(
                "provider emitted response.completed more than once".into(),
            ));
        }
        let response = event
            .get("response")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ProviderError::Malformed("response.completed has no response object".into())
            })?;
        if response.get("status").and_then(Value::as_str) != Some("completed") {
            return Err(ProviderError::Malformed(
                "response.completed does not carry completed status".into(),
            ));
        }
        self.capture_response_id(response.get("id"))?;
        let mut events = Vec::new();
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            for item in output.iter().filter_map(Value::as_object) {
                if let Some(event) = self.tool_event(item)? {
                    events.push(event);
                }
            }
        }
        if self.tool_call_ids.is_empty() && !self.text.is_empty() {
            events.push(ProviderEvent::FinalOutput {
                text: self.text.clone(),
            });
        }
        if let Some(usage) = normalize_usage(response.get("usage"), UsageShape::Responses)? {
            events.push(ProviderEvent::Usage { usage });
        }
        self.completed = true;
        Ok(events)
    }

    fn finish(&self) -> Result<Vec<ProviderEvent>, ProviderError> {
        if !self.completed || self.response_id.is_none() {
            return Err(ProviderError::Transport(
                "Responses stream ended before response.completed".into(),
            ));
        }
        let _ = self.done_marker;
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct ChatStreamState {
    response_id: Option<String>,
    text: String,
    tool_calls: BTreeMap<u64, PartialChatToolCall>,
    terminal_seen: bool,
    done_marker: bool,
    finalized: bool,
    usage_seen: bool,
}

#[derive(Default)]
struct PartialChatToolCall {
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ChatStreamState {
    fn ingest(&mut self, value: Value) -> Result<Vec<ProviderEvent>, ProviderError> {
        let object = value
            .as_object()
            .ok_or_else(|| ProviderError::Malformed("chat stream chunk is not an object".into()))?;
        if let Some(id) = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            if self
                .response_id
                .as_deref()
                .is_some_and(|current| current != id)
            {
                return Err(ProviderError::Malformed(
                    "chat stream changed response id".into(),
                ));
            }
            self.response_id = Some(id.into());
        }
        let mut events = Vec::new();
        if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Chat)? {
            if self.usage_seen {
                return Err(ProviderError::Malformed(
                    "chat stream emitted usage more than once".into(),
                ));
            }
            self.usage_seen = true;
            events.push(ProviderEvent::Usage { usage });
        }
        let choices = object
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Malformed("chat stream has no choices array".into()))?;
        for choice in choices {
            let choice = choice.as_object().ok_or_else(|| {
                ProviderError::Malformed("chat stream choice is not an object".into())
            })?;
            if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
                return Err(ProviderError::Malformed(
                    "chat stream returned an unexpected choice index".into(),
                ));
            }
            let delta = choice
                .get("delta")
                .and_then(Value::as_object)
                .ok_or_else(|| ProviderError::Malformed("chat choice has no delta".into()))?;
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                self.text.push_str(text);
                events.push(ProviderEvent::ModelDelta { text: text.into() });
            }
            self.ingest_tool_deltas(delta.get("tool_calls"))?;
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                match reason {
                    "stop" | "tool_calls" | "function_call" => self.terminal_seen = true,
                    "length" | "content_filter" => {
                        return Err(ProviderError::Malformed(format!(
                            "chat stream terminated with finish_reason={reason}"
                        )));
                    }
                    other => {
                        return Err(ProviderError::Malformed(format!(
                            "chat stream returned unknown finish_reason={other}"
                        )));
                    }
                }
            }
        }
        Ok(events)
    }

    fn ingest_tool_deltas(&mut self, value: Option<&Value>) -> Result<(), ProviderError> {
        let Some(calls) = value.and_then(Value::as_array) else {
            return Ok(());
        };
        for call in calls {
            let call = call.as_object().ok_or_else(|| {
                ProviderError::Malformed("chat tool delta is not an object".into())
            })?;
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| ProviderError::Malformed("chat tool delta has no index".into()))?;
            let partial = self.tool_calls.entry(index).or_default();
            set_partial_string(&mut partial.call_id, call.get("id"), "tool call id")?;
            if let Some(function) = call.get("function").and_then(Value::as_object) {
                set_partial_string(&mut partial.name, function.get("name"), "tool call name")?;
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    partial.arguments.push_str(arguments);
                    if partial.arguments.len() > MAX_PROVIDER_REQUEST_BYTES {
                        return Err(ProviderError::Malformed(
                            "streamed tool arguments exceed 1 MiB".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        if self.finalized {
            return Err(ProviderError::Malformed(
                "chat stream was finalized more than once".into(),
            ));
        }
        if !self.terminal_seen || self.response_id.is_none() {
            return Err(ProviderError::Transport(
                "chat stream ended before a terminal choice".into(),
            ));
        }
        let _ = self.done_marker;
        let mut events = Vec::new();
        if self.tool_calls.is_empty() {
            if self.text.is_empty() {
                return Err(ProviderError::Malformed(
                    "chat stream completed without visible text or tool calls".into(),
                ));
            }
            events.push(ProviderEvent::FinalOutput {
                text: self.text.clone(),
            });
        } else {
            for (expected, (index, partial)) in self.tool_calls.iter().enumerate() {
                if *index != expected as u64 {
                    return Err(ProviderError::Malformed(
                        "chat stream tool indexes are not contiguous".into(),
                    ));
                }
                let call_id = partial.call_id.clone().ok_or_else(|| {
                    ProviderError::Malformed("streamed tool call id is absent".into())
                })?;
                let name = partial.name.clone().ok_or_else(|| {
                    ProviderError::Malformed("streamed tool call name is absent".into())
                })?;
                let arguments_text = if partial.arguments.is_empty() {
                    "{}"
                } else {
                    &partial.arguments
                };
                let arguments: Value = serde_json::from_str(arguments_text).map_err(|error| {
                    ProviderError::Malformed(format!(
                        "tool call arguments are invalid JSON; call_id={call_id} tool={name} position={}",
                        error.column()
                    ))
                })?;
                if !arguments.is_object() {
                    return Err(ProviderError::Malformed(format!(
                        "tool call arguments are not an object; call_id={call_id} tool={name}"
                    )));
                }
                events.push(ProviderEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments,
                });
            }
        }
        self.finalized = true;
        Ok(events)
    }
}

fn set_partial_string(
    target: &mut Option<String>,
    value: Option<&Value>,
    label: &str,
) -> Result<(), ProviderError> {
    let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    if target.as_deref().is_some_and(|current| current != value) {
        return Err(ProviderError::Malformed(format!(
            "streamed {label} changed during assembly"
        )));
    }
    *target = Some(value.into());
    Ok(())
}

/// Role-to-profile routing and permit-bound adapters.
pub struct ProviderRegistry {
    profiles: BTreeMap<String, Arc<ProviderExecutor>>,
    roles: BTreeMap<String, String>,
}

impl ProviderRegistry {
    /// Validate unique profiles and role targets.
    pub fn new(
        profiles: Vec<ProviderExecutor>,
        roles: BTreeMap<String, String>,
    ) -> Result<Self, ProviderError> {
        let mut indexed = BTreeMap::new();
        for provider in profiles {
            let name = provider.profile.name.clone();
            if indexed.insert(name.clone(), Arc::new(provider)).is_some() {
                return Err(ProviderError::Configuration(format!(
                    "duplicate provider profile {name}"
                )));
            }
        }
        if indexed.is_empty() || !roles.contains_key("primary") {
            return Err(ProviderError::Configuration(
                "provider profiles and the primary role are required".into(),
            ));
        }
        for (role, profile) in &roles {
            if role.is_empty() || !indexed.contains_key(profile) {
                return Err(ProviderError::Configuration(format!(
                    "provider role {role} references unknown profile {profile}"
                )));
            }
        }
        Ok(Self {
            profiles: indexed,
            roles,
        })
    }

    /// Resolve a role, falling back to `primary` for an unconfigured specialized role.
    pub fn resolve(&self, role: &str) -> Result<Arc<ProviderExecutor>, ProviderError> {
        let profile = self
            .roles
            .get(role)
            .or_else(|| self.roles.get("primary"))
            .ok_or_else(|| ProviderError::Configuration("primary role is absent".into()))?;
        self.profiles.get(profile).cloned().ok_or_else(|| {
            ProviderError::Configuration(format!("provider profile {profile} is absent"))
        })
    }

    /// Resolve an exact profile without role fallback.
    pub fn profile(&self, profile: &str) -> Result<Arc<ProviderExecutor>, ProviderError> {
        self.profiles.get(profile).cloned().ok_or_else(|| {
            ProviderError::Configuration(format!("provider profile {profile} is absent"))
        })
    }

    /// Stable role mapping for diagnostics.
    pub fn routes(&self) -> &BTreeMap<String, String> {
        &self.roles
    }

    /// Sorted profile readiness without making network calls.
    pub fn profiles(&self) -> Vec<ProviderReadiness> {
        self.profiles
            .values()
            .map(|provider| provider.static_readiness())
            .collect()
    }
}

fn normalize_base_url(raw: &str) -> Result<String, ProviderError> {
    let url = Url::parse(raw)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::Configuration(
            "provider baseUrl requires HTTP(S), a host, and no credentials/query/fragment".into(),
        ));
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !loopback {
        return Err(ProviderError::Configuration(
            "non-loopback provider baseUrl requires HTTPS".into(),
        ));
    }
    Ok(raw.trim_end_matches('/').to_owned())
}

fn valid_credential_reference(reference: &str) -> bool {
    let Some(variable) = reference.strip_prefix("env:") else {
        return false;
    };
    let mut bytes = variable.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn validate_credential_disclosure(
    effect: &EffectRequest,
    profile: &ProviderProfile,
) -> Result<(), ProviderError> {
    let expected = profile.credential_reference.as_deref();
    let disclosed = effect
        .credential_references
        .iter()
        .map(|reference| reference.reference.as_str())
        .collect::<Vec<_>>();
    match expected {
        Some(expected) if disclosed == [expected] => Ok(()),
        None if disclosed.is_empty() => Ok(()),
        _ => Err(ProviderError::Configuration(
            "provider credential disclosure does not match its configured reference".into(),
        )),
    }
}

fn validate_model_request(
    request: &ModelRequest,
    profile: &ProviderProfile,
) -> Result<(), ProviderError> {
    if request.model != profile.model
        || request.messages.is_empty()
        || request.messages.len() > 512
        || request.tools.len() > 128
        || request
            .messages
            .iter()
            .any(|message| message.content.len() > MAX_PROVIDER_REQUEST_BYTES)
    {
        return Err(ProviderError::Configuration(
            "provider request model, messages, or bounds are invalid".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for tool in &request.tools {
        if tool.name.is_empty()
            || tool.description.len() > 16 * 1024
            || !tool.input_schema.is_object()
            || !names.insert(tool.name.as_str())
        {
            return Err(ProviderError::Configuration(
                "provider tools require unique names and object schemas".into(),
            ));
        }
    }
    Ok(())
}

fn responses_payload(request: &ModelRequest, streaming: bool) -> Result<Value, ProviderError> {
    let mut input = Vec::new();
    for message in &request.messages {
        input.extend(responses_messages(message)?);
    }
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
                "strict": true,
            })
        })
        .collect::<Vec<_>>();
    let mut payload = json!({
        "model": request.model,
        "instructions": request.instructions,
        "input": input,
        "store": false,
        "stream": streaming,
    });
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    Ok(payload)
}

fn responses_messages(message: &ModelMessage) -> Result<Vec<Value>, ProviderError> {
    match message.role {
        ModelMessageRole::System => Ok(vec![
            json!({"role": "developer", "content": message.content}),
        ]),
        ModelMessageRole::User => Ok(vec![json!({"role": "user", "content": message.content})]),
        ModelMessageRole::Assistant => {
            let mut items = Vec::new();
            if !message.content.is_empty() {
                items.push(json!({"role": "assistant", "content": message.content}));
            }
            items.extend(message.tool_calls.iter().map(responses_tool_call));
            if items.is_empty() {
                return Err(ProviderError::Configuration(
                    "assistant continuation message is empty".into(),
                ));
            }
            Ok(items)
        }
        ModelMessageRole::Tool => {
            let call_id = message.tool_call_id.as_ref().ok_or_else(|| {
                ProviderError::Configuration("tool result message has no call id".into())
            })?;
            Ok(vec![json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.content,
            })])
        }
    }
}

fn responses_tool_call(call: &ModelToolCall) -> Value {
    json!({
        "type": "function_call",
        "call_id": call.call_id,
        "name": call.name,
        "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
    })
}

fn chat_payload(request: &ModelRequest, streaming: bool) -> Result<Value, ProviderError> {
    let mut messages = Vec::new();
    if !request.instructions.is_empty() {
        messages.push(json!({"role": "system", "content": request.instructions}));
    }
    messages.extend(
        request
            .messages
            .iter()
            .map(chat_message)
            .collect::<Result<Vec<_>, _>>()?,
    );
    let tools = request.tools.iter().map(chat_tool).collect::<Vec<_>>();
    let mut payload = json!({"model": request.model, "messages": messages, "stream": streaming});
    if streaming {
        payload["stream_options"] = json!({"include_usage": true});
    }
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    Ok(payload)
}

fn chat_message(message: &ModelMessage) -> Result<Value, ProviderError> {
    let role = match message.role {
        ModelMessageRole::System => "system",
        ModelMessageRole::User => "user",
        ModelMessageRole::Assistant => "assistant",
        ModelMessageRole::Tool => "tool",
    };
    let mut value = json!({"role": role, "content": message.content});
    if message.role == ModelMessageRole::Tool {
        value["tool_call_id"] =
            Value::String(message.tool_call_id.clone().ok_or_else(|| {
                ProviderError::Configuration("tool result has no call id".into())
            })?);
    }
    if message.role == ModelMessageRole::Assistant && !message.tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(message.tool_calls.iter().map(chat_tool_call).collect());
    }
    Ok(value)
}

fn chat_tool_call(call: &ModelToolCall) -> Value {
    json!({
        "id": call.call_id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
        }
    })
}

fn chat_tool(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

fn normalize_responses(
    profile: &ProviderProfile,
    bytes: &[u8],
) -> Result<ProviderTurn, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let object = data
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("Responses payload is not an object".into()))?;
    let output = object
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Malformed(response_shape(object, "output")))?;
    let mut events = Vec::new();
    let mut text = String::new();
    let mut tool_calls = 0_usize;
    for item in output {
        let Some(item) = item.as_object() else {
            continue;
        };
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                if let Some(summaries) = item.get("summary").and_then(Value::as_array) {
                    events.extend(summaries.iter().filter_map(|summary| {
                        let summary = summary.as_object()?;
                        (summary.get("type").and_then(Value::as_str) == Some("summary_text"))
                            .then(|| summary.get("text").and_then(Value::as_str))
                            .flatten()
                            .filter(|text| !text.is_empty())
                            .map(|summary| ProviderEvent::ReasoningSummary {
                                summary: summary.to_owned(),
                            })
                    }));
                }
            }
            Some("message") => {
                let chunk = content_text(item.get("content"));
                if !chunk.is_empty() {
                    text.push_str(&chunk);
                    events.push(ProviderEvent::ModelDelta { text: chunk });
                }
            }
            Some("function_call") => {
                tool_calls = tool_calls.saturating_add(1);
                events.push(function_call_event(
                    item.get("call_id"),
                    item.get("name"),
                    item.get("arguments"),
                )?);
            }
            Some("custom_tool_call") => {
                tool_calls = tool_calls.saturating_add(1);
                let call_id = required_string(item, "call_id")?;
                let name = required_string(item, "name")?;
                let input = item
                    .get("input")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                events.push(ProviderEvent::ToolCallRequested {
                    call_id,
                    name,
                    arguments: json!({"input": input}),
                });
            }
            _ => {}
        }
    }
    if !text.is_empty() && tool_calls == 0 {
        events.push(ProviderEvent::FinalOutput { text });
    }
    if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Responses)? {
        events.push(ProviderEvent::Usage { usage });
    }
    if events.is_empty() {
        return Err(ProviderError::Malformed(response_shape(object, "output")));
    }
    Ok(ProviderTurn {
        profile: profile.name.clone(),
        provider: profile.kind.as_str().into(),
        model: profile.model.clone(),
        response_id: object.get("id").and_then(Value::as_str).map(str::to_owned),
        events,
    })
}

fn normalize_chat(profile: &ProviderProfile, bytes: &[u8]) -> Result<ProviderTurn, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let object = data
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("chat payload is not an object".into()))?;
    let message = object
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
        .and_then(|choice| choice.get("message"))
        .and_then(Value::as_object)
        .ok_or_else(|| ProviderError::Malformed(response_shape(object, "choices")))?;
    let mut events = reasoning_summary_events(message);
    let mut tool_calls = 0_usize;
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let Some(call) = call.as_object() else {
                continue;
            };
            let function = call
                .get("function")
                .and_then(Value::as_object)
                .ok_or_else(|| ProviderError::Malformed("tool call has no function".into()))?;
            tool_calls = tool_calls.saturating_add(1);
            events.push(function_call_event(
                call.get("id"),
                function.get("name"),
                function.get("arguments"),
            )?);
        }
    }
    let text = content_text(message.get("content"));
    if !text.is_empty() {
        events.push(ProviderEvent::ModelDelta { text: text.clone() });
        if tool_calls == 0 {
            events.push(ProviderEvent::FinalOutput { text });
        }
    }
    if let Some(usage) = normalize_usage(object.get("usage"), UsageShape::Chat)? {
        events.push(ProviderEvent::Usage { usage });
    }
    if !events.iter().any(|event| {
        matches!(
            event,
            ProviderEvent::ModelDelta { .. } | ProviderEvent::ToolCallRequested { .. }
        )
    }) {
        return Err(ProviderError::Malformed(response_shape(object, "choices")));
    }
    Ok(ProviderTurn {
        profile: profile.name.clone(),
        provider: profile.kind.as_str().into(),
        model: profile.model.clone(),
        response_id: object.get("id").and_then(Value::as_str).map(str::to_owned),
        events,
    })
}

#[derive(Clone, Copy)]
enum UsageShape {
    Responses,
    Chat,
}

fn normalize_usage(
    value: Option<&Value>,
    shape: UsageShape,
) -> Result<Option<ProviderUsage>, ProviderError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| ProviderError::Malformed("provider usage is not an object".into()))?;
    let (input_name, output_name, input_details, output_details) = match shape {
        UsageShape::Responses => (
            "input_tokens",
            "output_tokens",
            "input_tokens_details",
            "output_tokens_details",
        ),
        UsageShape::Chat => (
            "prompt_tokens",
            "completion_tokens",
            "prompt_tokens_details",
            "completion_tokens_details",
        ),
    };
    let input_tokens = usage_u64(object, input_name)?;
    let output_tokens = usage_u64(object, output_name)?;
    let total_tokens = usage_u64(object, "total_tokens")?;
    let cached_input_tokens = usage_detail(object, input_details, "cached_tokens")?;
    let reasoning_tokens = usage_detail(object, output_details, "reasoning_tokens")?;
    if input_tokens.saturating_add(output_tokens) > total_tokens
        || cached_input_tokens.is_some_and(|tokens| tokens > input_tokens)
        || reasoning_tokens.is_some_and(|tokens| tokens > output_tokens)
    {
        return Err(ProviderError::Malformed(
            "provider usage totals or details are inconsistent".into(),
        ));
    }
    Ok(Some(ProviderUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        reasoning_tokens,
    }))
}

fn usage_u64(object: &Map<String, Value>, field: &str) -> Result<u64, ProviderError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ProviderError::Malformed(format!("provider usage has no {field}")))
}

fn usage_detail(
    object: &Map<String, Value>,
    details_field: &str,
    value_field: &str,
) -> Result<Option<u64>, ProviderError> {
    let Some(details) = object.get(details_field) else {
        return Ok(None);
    };
    if details.is_null() {
        return Ok(None);
    }
    let details = details.as_object().ok_or_else(|| {
        ProviderError::Malformed(format!("provider usage {details_field} is not an object"))
    })?;
    details
        .get(value_field)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                ProviderError::Malformed(format!(
                    "provider usage {details_field}.{value_field} is invalid"
                ))
            })
        })
        .transpose()
}

fn normalize_models(bytes: &[u8]) -> Result<Vec<ProviderModelInfo>, ProviderError> {
    let data: Value = serde_json::from_slice(bytes)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?;
    let models = data
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Malformed("models payload has no data array".into()))?;
    let mut output = models
        .iter()
        .filter_map(|model| {
            let model = model.as_object()?;
            Some(ProviderModelInfo {
                id: model.get("id")?.as_str()?.to_owned(),
                object: model
                    .get("object")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                owned_by: model
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| left.id.cmp(&right.id));
    if output.is_empty() {
        return Err(ProviderError::Malformed(
            "models payload contains no valid model records".into(),
        ));
    }
    Ok(output)
}

fn function_call_event(
    call_id: Option<&Value>,
    name: Option<&Value>,
    arguments: Option<&Value>,
) -> Result<ProviderEvent, ProviderError> {
    let call_id = call_id
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProviderError::Malformed("tool call id is absent".into()))?
        .to_owned();
    let name = name
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProviderError::Malformed("tool call name is absent".into()))?
        .to_owned();
    let arguments_text = arguments.and_then(Value::as_str).unwrap_or("{}");
    let arguments: Value = serde_json::from_str(arguments_text).map_err(|error| {
        ProviderError::Malformed(format!(
            "tool call arguments are invalid JSON; call_id={call_id} tool={name} position={}",
            error.column()
        ))
    })?;
    if !arguments.is_object() {
        return Err(ProviderError::Malformed(format!(
            "tool call arguments are not an object; call_id={call_id} tool={name}"
        )));
    }
    Ok(ProviderEvent::ToolCallRequested {
        call_id,
        name,
        arguments,
    })
}

fn invalid_tool_argument_message(message: &str) -> bool {
    message.starts_with("tool call arguments are invalid JSON")
        || message.starts_with("tool call arguments are not an object")
}

fn reasoning_summary_events(message: &Map<String, Value>) -> Vec<ProviderEvent> {
    message
        .get("reasoning_details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let item = item.as_object()?;
            if item.get("type").and_then(Value::as_str) != Some("reasoning.summary") {
                return None;
            }
            let summary = item
                .get("summary")
                .or_else(|| item.get("text"))
                .and_then(Value::as_str)?;
            (!summary.is_empty()).then(|| ProviderEvent::ReasoningSummary {
                summary: summary.to_owned(),
            })
        })
        .collect()
}

fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                let part = part.as_object()?;
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("text" | "output_text")
                )
                .then(|| part.get("text").and_then(Value::as_str))
                .flatten()
            })
            .collect(),
        _ => String::new(),
    }
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, ProviderError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Malformed(format!("provider output has no {field}")))
}

fn response_shape(object: &Map<String, Value>, expected: &str) -> String {
    let keys = object.keys().take(32).cloned().collect::<Vec<_>>();
    let value_type = object.get(expected).map_or("absent", value_type);
    format!("response_shape keys={keys:?} {expected}_type={value_type}")
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn bounded_result<T: Serialize>(
    value: &T,
    permit: &ExecutionPermit,
) -> Result<QuarantinedEffectResult, ProviderError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ProviderError::Malformed(error.to_string()))?;
    if u64::try_from(bytes.len()).map_err(|error| ProviderError::Malformed(error.to_string()))?
        > permit.obligations().max_output_bytes
    {
        return Err(ProviderError::Malformed(
            "normalized provider output exceeds the permitted bound".into(),
        ));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}

async fn resolve_provider_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, ProviderError> {
    let host_ip = host.parse::<IpAddr>().ok();
    let loopback_name = host.eq_ignore_ascii_case("localhost");
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?
        .filter(|address| host_ip.is_some() || loopback_name || !non_public_ip(address.ip()))
        .collect::<Vec<_>>();
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(MAX_PROVIDER_ADDRESSES);
    if addresses.is_empty() {
        return Err(ProviderError::Transport(
            "provider resolved to no permitted address".into(),
        ));
    }
    Ok(addresses)
}

fn non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

#[cfg(test)]
mod tests;
