use super::*;

/// Maximum MCP pages accepted from one configured server discovery.
pub const MAX_MCP_PAGES: usize = 32;
/// Maximum allowlisted tools accepted across one configured server.
pub const MAX_MCP_TOOLS: usize = 1_024;
pub(super) const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
pub(super) const MCP_REQUEST_ID: i64 = 2;
pub(super) const INITIALIZE_REQUEST_ID: i64 = 1;
const MCP_TOOL_WILDCARD: &str = "*";

/// Strict configured MCP server collection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConfig {
    /// OAuth credential persistence selected for configured remote servers.
    #[serde(default)]
    pub oauth_credential_store: McpOAuthCredentialStoreKind,
    /// Exact configured servers by stable name.
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

/// OAuth credential persistence for remote MCP servers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthCredentialStoreKind {
    /// Select plaintext state for keyless deployments and protected storage otherwise.
    #[default]
    Auto,
    /// Store OAuth credentials in the operating-system credential store.
    Platform,
    /// Store OAuth credentials in an owner-private plaintext redb sidecar.
    PlaintextState,
    /// Store OAuth credentials in a separately encrypted redb database.
    EncryptedState,
}

/// Configured MCP transport.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    /// Launch one exact local process and communicate over standard input/output.
    #[default]
    Stdio,
    /// Connect to one exact MCP Streamable HTTP endpoint.
    StreamableHttp,
}

impl McpTransportKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::StreamableHttp => "streamable_http",
        }
    }
}

/// One secret-bearing HTTP header resolved only inside the permitted adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentialHeaderConfig {
    /// Optional authentication scheme prepended with one ASCII space, such as `Bearer`.
    #[serde(default)]
    pub scheme: Option<String>,
    /// Environment-backed credential reference.
    pub reference: String,
}

/// OAuth 2.1 authorization-code configuration for one remote server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOAuthConfig {
    /// Registered non-secret OAuth client identifier.
    pub client_id: String,
    /// Optional environment-backed confidential client secret.
    #[serde(default)]
    pub client_secret_reference: Option<String>,
    /// Exact loopback callback port registered with the authorization server.
    pub callback_port: u16,
    /// Explicit OAuth scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// One explicitly configured MCP server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerConfig {
    /// Transport selection. Omission preserves the legacy stdio configuration.
    #[serde(default)]
    pub transport: McpTransportKind,
    /// Exact absolute executable identity.
    #[serde(default)]
    pub command: PathBuf,
    /// Literal arguments passed without a shell.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional absolute or workspace-relative working directory.
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    /// Child environment name to `env:HOST_VARIABLE` credential reference.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Exact Streamable HTTP endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Non-secret literal HTTP headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Secret HTTP headers resolved from credential references after authorization.
    #[serde(default)]
    pub credential_headers: BTreeMap<String, McpCredentialHeaderConfig>,
    /// Permit explicitly configured remote servers to omit MCP session identifiers.
    #[serde(default)]
    pub allow_stateless: bool,
    /// Optional OAuth 2.1 authorization-code flow.
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
    /// Exact tools that may be discovered or invoked, or the sole wildcard `*`.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Configured research calls made for each research query.
    #[serde(default)]
    pub research_tools: Vec<McpResearchToolConfig>,
    /// Optional server-specific timeout bounded by sandbox policy.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Optional server-specific output cap bounded by sandbox policy.
    #[serde(default)]
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
    #[serde(default)]
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
    /// Whether this remote server may operate without MCP session identifiers.
    #[serde(default)]
    pub allow_stateless: bool,
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
    /// Optional advisory behavior hints from the untrusted server.
    pub annotations: Option<McpToolAnnotations>,
    /// Valid JSON object schema for arguments.
    pub input_schema: Value,
    /// SHA-256 of the canonical schema sent with an invocation request.
    pub schema_sha256: String,
}

/// Bounded advisory MCP tool annotations safe for model-assisted review.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpToolAnnotations {
    /// Optional human title supplied by the untrusted server.
    pub title: Option<String>,
    /// Advisory claim that the tool does not modify its environment.
    pub read_only_hint: Option<bool>,
    /// Advisory claim that the tool may perform destructive updates.
    pub destructive_hint: Option<bool>,
    /// Advisory claim that repeated identical calls have no additional effect.
    pub idempotent_hint: Option<bool>,
    /// Advisory claim that the tool may interact with external entities.
    pub open_world_hint: Option<bool>,
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

/// Safe OAuth credential status for one configured MCP server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthStatus {
    /// Configured server name.
    pub server: String,
    /// Whether OAuth is configured for this server.
    pub configured: bool,
    /// Whether a persisted access token is present.
    pub authenticated: bool,
}

/// One interactive OAuth authorization session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthLogin {
    /// Configured server name.
    pub server: String,
    /// Authorization URL to open or present to the operator.
    pub authorization_url: String,
    /// Exact loopback callback URL registered for the flow.
    pub callback_url: String,
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
        /// Optional bounded description from fresh discovery.
        description: Option<String>,
        /// Optional advisory annotations from fresh discovery.
        annotations: Option<McpToolAnnotations>,
        /// Strict JSON object arguments.
        arguments: Value,
        /// Exact discovered input schema, bound into policy and permit hashing.
        input_schema: Box<Value>,
        /// SHA-256 of the exact discovered input schema.
        schema_sha256: String,
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
    pub(super) transport: McpTransportKind,
    pub(super) cwd: Option<PathBuf>,
    pub(super) args: Vec<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) url: Option<String>,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) credential_headers: BTreeMap<String, McpCredentialHeaderConfig>,
    #[serde(default)]
    pub(super) allow_stateless: bool,
    pub(super) oauth: Option<McpOAuthConfig>,
    pub(super) timeout_ms: Option<u64>,
    pub(super) max_output_bytes: Option<u64>,
    pub(super) provenance: Option<Value>,
}

#[derive(Clone, Debug)]
pub(super) enum ToolAllowlist {
    All,
    Explicit(BTreeSet<String>),
}

impl ToolAllowlist {
    pub(super) fn from_config(server: &str, tools: &[String]) -> Result<Self, McpError> {
        if tools.is_empty() {
            return Err(McpError::Invalid(format!(
                "server {server} must configure at least one allowed tool"
            )));
        }
        if tools.iter().any(|tool| tool == MCP_TOOL_WILDCARD) {
            if tools.len() != 1 {
                return Err(McpError::Invalid(format!(
                    "server {server} tool wildcard must be the only allowedTools entry"
                )));
            }
            return Ok(Self::All);
        }
        let mut explicit = BTreeSet::new();
        for tool in tools {
            validate_name(tool, "tool")?;
            if !explicit.insert(tool.clone()) {
                return Err(McpError::Invalid(format!(
                    "server {server} contains duplicate allowed tool {tool}"
                )));
            }
        }
        Ok(Self::Explicit(explicit))
    }

    pub(super) fn allows(&self, tool: &str) -> bool {
        match self {
            Self::All => true,
            Self::Explicit(tools) => tools.contains(tool),
        }
    }

    pub(super) fn summary(&self) -> Vec<String> {
        match self {
            Self::All => vec![MCP_TOOL_WILDCARD.into()],
            Self::Explicit(tools) => tools.iter().cloned().collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ConfiguredServer {
    pub(super) name: String,
    pub(super) transport: McpTransportKind,
    pub(super) command: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) cwd: Option<PathBuf>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) url: Option<String>,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) credential_headers: BTreeMap<String, McpCredentialHeaderConfig>,
    pub(super) allow_stateless: bool,
    pub(super) oauth: Option<McpOAuthConfig>,
    pub(super) allowed_tools: ToolAllowlist,
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
    /// Interactive OAuth authorization is required.
    #[error("MCP authorization required: {0}")]
    AuthorizationRequired(String),
    /// OAuth lifecycle failure with secret-free diagnostics.
    #[error("MCP OAuth failure: {0}")]
    OAuth(String),
}

/// Resource declarations and ceilings used while validating configured MCP servers.
#[derive(Clone, Copy, Debug)]
pub struct McpValidationContext<'a> {
    /// Potential resource authority selected by the configured execution backend.
    /// Runtime acknowledgement is enforced separately before any effect permit is minted.
    pub resource_authority: ResourceAuthority,
    /// Exact executable identities declared by a confined sandbox.
    pub sandbox_executables: &'a [PathBuf],
    /// Filesystem roots declared by a confined sandbox.
    pub sandbox_filesystem: &'a [FilesystemGrant],
    /// Environment names declared by a confined sandbox.
    pub sandbox_environment: &'a [String],
    /// Maximum operation timeout enforced by the runtime.
    pub sandbox_timeout_ms: u64,
    /// Maximum output size enforced by the runtime.
    pub sandbox_max_output_bytes: u64,
}

/// Validate strict MCP config against sandbox identities and bounds.
pub fn validate_config(
    config: &McpConfig,
    workspace: &Path,
    context: McpValidationContext<'_>,
) -> Result<(), McpError> {
    let ambient_resources = context.resource_authority == ResourceAuthority::Ambient;
    if config.servers.len() > 64 {
        return Err(McpError::Invalid(
            "at most 64 servers may be configured".into(),
        ));
    }
    let allowed_environment = context.sandbox_environment.iter().collect::<BTreeSet<_>>();
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
        match server.transport {
            McpTransportKind::Stdio => validate_stdio_server(
                name,
                server,
                workspace,
                ambient_resources,
                context.sandbox_executables,
                context.sandbox_filesystem,
                &allowed_environment,
            )?,
            McpTransportKind::StreamableHttp => validate_streamable_http_server(
                name,
                server,
                ambient_resources,
                &allowed_environment,
            )?,
        }
        if server.allowed_tools.len() > MAX_MCP_TOOLS {
            return Err(McpError::Invalid(format!(
                "server {name} allowlist exceeds {MAX_MCP_TOOLS} tools"
            )));
        }
        let tools = ToolAllowlist::from_config(name, &server.allowed_tools)?;
        if server.research_tools.len() > 64 {
            return Err(McpError::Invalid(format!(
                "server {name} configures too many research tools"
            )));
        }
        for research in &server.research_tools {
            validate_name(&research.tool, "research tool")?;
            if !tools.allows(&research.tool) || !research.arguments.is_object() {
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
            .is_some_and(|value| value == 0 || value > context.sandbox_timeout_ms)
            || server
                .max_output_bytes
                .is_some_and(|value| value < 1024 || value > context.sandbox_max_output_bytes)
        {
            return Err(McpError::Invalid(format!(
                "server {name} timeout or output cap exceeds sandbox policy"
            )));
        }
    }
    Ok(())
}

fn validate_stdio_server(
    name: &str,
    server: &McpServerConfig,
    workspace: &Path,
    ambient_resources: bool,
    sandbox_executables: &[PathBuf],
    sandbox_filesystem: &[FilesystemGrant],
    allowed_environment: &BTreeSet<&String>,
) -> Result<(), McpError> {
    if server.url.is_some()
        || !server.headers.is_empty()
        || !server.credential_headers.is_empty()
        || server.allow_stateless
        || server.oauth.is_some()
    {
        return Err(McpError::Invalid(format!(
            "stdio server {name} cannot configure HTTP or OAuth fields"
        )));
    }
    if !server.command.is_absolute()
        || (!ambient_resources
            && !sandbox_executables
                .iter()
                .any(|value| value == &server.command))
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
    if !ambient_resources && !cwd_allowed {
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
            || (!ambient_resources && !allowed_environment.contains(child_name))
            || !valid_credential_reference(reference)
        {
            return Err(McpError::Invalid(format!(
                "server {name} environment requires allowed child names and credential references"
            )));
        }
    }
    Ok(())
}

fn validate_streamable_http_server(
    name: &str,
    server: &McpServerConfig,
    ambient_resources: bool,
    allowed_environment: &BTreeSet<&String>,
) -> Result<(), McpError> {
    if !server.command.as_os_str().is_empty()
        || !server.args.is_empty()
        || server.working_directory.is_some()
        || !server.environment.is_empty()
        || server.effect_action_prefix.is_some()
        || server.provenance.is_some()
    {
        return Err(McpError::Invalid(format!(
            "Streamable HTTP server {name} cannot configure stdio or pack fields"
        )));
    }
    let raw_url = server.url.as_deref().ok_or_else(|| {
        McpError::Invalid(format!(
            "Streamable HTTP server {name} requires an exact URL"
        ))
    })?;
    let url = url::Url::parse(raw_url)
        .map_err(|error| McpError::Invalid(format!("server {name} URL is invalid: {error}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(McpError::Invalid(format!(
            "server {name} URL requires HTTP(S), a host, and no credentials, query, or fragment"
        )));
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || colossus_network::parse_host_ip(host).is_some_and(|address| address.is_loopback())
    });
    if !ambient_resources && url.scheme() != "https" && !loopback {
        return Err(McpError::Invalid(format!(
            "non-loopback Streamable HTTP server {name} requires HTTPS"
        )));
    }
    if server.headers.len() > 64 || server.credential_headers.len() > 16 {
        return Err(McpError::Invalid(format!(
            "server {name} configures too many HTTP headers"
        )));
    }
    let mut header_names = BTreeSet::new();
    for (header, value) in &server.headers {
        validate_header_name(name, header, false)?;
        validate_header_value(name, value)?;
        if !header_names.insert(header.to_ascii_lowercase()) {
            return Err(McpError::Invalid(format!(
                "server {name} contains duplicate HTTP header {header}"
            )));
        }
    }
    for (header, credential) in &server.credential_headers {
        validate_header_name(name, header, true)?;
        if !header_names.insert(header.to_ascii_lowercase()) {
            return Err(McpError::Invalid(format!(
                "server {name} contains duplicate HTTP header {header}"
            )));
        }
        let reference = credential_reference(&credential.reference).ok_or_else(|| {
            McpError::Invalid(format!(
                "server {name} credential header {header} requires a credential reference"
            ))
        })?;
        if let CredentialReferenceKind::Environment(variable) = reference
            && !ambient_resources
            && !allowed_environment
                .iter()
                .any(|allowed| allowed.as_str() == variable)
        {
            return Err(McpError::Invalid(format!(
                "server {name} credential header {header} requires sandbox environment grant {variable}"
            )));
        }
        if let Some(scheme) = credential.scheme.as_deref()
            && (scheme.is_empty()
                || scheme.len() > 64
                || !scheme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        {
            return Err(McpError::Invalid(format!(
                "server {name} credential header {header} has an invalid scheme"
            )));
        }
    }
    if server.oauth.is_some() && !server.credential_headers.is_empty() {
        return Err(McpError::Invalid(format!(
            "server {name} cannot combine OAuth with credentialHeaders"
        )));
    }
    if let Some(oauth) = server.oauth.as_ref() {
        validate_oauth(name, oauth, ambient_resources, allowed_environment)?;
    }
    Ok(())
}

fn validate_header_name(server: &str, name: &str, credential: bool) -> Result<(), McpError> {
    let normalized = name.to_ascii_lowercase();
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || matches!(
            normalized.as_str(),
            "host"
                | "accept"
                | "content-type"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "proxy-connection"
                | "proxy-authorization"
                | "last-event-id"
                | "te"
                | "trailer"
                | "upgrade"
                | "cookie"
                | "set-cookie"
        )
        || normalized.starts_with("mcp-")
        || (!credential
            && matches!(
                normalized.as_str(),
                "authorization" | "api-key" | "x-api-key" | "x-auth-token" | "x-access-token"
            ))
    {
        return Err(McpError::Invalid(format!(
            "server {server} contains an unsafe HTTP header {name}"
        )));
    }
    Ok(())
}

fn validate_header_value(server: &str, value: &str) -> Result<(), McpError> {
    if value.is_empty()
        || value.len() > 8 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(McpError::Invalid(format!(
            "server {server} contains an invalid HTTP header value"
        )));
    }
    Ok(())
}

fn validate_oauth(
    server: &str,
    oauth: &McpOAuthConfig,
    ambient_resources: bool,
    allowed_environment: &BTreeSet<&String>,
) -> Result<(), McpError> {
    if oauth.client_id.is_empty()
        || oauth.client_id.len() > 1_024
        || oauth.client_id.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(McpError::Invalid(format!(
            "server {server} OAuth clientId must be a bounded control-free value"
        )));
    }
    if oauth.callback_port == 0 || oauth.scopes.len() > 32 {
        return Err(McpError::Invalid(format!(
            "server {server} OAuth requires a callback port and at most 32 scopes"
        )));
    }
    let mut scopes = BTreeSet::new();
    for scope in &oauth.scopes {
        if scope.is_empty()
            || scope.len() > 256
            || scope
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            || !scopes.insert(scope)
        {
            return Err(McpError::Invalid(format!(
                "server {server} OAuth scopes must be unique bounded tokens"
            )));
        }
    }
    if let Some(reference) = oauth.client_secret_reference.as_deref() {
        let reference = credential_reference(reference).ok_or_else(|| {
            McpError::Invalid(format!(
                "server {server} OAuth client secret requires a credential reference"
            ))
        })?;
        if let CredentialReferenceKind::Environment(variable) = reference
            && !ambient_resources
            && !allowed_environment
                .iter()
                .any(|allowed| allowed.as_str() == variable)
        {
            return Err(McpError::Invalid(format!(
                "server {server} OAuth client secret requires sandbox environment grant {variable}"
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

#[derive(Clone, Copy)]
pub(super) enum CredentialReferenceKind<'a> {
    Environment(&'a str),
    Host,
}

pub(super) fn credential_reference(value: &str) -> Option<CredentialReferenceKind<'_>> {
    if let Some(name) = environment_reference(value) {
        return Some(CredentialReferenceKind::Environment(name));
    }
    let identifier = value.strip_prefix("host:")?;
    (!identifier.is_empty()
        && identifier.len() <= 128
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then_some(CredentialReferenceKind::Host)
}

fn valid_credential_reference(value: &str) -> bool {
    credential_reference(value).is_some()
}
