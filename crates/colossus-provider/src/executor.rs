use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::fmt;

const MAX_HOST_CREDENTIALS: usize = 64;
const MAX_STREAMED_MODEL_DELTA_BATCH_BYTES: usize = 4 * 1024;
const STREAMED_MODEL_DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
/// Extra time an outer effect deadline should allow after a configured generation
/// deadline so the streaming adapter can settle the permit and transport cleanly.
pub const PROVIDER_STREAM_CLEANUP_RESERVE_MS: u64 = 1_000;
const GENERATION_DEADLINE_MESSAGE: &str =
    "provider generation exceeded its hard deadline after execution began";

/// Strict content placed inside a provider effect request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderEffectInput {
    /// Selected route streaming capability; absent retains streaming compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_response: Option<bool>,
    /// Optional absolute rendered-token threshold, only for public Responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_compaction_threshold: Option<u64>,
    /// Safe state provenance; opaque items are resolved only after a permit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<colossus_contracts::ProviderContinuationPlan>,
    /// Provider connection profile selected by routing.
    pub provider_profile: String,
    /// Model profile selected by routing. Absent only for provider diagnostics.
    pub model_profile: Option<String>,
    /// Exact provider model identifier. Absent only for provider diagnostics.
    pub model: Option<String>,
    /// Resolved output ceiling. Absent only for provider diagnostics.
    pub max_output_tokens: Option<u64>,
    /// Optional configured reasoning effort for generation requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Full logical model request. Absent only for model-catalog diagnostics.
    pub request: Option<ModelRequest>,
    /// Return a bounded failed or transport-incompatible response as explicit
    /// quarantined diagnostic output.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_response_diagnostics: bool,
}

enum ProviderJsonResponse {
    Success(Vec<u8>),
    HttpError(ProviderResponseDiagnostic),
    FeatureRejected(crate::ProviderFeatureRejection),
}

#[derive(Default)]
pub(super) struct RequestSecrets(Vec<zeroize::Zeroizing<String>>);

impl RequestSecrets {
    pub(super) fn retain(&mut self, secret: &str) {
        self.0.push(zeroize::Zeroizing::new(secret.to_owned()));
    }

    pub(super) fn redact_bytes(&self, bytes: &mut Vec<u8>) {
        for secret in &self.0 {
            redact_exact_bytes(bytes, Some(secret.as_str()));
        }
    }

    fn redact_value(&self, value: &mut Value) {
        for secret in &self.0 {
            redact_value_exact(value, Some(secret.as_str()));
        }
    }
}

struct ProviderStreamMetadata<'a> {
    model_profile: &'a str,
    model: &'a str,
    include_response_diagnostics: bool,
    generation_deadline: tokio::time::Instant,
    continuation_plan: Option<colossus_contracts::ProviderContinuationPlan>,
    candidate_id: String,
}

struct ProviderJsonGeneration<'a> {
    response: reqwest::Response,
    secrets: RequestSecrets,
    payload: &'a Value,
}

enum CollectedProviderOutput {
    Turn(ProviderTurn),
    Diagnostic(ProviderResponseDiagnostic),
}

struct CollectedProviderStream {
    events: Vec<ProviderEvent>,
    output: Option<CollectedProviderOutput>,
    last: Option<QuarantinedEffectResult>,
    total_bytes: usize,
    max_output_bytes: usize,
}

impl CollectedProviderStream {
    fn new(max_output_bytes: u64) -> Result<Self, ExecutionError> {
        Ok(Self {
            events: Vec::new(),
            output: None,
            last: None,
            total_bytes: 0,
            max_output_bytes: usize::try_from(max_output_bytes)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
        })
    }

    fn finish(
        self,
        terminal: QuarantinedEffectResult,
        permit: &ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if self.last.as_ref() != Some(&terminal) {
            return Err(ExecutionError::Failed(
                "collected provider stream terminal result did not match its last chunk".into(),
            ));
        }
        match self.output {
            Some(CollectedProviderOutput::Turn(turn)) => {
                bounded_result(&turn, permit).map_err(provider_execution_error)
            }
            Some(CollectedProviderOutput::Diagnostic(diagnostic)) => {
                bounded_result(&diagnostic, permit).map_err(provider_execution_error)
            }
            None => Err(ExecutionError::Failed(
                "collected provider stream has no terminal output".into(),
            )),
        }
    }
}

#[async_trait]
impl QuarantinedEffectObserver for CollectedProviderStream {
    async fn observe(&mut self, result: QuarantinedEffectResult) -> Result<(), ExecutionError> {
        if self.output.is_some() {
            return Err(ExecutionError::Failed(
                "collected provider stream emitted data after its terminal chunk".into(),
            ));
        }
        if !result.effect_succeeded
            || result.media_type != "application/vnd.colossus.provider-stream+json"
        {
            return Err(ExecutionError::Failed(
                "collected provider stream emitted an invalid chunk".into(),
            ));
        }
        self.total_bytes = self.total_bytes.saturating_add(result.bytes.len());
        if self.total_bytes > self.max_output_bytes {
            return Err(ExecutionError::OutcomeUnknown(
                "collected provider stream exceeds the cumulative permitted bound".into(),
            ));
        }
        let item = serde_json::from_slice::<ProviderStreamItem>(&result.bytes)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        match item {
            ProviderStreamItem::Retry { .. } => {}
            ProviderStreamItem::Event { event } => self.events.push(event),
            ProviderStreamItem::Diagnostic { diagnostic } => {
                if !self.events.is_empty() {
                    return Err(ExecutionError::Failed(
                        "collected provider stream emitted a diagnostic after model events".into(),
                    ));
                }
                self.output = Some(CollectedProviderOutput::Diagnostic(diagnostic));
            }
            ProviderStreamItem::Completed {
                profile,
                model_profile,
                provider_profile,
                provider,
                model,
                response_id,
            } => {
                self.output = Some(CollectedProviderOutput::Turn(ProviderTurn {
                    profile,
                    model_profile,
                    provider_profile,
                    provider,
                    model,
                    response_id,
                    events: std::mem::take(&mut self.events),
                }));
            }
        }
        self.last = Some(result);
        Ok(())
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

use colossus_ports::CredentialResolutionError;
pub use colossus_ports::{CredentialResolver, EnvironmentCredentialResolver};

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
    fn resolve(&self, reference: &str) -> Result<String, CredentialResolutionError> {
        if let Some(identifier) = reference.strip_prefix("host:") {
            if !valid_host_credential_identifier(identifier) {
                return Err(CredentialResolutionError::InvalidReference);
            }
            return self
                .credentials
                .get(identifier)
                .map(|secret| secret.as_str().to_owned())
                .ok_or(CredentialResolutionError::Unavailable);
        }
        self.environment.resolve(reference)
    }
}

/// One permit-bound provider adapter instance.
pub struct ProviderExecutor {
    pub(super) continuations: Option<Arc<dyn colossus_ports::ProviderContinuationRepository>>,
    pub(super) profile: ProviderProfile,
    pub(super) credentials: Arc<dyn CredentialResolver>,
    tls_roots: AdditionalRootCertificates,
    codex_auth: Option<CodexAuthStore>,
    codex_refresh: tokio::sync::Mutex<()>,
    media: Option<Arc<dyn RunInputMediaResolver>>,
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
            continuations: None,
            profile,
            credentials,
            tls_roots: AdditionalRootCertificates::default(),
            codex_auth: None,
            codex_refresh: tokio::sync::Mutex::new(()),
            media: None,
        }
    }

    /// Add validated runtime-wide CA roots to this provider's built-in public roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }

    /// Override the Codex auth file location for an embedded host or test.
    #[must_use]
    pub fn with_codex_auth_store(mut self, store: CodexAuthStore) -> Self {
        self.codex_auth = Some(store);
        self
    }

    /// Bind the application-owned encrypted run-input media resolver.
    #[must_use]
    pub fn with_run_input_media(mut self, media: Arc<dyn RunInputMediaResolver>) -> Self {
        self.media = Some(media);
        self
    }

    /// Resolve private Responses state within the permit-bearing adapter.
    #[must_use]
    pub fn with_continuations(
        mut self,
        repository: Arc<dyn colossus_ports::ProviderContinuationRepository>,
    ) -> Self {
        self.continuations = Some(repository);
        self
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
                provider_response: None,
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
        if self.profile.kind == ProviderKind::OpenAiCodex
            && request.action == self.profile.kind.generation_action()
        {
            let mut collector =
                CollectedProviderStream::new(permit.obligations().max_output_bytes)?;
            let terminal = self
                .execute_stream_with_deadline(request, &permit, &mut collector)
                .await?;
            return collector.finish(terminal, &permit);
        }
        self.execute_permitted(request, &permit)
            .await
            .map_err(provider_execution_error)
    }
}

pub(super) fn provider_execution_error(error: ProviderError) -> ExecutionError {
    match error {
        ProviderError::Rejected(failure) => ExecutionError::ProviderRejected(failure),
        ProviderError::Progress(error) => *error,
        ProviderError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
            "provider transport failed after execution began; outcome is unknown: {message}"
        )),
        ProviderError::Status {
            status: status @ 502..=504,
            retry_after_ms,
        } => ExecutionError::Recoverable {
            code: "provider.temporarily_unavailable".into(),
            message: format!(
                "provider endpoint returned HTTP {status}; retry after the endpoint reports ready"
            ),
            http_status: Some(status),
            retry_after_ms,
        },
        ProviderError::Status { status, .. } => ExecutionError::HttpStatus {
            status,
            message: format!("provider endpoint returned HTTP {status}"),
        },
        ProviderError::Malformed(message) if invalid_tool_argument_message(&message) => {
            ExecutionError::Recoverable {
                code: "provider.invalid_tool_arguments".into(),
                message,
                http_status: None,
                retry_after_ms: None,
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
        self.execute_stream_with_deadline(effect, &permit, observer)
            .await
    }
}

impl ProviderExecutor {
    async fn execute_stream_with_deadline(
        &self,
        effect: &EffectRequest,
        permit: &ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let timeout_ms = provider_generation_budget_ms(
            self.profile.generation_timeout_ms,
            permit.obligations().timeout_ms,
        );
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        self.execute_stream_permitted(effect, permit, observer, deadline)
            .await
    }

    async fn execute_stream_permitted(
        &self,
        effect: &EffectRequest,
        permit: &ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
        deadline: tokio::time::Instant,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let input: ProviderEffectInput =
            serde_json::from_value(effect.content.clone()).map_err(|error| {
                provider_execution_error(ProviderError::Malformed(error.to_string()))
            })?;
        if input.provider_profile != self.profile.name {
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
        let (model_profile, model, max_output_tokens) =
            generation_metadata(&input).map_err(provider_execution_error)?;
        let stream_response = input.stream_response.unwrap_or(true);
        let include_response_diagnostics = input.include_response_diagnostics;
        let reasoning_effort = input.reasoning_effort;
        let continuation_plan = input.continuation.clone();
        let continuation = self
            .resolve_continuation(
                continuation_plan.as_ref(),
                effect.context.session_id.as_deref(),
            )
            .map_err(provider_execution_error)?;
        let server_compaction_threshold = input.server_compaction_threshold;
        let model_request = input.request.ok_or_else(|| {
            provider_execution_error(ProviderError::Configuration(
                "provider generation request is absent".into(),
            ))
        })?;
        validate_model_request(&model_request, max_output_tokens)
            .map_err(provider_execution_error)?;
        let resolved_images =
            tokio::time::timeout_at(deadline, self.resolve_images(&model_request))
                .await
                .map_err(|_| generation_deadline_error())?
                .map_err(provider_execution_error)?;
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
                .map(|message| message.content.plain_text().into_owned())
                .ok_or_else(|| {
                    provider_execution_error(ProviderError::Malformed(
                        "echo request has no message".into(),
                    ))
                })?;
            return tokio::time::timeout_at(deadline, async {
                emit_stream_item(
                    ProviderStreamItem::Event {
                        event: ProviderEvent::ModelDelta { text: text.clone() },
                    },
                    permit,
                    observer,
                )
                .await?;
                emit_stream_item(
                    ProviderStreamItem::Event {
                        event: ProviderEvent::FinalOutput { text },
                    },
                    permit,
                    observer,
                )
                .await?;
                emit_stream_item(
                    ProviderStreamItem::Completed {
                        profile: model_profile.clone(),
                        model_profile,
                        provider_profile: self.profile.name.clone(),
                        provider: self.profile.kind.as_str().into(),
                        model,
                        response_id: None,
                    },
                    permit,
                    observer,
                )
                .await
            })
            .await
            .map_err(|_| generation_deadline_error())?;
        }
        self.validate_resource(effect, &endpoint, permit)
            .map_err(provider_execution_error)?;
        let tool_names =
            ProviderToolNames::from_request(&model_request).map_err(provider_execution_error)?;
        let mut payload = match self.profile.kind {
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => {
                responses_payload_with_images(
                    &model_request,
                    self.profile.kind,
                    &model,
                    max_output_tokens,
                    reasoning_effort,
                    stream_response,
                    ProviderProjection::new(&tool_names, &resolved_images)
                        .with_continuation(continuation.as_ref()),
                )
            }
            ProviderKind::OpenAiCompatible => chat_payload_with_images(
                &model_request,
                &model,
                max_output_tokens,
                self.profile.chat_completions_output_token_parameter,
                reasoning_effort,
                stream_response,
                ProviderProjection::new(&tool_names, &resolved_images),
            ),
            ProviderKind::Echo => unreachable!("handled above"),
        }
        .map_err(provider_execution_error)?;
        configure_compaction(&mut payload, self.profile.kind, server_compaction_threshold)
            .map_err(provider_execution_error)?;
        self.stream_generation(
            &endpoint,
            payload,
            ProviderStreamMetadata {
                model_profile: &model_profile,
                model: &model,
                include_response_diagnostics,
                generation_deadline: deadline,
                continuation_plan,
                candidate_id: effect.request_id.clone(),
            },
            tool_names,
            permit,
            observer,
        )
        .await
    }
}

fn generation_deadline_error() -> ExecutionError {
    ExecutionError::OutcomeUnknown(GENERATION_DEADLINE_MESSAGE.into())
}

pub(super) fn provider_generation_budget_ms(
    generation_timeout_ms: u64,
    effect_timeout_ms: u64,
) -> u64 {
    generation_timeout_ms.min(
        effect_timeout_ms
            .saturating_sub(PROVIDER_STREAM_CLEANUP_RESERVE_MS)
            .max(1),
    )
}

async fn emit_stream_item(
    item: impl Serialize,
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

struct ProviderStreamEmitter<'a> {
    permit: &'a ExecutionPermit,
    observer: &'a mut dyn QuarantinedEffectObserver,
    pending_model_delta: String,
}

impl<'a> ProviderStreamEmitter<'a> {
    fn new(permit: &'a ExecutionPermit, observer: &'a mut dyn QuarantinedEffectObserver) -> Self {
        Self {
            permit,
            observer,
            pending_model_delta: String::new(),
        }
    }

    fn has_pending_model_delta(&self) -> bool {
        !self.pending_model_delta.is_empty()
    }

    async fn push(&mut self, event: ProviderEvent) -> Result<(), ExecutionError> {
        let ProviderEvent::ModelDelta { text } = event else {
            self.flush().await?;
            emit_stream_item(
                ProviderStreamItem::Event { event },
                self.permit,
                self.observer,
            )
            .await?;
            return Ok(());
        };
        if text.is_empty() {
            return Ok(());
        }
        let mut remaining = text.as_str();
        while !remaining.is_empty() {
            let available =
                MAX_STREAMED_MODEL_DELTA_BATCH_BYTES.saturating_sub(self.pending_model_delta.len());
            let mut take = remaining.len().min(available);
            while take > 0 && !remaining.is_char_boundary(take) {
                take = take.saturating_sub(1);
            }
            if take == 0 {
                self.flush().await?;
                continue;
            }
            self.pending_model_delta.push_str(&remaining[..take]);
            remaining = &remaining[take..];
            if self.pending_model_delta.len() == MAX_STREAMED_MODEL_DELTA_BATCH_BYTES {
                self.flush().await?;
            }
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), ExecutionError> {
        if self.pending_model_delta.is_empty() {
            return Ok(());
        }
        let text = std::mem::take(&mut self.pending_model_delta);
        emit_stream_item(
            ProviderStreamItem::Event {
                event: ProviderEvent::ModelDelta { text },
            },
            self.permit,
            self.observer,
        )
        .await?;
        Ok(())
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
        if input.provider_profile != self.profile.name {
            return Err(ProviderError::Configuration(
                "provider effect profile does not match its adapter".into(),
            ));
        }
        validate_credential_disclosure(effect, &self.profile)?;
        let include_response_diagnostics = input.include_response_diagnostics;
        if effect.action == "provider.models" {
            if input.request.is_some()
                || input.model_profile.is_some()
                || input.model.is_some()
                || input.max_output_tokens.is_some()
                || input.reasoning_effort.is_some()
                || self.profile.kind == ProviderKind::Echo
            {
                return Err(ProviderError::Configuration(
                    "model catalog effect is invalid for this provider".into(),
                ));
            }
            let endpoint = self
                .profile
                .models_endpoint()?
                .ok_or_else(|| ProviderError::Configuration("provider has no catalog".into()))?;
            self.validate_resource(effect, &endpoint, permit)?;
            let bytes = match self
                .request_json(&endpoint, None, permit, include_response_diagnostics)
                .await?
            {
                ProviderJsonResponse::Success(bytes) => bytes,
                ProviderJsonResponse::FeatureRejected(rejection) => {
                    return bounded_result(&rejection, permit);
                }
                ProviderJsonResponse::HttpError(diagnostic) => {
                    return bounded_result(&diagnostic, permit);
                }
            };
            let models = normalize_models(&bytes)?;
            return bounded_result(&models, permit);
        }
        if effect.action != self.profile.kind.generation_action() {
            return Err(ProviderError::Configuration(
                "provider adapter received an unsupported action".into(),
            ));
        }
        let (model_profile, model, max_output_tokens) = generation_metadata(&input)?;
        let reasoning_effort = input.reasoning_effort;
        let continuation_plan = input.continuation.clone();
        let continuation = self.resolve_continuation(
            continuation_plan.as_ref(),
            effect.context.session_id.as_deref(),
        )?;
        let server_compaction_threshold = input.server_compaction_threshold;
        let model_request = input.request.ok_or_else(|| {
            ProviderError::Configuration("provider generation request is absent".into())
        })?;
        validate_model_request(&model_request, max_output_tokens)?;
        let resolved_images = self.resolve_images(&model_request).await?;
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
                .map(|message| message.content.plain_text().into_owned())
                .ok_or_else(|| ProviderError::Malformed("echo request has no message".into()))?;
            return bounded_result(
                &ProviderTurn {
                    profile: model_profile.clone(),
                    model_profile,
                    provider_profile: self.profile.name.clone(),
                    provider: self.profile.kind.as_str().into(),
                    model,
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
        let tool_names = ProviderToolNames::from_request(&model_request)?;
        let mut payload = match self.profile.kind {
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => {
                responses_payload_with_images(
                    &model_request,
                    self.profile.kind,
                    &model,
                    max_output_tokens,
                    reasoning_effort,
                    false,
                    ProviderProjection::new(&tool_names, &resolved_images)
                        .with_continuation(continuation.as_ref()),
                )
            }
            ProviderKind::OpenAiCompatible => chat_payload_with_images(
                &model_request,
                &model,
                max_output_tokens,
                self.profile.chat_completions_output_token_parameter,
                reasoning_effort,
                false,
                ProviderProjection::new(&tool_names, &resolved_images),
            ),
            ProviderKind::Echo => unreachable!("handled above"),
        }?;
        configure_compaction(&mut payload, self.profile.kind, server_compaction_threshold)?;
        let request_payload = payload.clone();
        let bytes = match self
            .request_json(
                &endpoint,
                Some(payload),
                permit,
                include_response_diagnostics,
            )
            .await?
        {
            ProviderJsonResponse::Success(bytes) => bytes,
            ProviderJsonResponse::FeatureRejected(rejection) => {
                return bounded_result(&rejection, permit);
            }
            ProviderJsonResponse::HttpError(diagnostic) => {
                return bounded_result(&diagnostic, permit);
            }
        };
        let turn = match self.profile.kind {
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => {
                normalize_responses(&self.profile, &model_profile, &model, &bytes, &tool_names)
            }
            ProviderKind::OpenAiCompatible => {
                normalize_chat(&self.profile, &model_profile, &model, &bytes, &tool_names)
            }
            ProviderKind::Echo => unreachable!("handled above"),
        }?;
        let response = serde_json::from_slice::<Value>(&bytes).ok();
        let continuation_id = if response
            .as_ref()
            .is_some_and(|value| value["status"] == "completed")
        {
            let output = response
                .as_ref()
                .and_then(|value| value["output"].as_array())
                .cloned()
                .unwrap_or_default();
            self.stage_continuation(
                &effect.request_id,
                continuation_plan.as_ref(),
                &request_payload,
                &output,
                &turn,
            )?
        } else {
            None
        };
        if continuation_id.is_some() {
            bounded_result(
                &crate::ProviderAdapterTurn {
                    turn,
                    continuation_id,
                },
                permit,
            )
        } else {
            bounded_result(&turn, permit)
        }
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
        self.profile
            .network_origin()?
            .ok_or_else(|| ProviderError::Configuration("network provider has no origin".into()))?;
        if http_transport_authority_match(permit.obligations(), endpoint)
            .map_err(|error| ProviderError::Configuration(error.to_string()))?
            .is_none()
        {
            return Err(ProviderError::Configuration(
                "provider origin is absent from permit obligations".into(),
            ));
        }
        Ok(())
    }

    async fn resolve_images(
        &self,
        request: &ModelRequest,
    ) -> Result<ProviderResolvedImages, ProviderError> {
        let references = request
            .messages
            .iter()
            .flat_map(|message| message.content.images())
            .collect::<Vec<_>>();
        if references.is_empty() {
            return Ok(ProviderResolvedImages::default());
        }
        if references.len() > 16 {
            return Err(ProviderError::Configuration(
                "provider-visible image count exceeds 16".into(),
            ));
        }
        let resolver = self.media.as_ref().ok_or_else(|| {
            ProviderError::Configuration("run-input image resolver is unavailable".into())
        })?;
        let mut combined = 0_u64;
        let mut resolved = ProviderResolvedImages::default();
        for reference in references {
            let image = resolver
                .resolve_image(reference)
                .await
                .map_err(|error| ProviderError::Configuration(error.to_string()))?;
            combined = combined
                .checked_add(image.reference.size_bytes)
                .ok_or_else(|| {
                    ProviderError::Configuration("provider-visible image size overflowed".into())
                })?;
            if combined > 32 * 1_048_576 {
                return Err(ProviderError::Configuration(
                    "provider-visible images exceed 32 MiB".into(),
                ));
            }
            let data_url = format!(
                "data:{};base64,{}",
                image.reference.media_type,
                BASE64.encode(&image.bytes)
            );
            resolved.insert(&image.reference, data_url)?;
        }
        Ok(resolved)
    }

    async fn request_json(
        &self,
        endpoint: &str,
        payload: Option<Value>,
        permit: &ExecutionPermit,
        include_response_diagnostics: bool,
    ) -> Result<ProviderJsonResponse, ProviderError> {
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(self.profile.timeout_ms.min(permit.obligations().timeout_ms));
        let (response, secret) = tokio::time::timeout_at(
            deadline,
            self.send_request(endpoint, payload.as_ref(), permit, deadline, None),
        )
        .await
        .map_err(|_| ProviderError::Transport("provider request exceeded its deadline".into()))??;
        if !response.status().is_success() {
            let error = provider_status_error(&response);
            let diagnostic = self
                .capture_http_error(
                    endpoint,
                    payload.as_ref().map(redacted_image_payload),
                    response,
                    secret,
                )
                .await?;
            if let Some(rejection) =
                crate::features::feature_rejection(&diagnostic, payload.as_ref())
            {
                return Ok(ProviderJsonResponse::FeatureRejected(rejection));
            }
            if include_response_diagnostics {
                return Ok(ProviderJsonResponse::HttpError(diagnostic));
            }
            if !matches!(diagnostic.status, 502..=504)
                && let Some(failure) = classify_response_diagnostic(&diagnostic)
            {
                return Err(ProviderError::Rejected(failure));
            }
            return Err(error);
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
        if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
            secret.redact_value(&mut value);
            bytes = serde_json::to_vec(&value)
                .map_err(|_| ProviderError::Malformed("invalid provider JSON".into()))?;
        } else {
            secret.redact_bytes(&mut bytes);
        }
        Ok(ProviderJsonResponse::Success(bytes))
    }

    async fn send_request(
        &self,
        endpoint: &str,
        payload: Option<&Value>,
        permit: &ExecutionPermit,
        deadline: tokio::time::Instant,
        mut observer: Option<&mut dyn QuarantinedEffectObserver>,
    ) -> Result<(reqwest::Response, RequestSecrets), ProviderError> {
        let mut retries = 0;
        let mut last_status = 0;
        loop {
            if retries > 0 {
                report_retry(
                    &mut observer,
                    permit,
                    retries,
                    last_status,
                    colossus_contracts::ProviderRetryState::Retrying,
                    None,
                )
                .await?;
            }
            let (response, secrets) = self
                .send_request_once(endpoint, payload, permit, deadline)
                .await?;
            let delay = match response_retry_after(&response) {
                Some(RetryAfter::ExceedsSupportedBound) => None,
                retry_after => super::retry::retry_delay(
                    response.status().as_u16(),
                    retries,
                    retry_after.and_then(RetryAfter::milliseconds),
                ),
            };
            let Some(delay) = delay else {
                if retries > 0 && response.status().is_success() {
                    report_retry(
                        &mut observer,
                        permit,
                        retries,
                        last_status,
                        colossus_contracts::ProviderRetryState::Recovered,
                        None,
                    )
                    .await?;
                }
                return Ok((response, secrets));
            };
            // Preserve the last confirmed status when its backoff cannot fit. The
            // caller's existing deadline also bounds setup, sleeping and dispatch.
            if delay >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                return Ok((response, secrets));
            }
            last_status = u32::from(response.status().as_u16());
            drop(response);
            drop(secrets);
            retries += 1;
            let retry_deadline = tokio::time::Instant::now() + delay;
            let retry_at = (OffsetDateTime::now_utc()
                + time::Duration::try_from(delay)
                    .map_err(|error| ProviderError::Configuration(error.to_string()))?)
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| ProviderError::Configuration(error.to_string()))?;
            report_retry(
                &mut observer,
                permit,
                retries,
                last_status,
                colossus_contracts::ProviderRetryState::Backoff,
                Some(retry_at),
            )
            .await?;
            tokio::time::sleep_until(retry_deadline).await;
        }
    }

    async fn send_request_once(
        &self,
        endpoint: &str,
        payload: Option<&Value>,
        permit: &ExecutionPermit,
        deadline: tokio::time::Instant,
    ) -> Result<(reqwest::Response, RequestSecrets), ProviderError> {
        let url = Url::parse(endpoint)?;
        let streaming = payload
            .and_then(|value| value.get("stream"))
            .and_then(Value::as_bool)
            == Some(true);
        let client = self.client_for_url(&url, permit, streaming).await?;
        let mut builder = payload
            .as_ref()
            .map_or_else(|| client.get(url.clone()), |_| client.post(url.clone()));
        if streaming {
            builder = builder.header(reqwest::header::ACCEPT, "text/event-stream");
        }
        let mut secrets = RequestSecrets::default();
        if self.profile.kind == ProviderKind::OpenAiCodex {
            let authorization = self.codex_authorization(permit).await?;
            builder = builder
                .bearer_auth(authorization.access_token())
                .header("ChatGPT-Account-ID", authorization.account_id())
                .header("originator", "Codex Colossus")
                .header("version", CODEX_PROTOCOL_VERSION)
                .header(
                    reqwest::header::USER_AGENT,
                    concat!("colossus/", env!("CARGO_PKG_VERSION")),
                );
            if authorization.is_fedramp() {
                builder = builder.header("X-OpenAI-Fedramp", "true");
            }
            secrets.retain(authorization.access_token());
            secrets.retain(authorization.account_id());
        } else if let Some(reference) = self.profile.credential_reference.as_deref() {
            let secret = zeroize::Zeroizing::new(self.credentials.resolve(reference)?);
            if secret.is_empty() {
                return Err(ProviderError::Credential(
                    "resolved provider credential is empty".into(),
                ));
            }
            builder = builder.bearer_auth(secret.as_str());
            secrets.0.push(secret);
        }
        if let Some(payload) = payload {
            retain_opaque_redactions(payload, &mut secrets);
            let body = serde_json::to_vec(payload)
                .map_err(|error| ProviderError::Malformed(error.to_string()))?;
            validate_serialized_provider_request(payload, body.len())?;
            builder = builder
                .header("content-type", "application/json")
                .body(body);
        }
        for (name, value) in colossus_observability::current_trace_headers() {
            builder = builder.header(name, value);
        }
        if !streaming {
            builder =
                builder.timeout(deadline.saturating_duration_since(tokio::time::Instant::now()));
        }
        let response = builder.send().await?;
        Ok((response, secrets))
    }

    async fn client_for_url(
        &self,
        url: &Url,
        permit: &ExecutionPermit,
        streaming: bool,
    ) -> Result<Client, ProviderError> {
        let host = url
            .host_str()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ProviderError::Configuration("provider URL has no port".into()))?;
        let matched = http_transport_authority_match(permit.obligations(), url.as_str())
            .map_err(|error| ProviderError::Configuration(error.to_string()))?
            .ok_or_else(|| {
                ProviderError::Configuration(
                    "provider origin is absent from permit obligations".into(),
                )
            })?;
        let allow_non_public = matched == NetworkDestinationMatch::Ambient
            || (matched == NetworkDestinationMatch::Exact
                && (host.eq_ignore_ascii_case("localhost")
                    || host.parse::<IpAddr>().is_ok_and(non_public_network_address)));
        let addresses = resolve_provider_addresses(host, port, allow_non_public).await?;
        let timeout_ms = self.profile.timeout_ms.min(permit.obligations().timeout_ms);
        let mut builder = self
            .tls_roots
            .configure_reqwest(Client::builder())
            .no_proxy()
            .redirect(RedirectPolicy::none())
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_millis(timeout_ms));
        builder = if streaming {
            // A streaming generation can legitimately outlive one inactivity interval.
            // Reqwest's read timeout resets after every successful read, while the effect
            // gateway independently enforces the configured hard generation ceiling.
            builder.read_timeout(Duration::from_millis(timeout_ms))
        } else {
            builder.timeout(Duration::from_millis(timeout_ms))
        };
        builder.build().map_err(ProviderError::from)
    }

    async fn codex_authorization(
        &self,
        permit: &ExecutionPermit,
    ) -> Result<CodexAuthorization, ProviderError> {
        let store = self
            .codex_auth
            .clone()
            .map(Ok)
            .unwrap_or_else(CodexAuthStore::from_environment)
            .map_err(codex_credential_error)?;
        let authorization = store.load().map_err(codex_credential_error)?;
        if !authorization.requires_refresh(OffsetDateTime::now_utc()) {
            return Ok(authorization);
        }
        let _refresh_guard = self.codex_refresh.lock().await;
        let authorization = store.load().map_err(codex_credential_error)?;
        if !authorization.requires_refresh(OffsetDateTime::now_utc()) {
            return Ok(authorization);
        }
        let url = Url::parse(CODEX_TOKEN_ENDPOINT)?;
        let client = self.client_for_url(&url, permit, false).await?;
        let response = client
            .post(url)
            .json(&CodexRefreshRequest::new(&authorization))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(ProviderError::Credential(format!(
                "Codex token refresh returned HTTP {}; run `colossus codex login`",
                response.status().as_u16()
            )));
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_CODEX_REFRESH_RESPONSE_BYTES {
                return Err(ProviderError::Credential(
                    "Codex token refresh response exceeded the safety bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        store
            .apply_refresh(&authorization, &bytes)
            .map_err(codex_credential_error)
    }

    async fn capture_http_error(
        &self,
        endpoint: &str,
        request_body: Option<Value>,
        response: reqwest::Response,
        secret: RequestSecrets,
    ) -> Result<ProviderResponseDiagnostic, ProviderError> {
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.chars().take(256).collect());
        let mut body = Vec::new();
        let mut body_truncated = false;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let remaining = MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES.saturating_sub(body.len());
            if chunk.len() > remaining {
                body.extend_from_slice(&chunk[..remaining]);
                body_truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        secret.redact_bytes(&mut body);
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            body = serde_json::to_vec(&redacted_image_payload(&value))
                .map_err(|_| ProviderError::Malformed("invalid diagnostic JSON".into()))?;
        }
        if body.len() > MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES {
            body.truncate(MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES);
            body_truncated = true;
        }
        let body_encoding = if std::str::from_utf8(&body).is_ok() {
            "utf8"
        } else {
            "utf8_lossy"
        };
        Ok(ProviderResponseDiagnostic {
            request_method: if request_body.is_some() {
                "POST".into()
            } else {
                "GET".into()
            },
            request_url: endpoint.into(),
            request_body,
            status,
            content_type,
            body: String::from_utf8_lossy(&body).into_owned(),
            body_encoding: body_encoding.into(),
            body_truncated,
        })
    }

    async fn release_json_generation(
        &self,
        generation: ProviderJsonGeneration<'_>,
        metadata: ProviderStreamMetadata<'_>,
        tool_names: ProviderToolNames,
        permit: &ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let ProviderJsonGeneration {
            response,
            secrets,
            payload,
        } = generation;
        let limit = usize::try_from(permit.obligations().max_output_bytes)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let mut bytes = Vec::new();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk
                .map_err(ProviderError::from)
                .map_err(provider_execution_error)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(ExecutionError::Failed(
                    "provider response exceeds the permitted output bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut response_value = serde_json::from_slice::<Value>(&bytes).ok();
        if let Some(value) = &mut response_value {
            secrets.redact_value(value);
            bytes = serde_json::to_vec(value)
                .map_err(|_| ExecutionError::Failed("invalid provider JSON".into()))?;
        } else {
            secrets.redact_bytes(&mut bytes);
        }
        let turn = match self.profile.kind {
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => normalize_responses(
                &self.profile,
                metadata.model_profile,
                metadata.model,
                &bytes,
                &tool_names,
            ),
            ProviderKind::OpenAiCompatible => normalize_chat(
                &self.profile,
                metadata.model_profile,
                metadata.model,
                &bytes,
                &tool_names,
            ),
            ProviderKind::Echo => unreachable!("echo handled before HTTP dispatch"),
        }
        .map_err(provider_execution_error)?;
        let continuation_id = if response_value
            .as_ref()
            .is_some_and(|value| value["status"] == "completed")
        {
            let output = response_value
                .as_ref()
                .and_then(|value| value["output"].as_array())
                .cloned()
                .unwrap_or_default();
            self.stage_continuation(
                &metadata.candidate_id,
                metadata.continuation_plan.as_ref(),
                payload,
                &output,
                &turn,
            )
            .map_err(provider_execution_error)?
        } else {
            None
        };
        for event in turn.events {
            emit_stream_item(ProviderStreamItem::Event { event }, permit, observer).await?;
        }
        emit_stream_item(
            crate::ProviderAdapterStreamItem {
                continuation_id,
                item: ProviderStreamItem::Completed {
                    profile: turn.profile,
                    model_profile: turn.model_profile,
                    provider_profile: turn.provider_profile,
                    provider: turn.provider,
                    model: turn.model,
                    response_id: turn.response_id,
                },
            },
            permit,
            observer,
        )
        .await
    }

    async fn stream_generation(
        &self,
        endpoint: &str,
        payload: Value,
        metadata: ProviderStreamMetadata<'_>,
        tool_names: ProviderToolNames,
        permit: &ExecutionPermit,
        observer: &mut dyn QuarantinedEffectObserver,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let streaming = payload.get("stream").and_then(Value::as_bool) == Some(true);
        let generation_deadline = if streaming {
            metadata.generation_deadline
        } else {
            metadata.generation_deadline.min(
                tokio::time::Instant::now()
                    + Duration::from_millis(
                        self.profile.timeout_ms.min(permit.obligations().timeout_ms),
                    ),
            )
        };
        let (response, secret) = tokio::time::timeout_at(
            generation_deadline,
            self.send_request(
                endpoint,
                Some(&payload),
                permit,
                generation_deadline,
                Some(observer),
            ),
        )
        .await
        .map_err(|_| generation_deadline_error())?
        .map_err(provider_execution_error)?;
        if !response.status().is_success() {
            let error = provider_status_error(&response);
            let diagnostic = tokio::time::timeout_at(
                generation_deadline,
                self.capture_http_error(
                    endpoint,
                    Some(redacted_image_payload(&payload)),
                    response,
                    secret,
                ),
            )
            .await
            .map_err(|_| generation_deadline_error())?
            .map_err(provider_execution_error)?;
            if let Some(rejection) = crate::features::feature_rejection(&diagnostic, Some(&payload))
            {
                return emit_stream_item(rejection, permit, observer).await;
            }
            if metadata.include_response_diagnostics {
                return emit_stream_item(
                    ProviderStreamItem::Diagnostic { diagnostic },
                    permit,
                    observer,
                )
                .await;
            }
            if !matches!(diagnostic.status, 502..=504)
                && let Some(failure) = classify_response_diagnostic(&diagnostic)
            {
                return Err(ExecutionError::ProviderRejected(failure));
            }
            return Err(provider_execution_error(error));
        }
        if !streaming {
            return tokio::time::timeout_at(
                generation_deadline,
                self.release_json_generation(
                    ProviderJsonGeneration {
                        response,
                        secrets: secret,
                        payload: &payload,
                    },
                    metadata,
                    tool_names,
                    permit,
                    observer,
                ),
            )
            .await
            .map_err(|_| generation_deadline_error())?;
        }
        let content_type_header = response.headers().get(reqwest::header::CONTENT_TYPE);
        let is_event_stream = content_type_header
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
        let is_untyped_codex_stream =
            self.profile.kind == ProviderKind::OpenAiCodex && content_type_header.is_none();
        if !is_event_stream && !is_untyped_codex_stream {
            if metadata.include_response_diagnostics {
                let diagnostic = tokio::time::timeout_at(
                    generation_deadline,
                    self.capture_http_error(endpoint, Some(payload), response, secret),
                )
                .await
                .map_err(|_| generation_deadline_error())?
                .map_err(provider_execution_error)?;
                return emit_stream_item(
                    ProviderStreamItem::Diagnostic { diagnostic },
                    permit,
                    observer,
                )
                .await;
            }
            return Err(provider_execution_error(ProviderError::Malformed(
                "streaming provider response is not text/event-stream".into(),
            )));
        }
        let limit = usize::try_from(permit.obligations().max_output_bytes)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let mut decoder = SseDecoder::default();
        let mut state = ProviderStreamState::new(self.profile.kind, tool_names);
        let mut continuation_events = Vec::new();
        let mut raw_bytes = 0_usize;
        let mut stream = response.bytes_stream();
        let mut emitter = ProviderStreamEmitter::new(permit, observer);
        let mut flush_interval = tokio::time::interval(STREAMED_MODEL_DELTA_FLUSH_INTERVAL);
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        flush_interval.tick().await;
        let generation_timeout = tokio::time::sleep_until(generation_deadline);
        tokio::pin!(generation_timeout);
        let stream_result = async {
            loop {
                let chunk = tokio::select! {
                    biased;
                    _ = &mut generation_timeout => {
                        return Err(generation_deadline_error());
                    }
                    chunk = stream.next() => chunk,
                    _ = flush_interval.tick(), if emitter.has_pending_model_delta() => {
                        emitter.flush().await?;
                        continue;
                    }
                };
                let Some(chunk) = chunk else {
                    break;
                };
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
                    if data == b"[DONE]" {
                        state.mark_done();
                        continue;
                    }
                    let mut value: Value = serde_json::from_slice(&data).map_err(|error| {
                        provider_execution_error(ProviderError::Malformed(format!(
                            "provider SSE data is not valid JSON: {error}"
                        )))
                    })?;
                    secret.redact_value(&mut value);
                    for event in state.ingest(value).map_err(provider_execution_error)? {
                        continuation_events.push(event.clone());
                        emitter.push(event).await?;
                    }
                }
            }
            decoder.finish().map_err(provider_execution_error)?;
            for event in state.finish().map_err(provider_execution_error)? {
                continuation_events.push(event.clone());
                emitter.push(event).await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = stream_result {
            emitter.flush().await?;
            return Err(error);
        }
        emitter.flush().await?;
        if tokio::time::Instant::now() >= generation_deadline {
            return Err(generation_deadline_error());
        }
        drop(emitter);
        let response_id = state.response_id().map(str::to_owned);
        let turn = ProviderTurn {
            profile: metadata.model_profile.into(),
            model_profile: metadata.model_profile.into(),
            provider_profile: self.profile.name.clone(),
            provider: self.profile.kind.as_str().into(),
            model: metadata.model.into(),
            response_id: response_id.clone(),
            events: continuation_events,
        };
        let continuation_id = self
            .stage_continuation(
                &metadata.candidate_id,
                metadata.continuation_plan.as_ref(),
                &payload,
                state.output_items(),
                &turn,
            )
            .map_err(provider_execution_error)?;
        tokio::time::timeout_at(
            generation_deadline,
            emit_stream_item(
                crate::ProviderAdapterStreamItem {
                    continuation_id,
                    item: ProviderStreamItem::Completed {
                        profile: metadata.model_profile.into(),
                        model_profile: metadata.model_profile.into(),
                        provider_profile: self.profile.name.clone(),
                        provider: self.profile.kind.as_str().into(),
                        model: metadata.model.into(),
                        response_id,
                    },
                },
                permit,
                observer,
            ),
        )
        .await
        .map_err(|_| generation_deadline_error())?
    }
}

pub(super) fn validate_serialized_provider_request(
    payload: &Value,
    body_len: usize,
) -> Result<(), ProviderError> {
    let redacted = redacted_image_payload(payload);
    let non_image_len = serde_json::to_vec(&redacted)
        .map_err(|error| ProviderError::Malformed(error.to_string()))?
        .len();
    let contains_images = payload_contains_image_data_url(payload);
    let body_limit = if contains_images {
        MAX_PROVIDER_REQUEST_WITH_IMAGES_BYTES
    } else {
        MAX_PROVIDER_REQUEST_BYTES
    };
    if body_len > body_limit || non_image_len > MAX_PROVIDER_REQUEST_BYTES {
        return Err(ProviderError::Rejected(
            colossus_contracts::ProviderFailure {
                reason: colossus_contracts::ProviderFailureReason::RequestTooLarge,
                http_status: None,
            },
        ));
    }
    Ok(())
}

fn payload_contains_image_data_url(value: &Value) -> bool {
    match value {
        Value::String(text) => text.starts_with("data:image/"),
        Value::Array(values) => values.iter().any(payload_contains_image_data_url),
        Value::Object(object) => object.values().any(payload_contains_image_data_url),
        _ => false,
    }
}

pub(super) fn redacted_image_payload(value: &Value) -> Value {
    match value {
        Value::String(text) if text.starts_with("data:image/") => {
            Value::String("[REDACTED_IMAGE_DATA_URL]".into())
        }
        Value::Array(values) => Value::Array(values.iter().map(redacted_image_payload).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        if key == "encrypted_content" {
                            Value::String("[REDACTED_PROVIDER_STATE]".into())
                        } else {
                            redacted_image_payload(value)
                        },
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn codex_credential_error(error: CodexAuthError) -> ProviderError {
    ProviderError::Credential(error.to_string())
}

async fn report_retry(
    observer: &mut Option<&mut dyn QuarantinedEffectObserver>,
    permit: &ExecutionPermit,
    attempt: u32,
    http_status: u32,
    state: colossus_contracts::ProviderRetryState,
    retry_at: Option<String>,
) -> Result<(), ProviderError> {
    if let Some(observer) = observer.as_deref_mut() {
        emit_stream_item(
            ProviderStreamItem::Retry {
                retry: colossus_contracts::ProviderRetry {
                    attempt,
                    max_retries: 5,
                    http_status,
                    state,
                    retry_at,
                },
            },
            permit,
            observer,
        )
        .await
        .map_err(|error| ProviderError::Progress(Box::new(error)))?;
    }
    Ok(())
}

fn provider_status_error(response: &reqwest::Response) -> ProviderError {
    ProviderError::Status {
        status: response.status().as_u16(),
        retry_after_ms: response_retry_after(response).and_then(RetryAfter::milliseconds),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryAfter {
    Delay(u64),
    ExceedsSupportedBound,
}

impl RetryAfter {
    fn milliseconds(self) -> Option<u64> {
        match self {
            Self::Delay(milliseconds) => Some(milliseconds),
            Self::ExceedsSupportedBound => None,
        }
    }
}

fn response_retry_after(response: &reqwest::Response) -> Option<RetryAfter> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after_ms)
}

fn parse_retry_after_ms(value: &str) -> Option<RetryAfter> {
    const MAX_RETRY_AFTER_MS: u64 = 24 * 60 * 60 * 1_000;

    let value = value.trim();
    let milliseconds = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let Some(milliseconds) = value
            .parse::<u64>()
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000))
        else {
            return Some(RetryAfter::ExceedsSupportedBound);
        };
        milliseconds
    } else {
        let deadline =
            OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822).ok()?;
        u64::try_from(
            (deadline - OffsetDateTime::now_utc())
                .whole_milliseconds()
                .max(0),
        )
        .ok()?
    };
    Some(if milliseconds <= MAX_RETRY_AFTER_MS {
        RetryAfter::Delay(milliseconds)
    } else {
        RetryAfter::ExceedsSupportedBound
    })
}

fn generation_metadata(
    input: &ProviderEffectInput,
) -> Result<(String, String, u64), ProviderError> {
    let model_profile = input
        .model_profile
        .as_ref()
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| ProviderError::Configuration("model profile is absent".into()))?;
    let model = input
        .model
        .as_ref()
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| ProviderError::Configuration("model identifier is absent".into()))?;
    let max_output_tokens = input
        .max_output_tokens
        .filter(|value| *value > 0)
        .ok_or_else(|| ProviderError::Configuration("model output limit is absent".into()))?;
    Ok((model_profile, model, max_output_tokens))
}

fn configure_compaction(
    payload: &mut Value,
    kind: ProviderKind,
    threshold: Option<u64>,
) -> Result<(), ProviderError> {
    if let Some(threshold) = threshold {
        if kind != ProviderKind::OpenAiResponses || threshold == 0 {
            return Err(ProviderError::Configuration(
                "server compaction requires public Responses and a positive threshold".into(),
            ));
        }
        payload["context_management"] =
            json!([{ "type": "compaction", "compact_threshold": threshold }]);
    }
    Ok(())
}

fn retain_opaque_redactions(value: &Value, secrets: &mut RequestSecrets) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "encrypted_content"
                    && let Some(secret) = value.as_str()
                {
                    secrets.retain(secret);
                } else {
                    retain_opaque_redactions(value, secrets);
                }
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| retain_opaque_redactions(value, secrets)),
        _ => {}
    }
}

#[cfg(test)]
mod retry_after_tests {
    use super::*;

    #[test]
    fn retry_after_accepts_seconds_and_http_dates_with_bounded_delays() {
        assert_eq!(parse_retry_after_ms(" 7 "), Some(RetryAfter::Delay(7_000)));
        assert_eq!(
            parse_retry_after_ms("86400"),
            Some(RetryAfter::Delay(86_400_000))
        );
        assert_eq!(
            parse_retry_after_ms("86401"),
            Some(RetryAfter::ExceedsSupportedBound)
        );
        assert_eq!(
            parse_retry_after_ms("18446744073709551616"),
            Some(RetryAfter::ExceedsSupportedBound)
        );
        assert_eq!(parse_retry_after_ms("invalid"), None);
        assert_eq!(
            parse_retry_after_ms("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(RetryAfter::Delay(0))
        );
        let future = (OffsetDateTime::now_utc() + time::Duration::seconds(60))
            .format(&time::format_description::well_known::Rfc2822)
            .expect("HTTP date");
        assert!(matches!(
            parse_retry_after_ms(&future),
            Some(RetryAfter::Delay(59_000..=60_000))
        ));
        let unsupported = (OffsetDateTime::now_utc() + time::Duration::days(2))
            .format(&time::format_description::well_known::Rfc2822)
            .expect("HTTP date");
        assert_eq!(
            parse_retry_after_ms(&unsupported),
            Some(RetryAfter::ExceedsSupportedBound)
        );
    }
}
