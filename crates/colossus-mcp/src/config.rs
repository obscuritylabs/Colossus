use super::*;

/// Maximum MCP pages accepted from one configured server discovery.
pub const MAX_MCP_PAGES: usize = 32;
/// Maximum allowlisted tools accepted across one configured server.
pub const MAX_MCP_TOOLS: usize = 1_024;
pub(super) const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
pub(super) const MCP_REQUEST_ID: i64 = 2;
pub(super) const INITIALIZE_REQUEST_ID: i64 = 1;

/// Strict configured MCP server collection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConfig {
    /// Exact configured stdio servers by stable name.
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

/// One explicitly configured stdio MCP server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    /// Exact absolute executable identity.
    pub command: PathBuf,
    /// Literal arguments passed without a shell.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional absolute or workspace-relative working directory.
    pub working_directory: Option<PathBuf>,
    /// Child environment name to `env:HOST_VARIABLE` credential reference.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Exact tools that may be discovered or invoked. Empty means none.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Configured research calls made for each research query.
    #[serde(default)]
    pub research_tools: Vec<McpResearchToolConfig>,
    /// Optional server-specific timeout bounded by sandbox policy.
    pub timeout_ms: Option<u64>,
    /// Optional server-specific output cap bounded by sandbox policy.
    pub max_output_bytes: Option<u64>,
    /// Runtime-only action prefix for verified pack-provided servers.
    #[serde(skip)]
    pub effect_action_prefix: Option<String>,
    /// Runtime-only verified pack provenance disclosed to policy.
    #[serde(skip)]
    pub provenance: Option<Value>,
}

/// One allowlisted MCP tool template for the research collector.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpResearchToolConfig {
    /// Exact allowlisted MCP tool name.
    pub tool: String,
    /// Optional bounded source title.
    pub title: Option<String>,
    /// JSON object whose string values may contain `{query}`.
    #[serde(default = "empty_object")]
    pub arguments: Value,
}

fn empty_object() -> Value {
    json!({})
}

/// Non-sensitive configured server metadata safe for a model or terminal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerSummary {
    /// Stable configuration name.
    pub name: String,
    /// Fixed transport implemented by this adapter.
    pub transport: String,
    /// Exact tool allowlist.
    pub allowed_tools: Vec<String>,
    /// Tool names configured for research collection.
    pub research_tools: Vec<String>,
}

/// Safe, allowlist-filtered MCP tool description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolSummary {
    /// Configured server name.
    pub server: String,
    /// Exact MCP tool name.
    pub name: String,
    /// Optional human title from the untrusted server.
    pub title: Option<String>,
    /// Optional bounded description from the untrusted server.
    pub description: Option<String>,
    /// Valid JSON object schema for arguments.
    pub input_schema: Value,
    /// SHA-256 of the canonical schema sent with an invocation request.
    pub schema_sha256: String,
}

/// One allowlist-filtered discovery page released from quarantine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolsPage {
    /// Configured server name.
    pub server: String,
    /// Filtered tools in deterministic name order.
    pub tools: Vec<McpToolSummary>,
    /// Opaque server cursor for the next separately authorized page.
    pub next_cursor: Option<String>,
}

/// One typed MCP tool result released from quarantine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCallOutput {
    /// Configured server name.
    pub server: String,
    /// Exact allowlisted tool name.
    pub tool: String,
    /// Official Rust SDK result model after hard secret redaction.
    pub result: CallToolResult,
}

/// A configured research call with resolved runtime metadata but no secret values.
#[derive(Clone, Debug, PartialEq)]
pub struct McpResearchCall {
    /// Configured server name.
    pub server: String,
    /// Exact allowlisted tool.
    pub tool: String,
    /// Bounded source title.
    pub title: String,
    /// Templated JSON arguments.
    pub arguments: Value,
}

/// One runtime-controlled MCP operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpOperation {
    /// Discover one page of tools.
    ListTools {
        /// Configured server name.
        server: String,
        /// Optional opaque pagination cursor.
        cursor: Option<String>,
    },
    /// Invoke one exact allowlisted tool against a previously discovered schema.
    CallTool {
        /// Configured server name.
        server: String,
        /// Exact tool name.
        tool: String,
        /// Strict JSON object arguments.
        arguments: Value,
        /// Exact discovered input schema, bound into policy and permit hashing.
        input_schema: Value,
    },
}

impl McpOperation {
    /// Effect identity for policy and auditing.
    pub fn action(&self) -> &'static str {
        match self {
            Self::ListTools { .. } => "mcp.tools",
            Self::CallTool { .. } => "mcp.call",
        }
    }

    pub(super) fn server(&self) -> &str {
        match self {
            Self::ListTools { server, .. } | Self::CallTool { server, .. } => server,
        }
    }

    pub(super) fn is_call(&self) -> bool {
        matches!(self, Self::CallTool { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct McpEffectInput {
    pub(super) operation: McpOperation,
    pub(super) cwd: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) timeout_ms: Option<u64>,
    pub(super) max_output_bytes: Option<u64>,
    pub(super) provenance: Option<Value>,
}

#[derive(Clone, Debug)]
pub(super) struct ConfiguredServer {
    pub(super) name: String,
    pub(super) command: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) cwd: PathBuf,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) allowed_tools: BTreeSet<String>,
    pub(super) research_tools: Vec<McpResearchToolConfig>,
    pub(super) timeout_ms: Option<u64>,
    pub(super) max_output_bytes: Option<u64>,
    pub(super) effect_action_prefix: Option<String>,
    pub(super) provenance: Option<Value>,
}

/// MCP configuration or protocol validation failure.
#[derive(Debug, Error)]
pub enum McpError {
    /// Invalid configuration or caller input.
    #[error("invalid MCP configuration: {0}")]
    Invalid(String),
    /// Configured server is absent.
    #[error("MCP server is not configured: {0}")]
    UnknownServer(String),
    /// Tool is not on the exact configured allowlist.
    #[error("MCP tool is not allowlisted: {0}")]
    ToolDenied(String),
    /// Argument object does not match the discovered tool schema.
    #[error("invalid MCP tool arguments: {0}")]
    InvalidArguments(String),
}

/// Validate strict MCP config against sandbox identities and bounds.
pub fn validate_config(
    config: &McpConfig,
    workspace: &Path,
    sandbox_executables: &[PathBuf],
    sandbox_filesystem: &[FilesystemGrant],
    sandbox_environment: &[String],
    sandbox_timeout_ms: u64,
    sandbox_max_output_bytes: u64,
) -> Result<(), McpError> {
    if config.servers.len() > 64 {
        return Err(McpError::Invalid(
            "at most 64 servers may be configured".into(),
        ));
    }
    let allowed_environment = sandbox_environment.iter().collect::<BTreeSet<_>>();
    for (name, server) in &config.servers {
        validate_name(name, "server")?;
        if server.effect_action_prefix.as_ref().is_some_and(|prefix| {
            !prefix.starts_with("pack.mcp.")
                || prefix.len() > 384
                || !prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        }) {
            return Err(McpError::Invalid(format!(
                "server {name} has an invalid runtime effect action prefix"
            )));
        }
        if !server.command.is_absolute()
            || !sandbox_executables
                .iter()
                .any(|value| value == &server.command)
        {
            return Err(McpError::Invalid(format!(
                "server {name} command must be an exact absolute sandbox executable"
            )));
        }
        if server.args.len() > 256
            || server
                .args
                .iter()
                .any(|value| value.len() > 64 * 1024 || value.contains('\0'))
        {
            return Err(McpError::Invalid(format!(
                "server {name} arguments exceed process bounds"
            )));
        }
        if server
            .working_directory
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(McpError::Invalid(format!(
                "server {name} workingDirectory is empty"
            )));
        }
        let cwd = server.working_directory.as_ref().map_or_else(
            || Ok(workspace.to_owned()),
            |path| resolve_path(workspace, path),
        )?;
        let cwd_allowed = sandbox_filesystem.iter().any(|grant| {
            matches!(grant.mode.as_str(), "read" | "write")
                && fs::canonicalize(&grant.root).is_ok_and(|root| cwd.starts_with(root))
        });
        if !cwd_allowed {
            return Err(McpError::Invalid(format!(
                "server {name} working directory requires a containing sandbox read or write grant"
            )));
        }
        if server.environment.len() > 128 {
            return Err(McpError::Invalid(format!(
                "server {name} environment exceeds 128 entries"
            )));
        }
        for (child_name, reference) in &server.environment {
            if !valid_environment_name(child_name)
                || !allowed_environment.contains(child_name)
                || environment_reference(reference).is_none()
            {
                return Err(McpError::Invalid(format!(
                    "server {name} environment requires allowed child names and env:VARIABLE references"
                )));
            }
        }
        if server.allowed_tools.len() > MAX_MCP_TOOLS {
            return Err(McpError::Invalid(format!(
                "server {name} allowlist exceeds {MAX_MCP_TOOLS} tools"
            )));
        }
        let mut tools = BTreeSet::new();
        for tool in &server.allowed_tools {
            validate_name(tool, "tool")?;
            if !tools.insert(tool) {
                return Err(McpError::Invalid(format!(
                    "server {name} contains duplicate allowed tool {tool}"
                )));
            }
        }
        if server.research_tools.len() > 64 {
            return Err(McpError::Invalid(format!(
                "server {name} configures too many research tools"
            )));
        }
        for research in &server.research_tools {
            if !tools.contains(&research.tool) || !research.arguments.is_object() {
                return Err(McpError::Invalid(format!(
                    "server {name} research tool {} must be allowlisted and use object arguments",
                    research.tool
                )));
            }
            if research
                .title
                .as_ref()
                .is_some_and(|title| title.len() > 8 * 1024)
            {
                return Err(McpError::Invalid(format!(
                    "server {name} research title exceeds its bound"
                )));
            }
        }
        if server
            .timeout_ms
            .is_some_and(|value| value == 0 || value > sandbox_timeout_ms)
            || server
                .max_output_bytes
                .is_some_and(|value| value < 1024 || value > sandbox_max_output_bytes)
        {
            return Err(McpError::Invalid(format!(
                "server {name} timeout or output cap exceeds sandbox policy"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_name(value: &str, kind: &str) -> Result<(), McpError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(McpError::Invalid(format!(
            "{kind} name must use 1..=128 ASCII letters, digits, dot, underscore, or hyphen"
        )));
    }
    Ok(())
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn environment_reference(value: &str) -> Option<&str> {
    let name = value.strip_prefix("env:")?;
    valid_environment_name(name).then_some(name)
}
