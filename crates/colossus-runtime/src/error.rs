use super::*;

/// Runtime construction or application failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Strict configuration failed.
    #[error("configuration error: {0}")]
    Config(String),
    /// Filesystem read/write failed before runtime composition.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Journal/key adapter failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Effect authorization or execution failed.
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    /// Provider configuration or normalized output failed.
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// Search profile, route, transport, or normalization failed.
    #[error(transparent)]
    Search(#[from] SearchAdapterError),
    /// Provider-neutral search port failed after runtime composition.
    #[error(transparent)]
    SearchPort(#[from] SearchError),
    /// Agent application loop failed.
    #[error(transparent)]
    Agent(#[from] AgentError),
    /// Context preparation or snapshot lifecycle failed.
    #[error(transparent)]
    Context(#[from] ContextError),
    /// Active tool catalog is invalid.
    #[error(transparent)]
    ToolCatalog(#[from] ToolCatalogError),
    /// Configured MCP adapter or protocol contract failed.
    #[error(transparent)]
    Mcp(#[from] McpError),
    /// Offline release-bundle contract failed.
    #[error(transparent)]
    Bundle(#[from] BundleError),
    /// Workflow validation or execution failed.
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
}

impl RuntimeError {
    /// Return explicitly released provider response evidence from a failed local run.
    pub fn provider_response_diagnostic(&self) -> Option<&ProviderResponseDiagnostic> {
        match self {
            Self::Agent(AgentError::Provider(error)) => error.response_diagnostic(),
            _ => None,
        }
    }

    /// Return whether this failure may follow an effect whose outcome is unconfirmed.
    pub fn outcome_unknown(&self) -> bool {
        match self {
            Self::Store(StoreError::OutcomeUnknown(_))
            | Self::SearchPort(SearchError::OutcomeUnknown(_)) => true,
            Self::Gateway(GatewayError::OutcomeUnknown(_))
            | Self::Gateway(GatewayError::Journal(StoreError::OutcomeUnknown(_))) => true,
            Self::Agent(error) => error.outcome_unknown(),
            Self::Context(ContextError::Store(StoreError::OutcomeUnknown(_)))
            | Self::Context(ContextError::Provider(ModelProviderError::OutcomeUnknown(_))) => true,
            Self::Config(_)
            | Self::Io(_)
            | Self::Store(_)
            | Self::Gateway(_)
            | Self::Provider(_)
            | Self::Search(_)
            | Self::SearchPort(_)
            | Self::Context(_)
            | Self::ToolCatalog(_)
            | Self::Mcp(_)
            | Self::Bundle(_)
            | Self::Workflow(_) => false,
        }
    }
}

pub(super) fn explicit_secret(variable: &str) -> Result<[u8; 32], RuntimeError> {
    let encoded = std::env::var(variable)
        .map_err(|_| RuntimeError::Config(format!("environment variable {variable} is unset")))?;
    let decoded = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| {
            RuntimeError::Config(format!(
                "environment variable {variable} must be hex or base64"
            ))
        })?;
    decoded.try_into().map_err(|_| {
        RuntimeError::Config(format!(
            "environment variable {variable} must decode to exactly 32 bytes"
        ))
    })
}

pub(super) fn read_optional(path: Option<&PathBuf>) -> Result<Option<Vec<u8>>, RuntimeError> {
    path.map(fs::read).transpose().map_err(Into::into)
}
