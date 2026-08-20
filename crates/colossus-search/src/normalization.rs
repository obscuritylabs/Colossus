use super::*;

pub(super) fn validate_request(request: &SearchRequest) -> Result<(), SearchAdapterError> {
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

pub(super) fn normalize_response(
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

pub(super) fn normalize_result(kind: SearchKind, value: &Value) -> Option<SearchResult> {
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

pub(super) fn search_execution_error(error: SearchAdapterError) -> ExecutionError {
    match error {
        SearchAdapterError::Transport(message) => ExecutionError::OutcomeUnknown(format!(
            "search transport failed after dispatch; provider usage may have occurred: {message}"
        )),
        error => ExecutionError::Failed(error.to_string()),
    }
}

pub(super) fn validate_credential_disclosure(
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

pub(super) fn normalize_endpoint(
    kind: SearchKind,
    raw: &str,
    resource_authority: ResourceAuthority,
) -> Result<String, SearchAdapterError> {
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
    if resource_authority != ResourceAuthority::Ambient && url.scheme() != "https" && !loopback {
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

pub(super) async fn resolve_search_addresses(
    host: &str,
    port: u16,
    allow_non_public: bool,
) -> Result<Vec<SocketAddr>, SearchAdapterError> {
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|_| SearchAdapterError::PreDispatch("DNS resolution failed".into()))?
        .filter(|address| allow_non_public || !non_public_network_address(address.ip()))
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

pub(super) fn url_host_is_non_public_literal(value: &str) -> Result<bool, SearchAdapterError> {
    let url = Url::parse(value)?;
    Ok(url
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(non_public_network_address))
}

pub(super) fn redact_exact_secret(bytes: &[u8], secret: Option<&str>) -> Vec<u8> {
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

pub(super) fn take_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(super) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(super) fn valid_credential_reference(reference: &str) -> bool {
    if let Some(variable) = reference.strip_prefix("env:") {
        let mut bytes = variable.bytes();
        return bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric());
    }
    reference.strip_prefix("host:").is_some_and(valid_name)
}

pub(super) fn valid_header_name(value: &str) -> bool {
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
