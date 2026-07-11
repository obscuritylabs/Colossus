//! Event-sourced integration connections, OpenAPI compilation, and permit-bound HTTP execution.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, EventClassification, ExecutionContext,
    IntegrationAuth, IntegrationConnection, IntegrationKind, IntegrationOperation,
    IntegrationStatus, IntegrationSummary, NewEvent, QuarantinedEffectResult, ToolSpec,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
use colossus_ports::{AggregateRepository, EventJournal, ExtensionRepository, StoreError};
use futures::StreamExt as _;
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

const MAX_CONNECTIONS: usize = 1_000;
const MAX_OPERATIONS: usize = 256;
const MAX_SCHEMA_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 8 * 1024;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

/// Immutable-journal implementation of extension connection state.
pub struct EventSourcedExtensionRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedExtensionRepository {
    /// Bind extension streams to the authoritative event journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(name: &str) -> String {
        format!("integration:{name}")
    }

    fn connection_events(
        &self,
        name: &str,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        self.journal.read_stream(&Self::stream(name))
    }

    fn reduce(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError> {
        let mut connection = None;
        for event in self.connection_events(name)? {
            match event.event_type.as_str() {
                "integration.connection_saved.v1" | "integration.disconnected.v1" => {
                    connection = Some(
                        serde_json::from_value(self.journal.decrypt_payload(&event)?)
                            .map_err(adapter)?,
                    );
                }
                _ => {}
            }
        }
        Ok(connection)
    }

    fn names(&self) -> Result<Vec<String>, StoreError> {
        let (head, _) = self.journal.head()?;
        let mut sequence = 1_u64;
        let mut names = BTreeSet::new();
        while sequence <= head {
            let events = self.journal.read_global(sequence, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(name) = event.stream_id.strip_prefix("integration:") {
                    names.insert(name.to_owned());
                }
            }
            sequence = events
                .last()
                .map_or(head.saturating_add(1), |event| event.global_sequence + 1);
        }
        Ok(names.into_iter().collect())
    }

    fn append(
        &self,
        connection: &IntegrationConnection,
        actor: Actor,
        event_type: &str,
    ) -> Result<(), StoreError> {
        let events = self.connection_events(&connection.name)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: Self::stream(&connection.name),
            expected_stream_version: events.len() as u64,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("integration:{}", connection.name),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(connection).map_err(adapter)?,
        })?;
        Ok(())
    }
}

impl AggregateRepository for EventSourcedExtensionRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.get_integration(id)?
            .map(serde_json::to_value)
            .transpose()
            .map_err(adapter)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        self.list_integrations(limit)?
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(adapter)
    }
}

impl ExtensionRepository for EventSourcedExtensionRepository {
    fn get_integration(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError> {
        validate_name(name)?;
        self.reduce(name)
    }

    fn list_integrations(&self, limit: usize) -> Result<Vec<IntegrationConnection>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "integration list limit must be in 1..=1000".into(),
            ));
        }
        self.names()?
            .into_iter()
            .take(limit)
            .filter_map(|name| self.reduce(&name).transpose())
            .collect()
    }

    fn save_integration(
        &self,
        connection: IntegrationConnection,
        actor: Actor,
    ) -> Result<IntegrationConnection, StoreError> {
        validate_connection(&connection)?;
        if let Some(existing) = self.reduce(&connection.name)?
            && existing.connected_at != connection.connected_at
        {
            return Err(StoreError::Adapter(
                "integration connected_at is immutable".into(),
            ));
        }
        self.append(&connection, actor, "integration.connection_saved.v1")?;
        Ok(connection)
    }

    fn disconnect_integration(
        &self,
        name: &str,
        actor: Actor,
        updated_at: &str,
    ) -> Result<IntegrationConnection, StoreError> {
        let mut connection = self
            .reduce(name)?
            .ok_or_else(|| StoreError::NotFound(format!("integration {name}")))?;
        connection.status = IntegrationStatus::Disconnected;
        connection.updated_at = updated_at.into();
        validate_connection(&connection)?;
        self.append(&connection, actor, "integration.disconnected.v1")?;
        Ok(connection)
    }
}

/// Strict effect payload shared by CLI/model callers and the integration adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntegrationRequest {
    /// Compile and persist a JSON OpenAPI connection.
    ImportOpenApi {
        /// Stable connection name.
        name: String,
        /// Full JSON document disclosed to policy before persistence.
        document: Value,
        /// Optional explicit base URL override.
        base_url: Option<String>,
        /// Credential placement.
        auth: IntegrationAuth,
        /// Environment-backed credential handle.
        credential_reference: Option<String>,
        /// Declared scopes.
        scopes: Vec<String>,
    },
    /// Explicitly disconnect one canonical connection.
    Disconnect {
        /// Connection name.
        name: String,
    },
    /// Invoke one exact operation from a connected manifest.
    Invoke {
        /// Connection name.
        connection: String,
        /// Exact generated tool name.
        tool_name: String,
        /// Strict model-visible operation arguments.
        arguments: Value,
    },
}

impl IntegrationRequest {
    /// Policy action identity.
    pub fn action(&self) -> &str {
        match self {
            Self::ImportOpenApi { .. } => "integration.openapi.import",
            Self::Disconnect { .. } => "integration.disconnect",
            Self::Invoke { tool_name, .. } => tool_name,
        }
    }

    /// Canonical resource identity.
    pub fn resource(&self) -> String {
        match self {
            Self::ImportOpenApi { name, .. } | Self::Disconnect { name } => {
                format!("integration:{name}")
            }
            Self::Invoke {
                connection,
                tool_name,
                ..
            } => format!("integration:{connection}:{tool_name}"),
        }
    }
}

/// Permit-bound connection management and HTTP operation adapter.
pub struct IntegrationExecutor {
    repository: Arc<dyn ExtensionRepository>,
    client: reqwest::Client,
}

impl IntegrationExecutor {
    /// Construct an adapter. Every effectful method still requires an opaque permit.
    pub fn new(repository: Arc<dyn ExtensionRepository>) -> Result<Self, StoreError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(adapter)?;
        Ok(Self { repository, client })
    }

    /// Connected tool specs in deterministic connection/operation order.
    pub fn tool_specs(&self) -> Result<Vec<ToolSpec>, StoreError> {
        let mut specs = self
            .repository
            .list_integrations(MAX_CONNECTIONS)?
            .into_iter()
            .filter(|connection| connection.status == IntegrationStatus::Connected)
            .flat_map(|connection| {
                connection
                    .operations
                    .into_iter()
                    .map(|operation| operation.tool)
            })
            .collect::<Vec<_>>();
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(specs)
    }

    /// Safe connection summaries without manifest bodies or credential values.
    pub fn summaries(&self, limit: usize) -> Result<Vec<IntegrationSummary>, StoreError> {
        Ok(self
            .repository
            .list_integrations(limit)?
            .into_iter()
            .map(|connection| IntegrationSummary {
                name: connection.name,
                kind: connection.kind,
                status: connection.status,
                title: connection.title,
                credential_reference: connection.credential_reference,
                tools: connection
                    .operations
                    .into_iter()
                    .map(|operation| operation.tool.name)
                    .collect(),
                updated_at: connection.updated_at,
            })
            .collect::<Vec<_>>())
    }

    /// Reconstruct one canonical connection.
    pub fn get_connection(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError> {
        self.repository.get_integration(name)
    }

    /// Build an invocation and its exact credential disclosure for a dynamic tool.
    pub fn invocation(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Option<(IntegrationRequest, Vec<CredentialReference>)>, StoreError> {
        for connection in self.repository.list_integrations(MAX_CONNECTIONS)? {
            if connection.status != IntegrationStatus::Connected {
                continue;
            }
            if connection
                .operations
                .iter()
                .any(|operation| operation.tool.name == tool_name)
            {
                let credentials = connection
                    .credential_reference
                    .as_ref()
                    .map(|reference| {
                        vec![CredentialReference {
                            reference: reference.clone(),
                            value_hash: None,
                        }]
                    })
                    .unwrap_or_default();
                return Ok(Some((
                    IntegrationRequest::Invoke {
                        connection: connection.name,
                        tool_name: tool_name.into(),
                        arguments,
                    },
                    credentials,
                )));
            }
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    fn import(
        &self,
        _permit: &ExecutionPermit,
        actor: Actor,
        name: &str,
        document: &Value,
        base_url: Option<&str>,
        auth: &IntegrationAuth,
        credential_reference: Option<&str>,
        scopes: &[String],
    ) -> Result<IntegrationConnection, ExecutionError> {
        let existing = self.repository.get_integration(name).map_err(execution)?;
        let now = now().map_err(execution)?;
        let mut connection = compile_openapi(
            name,
            document,
            base_url,
            auth.clone(),
            credential_reference.map(Into::into),
            scopes.to_vec(),
            existing
                .as_ref()
                .map_or_else(|| now.clone(), |value| value.connected_at.clone()),
            now,
        )
        .map_err(execution)?;
        if auth_requires_credential(auth)
            && credential_reference.is_some_and(|reference| resolve_environment(reference).is_err())
        {
            connection.status = IntegrationStatus::PendingAuth;
        }
        self.repository
            .save_integration(connection, actor)
            .map_err(execution)
    }

    async fn invoke(
        &self,
        permit: &ExecutionPermit,
        connection_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<Value, ExecutionError> {
        let connection = self
            .repository
            .get_integration(connection_name)
            .map_err(execution)?
            .ok_or_else(|| execution("integration connection is unavailable"))?;
        if connection.status != IntegrationStatus::Connected {
            return Err(execution("integration is not connected"));
        }
        let operation = connection
            .operations
            .iter()
            .find(|operation| operation.tool.name == tool_name)
            .ok_or_else(|| execution("integration operation is unavailable"))?;
        jsonschema::validator_for(&operation.tool.input_schema)
            .map_err(execution)?
            .validate(arguments)
            .map_err(execution)?;
        let mut url = operation_url(&connection, operation, arguments)?;
        add_query(&mut url, operation, arguments)?;
        require_origin(&url, permit)?;
        let method = reqwest::Method::from_bytes(operation.method.as_bytes()).map_err(execution)?;
        let mut request = self
            .client
            .request(method.clone(), url)
            .timeout(Duration::from_millis(permit.obligations().timeout_ms))
            .header("accept", "application/json")
            .header("user-agent", "colossus-rs/0.6");
        let credential_value = connection
            .credential_reference
            .as_deref()
            .map(resolve_environment)
            .transpose()?;
        if let Some(secret) = credential_value.as_deref() {
            let (name, value) = auth_header(&connection.auth, secret)?;
            request = request.header(name, value);
        }
        if operation.accepts_body
            && let Some(body) = arguments.get("body")
        {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|error| {
            if method == reqwest::Method::GET {
                execution(format!("integration request failed: {}", error.classify()))
            } else {
                ExecutionError::OutcomeUnknown(format!(
                    "integration transport failed after a potentially mutating request: {}",
                    error.classify()
                ))
            }
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .chars()
            .take(256)
            .collect::<String>();
        let limit = usize::try_from(
            permit
                .obligations()
                .max_output_bytes
                .min(operation.tool.max_output_bytes),
        )
        .map_err(execution)?;
        let mut bytes = bounded_response(response, limit.saturating_sub(1_024)).await?;
        if let Some(secret) = credential_value.as_deref() {
            bytes = redact_exact_secret(&bytes, secret.as_bytes());
        }
        let result = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        Ok(json!({
            "status_code": status,
            "content_type": content_type,
            "result": result,
        }))
    }
}

fn redact_exact_secret(bytes: &[u8], secret: &[u8]) -> Vec<u8> {
    if secret.is_empty() || secret.len() > bytes.len() {
        return bytes.to_vec();
    }
    let mut redacted = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset..].starts_with(secret) {
            redacted.extend_from_slice(b"[REDACTED]");
            offset += secret.len();
        } else {
            redacted.push(bytes[offset]);
            offset += 1;
        }
    }
    redacted
}

#[async_trait]
impl EffectExecutor for IntegrationExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: IntegrationRequest =
            serde_json::from_value(request.content.clone()).map_err(execution)?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(execution(
                "integration request does not match its authorized action and resource",
            ));
        }
        validate_request_credentials(
            &operation,
            &request.credential_references,
            self.repository.as_ref(),
        )?;
        let value = match operation {
            IntegrationRequest::ImportOpenApi {
                name,
                document,
                base_url,
                auth,
                credential_reference,
                scopes,
            } => serde_json::to_value(self.import(
                &permit,
                request.actor.clone(),
                &name,
                &document,
                base_url.as_deref(),
                &auth,
                credential_reference.as_deref(),
                &scopes,
            )?)
            .map_err(execution)?,
            IntegrationRequest::Disconnect { name } => serde_json::to_value(
                self.repository
                    .disconnect_integration(
                        &name,
                        request.actor.clone(),
                        &now().map_err(execution)?,
                    )
                    .map_err(execution)?,
            )
            .map_err(execution)?,
            IntegrationRequest::Invoke {
                connection,
                tool_name,
                arguments,
            } => {
                self.invoke(&permit, &connection, &tool_name, &arguments)
                    .await?
            }
        };
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value).map_err(execution)?,
            effect_succeeded: true,
        })
    }
}

/// Compile a bounded JSON OpenAPI 3 document into one canonical connection.
#[allow(clippy::too_many_arguments)]
pub fn compile_openapi(
    name: &str,
    document: &Value,
    base_url: Option<&str>,
    auth: IntegrationAuth,
    credential_reference: Option<String>,
    scopes: Vec<String>,
    connected_at: String,
    updated_at: String,
) -> Result<IntegrationConnection, StoreError> {
    validate_name(name)?;
    validate_auth(&auth)?;
    validate_credential_reference(credential_reference.as_deref())?;
    let bytes = serde_json::to_vec(document).map_err(adapter)?;
    if bytes.len() > MAX_SCHEMA_BYTES {
        return Err(StoreError::Adapter("OpenAPI document exceeds 1 MiB".into()));
    }
    let root = document
        .as_object()
        .ok_or_else(|| StoreError::Adapter("OpenAPI document must be an object".into()))?;
    if !root
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|version| version.starts_with("3."))
    {
        return Err(StoreError::Adapter(
            "only OpenAPI 3.x JSON documents are supported".into(),
        ));
    }
    let info = root.get("info").and_then(Value::as_object);
    let title = info
        .and_then(|value| value.get("title"))
        .and_then(Value::as_str)
        .unwrap_or(name)
        .trim()
        .chars()
        .take(512)
        .collect::<String>();
    let description = info
        .and_then(|value| value.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("Imported OpenAPI integration")
        .trim()
        .chars()
        .take(MAX_DESCRIPTION_BYTES)
        .collect::<String>();
    let base_url: String = base_url
        .map(str::to_owned)
        .or_else(|| {
            root.get("servers")
                .and_then(Value::as_array)
                .and_then(|servers| servers.first())
                .and_then(Value::as_object)
                .and_then(|server| server.get("url"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| StoreError::Adapter("OpenAPI base URL is required".into()))?;
    validate_base_url(&base_url)?;
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| StoreError::Adapter("OpenAPI paths object is required".into()))?;
    let mut operations = Vec::new();
    let mut tool_names = BTreeSet::new();
    for (path, item) in paths {
        validate_api_path(path)?;
        let item = item
            .as_object()
            .ok_or_else(|| StoreError::Adapter("OpenAPI path item must be an object".into()))?;
        let path_parameters = item.get("parameters").and_then(Value::as_array);
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = item.get(method).and_then(Value::as_object) else {
                continue;
            };
            if operations.len() >= MAX_OPERATIONS {
                return Err(StoreError::Adapter(
                    "OpenAPI operation count exceeds 256".into(),
                ));
            }
            let compiled = compile_operation(name, method, path, path_parameters, operation)?;
            if !tool_names.insert(compiled.tool.name.clone()) {
                return Err(StoreError::Adapter(
                    "OpenAPI operation tool names must be unique".into(),
                ));
            }
            operations.push(compiled);
        }
    }
    if operations.is_empty() {
        return Err(StoreError::Adapter(
            "OpenAPI document contains no supported operations".into(),
        ));
    }
    let status = if auth_requires_credential(&auth) && credential_reference.is_none() {
        IntegrationStatus::PendingAuth
    } else {
        IntegrationStatus::Connected
    };
    let connection = IntegrationConnection {
        name: name.into(),
        kind: IntegrationKind::OpenApi,
        status,
        title,
        description,
        base_url,
        auth,
        credential_reference,
        scopes,
        operations,
        manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
        connected_at,
        updated_at,
    };
    validate_connection(&connection)?;
    Ok(connection)
}

fn compile_operation(
    integration: &str,
    method: &str,
    path: &str,
    path_parameters: Option<&Vec<Value>>,
    operation: &Map<String, Value>,
) -> Result<IntegrationOperation, StoreError> {
    let operation_id = operation
        .get("operationId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{method}_{}", path.trim_matches('/').replace('/', "_")));
    let segment = sanitize_segment(&operation_id)?;
    let tool_name = format!("openapi.{integration}.{segment}");
    let description = operation
        .get("description")
        .or_else(|| operation.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or("Imported OpenAPI operation")
        .chars()
        .take(MAX_DESCRIPTION_BYTES)
        .collect::<String>();
    let mut properties = Map::new();
    let mut required = BTreeSet::<String>::new();
    let mut path_names = Vec::new();
    let mut query_names = Vec::new();
    for parameter in path_parameters.into_iter().flatten().chain(
        operation
            .get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    ) {
        let parameter = parameter.as_object().ok_or_else(|| {
            StoreError::Adapter("OpenAPI parameters must be inline objects".into())
        })?;
        let name = parameter
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| StoreError::Adapter("OpenAPI parameter name is required".into()))?;
        validate_argument_name(name)?;
        let location = parameter
            .get("in")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(location, "path" | "query") {
            continue;
        }
        properties.insert(
            name.into(),
            simple_schema(
                parameter.get("schema").unwrap_or(&json!({"type":"string"})),
                0,
            )?,
        );
        if location == "path" {
            path_names.push(name.into());
            required.insert(name.into());
        } else {
            query_names.push(name.into());
            if parameter.get("required") == Some(&Value::Bool(true)) {
                required.insert(name.into());
            }
        }
    }
    for name in &path_names {
        if !path.contains(&format!("{{{name}}}")) {
            return Err(StoreError::Adapter(format!(
                "OpenAPI path parameter {name} is absent from its template"
            )));
        }
    }
    let mut accepts_body = false;
    if let Some(body) = operation.get("requestBody").and_then(Value::as_object) {
        let schema = body
            .get("content")
            .and_then(Value::as_object)
            .and_then(|content| content.get("application/json"))
            .and_then(Value::as_object)
            .and_then(|media| media.get("schema"))
            .map_or_else(
                || Ok(json!({"type":"object"})),
                |schema| simple_schema(schema, 0),
            )?;
        properties.insert("body".into(), schema);
        accepts_body = true;
        if body.get("required") == Some(&Value::Bool(true)) {
            required.insert("body".into());
        }
    }
    let input_schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    jsonschema::validator_for(&input_schema).map_err(adapter)?;
    Ok(IntegrationOperation {
        tool: ToolSpec {
            name: tool_name.clone(),
            description,
            input_schema,
            effect_action: Some(tool_name.clone()),
            capability: Some("integration.invoke".into()),
            max_output_bytes: 64_000,
        },
        operation_id,
        method: method.to_ascii_uppercase(),
        path: path.into(),
        path_parameters: path_names,
        query_parameters: query_names,
        accepts_body,
    })
}

fn simple_schema(value: &Value, depth: usize) -> Result<Value, StoreError> {
    if depth > 8 {
        return Err(StoreError::Adapter(
            "OpenAPI schema nesting exceeds 8".into(),
        ));
    }
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::Adapter("OpenAPI schemas must be objects".into()))?;
    if object.contains_key("$ref") {
        return Err(StoreError::Adapter(
            "OpenAPI schema references are not supported by the bounded importer".into(),
        ));
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object");
    if !matches!(
        kind,
        "string" | "integer" | "number" | "boolean" | "array" | "object"
    ) {
        return Err(StoreError::Adapter(
            "unsupported OpenAPI schema type".into(),
        ));
    }
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String(kind.into()));
    for key in [
        "description",
        "enum",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "format",
        "default",
    ] {
        if let Some(value) = object.get(key) {
            schema.insert(key.into(), value.clone());
        }
    }
    if kind == "array" {
        schema.insert(
            "items".into(),
            simple_schema(
                object.get("items").unwrap_or(&json!({"type":"string"})),
                depth + 1,
            )?,
        );
        schema.insert("maxItems".into(), json!(1_000));
    }
    if kind == "object" {
        let mut properties = Map::new();
        if let Some(values) = object.get("properties").and_then(Value::as_object) {
            if values.len() > 256 {
                return Err(StoreError::Adapter(
                    "OpenAPI object property count exceeds 256".into(),
                ));
            }
            for (name, value) in values {
                validate_argument_name(name)?;
                properties.insert(name.clone(), simple_schema(value, depth + 1)?);
            }
        }
        schema.insert("properties".into(), Value::Object(properties));
        schema.insert("additionalProperties".into(), Value::Bool(false));
        if let Some(required) = object.get("required") {
            schema.insert("required".into(), required.clone());
        }
    }
    Ok(Value::Object(schema))
}

fn validate_connection(connection: &IntegrationConnection) -> Result<(), StoreError> {
    validate_name(&connection.name)?;
    validate_base_url(&connection.base_url)?;
    validate_auth(&connection.auth)?;
    validate_credential_reference(connection.credential_reference.as_deref())?;
    if connection.title.trim().is_empty()
        || connection.title.len() > 512
        || connection.description.len() > MAX_DESCRIPTION_BYTES
        || connection.operations.is_empty()
        || connection.operations.len() > MAX_OPERATIONS
        || connection.manifest_sha256.len() != 64
        || connection.connected_at.is_empty()
        || connection.updated_at.is_empty()
        || connection.scopes.len() > 128
        || connection
            .scopes
            .iter()
            .any(|scope| scope.is_empty() || scope.len() > 512)
    {
        return Err(StoreError::Adapter(
            "integration connection violates identity or size bounds".into(),
        ));
    }
    if auth_requires_credential(&connection.auth)
        && connection.status == IntegrationStatus::Connected
        && connection.credential_reference.is_none()
    {
        return Err(StoreError::Adapter(
            "connected authenticated integration requires a credential reference".into(),
        ));
    }
    if !auth_requires_credential(&connection.auth) && connection.credential_reference.is_some() {
        return Err(StoreError::Adapter(
            "auth-none integrations cannot retain a credential reference".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for operation in &connection.operations {
        if !names.insert(operation.tool.name.as_str())
            || operation.tool.effect_action.as_deref() != Some(&operation.tool.name)
            || operation.tool.capability.as_deref() != Some("integration.invoke")
            || !operation
                .tool
                .name
                .starts_with(&format!("openapi.{}.", connection.name))
            || !matches!(
                operation.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            )
        {
            return Err(StoreError::Adapter(
                "integration operation identity is invalid".into(),
            ));
        }
        jsonschema::validator_for(&operation.tool.input_schema).map_err(adapter)?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), StoreError> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_lowercase()
            } else {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            }
        });
    if valid {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration names must be bounded lowercase identifiers".into(),
        ))
    }
}

fn validate_argument_name(name: &str) -> Result<(), StoreError> {
    if !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "OpenAPI argument name is invalid".into(),
        ))
    }
}

fn sanitize_segment(value: &str) -> Result<String, StoreError> {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() || value.len() > 128 {
        Err(StoreError::Adapter(
            "OpenAPI operation id is invalid".into(),
        ))
    } else {
        Ok(value)
    }
}

fn validate_api_path(path: &str) -> Result<(), StoreError> {
    if path.starts_with('/')
        && path.len() <= 4_096
        && !path.contains(['?', '#', '\\'])
        && path
            .split('/')
            .all(|component| !matches!(component, "." | ".."))
    {
        Ok(())
    } else {
        Err(StoreError::Adapter("invalid OpenAPI operation path".into()))
    }
}

fn validate_base_url(value: &str) -> Result<(), StoreError> {
    let url = Url::parse(value).map_err(adapter)?;
    if matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
    {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration base URL requires HTTP(S), a host, and no credentials/query/fragment"
                .into(),
        ))
    }
}

fn validate_auth(auth: &IntegrationAuth) -> Result<(), StoreError> {
    match auth {
        IntegrationAuth::None => Ok(()),
        IntegrationAuth::Bearer { header, scheme }
            if valid_header(header) && !scheme.is_empty() && scheme.len() <= 64 =>
        {
            Ok(())
        }
        IntegrationAuth::ApiKey { header, scheme }
            if valid_header(header)
                && scheme
                    .as_ref()
                    .is_none_or(|value| !value.is_empty() && value.len() <= 64) =>
        {
            Ok(())
        }
        IntegrationAuth::ServiceAccount { header } if valid_header(header) => Ok(()),
        _ => Err(StoreError::Adapter(
            "integration auth header or scheme is invalid".into(),
        )),
    }
}

fn valid_header(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && HeaderName::from_bytes(value.as_bytes()).is_ok()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "host" | "content-length"
        )
}

fn validate_credential_reference(value: Option<&str>) -> Result<(), StoreError> {
    if value.is_none_or(valid_environment_reference) {
        Ok(())
    } else {
        Err(StoreError::Adapter(
            "integration credentials must use env:VARIABLE references".into(),
        ))
    }
}

fn valid_environment_reference(value: &str) -> bool {
    value.strip_prefix("env:").is_some_and(|name| {
        let mut bytes = name.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    })
}

fn auth_requires_credential(auth: &IntegrationAuth) -> bool {
    !matches!(auth, IntegrationAuth::None)
}

fn resolve_environment(reference: &str) -> Result<String, ExecutionError> {
    let name = reference
        .strip_prefix("env:")
        .filter(|_| valid_environment_reference(reference))
        .ok_or_else(|| execution("invalid integration credential reference"))?;
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 64 * 1024 && !value.contains('\0'))
        .ok_or_else(|| execution(format!("integration credential {reference} is unavailable")))
}

fn validate_request_credentials(
    operation: &IntegrationRequest,
    disclosed: &[CredentialReference],
    repository: &dyn ExtensionRepository,
) -> Result<(), ExecutionError> {
    let expected = match operation {
        IntegrationRequest::ImportOpenApi {
            credential_reference,
            ..
        } => credential_reference.clone(),
        IntegrationRequest::Disconnect { .. } => None,
        IntegrationRequest::Invoke { connection, .. } => repository
            .get_integration(connection)
            .map_err(execution)?
            .and_then(|connection| connection.credential_reference),
    };
    let matches = match expected.as_deref() {
        None => disclosed.is_empty(),
        Some(expected) => {
            disclosed.len() == 1
                && disclosed[0].reference == expected
                && disclosed[0].value_hash.is_none()
        }
    };
    if matches {
        Ok(())
    } else {
        Err(execution(
            "integration credential disclosure does not match the canonical connection",
        ))
    }
}

fn auth_header(
    auth: &IntegrationAuth,
    secret: &str,
) -> Result<(HeaderName, HeaderValue), ExecutionError> {
    let (header, value) = match auth {
        IntegrationAuth::None => return Err(execution("credential supplied for auth none")),
        IntegrationAuth::Bearer { header, scheme } => (header, format!("{scheme} {secret}")),
        IntegrationAuth::ApiKey { header, scheme } => (
            header,
            scheme
                .as_ref()
                .map_or_else(|| secret.into(), |scheme| format!("{scheme} {secret}")),
        ),
        IntegrationAuth::ServiceAccount { header } => (header, secret.into()),
    };
    Ok((
        HeaderName::from_bytes(header.as_bytes()).map_err(execution)?,
        HeaderValue::from_str(&value).map_err(execution)?,
    ))
}

fn operation_url(
    connection: &IntegrationConnection,
    operation: &IntegrationOperation,
    arguments: &Value,
) -> Result<Url, ExecutionError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| execution("integration arguments must be an object"))?;
    let mut path = operation.path.clone();
    for name in &operation.path_parameters {
        let value = scalar(
            object
                .get(name)
                .ok_or_else(|| execution(format!("missing integration path argument {name}")))?,
        )?;
        let encoded = encode_path_segment(&value);
        path = path.replace(&format!("{{{name}}}"), &encoded);
    }
    if path.contains(['{', '}']) {
        return Err(execution(
            "integration path contains an undeclared template parameter",
        ));
    }
    let base = Url::parse(&connection.base_url).map_err(execution)?;
    base.join(path.trim_start_matches('/')).map_err(execution)
}

fn encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn add_query(
    url: &mut Url,
    operation: &IntegrationOperation,
    arguments: &Value,
) -> Result<(), ExecutionError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| execution("integration arguments must be an object"))?;
    let mut query = url.query_pairs_mut();
    for name in &operation.query_parameters {
        if let Some(value) = object.get(name) {
            if let Some(values) = value.as_array() {
                for value in values {
                    query.append_pair(name, &scalar(value)?);
                }
            } else {
                query.append_pair(name, &scalar(value)?);
            }
        }
    }
    Ok(())
}

fn scalar(value: &Value) -> Result<String, ExecutionError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(execution("integration path/query values must be scalar")),
    }
}

fn canonical_origin(url: &Url) -> Result<String, ExecutionError> {
    let host = url
        .host_str()
        .ok_or_else(|| execution("integration URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| execution("unknown URL port"))?;
    Ok(format!(
        "{}://{}:{port}",
        url.scheme(),
        host.to_ascii_lowercase()
    ))
}

fn require_origin(url: &Url, permit: &ExecutionPermit) -> Result<(), ExecutionError> {
    let requested = canonical_origin(url)?;
    let allowed = permit
        .obligations()
        .network_destinations
        .iter()
        .filter_map(|value| Url::parse(value).ok())
        .filter_map(|value| canonical_origin(&value).ok())
        .any(|value| value == requested);
    if allowed {
        Ok(())
    } else {
        Err(execution(format!(
            "integration origin {requested} is not permitted"
        )))
    }
}

async fn bounded_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ExecutionError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(execution("integration response exceeds output bound"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(execution)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(execution("integration response exceeds output bound"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

trait ReqwestErrorClass {
    fn classify(&self) -> &'static str;
}

impl ReqwestErrorClass for reqwest::Error {
    fn classify(&self) -> &'static str {
        if self.is_timeout() {
            "timeout"
        } else if self.is_connect() {
            "connect"
        } else if self.is_request() {
            "request"
        } else {
            "transport"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EventSourcedExtensionRepository, IntegrationExecutor, IntegrationRequest, compile_openapi,
        redact_exact_secret,
    };
    use colossus_contracts::{DecisionOutcome, IntegrationAuth, IntegrationStatus};
    use colossus_policy::{EffectExecutor, system_actor};
    use colossus_ports::{EventJournal, ExtensionRepository};
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::sync::Arc;

    fn document() -> serde_json::Value {
        json!({
            "openapi": "3.1.0",
            "info": {"title": "Demo", "description": "Demo API"},
            "servers": [{"url": "https://api.example.test/v1/"}],
            "paths": {
                "/widgets/{id}": {
                    "get": {
                        "operationId": "getWidget",
                        "parameters": [
                            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                            {"name": "expand", "in": "query", "schema": {"type": "boolean"}}
                        ]
                    },
                    "patch": {
                        "operationId": "updateWidget",
                        "parameters": [
                            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}
                        ],
                        "requestBody": {
                            "required": true,
                            "content": {"application/json": {"schema": {
                                "type": "object",
                                "properties": {"name": {"type": "string", "maxLength": 100}},
                                "required": ["name"]
                            }}}
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn openapi_compilation_maps_path_query_and_body_without_auth_arguments() {
        let connection = compile_openapi(
            "demo",
            &document(),
            None,
            IntegrationAuth::Bearer {
                header: "Authorization".into(),
                scheme: "Bearer".into(),
            },
            Some("env:DEMO_TOKEN".into()),
            vec!["widgets:read".into()],
            "2026-01-01T00:00:00Z".into(),
            "2026-01-01T00:00:00Z".into(),
        )
        .expect("compile");
        assert_eq!(connection.status, IntegrationStatus::Connected);
        assert_eq!(connection.operations.len(), 2);
        let read = connection
            .operations
            .iter()
            .find(|operation| operation.method == "GET")
            .expect("read");
        assert_eq!(read.tool.name, "openapi.demo.getwidget");
        assert_eq!(read.path_parameters, ["id"]);
        assert_eq!(read.query_parameters, ["expand"]);
        let schema = serde_json::to_string(&read.tool.input_schema).expect("schema");
        assert!(!schema.contains("credential"));
        assert!(!schema.contains("Authorization"));
        let update = connection
            .operations
            .iter()
            .find(|operation| operation.method == "PATCH")
            .expect("update");
        assert!(update.accepts_body);
        assert!(
            update.tool.input_schema["required"]
                .as_array()
                .is_some_and(|required| required.contains(&json!("body")))
        );
    }

    #[test]
    fn extension_repository_reconstructs_reconnect_and_disconnect_history() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedExtensionRepository::new(journal);
        let connection = compile_openapi(
            "demo",
            &document(),
            None,
            IntegrationAuth::None,
            None,
            Vec::new(),
            "2026-01-01T00:00:00Z".into(),
            "2026-01-01T00:00:00Z".into(),
        )
        .expect("compile");
        repository
            .save_integration(connection, system_actor("test"))
            .expect("save");
        assert_eq!(repository.list_integrations(10).expect("list").len(), 1);
        let disconnected = repository
            .disconnect_integration("demo", system_actor("test"), "2026-01-02T00:00:00Z")
            .expect("disconnect");
        assert_eq!(disconnected.status, IntegrationStatus::Disconnected);
        assert_eq!(
            repository
                .get_integration("demo")
                .expect("get")
                .expect("connection")
                .status,
            IntegrationStatus::Disconnected
        );
    }

    #[test]
    fn importer_rejects_refs_embedded_origins_and_unsupported_schema_references() {
        let mut invalid = document();
        invalid["paths"]["/widgets/{id}"]["get"]["parameters"][0]["schema"] =
            json!({"$ref": "#/components/schemas/Id"});
        assert!(
            compile_openapi(
                "demo",
                &invalid,
                Some("https://user:secret@example.test"),
                IntegrationAuth::None,
                None,
                Vec::new(),
                "now".into(),
                "now".into(),
            )
            .is_err()
        );
        assert!(
            compile_openapi(
                "demo",
                &invalid,
                Some("https://example.test"),
                IntegrationAuth::None,
                None,
                Vec::new(),
                "now".into(),
                "now".into(),
            )
            .is_err()
        );
    }

    #[test]
    fn exact_credential_values_are_removed_from_quarantined_responses() {
        assert_eq!(
            redact_exact_secret(
                br#"{"authorization":"Bearer secret-token"}"#,
                b"secret-token"
            ),
            br#"{"authorization":"Bearer [REDACTED]"}"#
        );
    }

    #[tokio::test]
    async fn canonical_credential_reference_mismatch_fails_before_network_execution() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn ExtensionRepository> =
            Arc::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)));
        let connection = compile_openapi(
            "demo",
            &document(),
            Some("http://127.0.0.1:9/v1/"),
            IntegrationAuth::Bearer {
                header: "Authorization".into(),
                scheme: "Bearer".into(),
            },
            Some("env:PATH".into()),
            Vec::new(),
            "2026-01-01T00:00:00Z".into(),
            "2026-01-01T00:00:00Z".into(),
        )
        .expect("connection");
        repository
            .save_integration(connection, system_actor("test"))
            .expect("save");
        let executor = IntegrationExecutor::new(repository).expect("executor");
        let operation = IntegrationRequest::Invoke {
            connection: "demo".into(),
            tool_name: "openapi.demo.getwidget".into(),
            arguments: json!({"id":"1"}),
        };
        let mut request = colossus_policy::effect_request(
            system_actor("test"),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation).expect("request"),
        );
        request.capabilities = vec!["integration.invoke".into()];
        let gateway = colossus_policy::EffectGateway::new(
            journal,
            Arc::new(
                colossus_policy::BuiltInPolicy::offline_default()
                    .with_action("openapi.demo.getwidget", DecisionOutcome::Allow)
                    .with_network_destination("http://127.0.0.1:9"),
            ),
            Arc::new(colossus_policy::DenyApproval),
            colossus_policy::SafetyKernel::new(["integration.invoke".into()]),
            [31_u8; 32],
        );
        let error = gateway
            .execute(request, &executor as &dyn EffectExecutor)
            .await
            .expect_err("mismatched disclosure must fail");
        assert!(error.to_string().contains("credential disclosure"));
    }
}
