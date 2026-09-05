use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct McpDocument {
    #[serde(rename = "$schema")]
    schema: String,
    mcp_servers: BTreeMap<String, Value>,
}

pub(crate) fn load_mcp(
    root: &Path,
    manifest: &AgentPluginManifest,
    diagnostics: &mut Vec<PluginComponentDiagnostic>,
) -> Result<Vec<PluginMcpServer>, StoreError> {
    let path = root.join("mcp.json");
    let bytes = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                None,
                "invalid_mcp_location",
                "mcp.json exists but is not a regular file",
            ));
            return Ok(Vec::new());
        }
        Ok(_) => read_contained(root, Path::new("mcp.json"), MAX_MANIFEST_BYTES)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(adapter(error)),
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                None,
                "invalid_mcp_document",
                "mcp.json is not valid JSON",
            ));
            return Ok(Vec::new());
        }
    };
    let document: McpDocument = match serde_json::from_value(value.clone()) {
        Ok(document) => document,
        Err(_) => {
            diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                None,
                "invalid_mcp_document",
                "mcp.json must contain a v1 $schema string and mcpServers object",
            ));
            return Ok(Vec::new());
        }
    };
    if document.schema != AGENT_PLUGIN_MCP_SCHEMA_V1 || manifest.schema != AGENT_PLUGIN_SCHEMA_V1 {
        diagnostics.push(component_diagnostic(
            PluginComponentKind::McpServer,
            None,
            "unsupported_mcp_schema",
            "mcp.json does not target Agent Plugins v1",
        ));
        return Ok(Vec::new());
    }
    let validator = super::schema::mcp_server_validator()?;
    let mut servers = Vec::new();
    for (name, server) in document.mcp_servers {
        if !valid_component_name(&name) {
            diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                Some(name),
                "invalid_mcp_server_name",
                "MCP server name is invalid",
            ));
            continue;
        }
        let errors = validator
            .iter_errors(&server)
            .take(4)
            .map(|error| {
                format!(
                    "MCP server field {} violates schema rule {}",
                    error.instance_path, error.schema_path
                )
            })
            .collect::<Vec<_>>();
        if !errors.is_empty() {
            diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                Some(name),
                "invalid_mcp_server",
                errors.join("; "),
            ));
            continue;
        }
        match parse_mcp_server(root, &manifest.name, &name, &server) {
            Ok(server) if server.transport == PluginMcpTransport::Sse => {
                diagnostics.push(component_diagnostic(
                    PluginComponentKind::McpServer,
                    Some(name),
                    "unsupported_mcp_transport",
                    "legacy HTTP+SSE is not supported",
                ));
            }
            Ok(server) => servers.push(server),
            Err(error) => diagnostics.push(component_diagnostic(
                PluginComponentKind::McpServer,
                Some(name),
                "invalid_mcp_server",
                error.to_string(),
            )),
        }
    }
    Ok(servers)
}

pub(crate) fn parse_mcp_server(
    root: &Path,
    plugin: &str,
    name: &str,
    value: &Value,
) -> Result<PluginMcpServer, StoreError> {
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::Adapter("MCP server must be an object".into()))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = format!("{plugin}/{name}");
    if kind == "stdio" {
        let command = object
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Adapter("stdio command is required".into()))?;
        if command.chars().any(char::is_whitespace)
            || (!command.starts_with("./") && command.contains(['/', '\\']))
        {
            return Err(StoreError::Adapter(
                "stdio command must be one bare token or begin with ./".into(),
            ));
        }
        if command.starts_with("./") {
            resolve_plugin_relative(root, command)?;
        }
        let args = string_array(object.get("args"))?;
        let environment = string_map(object.get("env"))?;
        if environment.keys().any(|name| {
            name.eq_ignore_ascii_case("PLUGIN_ROOT") || name.eq_ignore_ascii_case("PLUGIN_DATA")
        }) {
            return Err(StoreError::Adapter(
                "stdio env cannot replace PLUGIN_ROOT or PLUGIN_DATA".into(),
            ));
        }
        let working_directory = object.get("cwd").and_then(Value::as_str).map(str::to_owned);
        if working_directory.as_deref().is_some_and(|cwd| {
            !cwd.starts_with("./")
                && cwd != "${PLUGIN_ROOT}"
                && !cwd.starts_with("${PLUGIN_ROOT}/")
                && cwd != "${PLUGIN_DATA}"
                && !cwd.starts_with("${PLUGIN_DATA}/")
        }) {
            return Err(StoreError::Adapter(
                "stdio cwd must be rooted in ./, PLUGIN_ROOT, or PLUGIN_DATA".into(),
            ));
        }
        if let Some(cwd) = working_directory.as_deref()
            && cwd.starts_with("./")
        {
            resolve_plugin_relative(root, cwd)?;
        }
        return Ok(PluginMcpServer {
            id,
            name: name.into(),
            transport: PluginMcpTransport::Stdio,
            command: Some(command.into()),
            args,
            environment,
            working_directory,
            url: None,
            headers: BTreeMap::new(),
        });
    }
    let transport = match kind {
        "streamable-http" => PluginMcpTransport::StreamableHttp,
        "sse" => PluginMcpTransport::Sse,
        _ => return Err(StoreError::Adapter("unsupported MCP transport".into())),
    };
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::Adapter("remote MCP URL is required".into()))?;
    validate_remote_mcp_url(url)?;
    let headers = string_map(object.get("headers"))?;
    validate_headers(&headers)?;
    Ok(PluginMcpServer {
        id,
        name: name.into(),
        transport,
        command: None,
        args: Vec::new(),
        environment: BTreeMap::new(),
        working_directory: None,
        url: Some(url.into()),
        headers,
    })
}

pub(crate) fn string_array(value: Option<&Value>) -> Result<Vec<String>, StoreError> {
    value.map_or_else(
        || Ok(Vec::new()),
        |value| {
            value
                .as_array()
                .ok_or_else(|| StoreError::Adapter("expected an array of strings".into()))?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| StoreError::Adapter("expected a string".into()))
                })
                .collect()
        },
    )
}

pub(crate) fn string_map(value: Option<&Value>) -> Result<BTreeMap<String, String>, StoreError> {
    value.map_or_else(
        || Ok(BTreeMap::new()),
        |value| {
            value
                .as_object()
                .ok_or_else(|| StoreError::Adapter("expected an object of strings".into()))?
                .iter()
                .map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.into()))
                        .ok_or_else(|| StoreError::Adapter("expected a string value".into()))
                })
                .collect()
        },
    )
}

pub(crate) fn validate_remote_mcp_url(value: &str) -> Result<(), StoreError> {
    let url = Url::parse(value).map_err(adapter)?;
    let host = url
        .host_str()
        .ok_or_else(|| StoreError::Adapter("MCP URL has no host".into()))?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.scheme(), "http" | "https")
        || (url.scheme() == "http" && !loopback)
    {
        return Err(StoreError::Adapter(
            "remote MCP URL requires HTTPS or explicit loopback HTTP and no user info or fragment"
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), StoreError> {
    let mut names = BTreeSet::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if !names.insert(lower.clone())
            || matches!(
                lower.as_str(),
                "authorization" | "cookie" | "proxy-authorization"
            )
            || http::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || http::header::HeaderValue::from_str(value).is_err()
        {
            return Err(StoreError::Adapter(
                "MCP headers must be unique valid non-secret HTTP fields".into(),
            ));
        }
    }
    Ok(())
}
