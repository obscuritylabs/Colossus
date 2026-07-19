use super::*;

/// Strict logical input placed inside a search effect request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchEffectInput {
    /// Trusted profile selected by role routing.
    pub profile: String,
    /// Provider-neutral validated search request.
    pub request: SearchRequest,
}

/// Search configuration, credential, transport, or normalization failure.
#[derive(Debug, Error)]
pub enum SearchAdapterError {
    /// Strict profile or request validation failed.
    #[error("search configuration error: {0}")]
    Configuration(String),
    /// A configured credential reference could not be resolved.
    #[error("search credential unavailable: {0}")]
    Credential(String),
    /// DNS or client setup failed before any request could be dispatched.
    #[error("search unavailable before dispatch: {0}")]
    PreDispatch(String),
    /// The request was dispatched but did not return a known terminal response.
    #[error("search transport failure: {0}")]
    Transport(String),
    /// The endpoint returned a non-success status.
    #[error("search endpoint returned HTTP {status}")]
    Status {
        /// HTTP status code only; response bodies are never included.
        status: u16,
    },
    /// The provider response violated the normalized contract.
    #[error("malformed search output: {0}")]
    Malformed(String),
}

impl From<url::ParseError> for SearchAdapterError {
    fn from(error: url::ParseError) -> Self {
        Self::Configuration(error.to_string())
    }
}

/// Resolves a credential only after the effect gateway supplies a permit.
pub trait CredentialResolver: Send + Sync {
    /// Resolve a configured reference without logging the returned value.
    fn resolve(&self, reference: &str) -> Result<String, SearchAdapterError>;
}

/// Environment-only credential resolver for first-party search adapters.
#[derive(Default)]
pub struct EnvironmentCredentialResolver;

impl CredentialResolver for EnvironmentCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, SearchAdapterError> {
        let variable = reference.strip_prefix("env:").ok_or_else(|| {
            SearchAdapterError::Credential("credential reference is not environment-backed".into())
        })?;
        std::env::var(variable).map_err(|_| {
            SearchAdapterError::Credential(format!("environment variable {variable} is unset"))
        })
    }
}

/// One permit-bound search adapter instance.
pub struct SearchExecutor {
    pub(super) profile: SearchProfile,
    credentials: Arc<dyn CredentialResolver>,
}

impl SearchExecutor {
    /// Construct an adapter using environment credential references.
    pub fn new(profile: SearchProfile) -> Self {
        Self::with_credentials(profile, Arc::new(EnvironmentCredentialResolver))
    }

    /// Construct an adapter with an injected credential resolver.
    pub fn with_credentials(
        profile: SearchProfile,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Self {
        Self {
            profile,
            credentials,
        }
    }

    /// Profile metadata without credentials.
    pub fn profile(&self) -> &SearchProfile {
        &self.profile
    }

    async fn execute_permitted(
        &self,
        effect: &EffectRequest,
        permit: &ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, SearchAdapterError> {
        let input: SearchEffectInput = serde_json::from_value(effect.content.clone())
            .map_err(|error| SearchAdapterError::Malformed(error.to_string()))?;
        if effect.action != "web.search"
            || effect.resource != self.profile.endpoint
            || input.profile != self.profile.name
        {
            return Err(SearchAdapterError::Configuration(
                "search effect does not match its selected profile".into(),
            ));
        }
        validate_request(&input.request)?;
        validate_credential_disclosure(effect, &self.profile)?;
        let origin = self.profile.network_origin()?;
        let matched =
            network_destination_match(&permit.obligations().network_destinations, &origin)
                .map_err(|error| SearchAdapterError::Configuration(error.to_string()))?
                .ok_or_else(|| {
                    SearchAdapterError::Configuration(
                        "search origin is absent from permit obligations".into(),
                    )
                })?;
        if matched == NetworkDestinationMatch::PublicWildcard
            && url_host_is_non_public_literal(&self.profile.endpoint)?
        {
            return Err(SearchAdapterError::Configuration(
                "public wildcard cannot authorize a non-public search origin".into(),
            ));
        }
        let mut url = Url::parse(&self.profile.endpoint)?;
        let secret = self
            .profile
            .credential_reference
            .as_deref()
            .map(|reference| self.credentials.resolve(reference))
            .transpose()?;
        if secret.as_deref().is_some_and(str::is_empty) {
            return Err(SearchAdapterError::Credential(
                "resolved search credential is empty".into(),
            ));
        }
        match self.profile.kind {
            SearchKind::Searxng => {
                url.query_pairs_mut()
                    .append_pair("q", &input.request.query)
                    .append_pair("format", "json");
            }
            SearchKind::SerpApi => {
                url.query_pairs_mut()
                    .append_pair("engine", "google")
                    .append_pair("q", &input.request.query)
                    .append_pair("num", &input.request.limit.to_string())
                    .append_pair("api_key", secret.as_deref().unwrap_or_default());
            }
        }
        let host = url
            .host_str()
            .ok_or_else(|| SearchAdapterError::Configuration("search URL has no host".into()))?;
        let port = url.port_or_known_default().ok_or_else(|| {
            SearchAdapterError::Configuration("search URL has no known port".into())
        })?;
        let allow_non_public = matched == NetworkDestinationMatch::Exact
            && (host.eq_ignore_ascii_case("localhost")
                || host.parse::<IpAddr>().is_ok_and(non_public_network_address));
        let addresses = resolve_search_addresses(host, port, allow_non_public).await?;
        let timeout_ms = self.profile.timeout_ms.min(permit.obligations().timeout_ms);
        let client = Client::builder()
            .no_proxy()
            .redirect(RedirectPolicy::none())
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|_| {
                SearchAdapterError::PreDispatch("HTTP client construction failed".into())
            })?;
        let mut request = client
            .get(url)
            .header("accept", "application/json")
            .header("user-agent", &self.profile.user_agent);
        if self.profile.kind == SearchKind::Searxng
            && let (Some(header), Some(value)) =
                (self.profile.auth_header.as_deref(), secret.as_deref())
        {
            request = request.header(header, value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| SearchAdapterError::Transport("HTTP request failed".into()))?;
        if !response.status().is_success() {
            return Err(SearchAdapterError::Status {
                status: response.status().as_u16(),
            });
        }
        let raw_limit = usize::try_from(permit.obligations().max_output_bytes)
            .map_err(|error| SearchAdapterError::Configuration(error.to_string()))?
            .min(MAX_RESPONSE_BYTES);
        if raw_limit == 0 {
            return Err(SearchAdapterError::Configuration(
                "search output permit must allow at least one byte".into(),
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| SearchAdapterError::Transport("response stream failed".into()))?;
            if bytes.len().saturating_add(chunk.len()) > raw_limit {
                return Err(SearchAdapterError::Malformed(
                    "search response exceeds the permitted output bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let bytes = redact_exact_secret(&bytes, secret.as_deref());
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| SearchAdapterError::Malformed("search response is not JSON".into()))?;
        let response = normalize_response(self.profile.kind, &input.request, &value)?;
        let bytes = serde_json::to_vec(&response)
            .map_err(|error| SearchAdapterError::Malformed(error.to_string()))?;
        if bytes.len() > raw_limit {
            return Err(SearchAdapterError::Malformed(
                "normalized search output exceeds the permitted bound".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes,
            effect_succeeded: true,
        })
    }
}

#[async_trait]
impl EffectExecutor for SearchExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.execute_permitted(request, &permit)
            .await
            .map_err(search_execution_error)
    }
}
