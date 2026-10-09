//! Permit-bound process and OCI HTTP proxy transport.

use super::*;

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
        Self::start_with_authorization(origins, None, cfg!(target_os = "macos")).await
    }

    pub(super) async fn start_authenticated(
        origins: Vec<String>,
        credential: &str,
    ) -> Result<Self, ExecutionError> {
        let authorization = format!("Basic {}", BASE64.encode(format!("colossus:{credential}")));
        Self::start_with_authorization(origins, Some(authorization), cfg!(target_os = "macos"))
            .await
    }

    pub(super) async fn start_with_authorization(
        origins: Vec<String>,
        authorization: Option<String>,
        dual_stack: bool,
    ) -> Result<Self, ExecutionError> {
        let (listener, ipv6_listener) = bind_proxy_loopbacks(dual_stack).await?;
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
                    accepted = async {
                        if let Some(ipv6_listener) = &ipv6_listener {
                            tokio::select! {
                                accepted = listener.accept() => accepted,
                                accepted = ipv6_listener.accept() => accepted,
                            }
                        } else {
                            listener.accept().await
                        }
                    } => {
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

async fn bind_proxy_loopbacks(
    dual_stack: bool,
) -> Result<(TcpListener, Option<TcpListener>), ExecutionError> {
    for _ in 0..8 {
        let ipv4 = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(adapter_failure)?;
        if !dual_stack {
            return Ok((ipv4, None));
        }
        let port = ipv4.local_addr().map_err(adapter_failure)?.port();
        // Own both localhost addresses before exposing its URL. Seatbelt already
        // grants this one loopback port; native IPv6 avoids IPv4-mapped socket denials.
        match TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await {
            Ok(ipv6) => return Ok((ipv4, Some(ipv6))),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => return Err(adapter_failure(error)),
        }
    }
    Err(adapter_failure(
        "could not reserve both private proxy loopbacks",
    ))
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
        validate_observed_origin(&origin)?;
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
    validate_observed_origin(&origin)?;
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

pub(super) fn validate_observed_origin(origin: &str) -> Result<(), ExecutionError> {
    if serde_json::to_vec(origin).map_err(adapter_failure)?.len() > MAX_OBSERVED_ORIGIN_JSON_BYTES {
        return Err(adapter_failure(
            "proxy origin exceeds completion evidence bound",
        ));
    }
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
