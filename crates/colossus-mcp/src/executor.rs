use super::*;

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
                effect_action_prefix: server.effect_action_prefix.clone(),
                provenance: server.provenance.clone(),
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
        let action = server.effect_action_prefix.as_ref().map_or_else(
            || operation.action().to_owned(),
            |prefix| match &operation {
                McpOperation::ListTools { .. } => format!("{prefix}.tools"),
                McpOperation::CallTool { .. } => format!("{prefix}.call"),
            },
        );
        let input = McpEffectInput {
            operation,
            cwd: server.cwd.clone(),
            args: server.args.clone(),
            environment: server.environment.clone(),
            timeout_ms: server.timeout_ms,
            max_output_bytes: server.max_output_bytes,
            provenance: server.provenance.clone(),
        };
        let mut request = effect_request(
            actor,
            &action,
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
        let expected_action = server.effect_action_prefix.as_ref().map_or_else(
            || input.operation.action().to_owned(),
            |prefix| match &input.operation {
                McpOperation::ListTools { .. } => format!("{prefix}.tools"),
                McpOperation::CallTool { .. } => format!("{prefix}.call"),
            },
        );
        if request.action != expected_action
            || request.resource != server.command.display().to_string()
            || input.cwd != server.cwd
            || input.args != server.args
            || input.environment != server.environment
            || input.timeout_ms != server.timeout_ms
            || input.max_output_bytes != server.max_output_bytes
            || input.provenance != server.provenance
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

pub(super) fn resolve_path(workspace: &Path, path: &Path) -> Result<PathBuf, McpError> {
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

pub(super) fn protocol_input(operation: &McpOperation) -> Result<Vec<u8>, ExecutionError> {
    let initialize = InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("colossus", env!("CARGO_PKG_VERSION")),
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
    #[serde(default)]
    observed_origins: Vec<String>,
}

fn process_stdout(bytes: &[u8], operation: &McpOperation) -> Result<Vec<u8>, ExecutionError> {
    let result: SandboxResult = serde_json::from_slice(bytes)
        .map_err(|error| operation_error(operation, format!("invalid sandbox result: {error}")))?;
    let _ = (
        &result.backend,
        &result.stderr_base64,
        &result.observed_origins,
    );
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

pub(super) fn redact_value(value: &mut Value, secrets: &[String]) {
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
        if !matches!(request.action.as_str(), "mcp.tools" | "mcp.call")
            && !request.action.starts_with("pack.mcp.")
        {
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
