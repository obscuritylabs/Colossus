use super::*;

/// Permit-bound HTTP adapter with exact-origin authorization, pinned DNS, no redirects,
/// and bounded response streaming.
#[derive(Default)]
pub struct HttpExecutor {
    tls_roots: AdditionalRootCertificates,
}

impl HttpExecutor {
    /// Construct the brokered HTTP adapter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add validated runtime-wide CA roots to HTTP clients' built-in public roots.
    #[must_use]
    pub fn with_tls_roots(mut self, tls_roots: AdditionalRootCertificates) -> Self {
        self.tls_roots = tls_roots;
        self
    }
}

#[async_trait]
impl EffectExecutor for HttpExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let worm_write = request.action == "audit.export.worm.write";
        if request.action != "network.http" && !worm_write {
            return Err(adapter_failure("HTTP executor received another action"));
        }
        let url = Url::parse(&request.resource).map_err(adapter_failure)?;
        if worm_write && url.scheme() != "https" {
            return Err(adapter_failure("WORM audit export requires HTTPS"));
        }
        if worm_write
            && (!url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some())
        {
            return Err(adapter_failure(
                "WORM audit export URL must not contain credentials, a query, or a fragment",
            ));
        }
        let matched = http_transport_authority_match(permit.obligations(), url.as_str())
            .map_err(adapter_failure)?
            .ok_or_else(|| adapter_failure("HTTP origin is not permitted"))?;
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("HTTP URL has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| adapter_failure("HTTP URL has no port"))?;
        let allow_non_public = matched == NetworkDestinationMatch::Ambient
            || (matched == NetworkDestinationMatch::Exact
                && (host.eq_ignore_ascii_case("localhost")
                    || host.parse::<IpAddr>().is_ok_and(non_public_network_address)));
        let addresses = resolve_destinations(host, port, allow_non_public).await?;
        let client = self
            .tls_roots
            .configure_reqwest(Client::builder())
            .redirect(RedirectPolicy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_millis(permit.obligations().timeout_ms))
            .build()
            .map_err(adapter_failure)?;
        let method = if worm_write {
            if request.content.get("method").and_then(Value::as_str) != Some("PUT")
                || request.content.get("create_only").and_then(Value::as_bool) != Some(true)
            {
                return Err(adapter_failure(
                    "WORM audit export requires an explicit create-only PUT",
                ));
            }
            reqwest::Method::PUT
        } else {
            request
                .content
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .parse()
                .map_err(adapter_failure)?
        };
        let mut builder = client.request(method, url.clone());
        if worm_write {
            if request.credential_references.len() > 1 {
                return Err(adapter_failure(
                    "WORM audit export accepts at most one credential reference",
                ));
            }
            if let Some(reference) = request.credential_references.first() {
                let variable = reference.reference.strip_prefix("env:").ok_or_else(|| {
                    adapter_failure("WORM audit credential must be environment-backed")
                })?;
                if permit.obligations().resource_authority != ResourceAuthority::Ambient
                    && !permit
                        .obligations()
                        .allowed_environment
                        .iter()
                        .any(|allowed| allowed == variable)
                {
                    return Err(adapter_failure(
                        "WORM audit credential is absent from permit obligations",
                    ));
                }
                let secret = std::env::var(variable).map_err(|_| {
                    adapter_failure(format!("environment variable {variable} is unset"))
                })?;
                if secret.is_empty() {
                    return Err(adapter_failure("resolved WORM audit credential is empty"));
                }
                builder = builder.bearer_auth(secret);
            }
            let encoded = request
                .content
                .get("body_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| adapter_failure("WORM audit export requires a body"))?;
            let body = BASE64.decode(encoded).map_err(adapter_failure)?;
            if u64::try_from(body.len()).map_err(adapter_failure)?
                > permit.obligations().max_output_bytes
            {
                return Err(adapter_failure(
                    "HTTP request body exceeds the permitted bound",
                ));
            }
            let content_hash = request
                .content
                .get("content_sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| adapter_failure("WORM audit export requires a content hash"))?;
            if content_hash.len() != 64
                || !content_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                || content_hash != sha256_hex(&body)
            {
                return Err(adapter_failure(
                    "WORM audit export content hash does not match the body",
                ));
            }
            let expected_suffix = format!("-{content_hash}.json");
            if !url.path().ends_with(&expected_suffix) {
                return Err(adapter_failure(
                    "WORM audit export object key is not bound to the content hash",
                ));
            }
            builder = builder
                .header("content-type", "application/json")
                .header("if-none-match", "*")
                .header("x-content-sha256", content_hash)
                .body(body);
        } else if let Some(headers) = request.content.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                let normalized = name.to_ascii_lowercase();
                if !matches!(
                    normalized.as_str(),
                    "accept" | "content-type" | "user-agent"
                ) {
                    return Err(adapter_failure(format!(
                        "HTTP header {name} is not in the safe adapter allowlist"
                    )));
                }
                let value = value
                    .as_str()
                    .ok_or_else(|| adapter_failure("HTTP header values must be strings"))?;
                builder = builder.header(name, value);
            }
        }
        if !worm_write
            && let Some(encoded) = request.content.get("body_base64").and_then(Value::as_str)
        {
            let body = BASE64.decode(encoded).map_err(adapter_failure)?;
            if u64::try_from(body.len()).map_err(adapter_failure)?
                > permit.obligations().max_output_bytes
            {
                return Err(adapter_failure(
                    "HTTP request body exceeds the permitted bound",
                ));
            }
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(|error| {
            if worm_write {
                ExecutionError::OutcomeUnknown(format!(
                    "WORM audit delivery transport failed: {error}"
                ))
            } else {
                adapter_failure(error)
            }
        })?;
        if worm_write
            && (response.status().is_success()
                || response.status() == reqwest::StatusCode::PRECONDITION_FAILED)
        {
            return Ok(QuarantinedEffectResult {
                media_type: "application/json".into(),
                bytes: Vec::new(),
                effect_succeeded: true,
            });
        }
        if !response.status().is_success() {
            return Err(adapter_failure(format!(
                "HTTP destination returned {}",
                response.status()
            )));
        }
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let limit =
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?;
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(adapter_failure)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(adapter_failure("HTTP response exceeds the permitted bound"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(QuarantinedEffectResult {
            media_type,
            bytes,
            effect_succeeded: true,
        })
    }
}

pub(super) async fn resolve_destinations(
    host: &str,
    port: u16,
    allow_non_public: bool,
) -> Result<Vec<SocketAddr>, ExecutionError> {
    colossus_network::resolve_destinations(host, port, allow_non_public)
        .await
        .map_err(adapter_failure)
}

pub(super) async fn connect_destination(
    host: &str,
    port: u16,
    pinned: Option<&[SocketAddr]>,
    allow_non_public: bool,
) -> Result<TcpStream, ExecutionError> {
    let mut attempts = FuturesUnordered::new();
    let addresses = if let Some(pinned) = pinned {
        pinned.to_vec()
    } else {
        resolve_destinations(host, port, allow_non_public).await?
    };
    for address in addresses {
        attempts.push(TcpStream::connect(address));
    }
    while let Some(result) = attempts.next().await {
        if let Ok(stream) = result {
            return Ok(stream);
        }
    }
    Err(adapter_failure(
        "network destination did not accept a connection on any permitted address",
    ))
}

pub(super) struct AllowlistProxy {
    pub(super) address: SocketAddr,
    pub(super) shutdown: Option<oneshot::Sender<()>>,
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) observed_origins: Arc<Mutex<BTreeSet<String>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OciProxyBootstrap {
    pub(super) schema_version: u16,
    pub(super) request_hash: String,
    pub(super) decision_id: String,
    pub(super) permit_nonce: String,
    pub(super) expires_at_unix_ms: i128,
    pub(super) allowed_origins: Vec<String>,
    pub(super) resolved_origins: BTreeMap<String, Vec<SocketAddr>>,
    pub(super) max_connections: usize,
    pub(super) connection_timeout_ms: u64,
}

/// Run the trusted OCI proxy sidecar from its bounded environment bootstrap.
pub async fn run_oci_proxy_from_environment() -> Result<(), ExecutionError> {
    let encoded = std::env::var(OCI_PROXY_CONFIG_VARIABLE).map_err(adapter_failure)?;
    let bytes = BASE64.decode(encoded).map_err(adapter_failure)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(adapter_failure(
            "OCI proxy bootstrap exceeds its input bound",
        ));
    }
    let bootstrap: OciProxyBootstrap = serde_json::from_slice(&bytes).map_err(adapter_failure)?;
    let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    if bootstrap.schema_version != 1
        || bootstrap.request_hash.is_empty()
        || bootstrap.decision_id.is_empty()
        || bootstrap.permit_nonce.is_empty()
        || bootstrap.expires_at_unix_ms < now_ms
        || bootstrap.allowed_origins.is_empty()
        || bootstrap.resolved_origins.len()
            != bootstrap
                .allowed_origins
                .iter()
                .filter(|origin| origin.as_str() != "*")
                .count()
        || bootstrap.max_connections == 0
        || bootstrap.max_connections > 256
        || bootstrap.connection_timeout_ms == 0
    {
        return Err(adapter_failure("invalid OCI proxy bootstrap"));
    }
    for origin in &bootstrap.allowed_origins {
        if origin == "*" {
            continue;
        }
        let url = Url::parse(origin).map_err(adapter_failure)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.origin().ascii_serialization() != *origin
        {
            return Err(adapter_failure(format!(
                "OCI proxy origin is not canonical: {origin}"
            )));
        }
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("OCI proxy origin has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| adapter_failure("OCI proxy origin has no port"))?;
        let host_ip = host.parse::<IpAddr>().ok();
        let allow_non_public = host.eq_ignore_ascii_case("localhost")
            || host_ip.is_some_and(non_public_network_address);
        let addresses = bootstrap
            .resolved_origins
            .get(origin)
            .ok_or_else(|| adapter_failure("OCI proxy origin has no pinned addresses"))?;
        if addresses.is_empty()
            || addresses.len() > 16
            || addresses.iter().any(|address| {
                address.port() != port
                    || host_ip.is_some_and(|host_ip| address.ip() != host_ip)
                    || (!allow_non_public && non_public_ip(address.ip()))
            })
        {
            return Err(adapter_failure(format!(
                "OCI proxy origin has invalid pinned addresses: {origin}"
            )));
        }
    }
    let listener = TcpListener::bind(("0.0.0.0", OCI_PROXY_PORT))
        .await
        .map_err(adapter_failure)?;
    let allowed = Arc::new(bootstrap.allowed_origins);
    let resolved = Arc::new(bootstrap.resolved_origins);
    let concurrency = Arc::new(Semaphore::new(bootstrap.max_connections));
    let connection_timeout = Duration::from_millis(bootstrap.connection_timeout_ms);
    println!("colossus-oci-proxy-ready");
    loop {
        let (stream, _) = listener.accept().await.map_err(adapter_failure)?;
        let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        if now_ms >= bootstrap.expires_at_unix_ms {
            drop(stream);
            return Err(adapter_failure("OCI proxy permit expired"));
        }
        let Ok(permit) = Arc::clone(&concurrency).try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let allowed = Arc::clone(&allowed);
        let resolved = Arc::clone(&resolved);
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(
                connection_timeout,
                proxy_connection(
                    stream,
                    allowed.as_slice(),
                    resolved.as_ref(),
                    None,
                    None,
                    true,
                ),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("colossus-oci-proxy-connection-failed: {error}"),
                Err(_) => eprintln!("colossus-oci-proxy-connection-timed-out"),
            }
        });
    }
}

impl AllowlistProxy {
    #[cfg(test)]
    pub(super) async fn start(origins: Vec<String>) -> Result<Self, ExecutionError> {
        Self::start_with_authorization(origins, None).await
    }

    pub(super) async fn start_authenticated(
        origins: Vec<String>,
        credential: &str,
    ) -> Result<Self, ExecutionError> {
        let authorization = format!("Basic {}", BASE64.encode(format!("colossus:{credential}")));
        Self::start_with_authorization(origins, Some(authorization)).await
    }

    async fn start_with_authorization(
        origins: Vec<String>,
        authorization: Option<String>,
    ) -> Result<Self, ExecutionError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(adapter_failure)?;
        let address = listener.local_addr().map_err(adapter_failure)?;
        let allowed = Arc::new(origins);
        let resolved = Arc::new(BTreeMap::new());
        let authorization = Arc::new(authorization);
        let observed_origins = Arc::new(Mutex::new(BTreeSet::new()));
        let task_observed_origins = Arc::clone(&observed_origins);
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let allowed = Arc::clone(&allowed);
                        let resolved = Arc::clone(&resolved);
                        let authorization = Arc::clone(&authorization);
                        let observed_origins = Arc::clone(&task_observed_origins);
                        tokio::spawn(async move {
                            let _ = proxy_connection(
                                stream,
                                allowed.as_slice(),
                                resolved.as_ref(),
                                authorization.as_deref(),
                                Some(observed_origins.as_ref()),
                                false,
                            )
                            .await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            task,
            observed_origins,
        })
    }

    pub(super) fn port(&self) -> u16 {
        self.address.port()
    }

    pub(super) fn observed_origins(&self) -> Vec<String> {
        self.observed_origins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .take(MAX_OBSERVED_ORIGINS)
            .cloned()
            .collect()
    }
}

impl Drop for AllowlistProxy {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

pub(super) async fn proxy_connection(
    mut client: TcpStream,
    allowed_origins: &[String],
    resolved_origins: &BTreeMap<String, Vec<SocketAddr>>,
    required_authorization: Option<&str>,
    observed_origins: Option<&Mutex<BTreeSet<String>>>,
    log_observed_origin: bool,
) -> Result<(), ExecutionError> {
    let mut header = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !header.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = client.read(&mut buffer).await.map_err(adapter_failure)?;
        if count == 0 || header.len().saturating_add(count) > MAX_PROXY_HEADER_BYTES {
            return Err(adapter_failure(
                "proxy request header is absent or oversized",
            ));
        }
        header.extend_from_slice(&buffer[..count]);
    }
    let header_end = header
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position.saturating_add(4))
        .ok_or_else(|| adapter_failure("proxy request header terminator is absent"))?;
    let text = std::str::from_utf8(&header[..header_end]).map_err(adapter_failure)?;
    let first_line = text
        .lines()
        .next()
        .ok_or_else(|| adapter_failure("proxy request line is absent"))?;
    if let Some(required_authorization) = required_authorization
        && single_header_value(text, "proxy-authorization")? != Some(required_authorization)
    {
        client
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"colossus\"\r\nConnection: close\r\n\r\n",
            )
            .await
            .map_err(adapter_failure)?;
        return Ok(());
    }
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = authority(target, 443)?;
        let origin = canonical_origin("https", &host, port)?;
        let Some(matched) =
            network_destination_match(allowed_origins, &origin).map_err(adapter_failure)?
        else {
            client
                .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
                .await
                .map_err(adapter_failure)?;
            return Ok(());
        };
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(adapter_failure)?;
        let client_hello = read_tls_client_hello(&mut client, &header[header_end..]).await?;
        let server_name = tls_server_name(&client_hello)?;
        if host.parse::<IpAddr>().is_err()
            && !server_name.is_some_and(|server_name| server_name.eq_ignore_ascii_case(&host))
        {
            return Err(adapter_failure(
                "TLS server name does not match the permitted CONNECT authority",
            ));
        }
        let mut upstream = connect_destination(
            &host,
            port,
            resolved_origins.get(&origin).map(Vec::as_slice),
            matched == NetworkDestinationMatch::Exact
                && (host.eq_ignore_ascii_case("localhost")
                    || host.parse::<IpAddr>().is_ok_and(non_public_network_address)),
        )
        .await?;
        record_observed_origin(&origin, observed_origins, log_observed_origin);
        upstream
            .write_all(&client_hello)
            .await
            .map_err(adapter_failure)?;
        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .map_err(adapter_failure)?;
        return Ok(());
    }
    let url = Url::parse(target).map_err(adapter_failure)?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(adapter_failure(
            "plain proxy requests require an absolute credential-free HTTP URL",
        ));
    }
    let origin = url.origin().ascii_serialization();
    let Some(matched) =
        network_destination_match(allowed_origins, &origin).map_err(adapter_failure)?
    else {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await
            .map_err(adapter_failure)?;
        return Ok(());
    };
    let host = url
        .host_str()
        .ok_or_else(|| adapter_failure("proxy URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| adapter_failure("proxy URL has no port"))?;
    let host_header = single_header_value(text, "host")?
        .ok_or_else(|| adapter_failure("proxy request has no Host header"))?;
    let (header_host, header_port) = authority(host_header, port)?;
    if canonical_origin("http", &header_host, header_port)? != origin {
        return Err(adapter_failure(
            "HTTP Host header does not match the permitted request origin",
        ));
    }
    let mut upstream = connect_destination(
        host,
        port,
        resolved_origins.get(&origin).map(Vec::as_slice),
        matched == NetworkDestinationMatch::Exact
            && (host.eq_ignore_ascii_case("localhost")
                || host.parse::<IpAddr>().is_ok_and(non_public_network_address)),
    )
    .await?;
    record_observed_origin(&origin, observed_origins, log_observed_origin);
    let path = if let Some(query) = url.query() {
        format!("{}?{query}", url.path())
    } else {
        url.path().to_owned()
    };
    let rewritten = text
        .lines()
        .filter(|line| {
            !line
                .to_ascii_lowercase()
                .starts_with("proxy-authorization:")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        .replacen(first_line, &format!("{method} {path} HTTP/1.1"), 1);
    upstream
        .write_all(format!("{rewritten}\r\n").as_bytes())
        .await
        .map_err(adapter_failure)?;
    upstream
        .write_all(&header[header_end..])
        .await
        .map_err(adapter_failure)?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(adapter_failure)?;
    Ok(())
}

fn record_observed_origin(
    origin: &str,
    observed_origins: Option<&Mutex<BTreeSet<String>>>,
    log_observed_origin: bool,
) {
    if let Some(observed_origins) = observed_origins {
        let mut observed = observed_origins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if observed.len() < MAX_OBSERVED_ORIGINS {
            observed.insert(origin.to_owned());
        }
    }
    if log_observed_origin {
        eprintln!("{OBSERVED_ORIGIN_PREFIX}{origin}");
    }
}

pub(super) fn non_public_ip(ip: IpAddr) -> bool {
    non_public_network_address(ip)
}

pub(super) fn single_header_value<'a>(
    header: &'a str,
    expected_name: &str,
) -> Result<Option<&'a str>, ExecutionError> {
    let mut value = None;
    for line in header.lines().skip(1) {
        let Some((name, candidate)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(expected_name) {
            if value.is_some() {
                return Err(adapter_failure(format!(
                    "proxy request contains multiple {expected_name} headers"
                )));
            }
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return Err(adapter_failure(format!(
                    "proxy request contains an empty {expected_name} header"
                )));
            }
            value = Some(candidate);
        }
    }
    Ok(value)
}

pub(super) async fn read_tls_client_hello(
    client: &mut TcpStream,
    initial: &[u8],
) -> Result<Vec<u8>, ExecutionError> {
    let mut captured = initial.to_vec();
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    loop {
        read_proxy_bytes(client, &mut captured, offset.saturating_add(5)).await?;
        if captured[offset] != 22 {
            return Err(adapter_failure(
                "CONNECT tunnel did not begin with a TLS handshake record",
            ));
        }
        let record_len = usize::from(u16::from_be_bytes([
            captured[offset + 3],
            captured[offset + 4],
        ]));
        if record_len == 0 || record_len > MAX_TLS_RECORD_BYTES {
            return Err(adapter_failure("TLS handshake record is oversized"));
        }
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        read_proxy_bytes(client, &mut captured, record_end).await?;
        handshake.extend_from_slice(&captured[offset + 5..record_end]);
        if handshake.len() > MAX_TLS_CLIENT_HELLO_BYTES {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        if handshake.len() >= 4 {
            if handshake[0] != 1 {
                return Err(adapter_failure(
                    "CONNECT tunnel did not begin with a TLS ClientHello",
                ));
            }
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if hello_len > MAX_TLS_CLIENT_HELLO_BYTES.saturating_sub(4) {
                return Err(adapter_failure("TLS ClientHello is oversized"));
            }
            if handshake.len() >= hello_len.saturating_add(4) {
                return Ok(captured);
            }
        }
        offset = record_end;
    }
}

pub(super) async fn read_proxy_bytes(
    client: &mut TcpStream,
    captured: &mut Vec<u8>,
    required: usize,
) -> Result<(), ExecutionError> {
    while captured.len() < required {
        if required > MAX_TLS_CLIENT_HELLO_BYTES.saturating_add(MAX_TLS_RECORD_BYTES) {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        let mut buffer = [0_u8; 4096];
        let count = client.read(&mut buffer).await.map_err(adapter_failure)?;
        if count == 0 {
            return Err(adapter_failure("TLS ClientHello ended unexpectedly"));
        }
        captured.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

pub(super) fn tls_server_name(
    client_hello_records: &[u8],
) -> Result<Option<String>, ExecutionError> {
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    while offset.saturating_add(5) <= client_hello_records.len() {
        if client_hello_records[offset] != 22 {
            break;
        }
        let record_len = usize::from(u16::from_be_bytes([
            client_hello_records[offset + 3],
            client_hello_records[offset + 4],
        ]));
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        if record_end > client_hello_records.len() {
            return Err(adapter_failure("TLS ClientHello record is truncated"));
        }
        handshake.extend_from_slice(&client_hello_records[offset + 5..record_end]);
        if handshake.len() >= 4 {
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if handshake.len() >= hello_len.saturating_add(4) {
                break;
            }
        }
        offset = record_end;
    }
    let hello_len = tls_u24(&handshake, 1)?;
    if handshake.first() != Some(&1) || handshake.len() < hello_len.saturating_add(4) {
        return Err(adapter_failure("TLS ClientHello is invalid"));
    }
    let body = &handshake[4..4 + hello_len];
    let mut cursor = 34;
    cursor = skip_tls_vector(body, cursor, 1)?;
    cursor = skip_tls_vector(body, cursor, 2)?;
    cursor = skip_tls_vector(body, cursor, 1)?;
    if cursor == body.len() {
        return Ok(None);
    }
    let extensions_len = tls_u16(body, cursor)?;
    cursor = cursor.saturating_add(2);
    let extensions_end = cursor.saturating_add(extensions_len);
    if extensions_end != body.len() {
        return Err(adapter_failure("TLS ClientHello extensions are invalid"));
    }
    while cursor < extensions_end {
        let extension_type = tls_u16(body, cursor)?;
        let extension_len = tls_u16(body, cursor.saturating_add(2))?;
        cursor = cursor.saturating_add(4);
        let extension_end = cursor.saturating_add(extension_len);
        if extension_end > extensions_end {
            return Err(adapter_failure("TLS ClientHello extension is truncated"));
        }
        if extension_type == 0 {
            let names_len = tls_u16(body, cursor)?;
            let mut name_cursor = cursor.saturating_add(2);
            if name_cursor.saturating_add(names_len) != extension_end {
                return Err(adapter_failure("TLS server-name extension is invalid"));
            }
            while name_cursor < extension_end {
                let name_type = *body
                    .get(name_cursor)
                    .ok_or_else(|| adapter_failure("TLS server name is truncated"))?;
                let name_len = tls_u16(body, name_cursor.saturating_add(1))?;
                name_cursor = name_cursor.saturating_add(3);
                let name_end = name_cursor.saturating_add(name_len);
                if name_end > extension_end {
                    return Err(adapter_failure("TLS server name is truncated"));
                }
                if name_type == 0 {
                    let name = std::str::from_utf8(&body[name_cursor..name_end])
                        .map_err(adapter_failure)?;
                    if name.is_empty() || !name.is_ascii() {
                        return Err(adapter_failure("TLS server name is invalid"));
                    }
                    return Ok(Some(name.to_owned()));
                }
                name_cursor = name_end;
            }
            return Ok(None);
        }
        cursor = extension_end;
    }
    Ok(None)
}

pub(super) fn tls_u16(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let high = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let low = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(high) << 8) | usize::from(low))
}

pub(super) fn tls_u24(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let first = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let second = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let third = *bytes
        .get(offset.saturating_add(2))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(first) << 16) | (usize::from(second) << 8) | usize::from(third))
}

pub(super) fn skip_tls_vector(
    bytes: &[u8],
    offset: usize,
    length_bytes: usize,
) -> Result<usize, ExecutionError> {
    let length = match length_bytes {
        1 => usize::from(
            *bytes
                .get(offset)
                .ok_or_else(|| adapter_failure("TLS vector is truncated"))?,
        ),
        2 => tls_u16(bytes, offset)?,
        _ => return Err(adapter_failure("TLS vector length is unsupported")),
    };
    let end = offset.saturating_add(length_bytes).saturating_add(length);
    if end > bytes.len() {
        return Err(adapter_failure("TLS vector is truncated"));
    }
    Ok(end)
}

pub(super) fn authority(value: &str, default_port: u16) -> Result<(String, u16), ExecutionError> {
    let url = Url::parse(&format!("https://{value}")).map_err(adapter_failure)?;
    let host = url
        .host_str()
        .ok_or_else(|| adapter_failure("proxy authority has no host"))?;
    Ok((host.into(), url.port().unwrap_or(default_port)))
}

pub(super) fn canonical_origin(
    scheme: &str,
    host: &str,
    port: u16,
) -> Result<String, ExecutionError> {
    Url::parse(&format!("{scheme}://{host}:{port}"))
        .map(|url| url.origin().ascii_serialization())
        .map_err(adapter_failure)
}
