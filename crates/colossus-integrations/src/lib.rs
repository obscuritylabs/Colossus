//! Event-sourced integration connections, OpenAPI compilation, and permit-bound HTTP execution.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, CredentialReference, EffectRequest, EventClassification, ExecutionContext,
    IntegrationAuth, IntegrationConnection, IntegrationKind, IntegrationOperation,
    IntegrationStatus, IntegrationSummary, NewEvent, PackInstallation, PackStatus, PublisherTrust,
    QuarantinedEffectResult, ToolSpec,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
use colossus_ports::{AggregateRepository, EventJournal, ExtensionRepository, StoreError};
use futures::StreamExt as _;
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
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

    fn pack_stream(name: &str) -> String {
        format!("pack:{name}")
    }

    fn trust_stream(publisher: &str, key_id: &str) -> String {
        format!("publisher-trust:{publisher}:{key_id}")
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

    fn stream_names(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        let (head, _) = self.journal.head()?;
        let mut sequence = 1_u64;
        let mut names = BTreeSet::new();
        while sequence <= head {
            let events = self.journal.read_global(sequence, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(name) = event.stream_id.strip_prefix(prefix) {
                    names.insert(name.to_owned());
                }
            }
            sequence = events
                .last()
                .map_or(head.saturating_add(1), |event| event.global_sequence + 1);
        }
        Ok(names.into_iter().collect())
    }

    fn reduce_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError> {
        let mut installation = None;
        for event in self.journal.read_stream(&Self::pack_stream(name))? {
            if matches!(
                event.event_type.as_str(),
                "pack.installed.v1"
                    | "pack.enabled.v1"
                    | "pack.disabled.v1"
                    | "pack.uninstalled.v1"
            ) {
                installation = Some(
                    serde_json::from_value(self.journal.decrypt_payload(&event)?)
                        .map_err(adapter)?,
                );
            }
        }
        Ok(installation)
    }

    fn reduce_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError> {
        let events = self
            .journal
            .read_stream(&Self::trust_stream(publisher, key_id))?;
        events
            .last()
            .map(|event| {
                serde_json::from_value(self.journal.decrypt_payload(event)?).map_err(adapter)
            })
            .transpose()
    }

    fn append_pack(
        &self,
        installation: &PackInstallation,
        actor: Actor,
        event_type: &str,
    ) -> Result<(), StoreError> {
        let stream_id = Self::pack_stream(&installation.manifest.name);
        let events = self.journal.read_stream(&stream_id)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: events.len() as u64,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("pack:{}", installation.manifest.name),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(installation).map_err(adapter)?,
        })?;
        Ok(())
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

    fn get_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError> {
        validate_name(name)?;
        self.reduce_pack(name)
    }

    fn list_packs(&self, limit: usize) -> Result<Vec<PackInstallation>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "pack list limit must be in 1..=1000".into(),
            ));
        }
        self.stream_names("pack:")?
            .into_iter()
            .take(limit)
            .filter_map(|name| self.reduce_pack(&name).transpose())
            .collect()
    }

    fn install_pack(
        &self,
        installation: PackInstallation,
        actor: Actor,
    ) -> Result<PackInstallation, StoreError> {
        validate_name(&installation.manifest.name)?;
        if installation.status == PackStatus::Uninstalled {
            return Err(StoreError::Adapter(
                "a new pack installation cannot start uninstalled".into(),
            ));
        }
        if let Some(existing) = self.reduce_pack(&installation.manifest.name)?
            && existing.status != PackStatus::Uninstalled
        {
            return Err(StoreError::Adapter(format!(
                "pack {} is already installed",
                installation.manifest.name
            )));
        }
        self.append_pack(&installation, actor, "pack.installed.v1")?;
        Ok(installation)
    }

    fn set_pack_status(
        &self,
        name: &str,
        status: PackStatus,
        actor: Actor,
        updated_at: &str,
    ) -> Result<PackInstallation, StoreError> {
        validate_name(name)?;
        let mut installation = self
            .reduce_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        if installation.status == PackStatus::Uninstalled {
            return Err(StoreError::Adapter(format!(
                "pack {name} has already been uninstalled"
            )));
        }
        installation.status = status;
        installation.updated_at = updated_at.into();
        let event_type = match status {
            PackStatus::Enabled => "pack.enabled.v1",
            PackStatus::Disabled => "pack.disabled.v1",
            PackStatus::Uninstalled => "pack.uninstalled.v1",
        };
        self.append_pack(&installation, actor, event_type)?;
        Ok(installation)
    }

    fn add_publisher_trust(
        &self,
        trust: PublisherTrust,
        actor: Actor,
    ) -> Result<PublisherTrust, StoreError> {
        validate_name(&trust.publisher)?;
        if self
            .reduce_trust(&trust.publisher, &trust.key_id)?
            .is_some()
        {
            return Err(StoreError::Adapter(format!(
                "publisher trust {}:{} already exists",
                trust.publisher, trust.key_id
            )));
        }
        let stream_id = Self::trust_stream(&trust.publisher, &trust.key_id);
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "publisher.trusted.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("publisher-trust:{}", trust.publisher),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(&trust).map_err(adapter)?,
        })?;
        Ok(trust)
    }

    fn get_publisher_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError> {
        validate_name(publisher)?;
        self.reduce_trust(publisher, key_id)
    }

    fn list_publisher_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "publisher trust list limit must be in 1..=1000".into(),
            ));
        }
        self.stream_names("publisher-trust:")?
            .into_iter()
            .take(limit)
            .filter_map(|suffix| {
                let (publisher, key_id) = suffix.rsplit_once(':')?;
                self.reduce_trust(publisher, key_id).transpose()
            })
            .collect()
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
    /// Compile and persist one built-in native connector.
    ConnectNative {
        /// `github`, `searxng`, or `opensearch`.
        name: String,
        /// Optional endpoint override.
        base_url: Option<String>,
        /// Credential placement.
        auth: IntegrationAuth,
        /// Single bearer/API-key handle.
        credential_reference: Option<String>,
        /// Named handles such as username/password.
        credential_references: BTreeMap<String, String>,
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
            Self::ConnectNative { .. } => "integration.connect",
            Self::Disconnect { .. } => "integration.disconnect",
            Self::Invoke { tool_name, .. } => tool_name,
        }
    }

    /// Canonical resource identity.
    pub fn resource(&self) -> String {
        match self {
            Self::ImportOpenApi { name, .. }
            | Self::ConnectNative { name, .. }
            | Self::Disconnect { name } => {
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
                credential_references: connection.credential_references,
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
                let mut credentials = connection
                    .credential_reference
                    .as_ref()
                    .map(|reference| {
                        vec![CredentialReference {
                            reference: reference.clone(),
                            value_hash: None,
                        }]
                    })
                    .unwrap_or_default();
                credentials.extend(connection.credential_references.values().map(|reference| {
                    CredentialReference {
                        reference: reference.clone(),
                        value_hash: None,
                    }
                }));
                credentials.sort_by(|left, right| left.reference.cmp(&right.reference));
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

    #[allow(clippy::too_many_arguments)]
    fn connect_native(
        &self,
        _permit: &ExecutionPermit,
        actor: Actor,
        name: &str,
        base_url: Option<&str>,
        auth: &IntegrationAuth,
        credential_reference: Option<&str>,
        credential_references: &BTreeMap<String, String>,
        scopes: &[String],
    ) -> Result<IntegrationConnection, ExecutionError> {
        let existing = self.repository.get_integration(name).map_err(execution)?;
        let now = now().map_err(execution)?;
        let mut connection = compile_native(
            name,
            base_url,
            auth.clone(),
            credential_reference.map(Into::into),
            credential_references.clone(),
            scopes.to_vec(),
            existing
                .as_ref()
                .map_or_else(|| now.clone(), |value| value.connected_at.clone()),
            now,
        )
        .map_err(execution)?;
        let missing_single =
            credential_reference.is_some_and(|reference| resolve_environment(reference).is_err());
        let missing_named = credential_references
            .values()
            .any(|reference| resolve_environment(reference).is_err());
        if missing_single || missing_named {
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
        let prepared = if connection.kind == IntegrationKind::Native {
            prepare_native_request(&connection, tool_name, arguments)?
        } else {
            let mut url = operation_url(&connection, operation, arguments)?;
            add_query(&mut url, operation, arguments)?;
            PreparedHttpRequest {
                method: reqwest::Method::from_bytes(operation.method.as_bytes())
                    .map_err(execution)?,
                url,
                body: operation
                    .accepts_body
                    .then(|| arguments.get("body").cloned())
                    .flatten(),
            }
        };
        require_origin(&prepared.url, permit)?;
        let PreparedHttpRequest { method, url, body } = prepared;
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
        let credential_values = connection
            .credential_references
            .iter()
            .map(|(name, reference)| Ok((name.clone(), resolve_environment(reference)?)))
            .collect::<Result<BTreeMap<_, _>, ExecutionError>>()?;
        let mut sensitive_values = credential_value.iter().cloned().collect::<Vec<_>>();
        sensitive_values.extend(credential_values.values().cloned());
        if let Some((name, value)) = auth_header(
            &connection.auth,
            credential_value.as_deref(),
            &credential_values,
        )? {
            if let Ok(value) = value.to_str() {
                sensitive_values.push(value.into());
                if let Some((_, token)) = value.split_once(' ') {
                    sensitive_values.push(token.into());
                }
            }
            request = request.header(name, value);
        }
        if connection.name == "github" {
            request = request.header("x-github-api-version", "2022-11-28");
        }
        if let Some(body) = body.as_ref() {
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
        for secret in &sensitive_values {
            bytes = redact_exact_secret(&bytes, secret.as_bytes());
        }
        let result = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        let result = normalize_native_response(&connection, tool_name, arguments, result)?;
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
            IntegrationRequest::ConnectNative {
                name,
                base_url,
                auth,
                credential_reference,
                credential_references,
                scopes,
            } => serde_json::to_value(self.connect_native(
                &permit,
                request.actor.clone(),
                &name,
                base_url.as_deref(),
                &auth,
                credential_reference.as_deref(),
                &credential_references,
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
    if matches!(auth, IntegrationAuth::Basic { .. }) {
        return Err(StoreError::Adapter(
            "OpenAPI imports do not accept named basic-auth credentials".into(),
        ));
    }
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
        credential_references: BTreeMap::new(),
        scopes,
        operations,
        manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
        connected_at,
        updated_at,
    };
    validate_connection(&connection)?;
    Ok(connection)
}

/// Compile one first-party native connector into strict dynamic tool contracts.
#[allow(clippy::too_many_arguments)]
pub fn compile_native(
    name: &str,
    base_url: Option<&str>,
    auth: IntegrationAuth,
    credential_reference: Option<String>,
    credential_references: BTreeMap<String, String>,
    scopes: Vec<String>,
    connected_at: String,
    updated_at: String,
) -> Result<IntegrationConnection, StoreError> {
    let (title, description, default_url, operations) = match name {
        "github" => (
            "GitHub",
            "Native GitHub connector for repositories, issues, pull requests, checks, and releases.",
            "https://api.github.com",
            github_operations()?,
        ),
        "searxng" => (
            "SearXNG",
            "Native local/private metasearch connector for normalized SearXNG JSON results.",
            "http://127.0.0.1:8888",
            searxng_operations()?,
        ),
        "opensearch" => (
            "OpenSearch",
            "Native OpenSearch connector for document search, retrieval, indexing, updates, deletes, mappings, and cluster health.",
            "http://127.0.0.1:9200",
            opensearch_operations()?,
        ),
        _ => {
            return Err(StoreError::Adapter(
                "native integration must be github, searxng, or opensearch".into(),
            ));
        }
    };
    validate_native_auth(
        name,
        &auth,
        credential_reference.as_deref(),
        &credential_references,
    )?;
    let base_url = base_url.unwrap_or(default_url).to_owned();
    validate_base_url(&base_url)?;
    for reference in credential_references.values() {
        validate_credential_reference(Some(reference))?;
    }
    let required_refs = usize::from(credential_reference.is_some()) + credential_references.len();
    let status = if auth_requires_credential(&auth) && required_refs == 0 {
        IntegrationStatus::PendingAuth
    } else {
        IntegrationStatus::Connected
    };
    let manifest = serde_json::to_vec(&json!({
        "name": name,
        "base_url": base_url,
        "auth": auth,
        "operations": operations,
    }))
    .map_err(adapter)?;
    let scopes = if scopes.is_empty() && name == "github" {
        vec!["repo".into(), "workflow".into()]
    } else {
        scopes
    };
    let connection = IntegrationConnection {
        name: name.into(),
        kind: IntegrationKind::Native,
        status,
        title: title.into(),
        description: description.into(),
        base_url,
        auth,
        credential_reference,
        credential_references,
        scopes,
        operations,
        manifest_sha256: format!("{:x}", Sha256::digest(manifest)),
        connected_at,
        updated_at,
    };
    validate_connection(&connection)?;
    Ok(connection)
}

fn native_operation(
    name: &str,
    description: &str,
    schema: Value,
    method: &str,
    path: &str,
    max_output_bytes: u64,
) -> Result<IntegrationOperation, StoreError> {
    jsonschema::validator_for(&schema).map_err(adapter)?;
    Ok(IntegrationOperation {
        tool: ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            effect_action: Some(name.into()),
            capability: Some("integration.invoke".into()),
            max_output_bytes,
        },
        operation_id: name
            .split_once('.')
            .map_or(name, |(_, operation)| operation)
            .into(),
        method: method.into(),
        path: path.into(),
        path_parameters: Vec::new(),
        query_parameters: Vec::new(),
        accepts_body: !matches!(method, "GET" | "DELETE"),
    })
}

fn github_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    let bounded = || json!({"type":"integer","minimum":1,"maximum":100});
    Ok(vec![
        native_operation(
            "github.repos",
            "List repositories visible to the connected GitHub token.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "visibility":{"type":"string","enum":["all","public","private"],"default":"all"},
                "max_results":bounded()
            }}),
            "GET",
            "/user/repos",
            64_000,
        )?,
        native_operation(
            "github.issues",
            "List issues for a GitHub repository.",
            github_repo_schema(
                json!({
                    "state":{"type":"string","enum":["open","closed","all"],"default":"open"},
                    "max_results":bounded()
                }),
                &[],
            ),
            "GET",
            "/repos/{owner}/{repo}/issues",
            64_000,
        )?,
        native_operation(
            "github.pull_requests",
            "List pull requests for a GitHub repository.",
            github_repo_schema(
                json!({
                    "state":{"type":"string","enum":["open","closed","all"],"default":"open"},
                    "max_results":bounded()
                }),
                &[],
            ),
            "GET",
            "/repos/{owner}/{repo}/pulls",
            64_000,
        )?,
        native_operation(
            "github.checks",
            "List check runs for a GitHub commit ref.",
            github_repo_schema(
                json!({
                    "ref":{"type":"string","minLength":1,"maxLength":512},
                    "max_results":bounded()
                }),
                &["ref"],
            ),
            "GET",
            "/repos/{owner}/{repo}/commits/{ref}/check-runs",
            64_000,
        )?,
        native_operation(
            "github.releases",
            "List releases for a GitHub repository.",
            github_repo_schema(json!({"max_results":bounded()}), &[]),
            "GET",
            "/repos/{owner}/{repo}/releases",
            64_000,
        )?,
    ])
}

fn github_repo_schema(extra: Value, extra_required: &[&str]) -> Value {
    let mut properties = Map::from_iter([
        (
            "owner".into(),
            json!({"type":"string","minLength":1,"maxLength":256}),
        ),
        (
            "repo".into(),
            json!({"type":"string","minLength":1,"maxLength":256}),
        ),
    ]);
    if let Some(extra) = extra.as_object() {
        properties.extend(extra.clone());
    }
    let required = ["owner", "repo"]
        .into_iter()
        .chain(extra_required.iter().copied())
        .collect::<Vec<_>>();
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":properties,
        "required":required
    })
}

fn searxng_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    Ok(vec![
        native_operation(
            "searxng.search",
            "Search a configured SearXNG instance and return normalized results.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "query":{"type":"string","minLength":1,"maxLength":4096},
                "max_results":{"type":"integer","minimum":1,"maximum":20,"default":10}
            },"required":["query"]}),
            "GET",
            "/search",
            128_000,
        )?,
        native_operation(
            "searxng.health",
            "Check that a configured SearXNG instance returns JSON results.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            "GET",
            "/search",
            16_000,
        )?,
    ])
}

fn opensearch_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    let empty = || json!({"type":"object","additionalProperties":false,"properties":{}});
    let index = || json!({"type":"string","minLength":1,"maxLength":1024});
    let id = || json!({"type":"string","minLength":1,"maxLength":1024});
    let refresh = || json!({"type":"string","enum":["false","true","wait_for"]});
    Ok(vec![
        native_operation(
            "opensearch.info",
            "Fetch basic OpenSearch endpoint information.",
            empty(),
            "GET",
            "/",
            16_000,
        )?,
        native_operation(
            "opensearch.health",
            "Fetch OpenSearch cluster health.",
            empty(),
            "GET",
            "/_cluster/health",
            16_000,
        )?,
        native_operation(
            "opensearch.list_indices",
            "List OpenSearch indices through the JSON cat API.",
            empty(),
            "GET",
            "/_cat/indices",
            64_000,
        )?,
        native_operation(
            "opensearch.get_mapping",
            "Fetch an OpenSearch index mapping.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index()},"required":["index"]}),
            "GET",
            "/{index}/_mapping",
            64_000,
        )?,
        native_operation(
            "opensearch.search",
            "Run a bounded OpenSearch query.",
            json!({"type":"object","additionalProperties":false,"properties":{
            "index":index(),"query":{"type":"object"},
            "size":{"type":"integer","minimum":1,"maximum":100,"default":10},
            "from":{"type":"integer","minimum":0,"maximum":10000,"default":0},
            "source_includes":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":1024}},
            "sort":{"type":"array","maxItems":64,"items":{"type":"object"}}
        },"required":["index","query"]}),
            "POST",
            "/{index}/_search",
            128_000,
        )?,
        native_operation(
            "opensearch.get_document",
            "Fetch one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id()},"required":["index","id"]}),
            "GET",
            "/{index}/_doc/{id}",
            64_000,
        )?,
        native_operation(
            "opensearch.index_document",
            "Create or replace one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"document":{"type":"object"},"refresh":refresh()},"required":["index","document"]}),
            "POST",
            "/{index}/_doc",
            32_000,
        )?,
        native_operation(
            "opensearch.update_document",
            "Partially update one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"doc":{"type":"object"},"doc_as_upsert":{"type":"boolean"},"refresh":refresh()},"required":["index","id","doc"]}),
            "POST",
            "/{index}/_update/{id}",
            32_000,
        )?,
        native_operation(
            "opensearch.delete_document",
            "Delete one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"refresh":refresh()},"required":["index","id"]}),
            "DELETE",
            "/{index}/_doc/{id}",
            32_000,
        )?,
    ])
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
    for reference in connection.credential_references.values() {
        validate_credential_reference(Some(reference))?;
    }
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
    if connection.status == IntegrationStatus::Connected
        && !credentials_satisfy_auth(
            &connection.auth,
            connection.credential_reference.as_deref(),
            &connection.credential_references,
        )
    {
        return Err(StoreError::Adapter(
            "connected authenticated integration requires a credential reference".into(),
        ));
    }
    if !auth_requires_credential(&connection.auth)
        && (connection.credential_reference.is_some()
            || !connection.credential_references.is_empty())
    {
        return Err(StoreError::Adapter(
            "auth-none integrations cannot retain a credential reference".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for operation in &connection.operations {
        let valid_prefix = match connection.kind {
            IntegrationKind::OpenApi => format!("openapi.{}.", connection.name),
            IntegrationKind::Native => format!("{}.", connection.name),
            IntegrationKind::Mcp => format!("mcp.{}.", connection.name),
        };
        if !names.insert(operation.tool.name.as_str())
            || operation.tool.effect_action.as_deref() != Some(&operation.tool.name)
            || operation.tool.capability.as_deref() != Some("integration.invoke")
            || !operation.tool.name.starts_with(&valid_prefix)
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
        IntegrationAuth::Basic { header } if valid_header(header) => Ok(()),
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

fn credentials_satisfy_auth(
    auth: &IntegrationAuth,
    credential_reference: Option<&str>,
    credential_references: &BTreeMap<String, String>,
) -> bool {
    match auth {
        IntegrationAuth::None => credential_reference.is_none() && credential_references.is_empty(),
        IntegrationAuth::Basic { .. } => {
            credential_reference.is_none()
                && credential_references.len() == 2
                && credential_references.contains_key("username")
                && credential_references.contains_key("password")
        }
        _ => credential_reference.is_some() && credential_references.is_empty(),
    }
}

fn validate_native_auth(
    name: &str,
    auth: &IntegrationAuth,
    credential_reference: Option<&str>,
    credential_references: &BTreeMap<String, String>,
) -> Result<(), StoreError> {
    validate_auth(auth)?;
    validate_credential_reference(credential_reference)?;
    let supported = match name {
        "github" => matches!(auth, IntegrationAuth::Bearer { .. }),
        "searxng" => matches!(
            auth,
            IntegrationAuth::None | IntegrationAuth::Bearer { .. } | IntegrationAuth::ApiKey { .. }
        ),
        "opensearch" => matches!(
            auth,
            IntegrationAuth::None | IntegrationAuth::Bearer { .. } | IntegrationAuth::Basic { .. }
        ),
        _ => false,
    };
    if !supported {
        return Err(StoreError::Adapter(
            "native integration auth type is not supported".into(),
        ));
    }
    let partial_basic = matches!(auth, IntegrationAuth::Basic { .. })
        && !credential_references.is_empty()
        && !credentials_satisfy_auth(auth, credential_reference, credential_references);
    let misplaced = match auth {
        IntegrationAuth::None => {
            credential_reference.is_some() || !credential_references.is_empty()
        }
        IntegrationAuth::Basic { .. } => credential_reference.is_some(),
        _ => !credential_references.is_empty(),
    };
    if partial_basic || misplaced {
        return Err(StoreError::Adapter(
            "native integration credential references do not match its auth type".into(),
        ));
    }
    Ok(())
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
    let mut expected = match operation {
        IntegrationRequest::ImportOpenApi {
            credential_reference,
            ..
        } => credential_reference.iter().cloned().collect::<Vec<_>>(),
        IntegrationRequest::ConnectNative {
            credential_reference,
            credential_references,
            ..
        } => credential_reference
            .iter()
            .cloned()
            .chain(credential_references.values().cloned())
            .collect(),
        IntegrationRequest::Disconnect { .. } => Vec::new(),
        IntegrationRequest::Invoke { connection, .. } => repository
            .get_integration(connection)
            .map_err(execution)?
            .map(|connection| {
                connection
                    .credential_reference
                    .into_iter()
                    .chain(connection.credential_references.into_values())
                    .collect()
            })
            .unwrap_or_default(),
    };
    expected.sort();
    let mut actual = disclosed
        .iter()
        .map(|reference| reference.reference.clone())
        .collect::<Vec<_>>();
    actual.sort();
    let matches = expected == actual && disclosed.iter().all(|value| value.value_hash.is_none());
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
    secret: Option<&str>,
    named: &BTreeMap<String, String>,
) -> Result<Option<(HeaderName, HeaderValue)>, ExecutionError> {
    let (header, value) = match auth {
        IntegrationAuth::None => return Ok(None),
        IntegrationAuth::Bearer { header, scheme } => (
            header,
            format!(
                "{scheme} {}",
                secret.ok_or_else(|| execution("bearer credential is unavailable"))?
            ),
        ),
        IntegrationAuth::ApiKey { header, scheme } => (
            header,
            scheme.as_ref().map_or_else(
                || {
                    secret
                        .ok_or_else(|| execution("API-key credential is unavailable"))
                        .map(Into::into)
                },
                |scheme| {
                    secret
                        .ok_or_else(|| execution("API-key credential is unavailable"))
                        .map(|secret| format!("{scheme} {secret}"))
                },
            )?,
        ),
        IntegrationAuth::Basic { header } => {
            let username = named
                .get("username")
                .ok_or_else(|| execution("basic-auth username is unavailable"))?;
            let password = named
                .get("password")
                .ok_or_else(|| execution("basic-auth password is unavailable"))?;
            (
                header,
                format!("Basic {}", BASE64.encode(format!("{username}:{password}"))),
            )
        }
        IntegrationAuth::ServiceAccount { header } => (
            header,
            secret
                .ok_or_else(|| execution("service-account credential is unavailable"))?
                .into(),
        ),
    };
    Ok(Some((
        HeaderName::from_bytes(header.as_bytes()).map_err(execution)?,
        HeaderValue::from_str(&value).map_err(execution)?,
    )))
}

struct PreparedHttpRequest {
    method: reqwest::Method,
    url: Url,
    body: Option<Value>,
}

fn prepare_native_request(
    connection: &IntegrationConnection,
    tool_name: &str,
    arguments: &Value,
) -> Result<PreparedHttpRequest, ExecutionError> {
    let arguments = arguments
        .as_object()
        .ok_or_else(|| execution("native integration arguments must be an object"))?;
    match connection.name.as_str() {
        "github" => github_request(connection, tool_name, arguments),
        "searxng" => searxng_request(connection, tool_name, arguments),
        "opensearch" => opensearch_request(connection, tool_name, arguments),
        _ => Err(execution("unsupported native integration")),
    }
}

fn github_request(
    connection: &IntegrationConnection,
    tool_name: &str,
    arguments: &Map<String, Value>,
) -> Result<PreparedHttpRequest, ExecutionError> {
    let max_results = bounded_integer(arguments, "max_results", 30, 1, 100)?;
    let (path, query) = match tool_name {
        "github.repos" => (
            "/user/repos".into(),
            vec![
                (
                    "visibility",
                    optional_string(arguments, "visibility")
                        .unwrap_or("all")
                        .into(),
                ),
                ("per_page", max_results.to_string()),
            ],
        ),
        "github.issues" => (
            format!(
                "/repos/{}/{}/issues",
                native_segment(arguments, "owner")?,
                native_segment(arguments, "repo")?
            ),
            vec![
                (
                    "state",
                    optional_string(arguments, "state").unwrap_or("open").into(),
                ),
                ("per_page", max_results.to_string()),
            ],
        ),
        "github.pull_requests" => (
            format!(
                "/repos/{}/{}/pulls",
                native_segment(arguments, "owner")?,
                native_segment(arguments, "repo")?
            ),
            vec![
                (
                    "state",
                    optional_string(arguments, "state").unwrap_or("open").into(),
                ),
                ("per_page", max_results.to_string()),
            ],
        ),
        "github.checks" => (
            format!(
                "/repos/{}/{}/commits/{}/check-runs",
                native_segment(arguments, "owner")?,
                native_segment(arguments, "repo")?,
                native_segment(arguments, "ref")?
            ),
            vec![("per_page", max_results.to_string())],
        ),
        "github.releases" => (
            format!(
                "/repos/{}/{}/releases",
                native_segment(arguments, "owner")?,
                native_segment(arguments, "repo")?
            ),
            vec![("per_page", max_results.to_string())],
        ),
        _ => return Err(execution("unsupported GitHub integration tool")),
    };
    let mut url = native_url(connection, &path)?;
    append_pairs(&mut url, query);
    Ok(PreparedHttpRequest {
        method: reqwest::Method::GET,
        url,
        body: None,
    })
}

fn searxng_request(
    connection: &IntegrationConnection,
    tool_name: &str,
    arguments: &Map<String, Value>,
) -> Result<PreparedHttpRequest, ExecutionError> {
    let mut url = native_url(connection, "/search")?;
    let query = match tool_name {
        "searxng.search" => required_string(arguments, "query")?,
        "searxng.health" => "colossus",
        _ => return Err(execution("unsupported SearXNG integration tool")),
    };
    append_pairs(
        &mut url,
        vec![("q", query.into()), ("format", "json".into())],
    );
    Ok(PreparedHttpRequest {
        method: reqwest::Method::GET,
        url,
        body: None,
    })
}

fn opensearch_request(
    connection: &IntegrationConnection,
    tool_name: &str,
    arguments: &Map<String, Value>,
) -> Result<PreparedHttpRequest, ExecutionError> {
    let mut query = Vec::<(&str, String)>::new();
    let (method, path, body) = match tool_name {
        "opensearch.info" => (reqwest::Method::GET, "/".into(), None),
        "opensearch.health" => (reqwest::Method::GET, "/_cluster/health".into(), None),
        "opensearch.list_indices" => {
            query.push(("format", "json".into()));
            (reqwest::Method::GET, "/_cat/indices".into(), None)
        }
        "opensearch.get_mapping" => (
            reqwest::Method::GET,
            format!("/{}/_mapping", opensearch_index(arguments)?),
            None,
        ),
        "opensearch.search" => {
            let mut body = Map::from_iter([
                (
                    "query".into(),
                    arguments
                        .get("query")
                        .cloned()
                        .ok_or_else(|| execution("OpenSearch query is required"))?,
                ),
                (
                    "size".into(),
                    json!(bounded_integer(arguments, "size", 10, 1, 100)?),
                ),
                (
                    "from".into(),
                    json!(bounded_integer(arguments, "from", 0, 0, 10_000)?),
                ),
            ]);
            for name in ["source_includes", "sort"] {
                if let Some(value) = arguments.get(name) {
                    body.insert(
                        if name == "source_includes" {
                            "_source"
                        } else {
                            name
                        }
                        .into(),
                        value.clone(),
                    );
                }
            }
            (
                reqwest::Method::POST,
                format!("/{}/_search", opensearch_index(arguments)?),
                Some(Value::Object(body)),
            )
        }
        "opensearch.get_document" => (
            reqwest::Method::GET,
            format!(
                "/{}/_doc/{}",
                opensearch_index(arguments)?,
                native_segment(arguments, "id")?
            ),
            None,
        ),
        "opensearch.index_document" => {
            add_refresh(arguments, &mut query)?;
            let document = arguments
                .get("document")
                .cloned()
                .ok_or_else(|| execution("OpenSearch document is required"))?;
            if let Some(id) = optional_string(arguments, "id").filter(|value| !value.is_empty()) {
                (
                    reqwest::Method::PUT,
                    format!(
                        "/{}/_doc/{}",
                        opensearch_index(arguments)?,
                        encode_path_segment(id)
                    ),
                    Some(document),
                )
            } else {
                (
                    reqwest::Method::POST,
                    format!("/{}/_doc", opensearch_index(arguments)?),
                    Some(document),
                )
            }
        }
        "opensearch.update_document" => {
            add_refresh(arguments, &mut query)?;
            let mut body = Map::from_iter([(
                "doc".into(),
                arguments
                    .get("doc")
                    .cloned()
                    .ok_or_else(|| execution("OpenSearch update doc is required"))?,
            )]);
            if let Some(value) = arguments.get("doc_as_upsert") {
                body.insert("doc_as_upsert".into(), value.clone());
            }
            (
                reqwest::Method::POST,
                format!(
                    "/{}/_update/{}",
                    opensearch_index(arguments)?,
                    native_segment(arguments, "id")?
                ),
                Some(Value::Object(body)),
            )
        }
        "opensearch.delete_document" => {
            add_refresh(arguments, &mut query)?;
            (
                reqwest::Method::DELETE,
                format!(
                    "/{}/_doc/{}",
                    opensearch_index(arguments)?,
                    native_segment(arguments, "id")?
                ),
                None,
            )
        }
        _ => return Err(execution("unsupported OpenSearch integration tool")),
    };
    let mut url = native_url(connection, &path)?;
    append_pairs(&mut url, query);
    Ok(PreparedHttpRequest { method, url, body })
}

fn normalize_native_response(
    connection: &IntegrationConnection,
    tool_name: &str,
    arguments: &Value,
    result: Value,
) -> Result<Value, ExecutionError> {
    if connection.name != "searxng" {
        return Ok(result);
    }
    let results = result
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| execution("SearXNG response must contain a results array"))?;
    if tool_name == "searxng.health" {
        return Ok(json!({"status":"ok","result_count":results.len().min(1)}));
    }
    let max_results = arguments
        .as_object()
        .map(|values| bounded_integer(values, "max_results", 10, 1, 20))
        .transpose()?
        .unwrap_or(10) as usize;
    let normalized = results
        .iter()
        .take(max_results)
        .filter_map(Value::as_object)
        .map(|source| {
            let mut metadata = source.clone();
            for key in ["title", "url", "content"] {
                metadata.remove(key);
            }
            json!({
                "title": source.get("title").and_then(Value::as_str).unwrap_or_default(),
                "url": source.get("url").and_then(Value::as_str).unwrap_or_default(),
                "content": source.get("content").and_then(Value::as_str).unwrap_or_default(),
                "metadata": metadata,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "query": arguments.get("query").and_then(Value::as_str).unwrap_or_default(),
        "count": normalized.len(),
        "results": normalized,
    }))
}

fn native_url(connection: &IntegrationConnection, path: &str) -> Result<Url, ExecutionError> {
    let parsed = Url::parse(&connection.base_url).map_err(execution)?;
    if connection.name == "searxng"
        && path == "/search"
        && parsed.path().trim_end_matches('/') == "/search"
    {
        return Ok(parsed);
    }
    let mut base = connection.base_url.clone();
    if !base.ends_with('/') {
        base.push('/');
    }
    Url::parse(&base)
        .map_err(execution)?
        .join(path.trim_start_matches('/'))
        .map_err(execution)
}

fn append_pairs(url: &mut Url, pairs: Vec<(&str, String)>) {
    let mut query = url.query_pairs_mut();
    for (name, value) in pairs {
        query.append_pair(name, &value);
    }
}

fn required_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, ExecutionError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| execution(format!("integration argument {name} is required")))
}

fn optional_string<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn native_segment(arguments: &Map<String, Value>, name: &str) -> Result<String, ExecutionError> {
    Ok(encode_path_segment(required_string(arguments, name)?))
}

fn opensearch_index(arguments: &Map<String, Value>) -> Result<String, ExecutionError> {
    let value = required_string(arguments, "index")?;
    if value.contains(['/', '\\']) || matches!(value, "." | "..") {
        return Err(execution(
            "OpenSearch index contains an unsafe path segment",
        ));
    }
    Ok(encode_path_segment_with(value, b",*"))
}

fn encode_path_segment_with(value: &str, additionally_safe: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || additionally_safe.contains(&byte)
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn bounded_integer(
    arguments: &Map<String, Value>,
    name: &str,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> Result<i64, ExecutionError> {
    let value = arguments
        .get(name)
        .and_then(Value::as_i64)
        .unwrap_or(default);
    if (minimum..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(execution(format!(
            "integration argument {name} is outside its bound"
        )))
    }
}

fn add_refresh(
    arguments: &Map<String, Value>,
    query: &mut Vec<(&'static str, String)>,
) -> Result<(), ExecutionError> {
    if let Some(refresh) = optional_string(arguments, "refresh") {
        if !matches!(refresh, "false" | "true" | "wait_for") {
            return Err(execution("invalid OpenSearch refresh value"));
        }
        query.push(("refresh", refresh.into()));
    }
    Ok(())
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
        EventSourcedExtensionRepository, IntegrationExecutor, IntegrationRequest, compile_native,
        compile_openapi, normalize_native_response, prepare_native_request, redact_exact_secret,
    };
    use colossus_contracts::{DecisionOutcome, IntegrationAuth, IntegrationStatus};
    use colossus_policy::{EffectExecutor, system_actor};
    use colossus_ports::{EventJournal, ExtensionRepository};
    use colossus_testkit::{InMemoryEventJournal, assert_extension_repository_conformance};
    use serde_json::json;
    use std::{collections::BTreeMap, sync::Arc};

    #[test]
    fn event_sourced_extension_repository_passes_shared_conformance() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        assert_extension_repository_conformance(|| {
            Box::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)))
        });
    }

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

    #[test]
    fn native_manifests_cover_github_searxng_and_opensearch_auth_contracts() {
        let github = compile_native(
            "github",
            None,
            IntegrationAuth::Bearer {
                header: "Authorization".into(),
                scheme: "Bearer".into(),
            },
            None,
            BTreeMap::new(),
            Vec::new(),
            "created".into(),
            "updated".into(),
        )
        .expect("GitHub");
        assert_eq!(github.status, IntegrationStatus::PendingAuth);
        assert_eq!(github.operations.len(), 5);
        assert_eq!(github.scopes, ["repo", "workflow"]);

        let searxng = compile_native(
            "searxng",
            Some("https://search.example.test/search"),
            IntegrationAuth::None,
            None,
            BTreeMap::new(),
            Vec::new(),
            "created".into(),
            "updated".into(),
        )
        .expect("SearXNG");
        let prepared = prepare_native_request(
            &searxng,
            "searxng.search",
            &json!({"query":"rust agents","max_results":2}),
        )
        .expect("request");
        assert_eq!(prepared.url.path(), "/search");
        assert_eq!(prepared.url.query(), Some("q=rust+agents&format=json"));
        let normalized = normalize_native_response(
            &searxng,
            "searxng.search",
            &json!({"query":"rust agents","max_results":1}),
            json!({"results":[
                {"title":"One","url":"https://one.test","content":"First","engine":"demo"},
                {"title":"Two","url":"https://two.test","content":"Second"}
            ]}),
        )
        .expect("normalize");
        assert_eq!(normalized["count"], 1);
        assert_eq!(normalized["results"][0]["metadata"]["engine"], "demo");

        let basic = BTreeMap::from([
            ("username".into(), "env:OPENSEARCH_USER".into()),
            ("password".into(), "env:OPENSEARCH_PASSWORD".into()),
        ]);
        let opensearch = compile_native(
            "opensearch",
            Some("https://search.example.test"),
            IntegrationAuth::Basic {
                header: "Authorization".into(),
            },
            None,
            basic,
            Vec::new(),
            "created".into(),
            "updated".into(),
        )
        .expect("OpenSearch");
        assert_eq!(opensearch.status, IntegrationStatus::Connected);
        assert_eq!(opensearch.operations.len(), 9);
        let prepared = prepare_native_request(
            &opensearch,
            "opensearch.update_document",
            &json!({
                "index":"notes-*","id":"a b","doc":{"status":"done"},
                "doc_as_upsert":true,"refresh":"wait_for"
            }),
        )
        .expect("update request");
        assert_eq!(prepared.method, reqwest::Method::POST);
        assert_eq!(prepared.url.path(), "/notes-*/_update/a%20b");
        assert_eq!(prepared.url.query(), Some("refresh=wait_for"));
        assert_eq!(prepared.body.expect("body")["doc_as_upsert"], true);
    }
}
