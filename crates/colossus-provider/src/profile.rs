use super::*;

/// Supported first-party provider adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Deterministic credential-free smoke provider.
    Echo,
    /// OpenAI Responses API.
    OpenAiResponses,
    /// OpenAI Responses API authenticated with a Codex/ChatGPT subscription.
    OpenAiCodex,
    /// OpenAI-compatible Chat Completions API.
    OpenAiCompatible,
}

impl ProviderKind {
    /// Stable adapter label used in normalized results.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "echo",
            Self::OpenAiResponses => "openai_responses",
            Self::OpenAiCodex => "openai_codex",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Exact effect action for a generation request.
    pub fn generation_action(self) -> &'static str {
        match self {
            Self::Echo => "provider.echo",
            Self::OpenAiResponses => "provider.openai.responses",
            Self::OpenAiCodex => "provider.openai.codex",
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
        base_url: Option<String>,
        credential_reference: Option<String>,
        timeout_ms: u64,
    ) -> Result<Self, ProviderError> {
        let name = name.into();
        if name.is_empty() || timeout_ms == 0 {
            return Err(ProviderError::Configuration(
                "provider name and timeout must be nonempty/nonzero".into(),
            ));
        }
        if let Some(reference) = credential_reference.as_deref()
            && !valid_credential_reference(reference)
        {
            return Err(ProviderError::Configuration(
                "provider credentials must use env:VARIABLE, host:IDENTIFIER, or codex:default"
                    .into(),
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
            ProviderKind::OpenAiCodex => {
                if base_url.is_some()
                    || credential_reference.as_deref() != Some(CODEX_CREDENTIAL_REFERENCE)
                {
                    return Err(ProviderError::Configuration(
                        "open_ai_codex profiles require credentialReference codex:default and do not allow baseUrl"
                            .into(),
                    ));
                }
                Some(CODEX_API_BASE_URL.into())
            }
        };
        Ok(Self {
            name,
            kind,
            base_url,
            credential_reference,
            timeout_ms,
        })
    }

    /// Exact endpoint for generation.
    pub fn generation_endpoint(&self) -> Result<String, ProviderError> {
        match self.kind {
            ProviderKind::Echo => Ok(format!("provider:{}", self.name)),
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => self.endpoint("responses"),
            ProviderKind::OpenAiCompatible => self.endpoint("chat/completions"),
        }
    }

    /// Exact endpoint for model catalog diagnostics.
    pub fn models_endpoint(&self) -> Result<Option<String>, ProviderError> {
        match self.kind {
            ProviderKind::Echo => Ok(None),
            ProviderKind::OpenAiCodex => self
                .endpoint(&format!("models?client_version={CODEX_PROTOCOL_VERSION}"))
                .map(Some),
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

    /// Fixed secondary origins required for adapter-owned authentication.
    pub fn authentication_origins(&self) -> &'static [&'static str] {
        match self.kind {
            ProviderKind::OpenAiCodex => &[CODEX_AUTH_ORIGIN],
            ProviderKind::Echo | ProviderKind::OpenAiResponses | ProviderKind::OpenAiCompatible => {
                &[]
            }
        }
    }

    fn endpoint(&self, suffix: &str) -> Result<String, ProviderError> {
        self.base_url
            .as_ref()
            .map(|base| format!("{base}/{suffix}"))
            .ok_or_else(|| ProviderError::Configuration("provider has no base URL".into()))
    }
}

/// One explicit model profile routed through a provider connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelProfile {
    /// Stable model-profile name.
    pub name: String,
    /// Referenced provider connection profile.
    pub provider_profile: String,
    /// Exact provider model identifier.
    pub model: String,
    /// Declared and effective token limits.
    pub limits: ModelLimits,
    /// Explicit request-shaping capabilities.
    pub capabilities: ModelCapabilities,
    /// Optional configured reasoning effort.
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl ModelProfile {
    /// Validate one explicit model profile and derive its conservative input budget.
    pub fn new(
        name: impl Into<String>,
        provider_profile: impl Into<String>,
        model: impl Into<String>,
        context_window_tokens: u64,
        max_output_tokens: u64,
        capabilities: ModelCapabilities,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<Self, ProviderError> {
        let name = name.into();
        let provider_profile = provider_profile.into();
        let model = model.into();
        let safety_margin_tokens = context_window_tokens.div_ceil(10).max(512);
        let input_budget_tokens = context_window_tokens
            .checked_sub(max_output_tokens)
            .and_then(|remaining| remaining.checked_sub(safety_margin_tokens))
            .ok_or_else(|| {
                ProviderError::Configuration(format!(
                    "model profile {name} output and safety reservations exhaust its context window"
                ))
            })?;
        if name.is_empty()
            || provider_profile.is_empty()
            || model.is_empty()
            || context_window_tokens < 1_024
            || max_output_tokens == 0
            || input_budget_tokens == 0
        {
            return Err(ProviderError::Configuration(
                "model profile names, model identifiers, and limits must be nonempty; context windows must be at least 1024"
                    .into(),
            ));
        }
        Ok(Self {
            name,
            provider_profile,
            model,
            limits: ModelLimits {
                context_window_tokens,
                max_output_tokens,
                safety_margin_tokens,
                input_budget_tokens,
            },
            capabilities,
            reasoning_effort,
        })
    }
}
