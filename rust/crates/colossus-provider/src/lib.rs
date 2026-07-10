//! Permit-bound model-provider adapters and normalized provider events.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, EffectRequest, ModelMessage, ModelMessageRole, ModelRequest,
    ModelToolDefinition, ProviderEvent, ProviderModelInfo, ProviderReadiness,
    ProviderReadinessCheck, ProviderTurn, QuarantinedEffectResult,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
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
            streaming: false,
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
            .map_err(|error| match error {
                ProviderError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
                    "provider transport failed after execution began; outcome is unknown: {message}"
                )),
                error => ExecutionError::Failed(error.to_string()),
            })
    }
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
            ProviderKind::OpenAiResponses => responses_payload(&model_request),
            ProviderKind::OpenAiCompatible => chat_payload(&model_request),
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
        if let Some(reference) = self.profile.credential_reference.as_deref() {
            let secret = self.credentials.resolve(reference)?;
            builder = builder.bearer_auth(secret);
        }
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
        Ok(bytes)
    }
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

fn responses_payload(request: &ModelRequest) -> Result<Value, ProviderError> {
    let input = request
        .messages
        .iter()
        .map(responses_message)
        .collect::<Result<Vec<_>, _>>()?;
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
    });
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    Ok(payload)
}

fn responses_message(message: &ModelMessage) -> Result<Value, ProviderError> {
    match message.role {
        ModelMessageRole::System => Ok(json!({"role": "developer", "content": message.content})),
        ModelMessageRole::User => Ok(json!({"role": "user", "content": message.content})),
        ModelMessageRole::Assistant => Ok(json!({"role": "assistant", "content": message.content})),
        ModelMessageRole::Tool => {
            let call_id = message.tool_call_id.as_ref().ok_or_else(|| {
                ProviderError::Configuration("tool result message has no call id".into())
            })?;
            Ok(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.content,
            }))
        }
    }
}

fn chat_payload(request: &ModelRequest) -> Result<Value, ProviderError> {
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
    let mut payload = json!({"model": request.model, "messages": messages, "stream": false});
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
    Ok(value)
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
mod tests {
    use super::*;
    use colossus_contracts::{DecisionOutcome, ProviderEvent};
    use colossus_policy::{
        BuiltInPolicy, DenyApproval, EffectGateway, GatewayError, SafetyKernel, effect_request,
        system_actor,
    };
    use colossus_ports::EventJournal;
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    struct CountingCredentialResolver {
        calls: AtomicUsize,
    }

    impl CountingCredentialResolver {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl CredentialResolver for CountingCredentialResolver {
        fn resolve(&self, reference: &str) -> Result<String, ProviderError> {
            assert_eq!(reference, "env:UNIT_PROVIDER_KEY");
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok("unit-secret".into())
        }
    }

    fn model_request() -> ModelRequest {
        ModelRequest {
            model: "unit-model".into(),
            instructions: "Be exact.".into(),
            messages: vec![ModelMessage {
                role: ModelMessageRole::User,
                content: "hello".into(),
                tool_call_id: None,
            }],
            tools: Vec::new(),
        }
    }

    fn provider_request(profile: &ProviderProfile) -> EffectRequest {
        let mut request = effect_request(
            system_actor("provider-test"),
            profile.kind.generation_action(),
            profile.generation_endpoint().expect("generation endpoint"),
            serde_json::to_value(ProviderEffectInput {
                profile: profile.name.clone(),
                request: Some(model_request()),
            })
            .expect("effect input"),
        );
        request.capabilities = vec!["provider.call".into()];
        request.credential_references = profile
            .credential_reference
            .as_ref()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
            .into_iter()
            .collect();
        request
    }

    async fn one_response_server(body: Value) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let mut scratch = [0_u8; 4096];
            loop {
                let read = stream.read(&mut scratch).await.expect("read request");
                assert_ne!(read, 0, "client closed before completing request");
                request.extend_from_slice(&scratch[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("content length"))
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request).into_owned();
            let response_body = serde_json::to_vec(&body).expect("response JSON");
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response_body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write headers");
            stream.write_all(&response_body).await.expect("write body");
            request_text
        });
        (format!("http://{address}/v1"), task)
    }

    #[test]
    fn malformed_tool_arguments_fail_closed() {
        let profile = ProviderProfile::new(
            "local",
            ProviderKind::OpenAiCompatible,
            "unit-model",
            Some("http://127.0.0.1:9000/v1".into()),
            None,
            1_000,
        )
        .expect("profile");
        let malformed = json!({
            "id": "response-1",
            "choices": [{"message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call-1",
                    "function": {"name": "lookup", "arguments": "[]"}
                }]
            }}]
        });
        let error = normalize_chat(&profile, &serde_json::to_vec(&malformed).expect("JSON"))
            .expect_err("non-object arguments must fail");
        assert!(matches!(error, ProviderError::Malformed(_)));
    }

    #[test]
    fn responses_output_normalizes_visible_text_and_strict_tool_calls() {
        let profile = ProviderProfile::new(
            "openai",
            ProviderKind::OpenAiResponses,
            "unit-model",
            Some("https://api.openai.com/v1".into()),
            Some("env:UNIT_PROVIDER_KEY".into()),
            1_000,
        )
        .expect("profile");
        let response = json!({
            "id": "response-1",
            "output": [
                {"type": "reasoning", "summary": [
                    {"type": "summary_text", "text": "safe plan"}
                ], "content": "hidden reasoning"},
                {"type": "message", "content": [
                    {"type": "output_text", "text": "working"}
                ]},
                {"type": "function_call", "call_id": "call-1", "name": "lookup",
                 "arguments": "{\"query\":\"rust\"}"}
            ]
        });
        let turn = normalize_responses(&profile, &serde_json::to_vec(&response).expect("JSON"))
            .expect("normalized response");
        assert!(matches!(
            &turn.events[0],
            ProviderEvent::ReasoningSummary { summary } if summary == "safe plan"
        ));
        assert!(matches!(
            &turn.events[1],
            ProviderEvent::ModelDelta { text } if text == "working"
        ));
        assert!(matches!(
            &turn.events[2],
            ProviderEvent::ToolCallRequested { call_id, name, arguments }
                if call_id == "call-1" && name == "lookup" && arguments["query"] == "rust"
        ));
        assert!(
            !serde_json::to_string(&turn)
                .expect("turn JSON")
                .contains("hidden reasoning")
        );
        assert!(
            !turn
                .events
                .iter()
                .any(|event| matches!(event, ProviderEvent::FinalOutput { .. })),
            "a turn requesting a tool must not be marked final"
        );
    }

    #[test]
    fn hidden_reasoning_is_not_released_but_safe_summary_is() {
        let profile = ProviderProfile::new(
            "local",
            ProviderKind::OpenAiCompatible,
            "unit-model",
            Some("http://127.0.0.1:9000/v1".into()),
            None,
            1_000,
        )
        .expect("profile");
        let response = json!({
            "id": "response-1",
            "choices": [{"message": {
                "role": "assistant",
                "content": "visible",
                "reasoning": "private chain of thought",
                "reasoning_details": [
                    {"type": "reasoning.encrypted", "text": "ciphertext"},
                    {"type": "reasoning.summary", "summary": "safe summary"}
                ]
            }}]
        });
        let turn = normalize_chat(&profile, &serde_json::to_vec(&response).expect("JSON"))
            .expect("normalized turn");
        assert!(turn.events.iter().any(|event| matches!(
            event,
            ProviderEvent::ReasoningSummary { summary } if summary == "safe summary"
        )));
        let released = serde_json::to_string(&turn).expect("turn JSON");
        assert!(!released.contains("private chain of thought"));
        assert!(!released.contains("ciphertext"));
    }

    #[tokio::test]
    async fn denial_happens_before_credential_resolution() {
        let profile = ProviderProfile::new(
            "local",
            ProviderKind::OpenAiCompatible,
            "unit-model",
            Some("http://127.0.0.1:9/v1".into()),
            Some("env:UNIT_PROVIDER_KEY".into()),
            1_000,
        )
        .expect("profile");
        let credentials = Arc::new(CountingCredentialResolver::new());
        let executor = ProviderExecutor::with_credentials(
            profile.clone(),
            Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
        );
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(BuiltInPolicy::offline_default()),
            Arc::new(DenyApproval),
            SafetyKernel::new(["provider.call".into()]),
            [7_u8; 32],
        );
        let error = gateway
            .execute(provider_request(&profile), &executor)
            .await
            .expect_err("policy must deny provider call");
        assert!(matches!(error, GatewayError::Denied(_)));
        assert_eq!(credentials.calls.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn allowed_provider_call_is_permit_bound_and_post_released() {
        let (base_url, server) = one_response_server(json!({
            "id": "response-1",
            "choices": [{"message": {"role": "assistant", "content": "hello back"}}]
        }))
        .await;
        let profile = ProviderProfile::new(
            "local",
            ProviderKind::OpenAiCompatible,
            "unit-model",
            Some(base_url),
            Some("env:UNIT_PROVIDER_KEY".into()),
            5_000,
        )
        .expect("profile");
        let origin = profile
            .network_origin()
            .expect("origin")
            .expect("network provider origin");
        let credentials = Arc::new(CountingCredentialResolver::new());
        let executor = ProviderExecutor::with_credentials(
            profile.clone(),
            Arc::clone(&credentials) as Arc<dyn CredentialResolver>,
        );
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
            .with_network_destination(origin)
            .with_post_effect(true);
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["provider.call".into()]),
            [8_u8; 32],
        );
        let released = gateway
            .execute(provider_request(&profile), &executor)
            .await
            .expect("allowed provider call");
        let turn: ProviderTurn = serde_json::from_slice(&released.bytes).expect("provider turn");
        assert!(matches!(
            turn.events.last(),
            Some(ProviderEvent::FinalOutput { text }) if text == "hello back"
        ));
        assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
        let raw_request = server.await.expect("server task");
        assert!(raw_request.contains("POST /v1/chat/completions HTTP/1.1"));
        assert!(
            raw_request
                .to_ascii_lowercase()
                .contains("authorization: bearer unit-secret")
        );
        let event_types = journal
            .read_global(1, 50)
            .expect("journal events")
            .into_iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"effect.release_requested.v1".into()));
        assert!(event_types.contains(&"effect.completed.v1".into()));
    }
}
