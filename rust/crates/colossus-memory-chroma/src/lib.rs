//! Permit-bound semantic-memory adapters for Chroma and embedding profiles.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, CredentialReference, EffectRequest, QuarantinedEffectResult,
};
use colossus_policy::{
    EffectExecutor, EffectGateway, ExecutionError, ExecutionPermit, GatewayError, effect_request,
};
use colossus_ports::{EmbeddingProvider, MemoryIndex, StoreError};
use futures::StreamExt as _;
use reqwest::{Client, Method, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::lookup_host;

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_VECTOR_DIMENSIONS: usize = 4_096;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 16;
const MAX_REBUILD_RECORDS: usize = 1_000;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_credential_reference(reference: Option<&str>) -> Result<(), StoreError> {
    let Some(reference) = reference else {
        return Ok(());
    };
    let Some(variable) = reference.strip_prefix("env:") else {
        return Err(adapter("credential references must use env:VARIABLE"));
    };
    let mut bytes = variable.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    {
        return Err(adapter("credential references must use env:VARIABLE"));
    }
    Ok(())
}

fn resolve_credential(reference: &str) -> Result<String, StoreError> {
    validate_credential_reference(Some(reference))?;
    let variable = reference
        .strip_prefix("env:")
        .ok_or_else(|| adapter("credential reference is invalid"))?;
    std::env::var(variable)
        .map_err(|_| adapter(format!("environment credential {variable} is unset")))
}

fn normalize_base_url(raw: &str, allow_path: bool) -> Result<String, StoreError> {
    let mut url = Url::parse(raw).map_err(adapter)?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (!allow_path && url.path() != "/" && !url.path().is_empty())
    {
        return Err(adapter(
            "semantic endpoints require HTTPS or loopback HTTP, no userinfo/query/fragment, and a compatible base path",
        ));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn origin(raw: &str) -> Result<String, StoreError> {
    Ok(Url::parse(raw)
        .map_err(adapter)?
        .origin()
        .ascii_serialization())
}

fn validate_vector(vector: &[f32], expected: Option<usize>) -> Result<(), StoreError> {
    if vector.is_empty()
        || vector.len() > MAX_VECTOR_DIMENSIONS
        || vector.iter().any(|value| !value.is_finite())
        || expected.is_some_and(|dimensions| dimensions != vector.len())
    {
        return Err(adapter(
            "embedding vector must be finite, nonempty, bounded, and match configured dimensions",
        ));
    }
    Ok(())
}

/// Deterministic, offline signed feature-hashing embeddings.
///
/// This adapter is intentionally lightweight. It provides useful token and bigram
/// similarity without claiming model-derived semantic quality.
pub struct LocalHashEmbeddingProvider {
    dimensions: usize,
}

impl LocalHashEmbeddingProvider {
    /// Create a bounded local embedding profile.
    pub fn new(dimensions: usize) -> Result<Self, StoreError> {
        if !(64..=MAX_VECTOR_DIMENSIONS).contains(&dimensions) {
            return Err(adapter("local embedding dimensions must be in 64..=4096"));
        }
        Ok(Self { dimensions })
    }
}

#[async_trait]
impl EmbeddingProvider for LocalHashEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StoreError> {
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(adapter(
                "embedding input must be nonempty and at most 64 KiB",
            ));
        }
        let normalized = text.to_ascii_lowercase();
        let mut vector = vec![0.0_f32; self.dimensions];
        for token in normalized
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            feature(&mut vector, token.as_bytes(), 1.0);
            for pair in token.as_bytes().windows(2) {
                feature(&mut vector, pair, 0.25);
            }
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm == 0.0 {
            return Err(adapter("embedding input has no indexable features"));
        }
        for value in &mut vector {
            *value /= norm;
        }
        Ok(vector)
    }
}

fn feature(vector: &mut [f32], feature: &[u8], weight: f32) {
    let digest = Sha256::digest(feature);
    let index = usize::from(u16::from_be_bytes([digest[0], digest[1]])) % vector.len();
    let sign = if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
    vector[index] += sign * weight;
}

/// Strict OpenAI-compatible embedding endpoint profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiEmbeddingProfile {
    name: String,
    model: String,
    base_url: String,
    credential_reference: Option<String>,
    timeout_ms: u64,
    dimensions: Option<usize>,
}

impl OpenAiEmbeddingProfile {
    /// Validate and create a profile without resolving credentials.
    pub fn new(
        name: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        credential_reference: Option<String>,
        timeout_ms: u64,
        dimensions: Option<usize>,
    ) -> Result<Self, StoreError> {
        let name = name.into();
        let model = model.into();
        let base_url = normalize_base_url(&base_url.into(), true)?;
        validate_credential_reference(credential_reference.as_deref())?;
        if !valid_name(&name)
            || model.trim().is_empty()
            || model.len() > 256
            || timeout_ms == 0
            || dimensions.is_some_and(|value| !(1..=MAX_VECTOR_DIMENSIONS).contains(&value))
        {
            return Err(adapter("OpenAI-compatible embedding profile is invalid"));
        }
        Ok(Self {
            name,
            model,
            base_url,
            credential_reference,
            timeout_ms,
            dimensions,
        })
    }

    /// Exact embeddings endpoint.
    pub fn endpoint(&self) -> String {
        format!("{}/embeddings", self.base_url)
    }

    /// Canonical endpoint origin for policy obligations.
    pub fn network_origin(&self) -> Result<String, StoreError> {
        origin(&self.base_url)
    }

    /// Configured credential reference, without resolving it.
    pub fn credential_reference(&self) -> Option<&str> {
        self.credential_reference.as_deref()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingEffectInput {
    profile: String,
    model: String,
    input: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
    index: usize,
    object: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingUsage {
    prompt_tokens: u64,
    total_tokens: u64,
}

/// Permit-requiring OpenAI-compatible embedding transport.
pub struct OpenAiEmbeddingExecutor {
    profile: OpenAiEmbeddingProfile,
}

impl OpenAiEmbeddingExecutor {
    /// Construct a transport. Execution remains impossible without a gateway permit.
    pub fn new(profile: OpenAiEmbeddingProfile) -> Self {
        Self { profile }
    }
}

#[async_trait]
impl EffectExecutor for OpenAiEmbeddingExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let input: EmbeddingEffectInput =
            serde_json::from_value(request.content.clone()).map_err(execution)?;
        if request.action != "embedding.openai.create"
            || request.resource != self.profile.endpoint()
            || input.profile != self.profile.name
            || input.model != self.profile.model
            || input.input.trim().is_empty()
            || input.input.len() > MAX_TEXT_BYTES
        {
            return Err(execution("embedding request does not match its profile"));
        }
        validate_credential_disclosure(request, self.profile.credential_reference())
            .map_err(execution)?;
        validate_destination(&permit, &self.profile.network_origin().map_err(execution)?)?;
        let payload = json!({"input": input.input, "model": input.model});
        let bytes = send_http(
            Method::POST,
            &self.profile.endpoint(),
            Some(&payload),
            self.profile.credential_reference(),
            "authorization",
            &permit,
            self.profile.timeout_ms,
        )
        .await
        .map_err(execution)?;
        let response: EmbeddingResponse = serde_json::from_slice(&bytes).map_err(execution)?;
        if response.data.len() != 1
            || response.data[0].index != 0
            || response.data[0].object != "embedding"
            || response
                .object
                .as_deref()
                .is_some_and(|object| object != "list")
            || response
                .model
                .as_deref()
                .is_some_and(|model| model != self.profile.model)
            || response
                .usage
                .as_ref()
                .is_some_and(|usage| usage.total_tokens < usage.prompt_tokens)
        {
            return Err(execution("embedding response shape is invalid"));
        }
        validate_vector(&response.data[0].embedding, self.profile.dimensions).map_err(execution)?;
        let bytes = serde_json::to_vec(&response.data[0].embedding).map_err(execution)?;
        bounded_result(bytes, &permit)
    }
}

/// Embedding port that obtains a permit for every remote request.
pub struct GatewayOpenAiEmbeddingProvider {
    gateway: Arc<EffectGateway>,
    executor: Arc<OpenAiEmbeddingExecutor>,
    profile: OpenAiEmbeddingProfile,
}

impl GatewayOpenAiEmbeddingProvider {
    /// Bind one validated transport to the shared effect gateway.
    pub fn new(
        gateway: Arc<EffectGateway>,
        executor: Arc<OpenAiEmbeddingExecutor>,
        profile: OpenAiEmbeddingProfile,
    ) -> Self {
        Self {
            gateway,
            executor,
            profile,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for GatewayOpenAiEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StoreError> {
        let input = EmbeddingEffectInput {
            profile: self.profile.name.clone(),
            model: self.profile.model.clone(),
            input: text.to_owned(),
        };
        let mut request = effect_request(
            system_index_actor(),
            "embedding.openai.create",
            self.profile.endpoint(),
            serde_json::to_value(input).map_err(adapter)?,
        );
        request.capabilities = vec!["embedding.openai.create".into()];
        request.credential_references = credential_references(self.profile.credential_reference());
        let result = self
            .gateway
            .execute(request, self.executor.as_ref())
            .await
            .map_err(adapter)?;
        let vector: Vec<f32> = serde_json::from_slice(&result.bytes).map_err(adapter)?;
        validate_vector(&vector, self.profile.dimensions)?;
        Ok(vector)
    }
}

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
        let base_url = normalize_base_url(&base_url.into(), false)?;
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
enum ChromaOperation {
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
    fn action(&self) -> &'static str {
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct QueryResponse {
    ids: Vec<Vec<String>>,
    distances: Option<Vec<Vec<Option<f32>>>>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

/// Permit-requiring Chroma v2 HTTP transport.
pub struct ChromaExecutor {
    profile: ChromaProfile,
}

impl ChromaExecutor {
    /// Construct a transport. Execution remains impossible without a gateway permit.
    pub fn new(profile: ChromaProfile) -> Self {
        Self { profile }
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
            self.profile.credential_reference(),
            "x-chroma-token",
            permit,
            self.profile.timeout_ms,
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
            self.profile.credential_reference(),
            "x-chroma-token",
            permit,
            self.profile.timeout_ms,
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
                    self.profile.credential_reference(),
                    "x-chroma-token",
                    permit,
                    self.profile.timeout_ms,
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
                    self.profile.credential_reference(),
                    "x-chroma-token",
                    permit,
                    self.profile.timeout_ms,
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
                    self.profile.credential_reference(),
                    "x-chroma-token",
                    permit,
                    self.profile.timeout_ms,
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
                    self.profile.credential_reference(),
                    "x-chroma-token",
                    permit,
                    self.profile.timeout_ms,
                )
                .await?;
                let documents: usize = serde_json::from_slice(&bytes).map_err(adapter)?;
                Ok(json!({"ready": true, "kind": "chroma", "documents": documents}))
            }
            ChromaOperation::Reset => {
                let id = self.get_collection(permit).await?;
                send_http(
                    Method::DELETE,
                    &self.profile.collection_url(&id, None)?,
                    None,
                    self.profile.credential_reference(),
                    "x-chroma-token",
                    permit,
                    self.profile.timeout_ms,
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

/// Disposable Chroma projection whose network effects are individually authorized.
pub struct ChromaMemoryIndex {
    gateway: Arc<EffectGateway>,
    executor: Arc<ChromaExecutor>,
    embedding: Arc<dyn EmbeddingProvider>,
    profile: ChromaProfile,
    position_path: PathBuf,
    state: Mutex<ProjectionState>,
}

impl ChromaMemoryIndex {
    /// Open local projection metadata and bind the remote adapter to the gateway.
    pub fn open(
        gateway: Arc<EffectGateway>,
        executor: Arc<ChromaExecutor>,
        embedding: Arc<dyn EmbeddingProvider>,
        profile: ChromaProfile,
        position_path: impl Into<PathBuf>,
    ) -> Result<Self, StoreError> {
        let position_path = position_path.into();
        let state = read_position(&position_path)?;
        Ok(Self {
            gateway,
            executor,
            embedding,
            profile,
            position_path,
            state: Mutex::new(state),
        })
    }

    fn ensure_known_outcome(&self) -> Result<(), StoreError> {
        if self.state.lock().map_err(adapter)?.outcome_unknown {
            Err(adapter(
                "Chroma mutation outcome is unknown; an operator-authorized rebuild is required",
            ))
        } else {
            Ok(())
        }
    }

    fn mark_outcome_unknown(&self) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.outcome_unknown = true;
        persist_position(&self.position_path, *state)
    }

    fn clear_outcome_unknown(&self) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.outcome_unknown = false;
        persist_position(&self.position_path, *state)
    }

    async fn execute(&self, operation: ChromaOperation) -> Result<Value, StoreError> {
        let action = operation.action();
        let idempotency_id = match &operation {
            ChromaOperation::Upsert { event_id, .. } | ChromaOperation::Remove { event_id, .. } => {
                Some(event_id.clone())
            }
            _ => None,
        };
        let mut request = effect_request(
            system_index_actor(),
            action,
            self.profile.resource(),
            serde_json::to_value(operation).map_err(adapter)?,
        );
        request.capabilities = vec![action.into()];
        request.idempotency_id = idempotency_id;
        request.credential_references = credential_references(self.profile.credential_reference());
        let result = match self.gateway.execute(request, self.executor.as_ref()).await {
            Ok(result) => result,
            Err(GatewayError::OutcomeUnknown(message)) => {
                self.mark_outcome_unknown()?;
                return Err(adapter(format!(
                    "Chroma mutation outcome is unknown and automatic retry is blocked: {message}"
                )));
            }
            Err(error) => return Err(adapter(error)),
        };
        serde_json::from_slice(&result.bytes).map_err(adapter)
    }
}

#[async_trait]
impl MemoryIndex for ChromaMemoryIndex {
    fn position(&self) -> Result<u64, StoreError> {
        self.state
            .lock()
            .map(|state| state.position)
            .map_err(adapter)
    }

    async fn set_position(&self, position: u64) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.position = position;
        persist_position(&self.position_path, *state)?;
        Ok(())
    }

    async fn upsert(
        &self,
        event_id: &str,
        memory_id: &str,
        text: &str,
        metadata: &Value,
        embedding: Option<&[f32]>,
    ) -> Result<(), StoreError> {
        self.ensure_known_outcome()?;
        let embedding = match embedding {
            Some(vector) => vector.to_vec(),
            None => self.embedding.embed(text).await?,
        };
        validate_projection_record(event_id, memory_id, text, metadata, &embedding)?;
        self.execute(ChromaOperation::Upsert {
            event_id: event_id.into(),
            memory_id: memory_id.into(),
            text: text.into(),
            metadata: metadata.clone(),
            embedding,
        })
        .await?;
        Ok(())
    }

    async fn remove(&self, event_id: &str, memory_id: &str) -> Result<(), StoreError> {
        self.ensure_known_outcome()?;
        self.execute(ChromaOperation::Remove {
            event_id: event_id.into(),
            memory_id: memory_id.into(),
        })
        .await?;
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError> {
        self.ensure_known_outcome()?;
        let embedding = self.embedding.embed(query).await?;
        let value = self
            .execute(ChromaOperation::Search { embedding, limit })
            .await?;
        serde_json::from_value(
            value
                .get("candidates")
                .cloned()
                .ok_or_else(|| adapter("Chroma candidate output is absent"))?,
        )
        .map_err(adapter)
    }

    async fn status(&self) -> Result<Value, StoreError> {
        if self.state.lock().map_err(adapter)?.outcome_unknown {
            return Ok(json!({
                "ready": false,
                "kind": "chroma",
                "outcome_unknown": true,
                "reason": "operator-authorized rebuild required before retry",
            }));
        }
        match self.execute(ChromaOperation::Status).await {
            Ok(status) => Ok(status),
            Err(error) => Ok(json!({
                "ready": false,
                "kind": "chroma",
                "outcome_unknown": false,
                "reason": error.to_string(),
            })),
        }
    }

    async fn rebuild(&self, records: &[(String, String, Value)]) -> Result<(), StoreError> {
        if records.len() > MAX_REBUILD_RECORDS {
            return Err(adapter("Chroma rebuild exceeds 1000 canonical records"));
        }
        self.execute(ChromaOperation::Reset).await?;
        self.clear_outcome_unknown()?;
        for (id, text, metadata) in records {
            self.upsert(&format!("rebuild:{id}"), id, text, metadata, None)
                .await?;
        }
        Ok(())
    }
}

fn validate_projection_record(
    event_id: &str,
    memory_id: &str,
    text: &str,
    metadata: &Value,
    embedding: &[f32],
) -> Result<(), StoreError> {
    let metadata_bytes = serde_json::to_vec(metadata).map_err(adapter)?;
    if event_id.is_empty()
        || memory_id.is_empty()
        || text.trim().is_empty()
        || text.len() > MAX_TEXT_BYTES
        || metadata_bytes.len() > MAX_METADATA_BYTES
        || !metadata.is_object()
    {
        return Err(adapter("Chroma projection record is invalid or oversized"));
    }
    validate_vector(embedding, None)
}

fn system_index_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "memory-indexer".into(),
    }
}

fn credential_references(reference: Option<&str>) -> Vec<CredentialReference> {
    reference
        .map(|reference| {
            vec![CredentialReference {
                reference: reference.into(),
                value_hash: None,
            }]
        })
        .unwrap_or_default()
}

fn validate_credential_disclosure(
    request: &EffectRequest,
    expected: Option<&str>,
) -> Result<(), StoreError> {
    let actual = request
        .credential_references
        .iter()
        .map(|reference| reference.reference.as_str())
        .collect::<Vec<_>>();
    match expected {
        Some(expected) if actual == [expected] => Ok(()),
        None if actual.is_empty() => Ok(()),
        _ => Err(adapter(
            "semantic request credential references do not match configuration",
        )),
    }
}

fn validate_destination(
    permit: &ExecutionPermit,
    expected_origin: &str,
) -> Result<(), ExecutionError> {
    if permit
        .obligations()
        .network_destinations
        .iter()
        .any(|destination| destination == expected_origin)
    {
        Ok(())
    } else {
        Err(execution(
            "semantic endpoint origin is absent from permit obligations",
        ))
    }
}

async fn send_http(
    method: Method,
    endpoint: &str,
    payload: Option<&Value>,
    credential_reference: Option<&str>,
    credential_header: &str,
    permit: &ExecutionPermit,
    configured_timeout_ms: u64,
) -> Result<Vec<u8>, StoreError> {
    let url = Url::parse(endpoint).map_err(adapter)?;
    let host = url
        .host_str()
        .ok_or_else(|| adapter("semantic endpoint has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| adapter("semantic endpoint has no port"))?;
    let addresses = resolve_addresses(host, port).await?;
    let timeout_ms = configured_timeout_ms.min(permit.obligations().timeout_ms);
    let client = Client::builder()
        .no_proxy()
        .redirect(RedirectPolicy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(adapter)?;
    let mut builder = client.request(method, url);
    if let Some(reference) = credential_reference {
        let secret = resolve_credential(reference)?;
        builder = if credential_header == "authorization" {
            builder.bearer_auth(secret)
        } else {
            builder.header(credential_header, secret)
        };
    }
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(payload).map_err(adapter)?;
        if bytes.len() > 1024 * 1024 {
            return Err(adapter("semantic request exceeds 1 MiB"));
        }
        builder = builder
            .header("content-type", "application/json")
            .body(bytes);
    }
    let response = builder.send().await.map_err(adapter)?;
    if !response.status().is_success() {
        return Err(adapter(format!(
            "semantic endpoint returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let permitted = usize::try_from(permit.obligations().max_output_bytes).map_err(adapter)?;
    let limit = permitted.min(MAX_RESPONSE_BYTES);
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(adapter)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(adapter(
                "semantic response exceeds the permitted output bound",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn resolve_addresses(host: &str, port: u16) -> Result<Vec<SocketAddr>, StoreError> {
    let addresses = lookup_host((host, port)).await.map_err(adapter)?;
    let mut unique = BTreeSet::new();
    for address in addresses {
        unique.insert(address);
        if unique.len() > MAX_RESOLVED_ADDRESSES {
            return Err(adapter("semantic endpoint resolved to too many addresses"));
        }
    }
    if unique.is_empty() {
        return Err(adapter("semantic endpoint did not resolve"));
    }
    Ok(unique.into_iter().collect())
}

fn bounded_result(
    bytes: Vec<u8>,
    permit: &ExecutionPermit,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let limit = usize::try_from(permit.obligations().max_output_bytes).map_err(execution)?;
    if bytes.len() > limit {
        return Err(execution("semantic output exceeds the permitted bound"));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}

fn read_position(path: &Path) -> Result<ProjectionState, StoreError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<PositionFile>(&bytes)
            .map_err(adapter)
            .and_then(|value| {
                if value.schema_version == 1 {
                    Ok(ProjectionState {
                        position: value.position,
                        outcome_unknown: value.outcome_unknown,
                    })
                } else {
                    Err(adapter("unsupported Chroma position schema"))
                }
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ProjectionState::default())
        }
        Err(error) => Err(adapter(error)),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionFile {
    schema_version: u16,
    position: u64,
    #[serde(default)]
    outcome_unknown: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProjectionState {
    position: u64,
    outcome_unknown: bool,
}

fn persist_position(path: &Path, state: ProjectionState) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| adapter("Chroma position path has no parent"))?;
    fs::create_dir_all(parent).map_err(adapter)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec(&PositionFile {
        schema_version: 1,
        position: state.position,
        outcome_unknown: state.outcome_unknown,
    })
    .map_err(adapter)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(adapter)?;
    if let Err(error) = (|| -> Result<(), std::io::Error> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })() {
        let _ = fs::remove_file(&temporary);
        return Err(adapter(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ChromaExecutor, ChromaMemoryIndex, ChromaProfile, GatewayOpenAiEmbeddingProvider,
        LocalHashEmbeddingProvider, OpenAiEmbeddingExecutor, OpenAiEmbeddingProfile,
        ProjectionState, persist_position, read_position,
    };
    use colossus_contracts::DecisionOutcome;
    use colossus_policy::{BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel};
    use colossus_ports::{EmbeddingProvider, EventJournal, MemoryIndex};
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    #[tokio::test]
    async fn local_embeddings_are_deterministic_normalized_and_distinct() {
        let provider = LocalHashEmbeddingProvider::new(128).expect("profile");
        let first = provider.embed("Rust audit journal").await.expect("embed");
        let same = provider.embed("Rust audit journal").await.expect("embed");
        let different = provider
            .embed("semantic memory search")
            .await
            .expect("embed");
        assert_eq!(first, same);
        assert_ne!(first, different);
        let norm = first.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.000_1);
    }

    #[test]
    fn position_metadata_round_trips_and_rejects_unknown_fields() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("position.json");
        assert_eq!(
            read_position(&path).expect("missing position"),
            ProjectionState::default()
        );
        let state = ProjectionState {
            position: 42,
            outcome_unknown: true,
        };
        persist_position(&path, state).expect("persist");
        assert_eq!(read_position(&path).expect("position"), state);
        std::fs::write(
            &path,
            br#"{"schemaVersion":1,"position":42,"outcomeUnknown":true,"unexpected":true}"#,
        )
        .expect("write");
        assert!(read_position(&path).is_err());
    }

    #[tokio::test]
    async fn chroma_conformance_runs_only_through_audited_effects() {
        let fixture = ChromaFixture::start().await;
        let (index, journal) = fixture.index(true).expect("index");
        index
            .upsert(
                "event-1",
                "memory-1",
                "Rust audit journal",
                &serde_json::json!({"scope": "global"}),
                None,
            )
            .await
            .expect("upsert");
        let candidates = index.search("audit journal", 4).await.expect("search");
        assert_eq!(candidates, vec![("memory-1".into(), 0.8)]);
        let status = index.status().await.expect("status");
        assert_eq!(status["kind"], "chroma");
        assert_eq!(status["documents"], 1);
        index.remove("event-2", "memory-1").await.expect("remove");
        index
            .rebuild(&[(
                "memory-2".into(),
                "durable workflow".into(),
                serde_json::json!({"scope": "global"}),
            )])
            .await
            .expect("rebuild");
        index.set_position(17).await.expect("position");
        assert_eq!(index.position().expect("position"), 17);

        let events = journal.read_global(1, 1_000).expect("events");
        let requested = events
            .iter()
            .filter(|event| event.event_type == "effect.requested.v1")
            .count();
        assert_eq!(requested, 6);
        let requests = fixture.requests.lock().expect("requests").clone();
        assert_eq!(requests.len(), 12);
        assert!(requests.iter().any(|request| request.starts_with(
            "POST /api/v2/tenants/default_tenant/databases/default_database/collections HTTP/1.1"
        )));
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/upsert HTTP/1.1"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/query HTTP/1.1"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/count HTTP/1.1"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("/delete HTTP/1.1"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("DELETE "))
        );
        fixture.task.abort();
    }

    #[tokio::test]
    async fn denied_chroma_effect_never_reaches_the_network() {
        let fixture = ChromaFixture::start().await;
        let (index, journal) = fixture.index(false).expect("index");
        assert!(
            index
                .upsert(
                    "event-1",
                    "memory-1",
                    "denied content",
                    &serde_json::json!({}),
                    None,
                )
                .await
                .is_err()
        );
        assert_eq!(fixture.accepted.load(Ordering::Acquire), 0);
        let events = journal.read_global(1, 100).expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.denied.v1")
        );
        fixture.task.abort();
    }

    #[tokio::test]
    async fn interrupted_chroma_mutation_is_audited_as_outcome_unknown() {
        let fixture = ChromaFixture::start().await;
        let (index, journal) = fixture.index(true).expect("index");
        fixture.task.abort();
        tokio::task::yield_now().await;
        assert!(
            index
                .upsert(
                    "event-unknown",
                    "memory-unknown",
                    "external outcome cannot be proven",
                    &serde_json::json!({}),
                    None,
                )
                .await
                .is_err()
        );
        let events = journal.read_global(1, 100).expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.outcome_unknown.v1")
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == "effect.failed.v1")
        );
        let accepted = fixture.accepted.load(Ordering::Acquire);
        assert!(
            index
                .upsert(
                    "event-retry",
                    "memory-unknown",
                    "automatic retry must remain blocked",
                    &serde_json::json!({}),
                    None,
                )
                .await
                .is_err()
        );
        assert_eq!(fixture.accepted.load(Ordering::Acquire), accepted);
        assert_eq!(
            index.status().await.expect("status")["outcome_unknown"],
            true
        );
        let (reopened, _) = fixture.index(false).expect("reopen");
        assert_eq!(
            reopened.status().await.expect("reopened status")["outcome_unknown"],
            true
        );
    }

    #[tokio::test]
    async fn explicit_rebuild_clears_durable_unknown_outcome_marker() {
        let fixture = ChromaFixture::start().await;
        let path = fixture.directory.path().join("position.json");
        persist_position(
            &path,
            ProjectionState {
                position: 19,
                outcome_unknown: true,
            },
        )
        .expect("unknown marker");
        let (index, _) = fixture.index(true).expect("index");
        assert_eq!(
            index.status().await.expect("blocked status")["ready"],
            false
        );
        assert_eq!(fixture.accepted.load(Ordering::Acquire), 0);
        index.rebuild(&[]).await.expect("explicit rebuild");
        assert!(!read_position(&path).expect("position").outcome_unknown);
        assert_eq!(index.status().await.expect("ready status")["ready"], true);
        fixture.task.abort();
    }

    #[tokio::test]
    async fn openai_compatible_embeddings_are_permit_bound_and_strictly_normalized() {
        let fixture = ChromaFixture::start().await;
        let profile = OpenAiEmbeddingProfile::new(
            "fixture",
            "embed-test",
            format!("{}/v1", fixture.origin),
            None,
            5_000,
            Some(3),
        )
        .expect("profile");
        let executor = Arc::new(OpenAiEmbeddingExecutor::new(profile.clone()));
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = Arc::new(EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("embedding.openai.create", DecisionOutcome::Allow)
                    .with_network_destination(&fixture.origin),
            ),
            Arc::new(DenyApproval),
            SafetyKernel::new(["embedding.openai.create".into()]),
            [41_u8; 32],
        ));
        let provider = GatewayOpenAiEmbeddingProvider::new(gateway, executor, profile);
        assert_eq!(
            provider
                .embed("bounded semantic input")
                .await
                .expect("embed"),
            vec![0.1, 0.2, 0.3]
        );
        let events = journal.read_global(1, 100).expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "effect.requested.v1")
        );
        assert!(
            fixture
                .requests
                .lock()
                .expect("requests")
                .iter()
                .any(|request| request.contains("/v1/embeddings HTTP/1.1"))
        );
        fixture.task.abort();
    }

    struct ChromaFixture {
        origin: String,
        requests: Arc<Mutex<Vec<String>>>,
        accepted: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
        directory: tempfile::TempDir,
    }

    impl ChromaFixture {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let address = listener.local_addr().expect("address");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let accepted = Arc::new(AtomicUsize::new(0));
            let requests_for_task = Arc::clone(&requests);
            let accepted_for_task = Arc::clone(&accepted);
            let task = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    accepted_for_task.fetch_add(1, Ordering::AcqRel);
                    let mut bytes = Vec::new();
                    let mut chunk = [0_u8; 4_096];
                    let header_end = loop {
                        let Ok(read) = stream.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..read]);
                        if let Some(index) =
                            bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            break index + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':').and_then(|(name, value)| {
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                        })
                        .unwrap_or(0);
                    while bytes.len() < header_end.saturating_add(content_length) {
                        let Ok(read) = stream.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&chunk[..read]);
                    }
                    let request_line = headers.lines().next().unwrap_or_default().to_owned();
                    requests_for_task
                        .lock()
                        .expect("requests")
                        .push(request_line.clone());
                    let body = fixture_response(&request_line);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
            Self {
                origin: format!("http://{address}"),
                requests,
                accepted,
                task,
                directory: tempfile::tempdir().expect("directory"),
            }
        }

        fn index(
            &self,
            allow: bool,
        ) -> Result<(ChromaMemoryIndex, Arc<dyn EventJournal>), colossus_ports::StoreError>
        {
            let profile = ChromaProfile::new(
                &self.origin,
                "default_tenant",
                "default_database",
                "colossus-memory",
                None,
                5_000,
            )?;
            let actions = [
                "memory.index.chroma.upsert",
                "memory.index.chroma.remove",
                "memory.index.chroma.search",
                "memory.index.chroma.status",
                "memory.index.chroma.reset",
            ];
            let mut policy =
                BuiltInPolicy::offline_default().with_network_destination(&self.origin);
            if allow {
                for action in actions {
                    policy = policy.with_action(action, DecisionOutcome::Allow);
                }
            }
            let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
            let gateway = Arc::new(EffectGateway::new(
                Arc::clone(&journal),
                Arc::new(policy),
                Arc::new(DenyApproval),
                SafetyKernel::new(actions.into_iter().map(str::to_owned)),
                [37_u8; 32],
            ));
            let executor = Arc::new(ChromaExecutor::new(profile.clone()));
            let embedding: Arc<dyn EmbeddingProvider> =
                Arc::new(LocalHashEmbeddingProvider::new(128)?);
            let index = ChromaMemoryIndex::open(
                gateway,
                executor,
                embedding,
                profile,
                self.directory.path().join("position.json"),
            )?;
            Ok((index, journal))
        }
    }

    fn fixture_response(request_line: &str) -> &'static str {
        if request_line.contains("/v1/embeddings HTTP/1.1") {
            r#"{"data":[{"embedding":[0.1,0.2,0.3],"index":0,"object":"embedding"}],"object":"list","model":"embed-test","usage":{"prompt_tokens":3,"total_tokens":3}}"#
        } else if request_line.contains("/query HTTP/1.1") {
            r#"{"ids":[["memory-1"]],"distances":[[0.25]]}"#
        } else if request_line.contains("/count HTTP/1.1") {
            "1"
        } else if request_line.ends_with("/collections HTTP/1.1")
            || request_line.contains("/collections/colossus-memory HTTP/1.1")
        {
            r#"{"id":"collection-id","name":"colossus-memory","tenant":"default_tenant","database":"default_database","dimension":128}"#
        } else if request_line.contains("/delete HTTP/1.1") {
            r#"{"deleted":1}"#
        } else {
            "{}"
        }
    }
}
