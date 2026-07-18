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
    /// Capability-pack or offline-bundle contract failed.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// Workflow validation or execution failed.
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
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
