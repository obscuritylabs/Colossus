use super::*;

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
                "provider credentials must use a valid env:VARIABLE or host:IDENTIFIER reference"
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
