use super::*;
use std::fmt;

const MAX_HOST_CREDENTIALS: usize = 64;

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

/// In-memory host credential resolver used by application-managed runtimes.
///
/// Host values are retained only in zeroizing memory. Existing `env:` references keep
/// their normal behavior so injecting this resolver does not change headless runtime
/// compatibility. The resolver deliberately has a redacted [`Debug`] implementation.
pub struct HostCredentialResolver {
    credentials: BTreeMap<String, zeroize::Zeroizing<String>>,
    environment: EnvironmentCredentialResolver,
}

impl HostCredentialResolver {
    /// Validate and retain one bounded set of opaque host credential identifiers.
    pub fn new(
        credentials: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, ProviderError> {
        let mut retained = BTreeMap::new();
        for (identifier, secret) in credentials {
            let secret = zeroize::Zeroizing::new(secret);
            if retained.len() >= MAX_HOST_CREDENTIALS {
                return Err(ProviderError::Configuration(
                    "host credential count exceeds the supported bound".into(),
                ));
            }
            if !valid_host_credential_identifier(&identifier) {
                return Err(ProviderError::Configuration(
                    "host credential identifier is invalid".into(),
                ));
            }
            if secret.is_empty() || secret.len() > 64 * 1024 || secret.contains('\0') {
                return Err(ProviderError::Configuration(
                    "host credential value is invalid".into(),
                ));
            }
            if retained.insert(identifier, secret).is_some() {
                return Err(ProviderError::Configuration(
                    "host credential identifier is duplicated".into(),
                ));
            }
        }
        Ok(Self {
            credentials: retained,
            environment: EnvironmentCredentialResolver,
        })
    }
}

impl fmt::Debug for HostCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostCredentialResolver")
            .field("credential_count", &self.credentials.len())
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

impl CredentialResolver for HostCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, ProviderError> {
        if let Some(identifier) = reference.strip_prefix("host:") {
            if !valid_host_credential_identifier(identifier) {
                return Err(ProviderError::Credential(
                    "host credential reference is invalid".into(),
                ));
            }
            return self
                .credentials
                .get(identifier)
                .map(|secret| secret.as_str().to_owned())
                .ok_or_else(|| {
                    ProviderError::Credential(format!(
                        "host credential {identifier} is unavailable"
                    ))
                });
        }
        self.environment.resolve(reference)
    }
}

/// One permit-bound provider adapter instance.
pub struct ProviderExecutor {
    pub(super) profile: ProviderProfile,
    pub(super) credentials: Arc<dyn CredentialResolver>,
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

pub(super) fn provider_execution_error(error: ProviderError) -> ExecutionError {
    match error {
        ProviderError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
            "provider transport failed after execution began; outcome is unknown: {message}"
        )),
        ProviderError::Status { status: 503 } => ExecutionError::Recoverable {
            code: "provider.temporarily_unavailable".into(),
            message: "provider endpoint returned HTTP 503; retry after the endpoint reports ready"
                .into(),
        },
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
        if network_destination_match(&permit.obligations().network_destinations, &origin)
            .map_err(|error| ProviderError::Configuration(error.to_string()))?
            .is_none()
        {
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
        redact_exact_bytes(&mut bytes, secret.as_ref().map(|secret| secret.as_str()));
        Ok(bytes)
    }

    async fn send_request(
        &self,
        endpoint: &str,
        payload: Option<Value>,
        permit: &ExecutionPermit,
    ) -> Result<(reqwest::Response, Option<zeroize::Zeroizing<String>>), ProviderError> {
        let url = Url::parse(endpoint)?;
        let host = url
            .host_str()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no port".into()))?;
        let matched =
            network_destination_match(&permit.obligations().network_destinations, endpoint)
                .map_err(|error| ProviderError::Configuration(error.to_string()))?
                .ok_or_else(|| {
                    ProviderError::Configuration(
                        "provider origin is absent from permit obligations".into(),
                    )
                })?;
        let allow_non_public = matched == NetworkDestinationMatch::Exact
            && (host.eq_ignore_ascii_case("localhost")
                || host.parse::<IpAddr>().is_ok_and(non_public_network_address));
        let addresses = resolve_provider_addresses(host, port, allow_non_public).await?;
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
            let secret = zeroize::Zeroizing::new(self.credentials.resolve(reference)?);
            if secret.is_empty() {
                return Err(ProviderError::Credential(
                    "resolved provider credential is empty".into(),
                ));
            }
            builder = builder.bearer_auth(secret.as_str());
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
                redact_exact_bytes(&mut data, secret.as_ref().map(|secret| secret.as_str()));
                if data == b"[DONE]" {
                    state.mark_done();
                    continue;
                }
                let mut value: Value = serde_json::from_slice(&data).map_err(|error| {
                    provider_execution_error(ProviderError::Malformed(format!(
                        "provider SSE data is not valid JSON: {error}"
                    )))
                })?;
                redact_value_exact(&mut value, secret.as_ref().map(|secret| secret.as_str()));
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
