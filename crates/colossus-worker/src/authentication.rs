use super::*;

pub(super) fn signed_request(
    key: &[u8; 32],
    operation: WorkerOperation,
    connection_nonce: &str,
) -> Result<WorkerRequest, WorkerError> {
    let request_id = Uuid::now_v7().to_string();
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let nonce = hex::encode(nonce);
    let timestamp_ms = now_ms();
    let tag = request_tag(
        key,
        &UnsignedRequest {
            version: PROTOCOL_VERSION,
            request_id: &request_id,
            timestamp_ms,
            nonce: &nonce,
            connection_nonce,
            operation: &operation,
        },
    )?;
    Ok(WorkerRequest {
        version: PROTOCOL_VERSION,
        request_id,
        timestamp_ms,
        nonce,
        connection_nonce: connection_nonce.into(),
        operation,
        authentication_tag: tag,
    })
}

pub(super) fn validate_request(
    key: &[u8; 32],
    request: &WorkerRequest,
    replay: &Mutex<ReplayGuard>,
    connection_nonce: &str,
) -> Result<(), WorkerError> {
    if request.version != PROTOCOL_VERSION
        || request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.connection_nonce != connection_nonce
        || (now_ms() - request.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "unsupported version, invalid id, or expired timestamp".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedRequest {
            version: request.version,
            request_id: &request.request_id,
            timestamp_ms: request.timestamp_ms,
            nonce: &request.nonce,
            connection_nonce: &request.connection_nonce,
            operation: &request.operation,
        },
        &request.authentication_tag,
        "worker request",
    )?;
    replay
        .lock()
        .map_err(|error| WorkerError::Protocol(error.to_string()))?
        .accept(&request.nonce)
}

pub(super) fn validate_frame(
    key: &[u8; 32],
    request_id: &str,
    sequence: &mut u64,
    frame: &WorkerFrame,
) -> Result<WorkerFrameContent, WorkerError> {
    let expected_sequence = sequence.saturating_add(1);
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.sequence != expected_sequence
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "response version, request id, sequence, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedFrame {
            version: frame.version,
            request_id: &frame.request_id,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content_base64: &frame.content_base64,
        },
        &frame.authentication_tag,
        "worker response",
    )?;
    let content = BASE64
        .decode(&frame.content_base64)
        .map_err(|_| WorkerError::Protocol("worker response payload is not base64".into()))?;
    let content = serde_json::from_slice(&content)
        .map_err(|error| WorkerError::Protocol(format!("invalid worker response: {error}")))?;
    *sequence = expected_sequence;
    Ok(content)
}

pub(super) fn validate_client_frame(
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: &mut u64,
    frame: &WorkerClientFrame,
) -> Result<ClientFrameContent, WorkerError> {
    let expected_sequence = sequence.saturating_add(1);
    if frame.version != PROTOCOL_VERSION
        || frame.request_id != request_id
        || frame.connection_nonce != connection_nonce
        || frame.sequence != expected_sequence
        || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
    {
        return Err(WorkerError::Protocol(
            "client frame version, request, connection, sequence, or timestamp is invalid".into(),
        ));
    }
    verify_tag(
        key,
        &UnsignedClientFrame {
            version: frame.version,
            request_id: &frame.request_id,
            connection_nonce: &frame.connection_nonce,
            sequence: frame.sequence,
            timestamp_ms: frame.timestamp_ms,
            content_base64: &frame.content_base64,
        },
        &frame.authentication_tag,
        "worker client frame",
    )?;
    let content = BASE64
        .decode(&frame.content_base64)
        .map_err(|_| WorkerError::Protocol("worker client payload is not base64".into()))?;
    let content = serde_json::from_slice(&content)
        .map_err(|error| WorkerError::Protocol(format!("invalid worker client frame: {error}")))?;
    *sequence = expected_sequence;
    Ok(content)
}

pub(super) async fn write_signed_frame<S>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    sequence: u64,
    content: WorkerFrameContent,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
{
    let timestamp_ms = now_ms();
    let content =
        serde_json::to_vec(&content).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let content_base64 = BASE64.encode(content);
    let authentication_tag = request_tag(
        key,
        &UnsignedFrame {
            version: PROTOCOL_VERSION,
            request_id,
            sequence,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )?;
    write_message(
        stream,
        &WorkerFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            sequence,
            timestamp_ms,
            content_base64,
            authentication_tag,
        },
        MAX_FRAME_BYTES,
    )
    .await
}

pub(super) async fn write_signed_client_frame<S>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: u64,
    content: ClientFrameContent,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
{
    let timestamp_ms = now_ms();
    let content =
        serde_json::to_vec(&content).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    if content.len() > MAX_REQUEST_BYTES {
        return Err(WorkerError::Protocol(
            "worker client frame exceeds the 1 MiB limit".into(),
        ));
    }
    let content_base64 = BASE64.encode(content);
    let authentication_tag = request_tag(
        key,
        &UnsignedClientFrame {
            version: PROTOCOL_VERSION,
            request_id,
            connection_nonce,
            sequence,
            timestamp_ms,
            content_base64: &content_base64,
        },
    )?;
    write_message(
        stream,
        &WorkerClientFrame {
            version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            connection_nonce: connection_nonce.into(),
            sequence,
            timestamp_ms,
            content_base64,
            authentication_tag,
        },
        MAX_REQUEST_BYTES,
    )
    .await
}

pub(super) fn request_tag<T: Serialize>(key: &[u8; 32], value: &T) -> Result<String, WorkerError> {
    let bytes = canonical_authentication_bytes(value)?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub(super) fn verify_tag<T: Serialize>(
    key: &[u8; 32],
    value: &T,
    tag: &str,
    context: &str,
) -> Result<(), WorkerError> {
    let bytes = canonical_authentication_bytes(value)?;
    let tag = hex::decode(tag)
        .map_err(|_| WorkerError::Protocol("authentication tag is not hexadecimal".into()))?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    mac.verify_slice(&tag)
        .map_err(|_| WorkerError::Protocol(format!("{context} authentication tag mismatch")))
}

pub(super) fn canonical_authentication_bytes<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, WorkerError> {
    let value =
        serde_json::to_value(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    let mut bytes = Vec::new();
    write_canonical_json(&value, &mut bytes)?;
    Ok(bytes)
}

pub(super) fn write_canonical_json(value: &Value, bytes: &mut Vec<u8>) -> Result<(), WorkerError> {
    match value {
        Value::Object(object) => {
            bytes.push(b'{');
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                serde_json::to_writer(&mut *bytes, key)
                    .map_err(|error| WorkerError::Protocol(error.to_string()))?;
                bytes.push(b':');
                write_canonical_json(value, bytes)?;
            }
            bytes.push(b'}');
        }
        Value::Array(array) => {
            bytes.push(b'[');
            for (index, value) in array.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                write_canonical_json(value, bytes)?;
            }
            bytes.push(b']');
        }
        _ => serde_json::to_writer(bytes, value)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?,
    }
    Ok(())
}

pub(super) async fn write_message<S, T>(
    stream: &mut S,
    value: &T,
    limit: usize,
) -> Result<(), WorkerError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Protocol(error.to_string()))?;
    if bytes.len() > limit || bytes.len() > u32::MAX as usize {
        return Err(WorkerError::Protocol("IPC message exceeds bound".into()));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub(super) async fn read_message<S, T>(stream: &mut S, limit: usize) -> Result<T, WorkerError>
where
    S: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > limit {
        return Err(WorkerError::Protocol(
            "IPC message length is empty or exceeds bound".into(),
        ));
    }
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|error| WorkerError::Protocol(error.to_string()))
}

pub(super) fn now_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

pub(super) fn bounded_error(message: &str) -> String {
    message.chars().take(4_096).collect()
}

pub(super) fn bounded_diagnostic_error(message: &str) -> String {
    message.chars().take(72 * 1024).collect()
}
