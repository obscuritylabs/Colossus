//! Permit-bound provider-neutral web-search adapters and role routing.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use colossus_contracts::{
    CredentialReference, EffectRequest, QuarantinedEffectResult, SearchProfileSummary,
    SearchRequest, SearchResponse, SearchResult,
};
use colossus_policy::{EffectExecutor, ExecutionError, ExecutionPermit};
use futures::StreamExt as _;
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::net::lookup_host;

const MAX_QUERY_BYTES: usize = 4_096;
const MAX_RESULTS: usize = 20;
const DEFAULT_RESULTS: usize = 10;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SEARCH_ADDRESSES: usize = 16;
const MAX_TITLE_CHARS: usize = 4_096;
const MAX_URL_CHARS: usize = 8_192;
const MAX_SNIPPET_CHARS: usize = 32_768;
const MAX_SOURCE_CHARS: usize = 256;

/// Supported first-party search adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// SearXNG JSON search endpoint.
    Searxng,
    /// SerpAPI Google organic-results endpoint.
    SerpApi,
}

impl SearchKind {
    /// Stable adapter label used in route and audit metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Searxng => "searxng",
            Self::SerpApi => "serp_api",
        }
    }
}

/// Strict normalized search profile composed by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchProfile {
    name: String,
    kind: SearchKind,
    endpoint: String,
    credential_reference: Option<String>,
    auth_header: Option<String>,
    user_agent: String,
    timeout_ms: u64,
}

impl SearchProfile {
    /// Validate one profile without resolving credentials.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        kind: SearchKind,
        endpoint: impl Into<String>,
        credential_reference: Option<String>,
        auth_header: Option<String>,
        user_agent: impl Into<String>,
        timeout_ms: u64,
    ) -> Result<Self, SearchAdapterError> {
        let name = name.into();
        let endpoint = normalize_endpoint(kind, &endpoint.into())?;
        let user_agent = user_agent.into();
        if !valid_name(&name) || timeout_ms == 0 {
            return Err(SearchAdapterError::Configuration(
                "profile name and timeout must be valid and nonzero".into(),
            ));
        }
        if user_agent.trim().is_empty() || user_agent.len() > 256 {
            return Err(SearchAdapterError::Configuration(
                "search user agent must contain 1 through 256 bytes".into(),
            ));
        }
        if let Some(reference) = credential_reference.as_deref()
            && !valid_credential_reference(reference)
        {
            return Err(SearchAdapterError::Configuration(
                "search credentials must use an env:VARIABLE reference".into(),
            ));
        }
        match kind {
            SearchKind::Searxng => {
                if credential_reference.is_some()
                    && auth_header.as_deref().is_none_or(str::is_empty)
                {
                    return Err(SearchAdapterError::Configuration(
                        "credentialed SearXNG profiles require authHeader".into(),
                    ));
                }
                if auth_header
                    .as_deref()
                    .is_some_and(|header| !valid_header_name(header))
                {
                    return Err(SearchAdapterError::Configuration(
                        "SearXNG authHeader is invalid".into(),
                    ));
                }
            }
            SearchKind::SerpApi => {
                if credential_reference.is_none() || auth_header.is_some() {
                    return Err(SearchAdapterError::Configuration(
                        "SerpAPI requires a credential reference and does not accept authHeader"
                            .into(),
                    ));
                }
            }
        }
        Ok(Self {
            name,
            kind,
            endpoint,
            credential_reference,
            auth_header,
            user_agent,
            timeout_ms,
        })
    }

    /// Stable profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Adapter kind.
    pub fn kind(&self) -> SearchKind {
        self.kind
    }

    /// Exact credential-free endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Canonical endpoint origin for policy validation.
    pub fn network_origin(&self) -> Result<String, SearchAdapterError> {
        Url::parse(&self.endpoint)
            .map(|url| url.origin().ascii_serialization())
            .map_err(SearchAdapterError::from)
    }

    /// Credential reference suitable for policy input.
    pub fn credential_reference(&self) -> Option<CredentialReference> {
        self.credential_reference
            .as_ref()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
    }

    /// Safe profile metadata for diagnostics.
    pub fn summary(&self) -> SearchProfileSummary {
        SearchProfileSummary {
            profile: self.name.clone(),
            provider: self.kind.as_str().into(),
            endpoint: self.endpoint.clone(),
            credential_reference: self.credential_reference.clone(),
            timeout_ms: self.timeout_ms,
        }
    }
}

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
    profile: SearchProfile,
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
        if !permit.obligations().network_destinations.contains(&origin) {
            return Err(SearchAdapterError::Configuration(
                "search origin is absent from permit obligations".into(),
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
        let addresses = resolve_search_addresses(host, port).await?;
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

/// Role-to-profile routing over permit-bound search adapters.
pub struct SearchRegistry {
    profiles: BTreeMap<String, Arc<SearchExecutor>>,
    roles: BTreeMap<String, String>,
}

impl SearchRegistry {
    /// Validate unique profiles and every configured role target.
    pub fn new(
        profiles: Vec<SearchExecutor>,
        roles: BTreeMap<String, String>,
    ) -> Result<Self, SearchAdapterError> {
        let mut indexed = BTreeMap::new();
        for executor in profiles {
            let name = executor.profile.name.clone();
            if indexed.insert(name.clone(), Arc::new(executor)).is_some() {
                return Err(SearchAdapterError::Configuration(format!(
                    "duplicate search profile {name}"
                )));
            }
        }
        for (role, profile) in &roles {
            if role.is_empty() || !indexed.contains_key(profile) {
                return Err(SearchAdapterError::Configuration(format!(
                    "search role {role} references unknown profile {profile}"
                )));
            }
        }
        Ok(Self {
            profiles: indexed,
            roles,
        })
    }

    /// Resolve one exact configured role without fallback.
    pub fn resolve(&self, role: &str) -> Result<Arc<SearchExecutor>, SearchAdapterError> {
        let profile = self.roles.get(role).ok_or_else(|| {
            SearchAdapterError::Configuration(format!("search role {role} is not configured"))
        })?;
        self.profiles.get(profile).cloned().ok_or_else(|| {
            SearchAdapterError::Configuration(format!("search profile {profile} is absent"))
        })
    }

    /// Stable role mappings for diagnostics.
    pub fn routes(&self) -> &BTreeMap<String, String> {
        &self.roles
    }

    /// Sorted safe profile summaries.
    pub fn profiles(&self) -> Vec<SearchProfileSummary> {
        self.profiles
            .values()
            .map(|executor| executor.profile.summary())
            .collect()
    }
}

fn validate_request(request: &SearchRequest) -> Result<(), SearchAdapterError> {
    if request.query.trim().is_empty()
        || request.query.len() > MAX_QUERY_BYTES
        || !(1..=MAX_RESULTS).contains(&request.limit)
    {
        return Err(SearchAdapterError::Configuration(format!(
            "search query must contain 1 through {MAX_QUERY_BYTES} bytes and limit must be in 1..={MAX_RESULTS}"
        )));
    }
    Ok(())
}

fn normalize_response(
    kind: SearchKind,
    request: &SearchRequest,
    value: &Value,
) -> Result<SearchResponse, SearchAdapterError> {
    if kind == SearchKind::SerpApi && value.get("error").is_some() {
        return Err(SearchAdapterError::Malformed(
            "SerpAPI returned an application error".into(),
        ));
    }
    let key = match kind {
        SearchKind::Searxng => "results",
        SearchKind::SerpApi => "organic_results",
    };
    let results = value.get(key).and_then(Value::as_array).ok_or_else(|| {
        SearchAdapterError::Malformed(format!("search response has no {key} array"))
    })?;
    let results = results
        .iter()
        .filter_map(|item| normalize_result(kind, item))
        .take(request.limit)
        .enumerate()
        .map(|(index, mut result)| {
            result.rank = index.saturating_add(1);
            result
        })
        .collect::<Vec<_>>();
    Ok(SearchResponse {
        query: request.query.clone(),
        count: results.len(),
        results,
    })
}

fn normalize_result(kind: SearchKind, value: &Value) -> Option<SearchResult> {
    let object = value.as_object()?;
    let url = match kind {
        SearchKind::Searxng => object.get("url"),
        SearchKind::SerpApi => object.get("link"),
    }
    .and_then(Value::as_str)?;
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return None;
    }
    let normalized_url = take_chars(parsed.as_str(), MAX_URL_CHARS);
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(&normalized_url);
    let snippet = match kind {
        SearchKind::Searxng => object.get("content").or_else(|| object.get("snippet")),
        SearchKind::SerpApi => object.get("snippet"),
    }
    .and_then(Value::as_str)
    .unwrap_or_default();
    let source = match kind {
        SearchKind::Searxng => object.get("engine"),
        SearchKind::SerpApi => object.get("source"),
    }
    .and_then(Value::as_str)
    .filter(|source| !source.trim().is_empty())
    .map(|source| take_chars(source, MAX_SOURCE_CHARS));
    Some(SearchResult {
        rank: 0,
        title: take_chars(title, MAX_TITLE_CHARS),
        url: normalized_url,
        snippet: take_chars(snippet, MAX_SNIPPET_CHARS),
        source,
    })
}

fn search_execution_error(error: SearchAdapterError) -> ExecutionError {
    match error {
        SearchAdapterError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
            "search transport failed after dispatch; provider usage may have occurred: {message}"
        )),
        error => ExecutionError::Failed(error.to_string()),
    }
}

fn validate_credential_disclosure(
    effect: &EffectRequest,
    profile: &SearchProfile,
) -> Result<(), SearchAdapterError> {
    let expected = profile
        .credential_reference
        .as_deref()
        .into_iter()
        .collect::<Vec<_>>();
    let actual = effect
        .credential_references
        .iter()
        .map(|reference| reference.reference.as_str())
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(SearchAdapterError::Configuration(
            "search credential disclosure does not match its selected profile".into(),
        ));
    }
    Ok(())
}

fn normalize_endpoint(kind: SearchKind, raw: &str) -> Result<String, SearchAdapterError> {
    let url = Url::parse(raw)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SearchAdapterError::Configuration(
            "search endpoint requires HTTP(S), a host, and no credentials/query/fragment".into(),
        ));
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if url.scheme() != "https" && !loopback {
        return Err(SearchAdapterError::Configuration(
            "non-loopback search endpoints require HTTPS".into(),
        ));
    }
    let path_valid = match kind {
        SearchKind::Searxng => url.path().ends_with("/search"),
        SearchKind::SerpApi => {
            url.path().ends_with("/search") || url.path().ends_with("/search.json")
        }
    };
    if !path_valid {
        return Err(SearchAdapterError::Configuration(
            "search endpoint path does not match its configured adapter".into(),
        ));
    }
    Ok(raw.to_owned())
}

async fn resolve_search_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, SearchAdapterError> {
    let host_ip = host.parse::<IpAddr>().ok();
    let loopback_name = host.eq_ignore_ascii_case("localhost");
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|_| SearchAdapterError::PreDispatch("DNS resolution failed".into()))?
        .filter(|address| {
            if loopback_name {
                address.ip().is_loopback()
            } else {
                host_ip.is_some() || !non_public_ip(address.ip())
            }
        })
        .collect::<Vec<_>>();
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(MAX_SEARCH_ADDRESSES);
    if addresses.is_empty() {
        return Err(SearchAdapterError::PreDispatch(
            "search endpoint resolved to no permitted address".into(),
        ));
    }
    Ok(addresses)
}

fn non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn redact_exact_secret(bytes: &[u8], secret: Option<&str>) -> Vec<u8> {
    let Some(secret) = secret
        .map(str::as_bytes)
        .filter(|secret| !secret.is_empty())
    else {
        return bytes.to_vec();
    };
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

fn take_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_credential_reference(reference: &str) -> bool {
    let Some(variable) = reference.strip_prefix("env:") else {
        return false;
    };
    let mut bytes = variable.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        ..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
                )
        })
}

/// Default result count applied by model and CLI surfaces when omitted.
pub const fn default_search_limit() -> usize {
    DEFAULT_RESULTS
}

#[cfg(test)]
mod tests;
