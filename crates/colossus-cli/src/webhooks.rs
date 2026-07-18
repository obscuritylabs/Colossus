use super::*;

pub(super) struct WebhookHttpDelivery {
    pub(super) webhook_id: String,
    pub(super) delivery_id: String,
    pub(super) timestamp: String,
    pub(super) signature: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

pub(super) enum WebhookIngressBackend<'a> {
    Runtime(&'a Runtime),
    Worker(&'a WorkerClient),
}

impl WebhookIngressBackend<'_> {
    async fn ingest(&self, delivery: WebhookHttpDelivery) -> Result<Value, Box<dyn Error>> {
        match self {
            Self::Runtime(runtime) => Ok(serde_json::to_value(
                runtime
                    .ingest_workflow_webhook(
                        &delivery.webhook_id,
                        &delivery.delivery_id,
                        &delivery.timestamp,
                        &delivery.signature,
                        delivery.headers,
                        &delivery.body,
                    )
                    .await?,
            )?),
            Self::Worker(client) => Ok(client
                .call(WorkerOperation::WorkflowWebhookIngest {
                    webhook_id: delivery.webhook_id,
                    delivery_id: delivery.delivery_id,
                    timestamp: delivery.timestamp,
                    signature: delivery.signature,
                    headers: delivery.headers,
                    body_source: String::from_utf8(delivery.body)
                        .map_err(|_| "webhook JSON body must be UTF-8")?,
                })
                .await?),
        }
    }
}

pub(super) async fn serve_workflow_webhooks(
    bind: SocketAddr,
    backend: WebhookIngressBackend<'_>,
) -> Result<(), Box<dyn Error>> {
    if !bind.ip().is_loopback() {
        return Err("workflow webhook listener must bind to a loopback address".into());
    }
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "workflow webhook listener ready on http://{}/v1/workflow-webhooks/WEBHOOK_ID",
        listener.local_addr()?
    );
    loop {
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(());
            }
        };
        let response = match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            read_webhook_http_delivery(&mut stream),
        )
        .await
        {
            Ok(Ok(delivery)) => match backend.ingest(delivery).await {
                Ok(value) => webhook_http_response(202, "Accepted", &value),
                Err(error) => {
                    eprintln!("workflow webhook delivery rejected: {error}");
                    webhook_http_response(
                        400,
                        "Bad Request",
                        &json!({"accepted": false, "error": "delivery rejected"}),
                    )
                }
            },
            Ok(Err(error)) => webhook_http_response(
                400,
                "Bad Request",
                &json!({"accepted": false, "error": error.to_string()}),
            ),
            Err(_) => webhook_http_response(
                408,
                "Request Timeout",
                &json!({"accepted": false, "error": "request timed out"}),
            ),
        };
        let _ = stream.write_all(&response).await;
        let _ = stream.shutdown().await;
    }
}

pub(super) async fn read_webhook_http_delivery(
    stream: &mut TcpStream,
) -> Result<WebhookHttpDelivery, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() >= MAX_WEBHOOK_HTTP_HEADER_BYTES {
            return Err("webhook HTTP headers exceed 65536 bytes".into());
        }
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err("webhook HTTP request ended before its headers".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    if header_end > MAX_WEBHOOK_HTTP_HEADER_BYTES {
        return Err("webhook HTTP headers exceed 65536 bytes".into());
    }
    let (_, content_length) = parse_webhook_http_head(&bytes[..header_end])?;
    if content_length == 0 || content_length > MAX_WEBHOOK_HTTP_BODY_BYTES {
        return Err("webhook HTTP body must contain 1..=1048576 bytes".into());
    }
    let expected = header_end
        .checked_add(content_length)
        .ok_or("webhook HTTP request size overflow")?;
    while bytes.len() < expected {
        let mut buffer = [0_u8; 8192];
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err("webhook HTTP body ended before Content-Length bytes".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > expected {
            return Err("webhook HTTP request contains bytes after its declared body".into());
        }
    }
    parse_webhook_http_request(&bytes)
}

pub(super) fn parse_webhook_http_head(
    bytes: &[u8],
) -> Result<(BTreeMap<String, String>, usize), Box<dyn Error>> {
    let text = std::str::from_utf8(bytes).map_err(|_| "webhook HTTP headers must be UTF-8")?;
    let mut lines = text.strip_suffix("\r\n\r\n").unwrap_or(text).split("\r\n");
    let request_line = lines.next().ok_or("webhook HTTP request line is absent")?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "POST" || parts[2] != "HTTP/1.1" {
        return Err("webhook listener requires POST over HTTP/1.1".into());
    }
    if !parts[1].starts_with("/v1/workflow-webhooks/") || parts[1].contains(['?', '#']) {
        return Err("webhook HTTP path must be /v1/workflow-webhooks/WEBHOOK_ID".into());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or("webhook HTTP header is malformed")?;
        let name = name.to_ascii_lowercase();
        if name.is_empty()
            || headers
                .insert(name.clone(), value.trim().to_owned())
                .is_some()
        {
            return Err(format!("webhook HTTP header is empty or duplicated: {name}").into());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err("chunked webhook HTTP requests are not accepted".into());
    }
    let content_length = headers
        .get("content-length")
        .ok_or("webhook HTTP Content-Length is required")?
        .parse::<usize>()
        .map_err(|_| "webhook HTTP Content-Length is invalid")?;
    Ok((headers, content_length))
}

pub(super) fn parse_webhook_http_request(
    bytes: &[u8],
) -> Result<WebhookHttpDelivery, Box<dyn Error>> {
    let header_end = find_bytes(bytes, b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or("webhook HTTP header delimiter is absent")?;
    let (mut headers, content_length) = parse_webhook_http_head(&bytes[..header_end])?;
    if bytes.len() != header_end + content_length {
        return Err("webhook HTTP body does not match Content-Length".into());
    }
    let request_line = std::str::from_utf8(&bytes[..header_end])?
        .split("\r\n")
        .next()
        .ok_or("webhook HTTP request line is absent")?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or("webhook HTTP path is absent")?;
    let webhook_id = path
        .strip_prefix("/v1/workflow-webhooks/")
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or("webhook HTTP identifier is invalid")?
        .to_owned();
    let delivery_id = headers
        .remove("x-colossus-delivery-id")
        .ok_or("x-colossus-delivery-id is required")?;
    let timestamp = headers
        .remove("x-colossus-timestamp")
        .ok_or("x-colossus-timestamp is required")?;
    let signature = headers
        .remove("x-colossus-signature")
        .ok_or("x-colossus-signature is required")?;
    for transport in ["connection", "content-length", "host"] {
        headers.remove(transport);
    }
    Ok(WebhookHttpDelivery {
        webhook_id,
        delivery_id,
        timestamp,
        signature,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(super) fn webhook_http_response(status: u16, reason: &str, value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{\"accepted\":false}".to_vec());
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    response
}
