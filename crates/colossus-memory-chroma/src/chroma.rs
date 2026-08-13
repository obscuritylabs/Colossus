use super::*;

/// Strict Chroma v2 collection profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChromaProfile {
    base_url: String,
    tenant: String,
    database: String,
    collection: String,
    credential_reference: Option<String>,
    timeout_ms: u64,
}

impl ChromaProfile {
    /// Validate an exact Chroma origin, namespace, and credential reference.
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        database: impl Into<String>,
        collection: impl Into<String>,
        credential_reference: Option<String>,
        timeout_ms: u64,
    ) -> Result<Self, StoreError> {
        Self::new_with_resource_authority(
            base_url,
            tenant,
            database,
            collection,
            credential_reference,
            timeout_ms,
            ResourceAuthority::Declared,
        )
    }

    /// Validate a Chroma profile under an explicit runtime resource authority.
    pub fn new_with_resource_authority(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        database: impl Into<String>,
        collection: impl Into<String>,
        credential_reference: Option<String>,
        timeout_ms: u64,
        resource_authority: ResourceAuthority,
    ) -> Result<Self, StoreError> {
        let base_url = normalize_base_url(&base_url.into(), false, resource_authority)?;
        let tenant = tenant.into();
        let database = database.into();
        let collection = collection.into();
        validate_credential_reference(credential_reference.as_deref())?;
        if !valid_name(&tenant)
            || !valid_name(&database)
            || !valid_name(&collection)
            || timeout_ms == 0
        {
            return Err(adapter(
                "Chroma tenant/database/collection/timeout is invalid",
            ));
        }
        Ok(Self {
            base_url,
            tenant,
            database,
            collection,
            credential_reference,
            timeout_ms,
        })
    }

    /// Canonical Chroma origin for policy obligations.
    pub fn network_origin(&self) -> Result<String, StoreError> {
        origin(&self.base_url)
    }

    /// Stable logical collection resource used by effect requests.
    pub fn resource(&self) -> String {
        format!(
            "{}/api/v2/tenants/{}/databases/{}/collections/{}",
            self.base_url, self.tenant, self.database, self.collection
        )
    }

    /// Configured credential reference, without resolving it.
    pub fn credential_reference(&self) -> Option<&str> {
        self.credential_reference.as_deref()
    }

    fn collections_url(&self) -> Result<String, StoreError> {
        path_url(
            &self.base_url,
            &[
                "api",
                "v2",
                "tenants",
                &self.tenant,
                "databases",
                &self.database,
                "collections",
            ],
        )
    }

    fn collection_url(&self, id: &str, suffix: Option<&str>) -> Result<String, StoreError> {
        let mut segments = vec![
            "api",
            "v2",
            "tenants",
            &self.tenant,
            "databases",
            &self.database,
            "collections",
            id,
        ];
        if let Some(suffix) = suffix {
            segments.push(suffix);
        }
        path_url(&self.base_url, &segments)
    }
}

fn path_url(base: &str, segments: &[&str]) -> Result<String, StoreError> {
    let mut url = Url::parse(base).map_err(adapter)?;
    url.path_segments_mut()
        .map_err(|_| adapter("semantic base URL cannot accept path segments"))?
        .clear()
        .extend(segments.iter().copied());
    Ok(url.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ChromaOperation {
    Upsert {
        event_id: String,
        memory_id: String,
        text: String,
        metadata: Value,
        embedding: Vec<f32>,
    },
    Remove {
        event_id: String,
        memory_id: String,
    },
    Search {
        embedding: Vec<f32>,
        limit: usize,
    },
    Status,
    Reset,
}

impl ChromaOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Upsert { .. } => "memory.index.chroma.upsert",
            Self::Remove { .. } => "memory.index.chroma.remove",
            Self::Search { .. } => "memory.index.chroma.search",
            Self::Status => "memory.index.chroma.status",
            Self::Reset => "memory.index.chroma.reset",
        }
    }

    fn mutates(&self) -> bool {
        matches!(
            self,
            Self::Upsert { .. } | Self::Remove { .. } | Self::Reset
        )
    }

    fn validate(&self) -> Result<(), StoreError> {
        match self {
            Self::Upsert {
                event_id,
                memory_id,
                text,
                metadata,
                embedding,
            } => validate_projection_record(event_id, memory_id, text, metadata, embedding),
            Self::Remove {
                event_id,
                memory_id,
            } if event_id.is_empty() || memory_id.is_empty() => {
                Err(adapter("Chroma removal identity is invalid"))
            }
            Self::Search { embedding, limit } => {
                validate_vector(embedding, None)?;
                if !(1..=400).contains(limit) {
                    return Err(adapter("Chroma search limit must be in 1..=400"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CollectionResponse {
    id: String,
    name: String,
    tenant: String,
    database: String,
    #[serde(default)]
    dimension: Option<usize>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    ids: Vec<Vec<String>>,
    distances: Option<Vec<Vec<Option<f32>>>>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

/// Permit-requiring Chroma v2 HTTP transport.
pub struct ChromaExecutor {
    profile: ChromaProfile,
    tls_roots: AdditionalRootCertificates,
}

impl ChromaExecutor {
    /// Construct a transport. Execution remains impossible without a gateway permit.
    pub fn new(profile: ChromaProfile) -> Self {
        Self {
            profile,
            tls_roots: AdditionalRootCertificates::default(),
        }
    }

    /// Add validated runtime-wide CA roots to Chroma clients' built-in public roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }

    async fn ensure_collection(&self, permit: &ExecutionPermit) -> Result<String, StoreError> {
        let payload = json!({
            "name": self.profile.collection,
            "get_or_create": true,
            "metadata": {"colossus_projection": true},
        });
        let bytes = send_http(
            Method::POST,
            &self.profile.collections_url()?,
            Some(&payload),
            permit,
            HttpTransport::new(
                self.profile.credential_reference(),
                "x-chroma-token",
                self.profile.timeout_ms,
                &self.tls_roots,
            ),
        )
        .await?;
        self.parse_collection(&bytes)
    }

    async fn get_collection(&self, permit: &ExecutionPermit) -> Result<String, StoreError> {
        let bytes = send_http(
            Method::GET,
            &self
                .profile
                .collection_url(&self.profile.collection, None)?,
            None,
            permit,
            HttpTransport::new(
                self.profile.credential_reference(),
                "x-chroma-token",
                self.profile.timeout_ms,
                &self.tls_roots,
            ),
        )
        .await?;
        self.parse_collection(&bytes)
    }

    fn parse_collection(&self, bytes: &[u8]) -> Result<String, StoreError> {
        let collection: CollectionResponse = serde_json::from_slice(bytes).map_err(adapter)?;
        if !valid_name(&collection.id)
            || collection.name != self.profile.collection
            || collection.tenant != self.profile.tenant
            || collection.database != self.profile.database
            || collection
                .dimension
                .is_some_and(|value| value > MAX_VECTOR_DIMENSIONS)
        {
            return Err(adapter(
                "Chroma collection response does not match its profile",
            ));
        }
        let _ = collection.extra;
        Ok(collection.id)
    }

    async fn execute_operation(
        &self,
        operation: ChromaOperation,
        permit: &ExecutionPermit,
    ) -> Result<Value, StoreError> {
        validate_destination(permit, &self.profile.network_origin()?).map_err(adapter)?;
        match operation {
            ChromaOperation::Upsert {
                event_id,
                memory_id,
                text,
                mut metadata,
                embedding,
            } => {
                let object = metadata
                    .as_object_mut()
                    .ok_or_else(|| adapter("Chroma metadata must be an object"))?;
                object.insert("colossus_event_id".into(), Value::String(event_id));
                object.insert("colossus_projection".into(), Value::Bool(true));
                let id = self.ensure_collection(permit).await?;
                let payload = json!({
                    "ids": [memory_id],
                    "embeddings": [embedding],
                    "documents": [text],
                    "metadatas": [metadata],
                });
                send_http(
                    Method::POST,
                    &self.profile.collection_url(&id, Some("upsert"))?,
                    Some(&payload),
                    permit,
                    HttpTransport::new(
                        self.profile.credential_reference(),
                        "x-chroma-token",
                        self.profile.timeout_ms,
                        &self.tls_roots,
                    ),
                )
                .await?;
                Ok(json!({"ok": true}))
            }
            ChromaOperation::Remove {
                event_id,
                memory_id,
            } => {
                let id = self.get_collection(permit).await?;
                let payload = json!({"ids": [memory_id]});
                send_http(
                    Method::POST,
                    &self.profile.collection_url(&id, Some("delete"))?,
                    Some(&payload),
                    permit,
                    HttpTransport::new(
                        self.profile.credential_reference(),
                        "x-chroma-token",
                        self.profile.timeout_ms,
                        &self.tls_roots,
                    ),
                )
                .await?;
                Ok(json!({"ok": true, "event_id": event_id}))
            }
            ChromaOperation::Search { embedding, limit } => {
                let id = self.get_collection(permit).await?;
                let payload = json!({
                    "query_embeddings": [embedding],
                    "n_results": limit,
                    "include": ["distances"],
                });
                let bytes = send_http(
                    Method::POST,
                    &self.profile.collection_url(&id, Some("query"))?,
                    Some(&payload),
                    permit,
                    HttpTransport::new(
                        self.profile.credential_reference(),
                        "x-chroma-token",
                        self.profile.timeout_ms,
                        &self.tls_roots,
                    ),
                )
                .await?;
                let response: QueryResponse = serde_json::from_slice(&bytes).map_err(adapter)?;
                let ids = response.ids.into_iter().next().unwrap_or_default();
                let distances = response
                    .distances
                    .and_then(|rows| rows.into_iter().next())
                    .unwrap_or_default();
                let _ = response.extra;
                if ids.len() != distances.len() || ids.len() > limit {
                    return Err(adapter("Chroma query response arrays are inconsistent"));
                }
                let mut candidates = Vec::with_capacity(ids.len());
                for (id, distance) in ids.into_iter().zip(distances) {
                    let distance = distance
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(|| adapter("Chroma returned an invalid distance"))?;
                    candidates.push((id, 1.0_f32 / (1.0 + distance)));
                }
                Ok(json!({"candidates": candidates}))
            }
            ChromaOperation::Status => {
                let id = self.get_collection(permit).await?;
                let bytes = send_http(
                    Method::GET,
                    &self.profile.collection_url(&id, Some("count"))?,
                    None,
                    permit,
                    HttpTransport::new(
                        self.profile.credential_reference(),
                        "x-chroma-token",
                        self.profile.timeout_ms,
                        &self.tls_roots,
                    ),
                )
                .await?;
                let documents: usize = serde_json::from_slice(&bytes).map_err(adapter)?;
                Ok(json!({"ready": true, "kind": "chroma", "documents": documents}))
            }
            ChromaOperation::Reset => {
                send_http(
                    Method::DELETE,
                    &self
                        .profile
                        .collection_url(&self.profile.collection, None)?,
                    None,
                    permit,
                    HttpTransport::new(
                        self.profile.credential_reference(),
                        "x-chroma-token",
                        self.profile.timeout_ms,
                        &self.tls_roots,
                    ),
                )
                .await?;
                Ok(json!({"ok": true}))
            }
        }
    }
}

#[async_trait]
impl EffectExecutor for ChromaExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: ChromaOperation =
            serde_json::from_value(request.content.clone()).map_err(execution)?;
        if request.action != operation.action() || request.resource != self.profile.resource() {
            return Err(execution(
                "Chroma action/resource does not match its operation",
            ));
        }
        validate_credential_disclosure(request, self.profile.credential_reference())
            .map_err(execution)?;
        operation.validate().map_err(execution)?;
        validate_destination(&permit, &self.profile.network_origin().map_err(execution)?)?;
        let mutates = operation.mutates();
        let value = match self.execute_operation(operation, &permit).await {
            Ok(value) => value,
            Err(error) if mutates => {
                return Err(ExecutionError::OutcomeUnknown(format!(
                    "Chroma mutation may have reached the configured endpoint: {error}"
                )));
            }
            Err(error) => return Err(execution(error)),
        };
        let bytes = serde_json::to_vec(&value).map_err(execution)?;
        bounded_result(bytes, &permit)
    }
}
