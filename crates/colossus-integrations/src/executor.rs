use super::*;

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
            .header("user-agent", "colossus/0.6");
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

pub(super) fn redact_exact_secret(bytes: &[u8], secret: &[u8]) -> Vec<u8> {
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
