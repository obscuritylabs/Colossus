use super::*;

pub(super) fn resolve_environment(reference: &str) -> Result<String, ExecutionError> {
    let name = reference
        .strip_prefix("env:")
        .filter(|_| valid_environment_reference(reference))
        .ok_or_else(|| execution("invalid integration credential reference"))?;
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 64 * 1024 && !value.contains('\0'))
        .ok_or_else(|| execution(format!("integration credential {reference} is unavailable")))
}

pub(super) fn validate_request_credentials(
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

pub(super) fn auth_header(
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

pub(super) struct PreparedHttpRequest {
    pub(super) method: reqwest::Method,
    pub(super) url: Url,
    pub(super) body: Option<Value>,
}

pub(super) fn prepare_native_request(
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

pub(super) fn github_request(
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

pub(super) fn searxng_request(
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

pub(super) fn opensearch_request(
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

pub(super) fn normalize_native_response(
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

pub(super) fn native_url(
    connection: &IntegrationConnection,
    path: &str,
) -> Result<Url, ExecutionError> {
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

pub(super) fn append_pairs(url: &mut Url, pairs: Vec<(&str, String)>) {
    let mut query = url.query_pairs_mut();
    for (name, value) in pairs {
        query.append_pair(name, &value);
    }
}

pub(super) fn required_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, ExecutionError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| execution(format!("integration argument {name} is required")))
}

pub(super) fn optional_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

pub(super) fn native_segment(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<String, ExecutionError> {
    Ok(encode_path_segment(required_string(arguments, name)?))
}

pub(super) fn opensearch_index(arguments: &Map<String, Value>) -> Result<String, ExecutionError> {
    let value = required_string(arguments, "index")?;
    if value.contains(['/', '\\']) || matches!(value, "." | "..") {
        return Err(execution(
            "OpenSearch index contains an unsafe path segment",
        ));
    }
    Ok(encode_path_segment_with(value, b",*"))
}

pub(super) fn encode_path_segment_with(value: &str, additionally_safe: &[u8]) -> String {
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

pub(super) fn bounded_integer(
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

pub(super) fn add_refresh(
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

pub(super) fn operation_url(
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
    let mut base = connection.base_url.clone();
    if !base.ends_with('/') {
        base.push('/');
    }
    Url::parse(&base)
        .map_err(execution)?
        .join(path.trim_start_matches('/'))
        .map_err(execution)
}

pub(super) fn encode_path_segment(value: &str) -> String {
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

pub(super) fn add_query(
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

pub(super) fn scalar(value: &Value) -> Result<String, ExecutionError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(execution("integration path/query values must be scalar")),
    }
}

pub(super) fn canonical_origin(url: &Url) -> Result<String, ExecutionError> {
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

pub(super) fn require_origin(
    url: &Url,
    permit: &ExecutionPermit,
) -> Result<NetworkDestinationMatch, ExecutionError> {
    network_destination_match(&permit.obligations().network_destinations, url.as_str())
        .map_err(execution)?
        .ok_or_else(|| {
            execution(format!(
                "integration origin {} is not permitted",
                canonical_origin(url).unwrap_or_else(|_| "<invalid>".into())
            ))
        })
}

pub(super) async fn resolve_integration_addresses(
    host: &str,
    port: u16,
    allow_non_public: bool,
) -> Result<Vec<SocketAddr>, ExecutionError> {
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(execution)?
        .filter(|address| allow_non_public || !non_public_network_address(address.ip()))
        .collect::<Vec<_>>();
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(16);
    if addresses.is_empty() {
        return Err(execution(
            "integration origin resolved to no permitted address",
        ));
    }
    Ok(addresses)
}

pub(super) async fn bounded_response(
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

pub(super) fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

pub(super) trait ReqwestErrorClass {
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
