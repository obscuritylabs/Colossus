use super::*;
use http::{HeaderName, HeaderValue};
use rmcp::{
    ServiceExt as _,
    transport::{
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationSession,
            CredentialStore as _, OAuthClientConfig,
        },
        common::client_side_sse::NeverRetry,
        streamable_http_client::{
            StreamableHttpClient, StreamableHttpClientTransport,
            StreamableHttpClientTransportConfig,
        },
    },
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// Permit-bound configured MCP adapter.
pub struct McpExecutor {
    servers: BTreeMap<String, ConfiguredServer>,
    process: Arc<dyn EffectExecutor>,
    tls_roots: AdditionalRootCertificates,
    oauth_store: Option<OAuthStoreFactory>,
    oauth_network_destinations: Vec<String>,
    oauth_allowed_environment: Vec<String>,
    oauth_timeout_ms: u64,
    oauth_max_output_bytes: u64,
    oauth_sessions: tokio::sync::Mutex<BTreeMap<String, AuthorizationSession>>,
}

impl McpExecutor {
    /// Resolve configured paths and construct a permit-bound adapter.
    pub fn new(
        config: &McpConfig,
        workspace: &Path,
        sandbox_backend: &str,
        process: Arc<dyn EffectExecutor>,
    ) -> Result<Self, McpError> {
        let mut servers = BTreeMap::new();
        for (name, server) in &config.servers {
            let command = match server.transport {
                McpTransportKind::Stdio if sandbox_backend == "oci" => server.command.clone(),
                McpTransportKind::Stdio => fs::canonicalize(&server.command).map_err(|error| {
                    McpError::Invalid(format!("server {name} command could not resolve: {error}"))
                })?,
                McpTransportKind::StreamableHttp => PathBuf::new(),
            };
            let cwd = match server.transport {
                McpTransportKind::Stdio => Some(server.working_directory.as_ref().map_or_else(
                    || Ok(workspace.to_owned()),
                    |path| resolve_path(workspace, path),
                )?),
                McpTransportKind::StreamableHttp => None,
            };
            let configured = ConfiguredServer {
                name: name.clone(),
                transport: server.transport,
                command,
                args: server.args.clone(),
                cwd,
                environment: server.environment.clone(),
                url: server.url.clone(),
                headers: server.headers.clone(),
                credential_headers: server.credential_headers.clone(),
                oauth: server.oauth.clone(),
                allowed_tools: ToolAllowlist::from_config(name, &server.allowed_tools)?,
                research_tools: server.research_tools.clone(),
                timeout_ms: server.timeout_ms,
                max_output_bytes: server.max_output_bytes,
                effect_action_prefix: server.effect_action_prefix.clone(),
                provenance: server.provenance.clone(),
            };
            servers.insert(name.clone(), configured);
        }
        Ok(Self {
            servers,
            process,
            tls_roots: AdditionalRootCertificates::default(),
            oauth_store: None,
            oauth_network_destinations: Vec::new(),
            oauth_allowed_environment: Vec::new(),
            oauth_timeout_ms: 30_000,
            oauth_max_output_bytes: 1024 * 1024,
            oauth_sessions: tokio::sync::Mutex::new(BTreeMap::new()),
        })
    }

    /// Add validated runtime-wide CA roots to remote MCP clients.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }

    /// Configure the bounded network and environment grants used by operator OAuth commands.
    #[must_use]
    pub fn with_oauth_policy(
        mut self,
        network_destinations: Vec<String>,
        allowed_environment: Vec<String>,
        timeout_ms: u64,
        max_output_bytes: u64,
    ) -> Self {
        self.oauth_network_destinations = network_destinations;
        self.oauth_allowed_environment = allowed_environment;
        self.oauth_timeout_ms = timeout_ms;
        self.oauth_max_output_bytes = max_output_bytes;
        self
    }

    /// Persist OAuth records in the operating-system credential store.
    #[must_use]
    pub fn with_platform_oauth_storage(
        mut self,
        service: impl Into<String>,
        repository_id: impl Into<String>,
    ) -> Self {
        self.oauth_store = Some(OAuthStoreFactory::platform(
            service.into(),
            repository_id.into(),
        ));
        self
    }

    /// Persist OAuth records in a dedicated encrypted redb sidecar.
    pub fn with_encrypted_oauth_storage(
        mut self,
        path: &Path,
        keys: Arc<dyn colossus_ports::KeyProvider>,
        repository_id: impl Into<String>,
    ) -> Result<Self, McpError> {
        self.oauth_store = Some(
            OAuthStoreFactory::encrypted_state(path, keys, repository_id.into())
                .map_err(safe_oauth_error)?,
        );
        Ok(self)
    }

    /// Return the exact configured loopback callback port.
    pub fn oauth_callback_port(&self, server: &str) -> Result<u16, McpError> {
        self.oauth_server(server)
            .map(|(_, oauth)| oauth.callback_port)
    }

    /// Begin a PKCE-S256 OAuth authorization session.
    pub async fn oauth_login_begin(&self, server: &str) -> Result<McpOAuthLogin, McpError> {
        let (configured, oauth) = self.oauth_server(server)?;
        let callback_url = format!("http://127.0.0.1:{}/callback", oauth.callback_port);
        let manager = self
            .oauth_manager(
                configured,
                &self.oauth_network_destinations,
                &self.oauth_allowed_environment,
                self.oauth_timeout_ms,
                self.oauth_max_output_bytes,
            )
            .await?;
        let scopes = oauth.scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let authorization_url = manager
            .get_authorization_url(&scopes)
            .await
            .map_err(safe_oauth_error)?;
        validate_oauth_authorization_url(&authorization_url, &self.oauth_network_destinations)?;
        let session = AuthorizationSession::for_scope_upgrade(
            manager,
            authorization_url.clone(),
            &callback_url,
        );
        self.oauth_sessions
            .lock()
            .await
            .insert(server.to_owned(), session);
        Ok(McpOAuthLogin {
            server: server.into(),
            authorization_url,
            callback_url,
        })
    }

    /// Complete a pending OAuth session from its final redirected URL.
    pub async fn oauth_login_complete(
        &self,
        server: &str,
        callback_url: &str,
    ) -> Result<McpOAuthStatus, McpError> {
        let port = self.oauth_callback_port(server)?;
        validate_callback_url(callback_url, port)?;
        let session = self
            .oauth_sessions
            .lock()
            .await
            .remove(server)
            .ok_or_else(|| McpError::OAuth("no pending authorization session".into()))?;
        session
            .handle_callback_url(callback_url)
            .await
            .map_err(safe_oauth_error)?;
        self.oauth_status(server).await
    }

    /// Inspect local OAuth token presence without initiating authorization.
    pub async fn oauth_status(&self, server: &str) -> Result<McpOAuthStatus, McpError> {
        let configured = self
            .servers
            .get(server)
            .ok_or_else(|| McpError::UnknownServer(server.into()))?;
        if configured.oauth.is_none() {
            return Ok(McpOAuthStatus {
                server: server.into(),
                configured: false,
                authenticated: false,
            });
        }
        let authenticated = self
            .oauth_credential_store(configured)?
            .load()
            .await
            .map_err(safe_oauth_error)?
            .is_some_and(|credentials| credentials.token_response.is_some());
        Ok(McpOAuthStatus {
            server: server.into(),
            configured: true,
            authenticated,
        })
    }

    /// Clear local OAuth tokens without remote revocation.
    pub async fn oauth_logout(&self, server: &str) -> Result<McpOAuthStatus, McpError> {
        let (configured, _) = self.oauth_server(server)?;
        self.oauth_credential_store(configured)?
            .clear()
            .await
            .map_err(safe_oauth_error)?;
        self.oauth_sessions.lock().await.remove(server);
        self.oauth_status(server).await
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
                transport: server.transport.as_str().into(),
                allowed_tools: server.allowed_tools.summary(),
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
            description,
            annotations,
            arguments,
            input_schema,
            schema_sha256,
            ..
        } = &operation
        {
            if !server.allowed_tools.allows(tool) {
                return Err(McpError::ToolDenied(format!("{}:{tool}", server.name)));
            }
            validate_call_review_metadata(
                description.as_deref(),
                annotations.as_ref(),
                input_schema,
                schema_sha256,
            )?;
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
            transport: server.transport,
            cwd: server.cwd.clone(),
            args: server.args.clone(),
            environment: server.environment.clone(),
            url: server.url.clone(),
            headers: server.headers.clone(),
            credential_headers: server.credential_headers.clone(),
            oauth: server.oauth.clone(),
            timeout_ms: server.timeout_ms,
            max_output_bytes: server.max_output_bytes,
            provenance: server.provenance.clone(),
        };
        let resource = match server.transport {
            McpTransportKind::Stdio => server.command.display().to_string(),
            McpTransportKind::StreamableHttp => server.url.clone().ok_or_else(|| {
                McpError::Invalid(format!("server {} has no configured URL", server.name))
            })?,
        };
        let mut request = effect_request(
            actor,
            &action,
            resource,
            serde_json::to_value(input).map_err(|error| McpError::Invalid(error.to_string()))?,
        );
        request.capabilities = vec!["mcp.invoke".into()];
        request.credential_references = server
            .environment
            .values()
            .cloned()
            .chain(
                server
                    .credential_headers
                    .values()
                    .map(|credential| credential.reference.clone()),
            )
            .chain(
                server
                    .oauth
                    .iter()
                    .filter_map(|oauth| oauth.client_secret_reference.clone()),
            )
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
        let expected_resource = match server.transport {
            McpTransportKind::Stdio => server.command.display().to_string(),
            McpTransportKind::StreamableHttp => server.url.clone().unwrap_or_default(),
        };
        if request.action != expected_action
            || request.resource != expected_resource
            || input.transport != server.transport
            || input.cwd != server.cwd
            || input.args != server.args
            || input.environment != server.environment
            || input.url != server.url
            || input.headers != server.headers
            || input.credential_headers != server.credential_headers
            || input.oauth != server.oauth
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
            description,
            annotations,
            arguments,
            input_schema,
            schema_sha256,
            ..
        } = &input.operation
        {
            if !server.allowed_tools.allows(tool) {
                return Err(failed("MCP tool is not allowlisted"));
            }
            validate_call_review_metadata(
                description.as_deref(),
                annotations.as_ref(),
                input_schema,
                schema_sha256,
            )
            .map_err(failed)?;
            validate_arguments(input_schema, arguments).map_err(failed)?;
        }
        Ok(server)
    }

    fn oauth_server(&self, server: &str) -> Result<(&ConfiguredServer, &McpOAuthConfig), McpError> {
        let configured = self
            .servers
            .get(server)
            .ok_or_else(|| McpError::UnknownServer(server.into()))?;
        let oauth = configured
            .oauth
            .as_ref()
            .ok_or_else(|| McpError::OAuth(format!("server {server} does not configure OAuth")))?;
        Ok((configured, oauth))
    }

    fn oauth_credential_store(
        &self,
        server: &ConfiguredServer,
    ) -> Result<oauth_store::OAuthCredentialStore, McpError> {
        let endpoint = server
            .url
            .as_deref()
            .ok_or_else(|| McpError::OAuth("OAuth server has no endpoint".into()))?;
        self.oauth_store
            .as_ref()
            .map(|factory| factory.store(&server.name, endpoint))
            .ok_or_else(|| McpError::OAuth("OAuth credential storage is unavailable".into()))
    }

    async fn oauth_manager(
        &self,
        server: &ConfiguredServer,
        network_destinations: &[String],
        allowed_environment: &[String],
        timeout_ms: u64,
        max_output_bytes: u64,
    ) -> Result<AuthorizationManager, McpError> {
        let oauth = server
            .oauth
            .as_ref()
            .ok_or_else(|| McpError::OAuth("OAuth is not configured".into()))?;
        let endpoint = server
            .url
            .as_deref()
            .ok_or_else(|| McpError::OAuth("OAuth server has no endpoint".into()))?;
        let max_output_bytes = usize::try_from(max_output_bytes)
            .map_err(|_| McpError::OAuth("OAuth output bound is invalid".into()))?;
        let http = Arc::new(HardenedOAuthHttpClient::new(
            network_destinations.to_vec(),
            self.tls_roots.clone(),
            timeout_ms,
            max_output_bytes,
        ));
        let mut manager = AuthorizationManager::new_with_oauth_http_client(endpoint, http)
            .await
            .map_err(safe_oauth_error)?;
        manager.set_credential_store(self.oauth_credential_store(server)?);
        let metadata = manager
            .discover_metadata()
            .await
            .map_err(safe_oauth_error)?;
        manager.set_metadata(metadata);
        let redirect_uri = format!("http://127.0.0.1:{}/callback", oauth.callback_port);
        let mut client = OAuthClientConfig::new(&oauth.client_id, redirect_uri)
            .with_scopes(oauth.scopes.clone());
        if let Some(reference) = oauth.client_secret_reference.as_deref() {
            client = client
                .with_client_secret(resolve_oauth_client_secret(reference, allowed_environment)?);
        }
        manager.configure_client(client).map_err(safe_oauth_error)?;
        Ok(manager)
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

fn validate_call_review_metadata(
    description: Option<&str>,
    annotations: Option<&McpToolAnnotations>,
    input_schema: &Value,
    schema_sha256: &str,
) -> Result<(), McpError> {
    if description.is_some_and(|value| value.len() > 32 * 1024)
        || annotations
            .and_then(|value| value.title.as_deref())
            .is_some_and(|value| value.len() > 8 * 1024)
    {
        return Err(McpError::InvalidArguments(
            "MCP review description or annotation title exceeds its bound".into(),
        ));
    }
    let schema_bytes = serde_json::to_vec(input_schema)
        .map_err(|error| McpError::InvalidArguments(error.to_string()))?;
    if schema_bytes.len() > 256 * 1024 || schema_sha256 != hex_sha256(&schema_bytes) {
        return Err(McpError::InvalidArguments(
            "MCP review schema hash does not match the bounded input schema".into(),
        ));
    }
    Ok(())
}

/// Validate a call against one released discovery record.
pub fn validate_tool_arguments(tool: &McpToolSummary, arguments: &Value) -> Result<(), McpError> {
    validate_call_review_metadata(
        tool.description.as_deref(),
        tool.annotations.as_ref(),
        &tool.input_schema,
        &tool.schema_sha256,
    )?;
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

fn resolve_http_headers(
    server: &ConfiguredServer,
    permit: &ExecutionPermit,
) -> Result<(HashMap<HeaderName, HeaderValue>, Vec<String>), ExecutionError> {
    let mut headers = HashMap::new();
    for (name, value) in &server.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| failed("MCP HTTP header name is invalid"))?;
        let value =
            HeaderValue::from_str(value).map_err(|_| failed("MCP HTTP header value is invalid"))?;
        headers.insert(name, value);
    }
    let mut secrets = Vec::new();
    for (name, credential) in &server.credential_headers {
        let variable = environment_reference(&credential.reference)
            .ok_or_else(|| failed("MCP credential reference is invalid"))?;
        if !permit
            .obligations()
            .allowed_environment
            .iter()
            .any(|allowed| allowed == variable)
        {
            return Err(failed(format!(
                "MCP credential environment variable {variable} is absent from permit obligations"
            )));
        }
        let secret = env::var(variable).map_err(|_| {
            failed(format!(
                "MCP credential environment variable {variable} is unavailable"
            ))
        })?;
        if secret.is_empty() || secret.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return Err(failed("MCP credential resolved to an invalid value"));
        }
        let value = credential
            .scheme
            .as_ref()
            .map_or_else(|| secret.clone(), |scheme| format!("{scheme} {secret}"));
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| failed("MCP credential header name is invalid"))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| failed("MCP credential header value is invalid"))?;
        secrets.push(secret);
        headers.insert(name, value);
    }
    Ok((headers, secrets))
}

fn resolve_oauth_client_secret(
    reference: &str,
    allowed_environment: &[String],
) -> Result<String, McpError> {
    let variable = environment_reference(reference)
        .ok_or_else(|| McpError::OAuth("OAuth client secret reference is invalid".into()))?;
    if !allowed_environment
        .iter()
        .any(|allowed| allowed == variable)
    {
        return Err(McpError::OAuth(format!(
            "OAuth client secret environment variable {variable} is not permitted"
        )));
    }
    let secret = env::var(variable).map_err(|_| {
        McpError::OAuth(format!(
            "OAuth client secret environment variable {variable} is unavailable"
        ))
    })?;
    if secret.is_empty() || secret.bytes().any(|byte| matches!(byte, b'\r' | b'\n')) {
        return Err(McpError::OAuth(
            "OAuth client secret resolved to an invalid value".into(),
        ));
    }
    Ok(secret)
}

fn validate_oauth_authorization_url(
    authorization_url: &str,
    destinations: &[String],
) -> Result<(), McpError> {
    let url = url::Url::parse(authorization_url)
        .map_err(|_| McpError::OAuth("authorization URL is invalid".into()))?;
    let host = url
        .host_str()
        .ok_or_else(|| McpError::OAuth("authorization URL has no host".into()))?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || colossus_network::parse_host_ip(host).is_some_and(|address| address.is_loopback());
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(McpError::OAuth("authorization URL is unsafe".into()));
    }
    if colossus_policy::network_destination_match(destinations, authorization_url)
        .map_err(|_| McpError::OAuth("authorization URL origin is invalid".into()))?
        .is_none()
    {
        return Err(McpError::OAuth(
            "authorization URL origin is not permitted".into(),
        ));
    }
    Ok(())
}

fn validate_callback_url(callback_url: &str, port: u16) -> Result<(), McpError> {
    let url = url::Url::parse(callback_url)
        .map_err(|_| McpError::OAuth("callback URL is invalid".into()))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port_or_known_default() != Some(port)
        || url.path() != "/callback"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(McpError::OAuth(
            "callback URL does not match the configured loopback endpoint".into(),
        ));
    }
    Ok(())
}

fn safe_oauth_error(error: AuthError) -> McpError {
    match error {
        AuthError::AuthorizationRequired | AuthError::TokenExpired => {
            McpError::AuthorizationRequired("interactive login is required".into())
        }
        AuthError::PkceUnsupported => {
            McpError::OAuth("authorization server does not support PKCE-S256".into())
        }
        AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. } => {
            McpError::OAuth("authorization server issuer validation failed".into())
        }
        AuthError::NoAuthorizationSupport => {
            McpError::OAuth("server does not advertise OAuth authorization support".into())
        }
        _ => McpError::OAuth("authorization protocol failed".into()),
    }
}

fn mcp_oauth_execution_error(error: McpError) -> ExecutionError {
    match error {
        McpError::AuthorizationRequired(_) => {
            failed("MCP authorization required; run `colossus mcp auth login SERVER`")
        }
        _ => failed("MCP OAuth setup failed"),
    }
}

fn auth_execution_error(error: AuthError) -> ExecutionError {
    mcp_oauth_execution_error(safe_oauth_error(error))
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
    parse_tools_result(result, server)
}

pub(super) fn parse_tools_result(
    result: ListToolsResult,
    server: &ConfiguredServer,
) -> Result<McpToolsPage, String> {
    if result.tools.len() > MAX_MCP_TOOLS {
        return Err(format!("MCP page exceeds {MAX_MCP_TOOLS} tools"));
    }
    let mut tools = Vec::new();
    let mut names = BTreeSet::new();
    for tool in result.tools {
        let name = tool.name.into_owned();
        if !server.allowed_tools.allows(&name) {
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
        if matches!(&server.allowed_tools, ToolAllowlist::All)
            && (tool
                .title
                .as_ref()
                .is_some_and(|value| value.len() > 8 * 1024)
                || tool
                    .description
                    .as_ref()
                    .is_some_and(|value| value.len() > 32 * 1024)
                || tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.title.as_ref())
                    .is_some_and(|value| value.len() > 8 * 1024))
        {
            return Err(format!(
                "MCP tool {name} title, description, or annotation title exceeds its bound"
            ));
        }
        tools.push(McpToolSummary {
            server: server.name.clone(),
            name,
            title: tool.title.map(|value| bounded_string(&value, 8 * 1024)),
            description: tool
                .description
                .map(|value| bounded_string(&value, 32 * 1024)),
            annotations: tool.annotations.map(|annotations| McpToolAnnotations {
                title: annotations
                    .title
                    .map(|value| bounded_string(&value, 8 * 1024)),
                read_only_hint: annotations.read_only_hint,
                destructive_hint: annotations.destructive_hint,
                idempotent_hint: annotations.idempotent_hint,
                open_world_hint: annotations.open_world_hint,
            }),
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
    parse_call_result(result, server, tool, secrets)
}

fn parse_call_result(
    result: CallToolResult,
    server: &ConfiguredServer,
    tool: &str,
    secrets: &[String],
) -> Result<McpCallOutput, String> {
    let mut value = serde_json::to_value(result).map_err(|error| error.to_string())?;
    redact_value(&mut value, secrets);
    let result = serde_json::from_value(value).map_err(|error| error.to_string())?;
    Ok(McpCallOutput {
        server: server.name.clone(),
        tool: tool.into(),
        result,
    })
}

pub(super) enum RemoteOperationResult {
    Tools(ListToolsResult),
    Call(CallToolResult),
}

pub(super) async fn execute_remote_operation<C>(
    http: C,
    server: &ConfiguredServer,
    operation: &McpOperation,
    headers: HashMap<HeaderName, HeaderValue>,
    call_dispatched: &AtomicBool,
) -> Result<RemoteOperationResult, ExecutionError>
where
    C: StreamableHttpClient + Send + Sync,
{
    let endpoint = server
        .url
        .clone()
        .ok_or_else(|| failed("MCP Streamable HTTP server has no endpoint"))?;
    let mut config = StreamableHttpClientTransportConfig::with_uri(endpoint);
    config.retry_config = Arc::new(NeverRetry::default());
    config.allow_stateless = false;
    config.reinit_on_expired_session = false;
    config.custom_headers = headers;
    let transport = StreamableHttpClientTransport::with_client(http, config);
    let mut service = ()
        .serve(transport)
        .await
        .map_err(|_| failed("MCP Streamable HTTP initialization failed"))?;
    let info = service
        .peer_info()
        .ok_or_else(|| failed("MCP server omitted initialize metadata"))?;
    if info.capabilities.tools.is_none()
        || !ProtocolVersion::KNOWN_VERSIONS.contains(&info.protocol_version)
        || info.protocol_version > ProtocolVersion::LATEST
    {
        let _ = service.close_with_timeout(Duration::from_millis(500)).await;
        return Err(failed(format!(
            "MCP server negotiated unsupported protocol {} or omitted tools capability",
            info.protocol_version
        )));
    }
    let result = match operation {
        McpOperation::ListTools { cursor, .. } => service
            .list_tools(Some(
                PaginatedRequestParams::default().with_cursor(cursor.clone()),
            ))
            .await
            .map(RemoteOperationResult::Tools)
            .map_err(|_| failed("MCP Streamable HTTP operation failed")),
        McpOperation::CallTool {
            tool, arguments, ..
        } => {
            let arguments = arguments
                .as_object()
                .cloned()
                .ok_or_else(|| failed("MCP tool arguments must be an object"))?;
            call_dispatched.store(true, Ordering::Release);
            service
                .call_tool(CallToolRequestParams::new(tool.clone()).with_arguments(arguments))
                .await
                .map(RemoteOperationResult::Call)
                .map_err(|_| operation_error(operation, "MCP Streamable HTTP operation failed"))
        }
    };
    let _ = service.close_with_timeout(Duration::from_millis(500)).await;
    result
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

fn value_contains_secret(value: &Value, secrets: &[String]) -> bool {
    match value {
        Value::String(text) => secrets
            .iter()
            .any(|secret| !secret.is_empty() && text.contains(secret)),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_secret(value, secrets)),
        Value::Object(values) => values
            .values()
            .any(|value| value_contains_secret(value, secrets)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

pub(super) fn tools_page_contains_secret(page: &McpToolsPage, secrets: &[String]) -> bool {
    serde_json::to_value(page).is_ok_and(|value| value_contains_secret(&value, secrets))
}

fn redact_text(mut text: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret, "<redacted>");
    }
    text
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

pub(super) fn remote_timeout_error(
    operation: &McpOperation,
    call_dispatched: bool,
) -> ExecutionError {
    if operation.is_call() && call_dispatched {
        operation_error(operation, "MCP HTTP operation timed out")
    } else if operation.is_call() {
        failed("MCP HTTP operation timed out before tool dispatch")
    } else {
        failed("MCP HTTP operation timed out")
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
        if server.transport == McpTransportKind::StreamableHttp {
            let (headers, mut secrets) = resolve_http_headers(&server, &permit)?;
            let endpoint = server
                .url
                .as_deref()
                .ok_or_else(|| failed("MCP Streamable HTTP server has no endpoint"))?;
            let http =
                HardenedStreamableHttpClient::new(endpoint, &permit, &self.tls_roots).await?;
            let timeout = Duration::from_millis(permit.obligations().timeout_ms);
            let call_dispatched = AtomicBool::new(false);
            let result = if server.oauth.is_some() {
                let manager = self
                    .oauth_manager(
                        &server,
                        &permit.obligations().network_destinations,
                        &permit.obligations().allowed_environment,
                        permit.obligations().timeout_ms,
                        permit.obligations().max_output_bytes,
                    )
                    .await
                    .map_err(mcp_oauth_execution_error)?;
                let http = AuthClient::new(http, manager);
                let access_token = http
                    .get_access_token()
                    .await
                    .map_err(auth_execution_error)?;
                secrets.push(access_token);
                tokio::time::timeout(
                    timeout,
                    execute_remote_operation(
                        http,
                        &server,
                        &input.operation,
                        headers,
                        &call_dispatched,
                    ),
                )
                .await
            } else {
                tokio::time::timeout(
                    timeout,
                    execute_remote_operation(
                        http,
                        &server,
                        &input.operation,
                        headers,
                        &call_dispatched,
                    ),
                )
                .await
            }
            .map_err(|_| {
                remote_timeout_error(&input.operation, call_dispatched.load(Ordering::Acquire))
            })??;
            return match (&input.operation, result) {
                (McpOperation::ListTools { .. }, RemoteOperationResult::Tools(result)) => {
                    let page = parse_tools_result(result, &server)
                        .map_err(|error| failed(redact_text(error, &secrets)))?;
                    if tools_page_contains_secret(&page, &secrets) {
                        return Err(failed(
                            "MCP discovery contained a configured credential and was rejected",
                        ));
                    }
                    bounded_result(&page, max_output_bytes)
                }
                (McpOperation::CallTool { tool, .. }, RemoteOperationResult::Call(result)) => {
                    let output = parse_call_result(result, &server, tool, &secrets)
                        .map_err(|error| operation_error(&input.operation, error))?;
                    bounded_result(&output, max_output_bytes)
                        .map_err(|error| operation_error(&input.operation, error))
                }
                _ => Err(operation_error(
                    &input.operation,
                    "MCP HTTP response did not match the requested operation",
                )),
            };
        }
        let (environment, secrets) = resolve_environment(&server.environment)?;
        let protocol = protocol_input(&input.operation)?;
        let process = ProcessSpec {
            cwd: server
                .cwd
                .clone()
                .ok_or_else(|| failed("stdio MCP server has no working directory"))?,
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
                let page = parse_tools(&stdout, &server)
                    .map_err(|error| failed(redact_text(error, &secrets)))?;
                if tools_page_contains_secret(&page, &secrets) {
                    return Err(failed(
                        "MCP discovery contained a configured credential and was rejected",
                    ));
                }
                bounded_result(&page, max_output_bytes)
            }
            McpOperation::CallTool { tool, .. } => {
                let output = parse_call(&stdout, &server, tool, &secrets).map_err(|error| {
                    operation_error(&input.operation, redact_text(error, &secrets))
                })?;
                bounded_result(&output, max_output_bytes)
                    .map_err(|error| operation_error(&input.operation, error))
            }
        }
    }
}
