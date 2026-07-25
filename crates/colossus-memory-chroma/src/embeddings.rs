use super::*;

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
    tls_roots: AdditionalRootCertificates,
}

impl OpenAiEmbeddingExecutor {
    /// Construct a transport. Execution remains impossible without a gateway permit.
    pub fn new(profile: OpenAiEmbeddingProfile) -> Self {
        Self {
            profile,
            tls_roots: AdditionalRootCertificates::default(),
        }
    }

    /// Add validated runtime-wide CA roots to the embedding client's built-in roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
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
            &permit,
            HttpTransport::new(
                self.profile.credential_reference(),
                "authorization",
                self.profile.timeout_ms,
                &self.tls_roots,
            ),
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
