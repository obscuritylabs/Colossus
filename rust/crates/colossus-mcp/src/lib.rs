//! Configured Model Context Protocol adapters executed through Colossus permits.

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, ExecutionContext, FilesystemGrant,
    QuarantinedEffectResult,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit, effect_request};
use colossus_sandbox::{ProcessSpec, SandboxProcessExecutor};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

/// Maximum MCP pages accepted from one configured server discovery.
pub const MAX_MCP_PAGES: usize = 32;
/// Maximum allowlisted tools accepted across one configured server.
pub const MAX_MCP_TOOLS: usize = 1_024;
const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
const MCP_REQUEST_ID: i64 = 2;
const INITIALIZE_REQUEST_ID: i64 = 1;

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

    fn server(&self) -> &str {
        match self {
            Self::ListTools { server, .. } | Self::CallTool { server, .. } => server,
        }
    }

    fn is_call(&self) -> bool {
        matches!(self, Self::CallTool { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpEffectInput {
    operation: McpOperation,
    cwd: PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
struct ConfiguredServer {
    name: String,
    command: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    allowed_tools: BTreeSet<String>,
    research_tools: Vec<McpResearchToolConfig>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
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

fn validate_name(value: &str, kind: &str) -> Result<(), McpError> {
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

fn environment_reference(value: &str) -> Option<&str> {
    let name = value.strip_prefix("env:")?;
    valid_environment_name(name).then_some(name)
}

/// Permit-bound configured MCP adapter.
pub struct McpExecutor {
    servers: BTreeMap<String, ConfiguredServer>,
    process: Arc<SandboxProcessExecutor>,
}

impl McpExecutor {
    /// Resolve configured paths and construct a permit-bound adapter.
    pub fn new(
        config: &McpConfig,
        workspace: &Path,
        sandbox_backend: &str,
        process: Arc<SandboxProcessExecutor>,
    ) -> Result<Self, McpError> {
        let mut servers = BTreeMap::new();
        for (name, server) in &config.servers {
            let command = if sandbox_backend == "oci" {
                server.command.clone()
            } else {
                fs::canonicalize(&server.command).map_err(|error| {
                    McpError::Invalid(format!("server {name} command could not resolve: {error}"))
                })?
            };
            let cwd = server.working_directory.as_ref().map_or_else(
                || Ok(workspace.to_owned()),
                |path| resolve_path(workspace, path),
            )?;
            let configured = ConfiguredServer {
                name: name.clone(),
                command,
                args: server.args.clone(),
                cwd,
                environment: server.environment.clone(),
                allowed_tools: server.allowed_tools.iter().cloned().collect(),
                research_tools: server.research_tools.clone(),
                timeout_ms: server.timeout_ms,
                max_output_bytes: server.max_output_bytes,
            };
            servers.insert(name.clone(), configured);
        }
        Ok(Self { servers, process })
    }

    /// Whether at least one server is explicitly configured.
    pub fn is_configured(&self) -> bool {
        !self.servers.is_empty()
    }

    /// Safe configured discovery metadata.
    pub fn servers(&self) -> Vec<McpServerSummary> {
        self.servers
            .values()
            .map(|server| McpServerSummary {
                name: server.name.clone(),
                transport: "stdio".into(),
                allowed_tools: server.allowed_tools.iter().cloned().collect(),
                research_tools: server
                    .research_tools
                    .iter()
                    .map(|tool| tool.tool.clone())
                    .collect(),
            })
            .collect()
    }

    /// Deterministic configured server names.
    pub fn server_names(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    /// Build research call templates with recursive `{query}` substitution.
    pub fn research_calls(&self, query: &str) -> Vec<McpResearchCall> {
        self.servers
            .values()
            .flat_map(|server| {
                server
                    .research_tools
                    .iter()
                    .map(move |configured| McpResearchCall {
                        server: server.name.clone(),
                        tool: configured.tool.clone(),
                        title: configured
                            .title
                            .clone()
                            .unwrap_or_else(|| format!("{} {}", server.name, configured.tool)),
                        arguments: template_value(&configured.arguments, query),
                    })
            })
            .collect()
    }

    /// Construct a complete logical effect request without resolving credentials.
    pub fn request(
        &self,
        actor: Actor,
        context: ExecutionContext,
        operation: McpOperation,
    ) -> Result<EffectRequest, McpError> {
        let server = self
            .servers
            .get(operation.server())
            .ok_or_else(|| McpError::UnknownServer(operation.server().into()))?;
        if let McpOperation::CallTool {
            tool,
            arguments,
            input_schema,
            ..
        } = &operation
        {
            if !server.allowed_tools.contains(tool) {
                return Err(McpError::ToolDenied(format!("{}:{tool}", server.name)));
            }
            validate_arguments(input_schema, arguments)?;
        }
        let action = operation.action();
        let input = McpEffectInput {
            operation,
            cwd: server.cwd.clone(),
            args: server.args.clone(),
            environment: server.environment.clone(),
            timeout_ms: server.timeout_ms,
            max_output_bytes: server.max_output_bytes,
        };
        let mut request = effect_request(
            actor,
            action,
            server.command.display().to_string(),
            serde_json::to_value(input).map_err(|error| McpError::Invalid(error.to_string()))?,
        );
        request.capabilities = vec!["mcp.invoke".into()];
        request.credential_references = server
            .environment
            .values()
            .cloned()
            .map(|reference| CredentialReference {
                reference,
                value_hash: None,
            })
            .collect();
        request.context = context;
        Ok(request)
    }

    fn configured(
        &self,
        input: &McpEffectInput,
        request: &EffectRequest,
    ) -> Result<&ConfiguredServer, ExecutionError> {
        let server = self
            .servers
            .get(input.operation.server())
            .ok_or_else(|| failed("MCP server is not configured"))?;
        if request.action != input.operation.action()
            || request.resource != server.command.display().to_string()
            || input.cwd != server.cwd
            || input.args != server.args
            || input.environment != server.environment
            || input.timeout_ms != server.timeout_ms
            || input.max_output_bytes != server.max_output_bytes
        {
            return Err(failed(
                "MCP effect does not match its configured server identity",
            ));
        }
        if let McpOperation::CallTool {
            tool,
            arguments,
            input_schema,
            ..
        } = &input.operation
        {
            if !server.allowed_tools.contains(tool) {
                return Err(failed("MCP tool is not allowlisted"));
            }
            validate_arguments(input_schema, arguments).map_err(failed)?;
        }
        Ok(server)
    }
}

fn resolve_path(workspace: &Path, path: &Path) -> Result<PathBuf, McpError> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    };
    fs::canonicalize(&path).map_err(|error| {
        McpError::Invalid(format!(
            "MCP working directory {} could not resolve: {error}",
            path.display()
        ))
    })
}

fn validate_arguments(schema: &Value, arguments: &Value) -> Result<(), McpError> {
    if !arguments.is_object() || !schema.is_object() {
        return Err(McpError::InvalidArguments(
            "arguments and input schema must be JSON objects".into(),
        ));
    }
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| McpError::InvalidArguments(format!("schema is invalid: {error}")))?;
    let messages = validator
        .iter_errors(arguments)
        .take(8)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !messages.is_empty() {
        return Err(McpError::InvalidArguments(messages.join("; ")));
    }
    Ok(())
}

/// Validate a call against one released discovery record.
pub fn validate_tool_arguments(tool: &McpToolSummary, arguments: &Value) -> Result<(), McpError> {
    validate_arguments(&tool.input_schema, arguments)
}

fn template_value(value: &Value, query: &str) -> Value {
    match value {
        Value::String(value) => Value::String(value.replace("{query}", query)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| template_value(value, query))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), template_value(value, query)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn resolve_environment(
    references: &BTreeMap<String, String>,
) -> Result<(BTreeMap<String, String>, Vec<String>), ExecutionError> {
    let mut environment = BTreeMap::new();
    let mut secrets = Vec::new();
    for (child_name, reference) in references {
        let host_name = environment_reference(reference)
            .ok_or_else(|| failed("MCP credential reference is invalid"))?;
        let value = env::var(host_name).map_err(|_| {
            failed(format!(
                "MCP credential environment variable {host_name} is unavailable"
            ))
        })?;
        secrets.push(value.clone());
        environment.insert(child_name.clone(), value);
    }
    Ok((environment, secrets))
}

fn protocol_input(operation: &McpOperation) -> Result<Vec<u8>, ExecutionError> {
    let initialize = InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("colossus-rs", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::LATEST);
    let operation_message = match operation {
        McpOperation::ListTools { cursor, .. } => {
            let params = PaginatedRequestParams::default().with_cursor(cursor.clone());
            json!({
                "jsonrpc": "2.0",
                "id": MCP_REQUEST_ID,
                "method": "tools/list",
                "params": params,
            })
        }
        McpOperation::CallTool {
            tool, arguments, ..
        } => {
            let arguments = arguments
                .as_object()
                .cloned()
                .ok_or_else(|| failed("MCP tool arguments must be an object"))?;
            let params = CallToolRequestParams::new(tool.clone()).with_arguments(arguments);
            json!({
                "jsonrpc": "2.0",
                "id": MCP_REQUEST_ID,
                "method": "tools/call",
                "params": params,
            })
        }
    };
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": initialize,
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }),
        operation_message,
    ];
    let mut bytes = Vec::new();
    for message in messages {
        let line = serde_json::to_vec(&message).map_err(failed)?;
        if line.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err(failed("MCP protocol message exceeds the line bound"));
        }
        bytes.extend(line);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxResult {
    backend: String,
    exit_code: Option<i32>,
    success: bool,
    timed_out: bool,
    resource_limit_exceeded: Option<String>,
    output_truncated: bool,
    stdout_base64: String,
    stderr_base64: String,
}

fn process_stdout(bytes: &[u8], operation: &McpOperation) -> Result<Vec<u8>, ExecutionError> {
    let result: SandboxResult = serde_json::from_slice(bytes)
        .map_err(|error| operation_error(operation, format!("invalid sandbox result: {error}")))?;
    let _ = (&result.backend, &result.stderr_base64);
    if result.timed_out || result.resource_limit_exceeded.is_some() || result.output_truncated {
        return Err(operation_error(
            operation,
            "MCP process timed out, exceeded a resource limit, or truncated protocol output",
        ));
    }
    let stdout = BASE64
        .decode(&result.stdout_base64)
        .map_err(|error| operation_error(operation, format!("invalid MCP stdout: {error}")))?;
    if !result.success || result.exit_code != Some(0) {
        // A valid complete response is still considered below; absence remains unknown for calls.
        if response_value(&stdout, MCP_REQUEST_ID).is_err() {
            return Err(operation_error(
                operation,
                format!("MCP server exited with status {:?}", result.exit_code),
            ));
        }
    }
    Ok(stdout)
}

fn response_value(bytes: &[u8], id: i64) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "MCP stdout is not UTF-8".to_owned())?;
    let mut found = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if line.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err("MCP response line exceeds its bound".into());
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|error| format!("MCP stdout contains non-protocol data: {error}"))?;
        if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err("MCP response has an invalid JSON-RPC version".into());
        }
        if value.get("id").and_then(Value::as_i64) == Some(id) && found.replace(value).is_some() {
            return Err(format!("MCP server returned duplicate response id {id}"));
        }
    }
    found.ok_or_else(|| format!("MCP server returned no response for id {id}"))
}

fn response_result(bytes: &[u8], id: i64) -> Result<Value, String> {
    let value = response_value(bytes, id)?;
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32_000);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown MCP error");
        return Err(format!("MCP JSON-RPC error {code}: {message}"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "MCP response has neither result nor error".into())
}

fn validate_initialize(stdout: &[u8]) -> Result<(), String> {
    let result: InitializeResult =
        serde_json::from_value(response_result(stdout, INITIALIZE_REQUEST_ID)?)
            .map_err(|error| format!("invalid MCP initialize result: {error}"))?;
    if !ProtocolVersion::KNOWN_VERSIONS.contains(&result.protocol_version)
        || result.protocol_version > ProtocolVersion::LATEST
        || result.capabilities.tools.is_none()
    {
        return Err(format!(
            "MCP server negotiated unsupported protocol {} or omitted tools capability",
            result.protocol_version
        ));
    }
    Ok(())
}

fn parse_tools(stdout: &[u8], server: &ConfiguredServer) -> Result<McpToolsPage, String> {
    validate_initialize(stdout)?;
    let result: ListToolsResult = serde_json::from_value(response_result(stdout, MCP_REQUEST_ID)?)
        .map_err(|error| format!("invalid MCP tools result: {error}"))?;
    if result.tools.len() > MAX_MCP_TOOLS {
        return Err(format!("MCP page exceeds {MAX_MCP_TOOLS} tools"));
    }
    let mut tools = Vec::new();
    let mut names = BTreeSet::new();
    for tool in result.tools {
        let name = tool.name.into_owned();
        if !server.allowed_tools.contains(&name) {
            continue;
        }
        validate_name(&name, "tool").map_err(|error| error.to_string())?;
        if !names.insert(name.clone()) {
            return Err(format!("MCP server returned duplicate tool {name}"));
        }
        let input_schema = Value::Object((*tool.input_schema).clone());
        jsonschema::validator_for(&input_schema)
            .map_err(|error| format!("MCP tool {name} schema is invalid: {error}"))?;
        let schema_bytes = serde_json::to_vec(&input_schema).map_err(|error| error.to_string())?;
        if schema_bytes.len() > 256 * 1024 {
            return Err(format!("MCP tool {name} schema exceeds its bound"));
        }
        tools.push(McpToolSummary {
            server: server.name.clone(),
            name,
            title: tool.title.map(|value| bounded_string(&value, 8 * 1024)),
            description: tool
                .description
                .map(|value| bounded_string(&value, 32 * 1024)),
            input_schema,
            schema_sha256: hex_sha256(&schema_bytes),
        });
    }
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(McpToolsPage {
        server: server.name.clone(),
        tools,
        next_cursor: result
            .next_cursor
            .map(|value| bounded_string(&value, 8 * 1024)),
    })
}

fn parse_call(
    stdout: &[u8],
    server: &ConfiguredServer,
    tool: &str,
    secrets: &[String],
) -> Result<McpCallOutput, String> {
    validate_initialize(stdout)?;
    let result: CallToolResult = serde_json::from_value(response_result(stdout, MCP_REQUEST_ID)?)
        .map_err(|error| format!("invalid MCP tool result: {error}"))?;
    let mut value = serde_json::to_value(result).map_err(|error| error.to_string())?;
    redact_value(&mut value, secrets);
    let result = serde_json::from_value(value).map_err(|error| error.to_string())?;
    Ok(McpCallOutput {
        server: server.name.clone(),
        tool: tool.into(),
        result,
    })
}

fn redact_value(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
                if text.contains(secret) {
                    *text = text.replace(secret, "<redacted>");
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_value(value, secrets);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                redact_value(value, secrets);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn bounded_string(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn failed(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

fn operation_error(operation: &McpOperation, error: impl std::fmt::Display) -> ExecutionError {
    if operation.is_call() {
        ExecutionError::OutcomeUnknown(format!("MCP tool outcome cannot be confirmed: {error}"))
    } else {
        failed(error)
    }
}

fn bounded_result(
    value: &impl Serialize,
    max_output_bytes: u64,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let bytes = serde_json::to_vec(value).map_err(failed)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_output_bytes {
        return Err(failed("MCP adapter result exceeds its policy output bound"));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}

#[async_trait]
impl EffectExecutor for McpExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if !matches!(request.action.as_str(), "mcp.tools" | "mcp.call") {
            return Err(failed("MCP executor received another action"));
        }
        let input: McpEffectInput = serde_json::from_value(request.content.clone())
            .map_err(|error| failed(format!("invalid MCP effect input: {error}")))?;
        let server = self.configured(&input, request)?.clone();
        let max_output_bytes = permit.obligations().max_output_bytes;
        let (environment, secrets) = resolve_environment(&server.environment)?;
        let protocol = protocol_input(&input.operation)?;
        let process = ProcessSpec {
            cwd: server.cwd.clone(),
            args: server.args.clone(),
            environment,
            stdin_base64: Some(BASE64.encode(protocol)),
            timeout_ms: server.timeout_ms,
            max_output_bytes: server.max_output_bytes,
        };
        let mut process_request = request.clone();
        process_request.content = serde_json::to_value(process).map_err(failed)?;
        let process_result = match self.process.execute(&process_request, permit).await {
            Ok(result) => result,
            Err(ExecutionError::OutcomeUnknown(error)) => {
                return Err(ExecutionError::OutcomeUnknown(error));
            }
            Err(error) if input.operation.is_call() => {
                return Err(operation_error(&input.operation, error));
            }
            Err(error) => return Err(error),
        };
        let stdout = process_stdout(&process_result.bytes, &input.operation)?;
        match &input.operation {
            McpOperation::ListTools { .. } => {
                let page = parse_tools(&stdout, &server).map_err(failed)?;
                bounded_result(&page, max_output_bytes)
            }
            McpOperation::CallTool { tool, .. } => {
                let output = parse_call(&stdout, &server, tool, &secrets)
                    .map_err(|error| operation_error(&input.operation, error))?;
                bounded_result(&output, max_output_bytes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_official_protocol_models_and_no_secret_values() {
        let operation = McpOperation::CallTool {
            server: "local".into(),
            tool: "echo".into(),
            arguments: json!({"text": "hello"}),
            input_schema: json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false
            }),
        };
        let bytes = protocol_input(&operation).expect("protocol");
        let lines = std::str::from_utf8(&bytes)
            .expect("UTF-8")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
            .collect::<Vec<_>>();
        assert_eq!(lines[0]["method"], "initialize");
        assert_eq!(lines[0]["params"]["protocolVersion"], "2025-11-25");
        assert_eq!(lines[1]["method"], "notifications/initialized");
        assert_eq!(lines[2]["method"], "tools/call");
        assert_eq!(lines[2]["params"]["name"], "echo");
    }

    #[test]
    fn discovered_schema_is_enforced_before_call() {
        let tool = McpToolSummary {
            server: "local".into(),
            name: "echo".into(),
            title: None,
            description: None,
            input_schema: json!({
                "type": "object",
                "properties": {"count": {"type": "integer"}},
                "required": ["count"],
                "additionalProperties": false
            }),
            schema_sha256: "unused".into(),
        };
        assert!(validate_tool_arguments(&tool, &json!({"count": 2})).is_ok());
        assert!(validate_tool_arguments(&tool, &json!({"count": "two"})).is_err());
    }

    #[test]
    fn secret_values_are_redacted_from_nested_results() {
        let mut value = json!({"text": "token=secret-value", "nested": ["secret-value"]});
        redact_value(&mut value, &["secret-value".into()]);
        assert_eq!(value["text"], "token=<redacted>");
        assert_eq!(value["nested"][0], "<redacted>");
    }
}
