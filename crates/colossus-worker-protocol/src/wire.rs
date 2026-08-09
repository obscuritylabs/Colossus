use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac as _};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::Sha256;
use time::OffsetDateTime;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Exact authenticated worker protocol version.
///
/// Version 11 changed audit reads from encrypted `EventEnvelope` values to
/// ciphertext-free `AuditEvidence`. A version-10 worker would otherwise violate
/// the export contract for a version-11 client, so both sides must reject the
/// mismatch and require a worker restart.
pub const PROTOCOL_VERSION: u16 = 11;
pub(crate) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_CLOCK_SKEW_MS: i128 = 30_000;
type HmacSha256 = Hmac<Sha256>;

/// Worker-side policy mode used by attached and headless clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerApprovalMode {
    /// Deny approval obligations without prompting.
    Deny,
    /// Ask an attached interactive client.
    Ask,
    /// Preserve model-assisted low-risk auto-approval and ask otherwise.
    RiskAuto,
    /// Satisfy approval obligations without prompting.
    FullAccess,
}

/// Sanitized local worker-control transport or strict-contract failure.
#[derive(Debug, thiserror::Error)]
pub enum WorkerControlError {
    /// The local IPC transport failed.
    #[error("worker control transport failed")]
    Io(#[source] std::io::Error),
    /// A frame violated the strict authenticated protocol.
    #[error("worker control protocol rejected a message: {0}")]
    Protocol(String),
    /// The authenticated worker rejected the operation.
    #[error("worker control operation failed: {0}")]
    Remote(String),
    /// No worker is accepting connections at the endpoint.
    #[error("worker control endpoint is unavailable")]
    Unavailable,
    /// The endpoint did not become available within its bounded wait.
    #[error("worker control endpoint is busy")]
    Busy,
}

impl From<std::io::Error> for WorkerControlError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum ControlOperation {
    Ping,
    SetApprovalMode { approval_mode: WorkerApprovalMode },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientHello {
    pub(crate) version: u16,
    pub(crate) challenge: String,
}

#[derive(Serialize)]
pub(crate) struct UnsignedServerHello<'a> {
    pub(crate) version: u16,
    pub(crate) challenge: &'a str,
    pub(crate) server_nonce: &'a str,
    pub(crate) timestamp_ms: i128,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerHello {
    pub(crate) version: u16,
    pub(crate) challenge: String,
    pub(crate) server_nonce: String,
    pub(crate) timestamp_ms: i128,
    pub(crate) authentication_tag: String,
}

#[derive(Serialize)]
pub(crate) struct UnsignedRequest<'a> {
    pub(crate) version: u16,
    pub(crate) request_id: &'a str,
    pub(crate) timestamp_ms: i128,
    pub(crate) nonce: &'a str,
    pub(crate) connection_nonce: &'a str,
    pub(crate) operation: &'a ControlOperation,
}

#[derive(Serialize)]
pub(crate) struct WorkerRequest {
    pub(crate) version: u16,
    pub(crate) request_id: String,
    pub(crate) timestamp_ms: i128,
    pub(crate) nonce: String,
    pub(crate) connection_nonce: String,
    pub(crate) operation: ControlOperation,
    pub(crate) authentication_tag: String,
}

#[derive(Serialize)]
pub(crate) struct UnsignedFrame<'a> {
    pub(crate) version: u16,
    pub(crate) request_id: &'a str,
    pub(crate) sequence: u64,
    pub(crate) timestamp_ms: i128,
    pub(crate) content_base64: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerFrame {
    pub(crate) version: u16,
    pub(crate) request_id: String,
    pub(crate) sequence: u64,
    pub(crate) timestamp_ms: i128,
    pub(crate) content_base64: String,
    pub(crate) authentication_tag: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ControlFrameContent {
    Complete { result: Value },
    Error { message: String },
}

pub(crate) fn request_tag<T: Serialize>(
    key: &[u8; 32],
    value: &T,
) -> Result<String, WorkerControlError> {
    let bytes = canonical_authentication_bytes(value)?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub(crate) fn verify_tag<T: Serialize>(
    key: &[u8; 32],
    value: &T,
    tag: &str,
    context: &str,
) -> Result<(), WorkerControlError> {
    let bytes = canonical_authentication_bytes(value)?;
    let tag = hex::decode(tag).map_err(|_| {
        WorkerControlError::Protocol("authentication tag is not hexadecimal".into())
    })?;
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    mac.update(&bytes);
    mac.verify_slice(&tag)
        .map_err(|_| WorkerControlError::Protocol(format!("{context} authentication tag mismatch")))
}

fn canonical_authentication_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, WorkerControlError> {
    let value = serde_json::to_value(value)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    let mut bytes = Vec::new();
    write_canonical_json(&value, &mut bytes)?;
    Ok(bytes)
}

fn write_canonical_json(value: &Value, bytes: &mut Vec<u8>) -> Result<(), WorkerControlError> {
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
                    .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
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
            .map_err(|error| WorkerControlError::Protocol(error.to_string()))?,
    }
    Ok(())
}

pub(crate) async fn write_message<S, T>(
    stream: &mut S,
    value: &T,
    limit: usize,
) -> Result<(), WorkerControlError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .map_err(|error| WorkerControlError::Protocol(error.to_string()))?;
    if bytes.len() > limit || bytes.len() > u32::MAX as usize {
        return Err(WorkerControlError::Protocol(
            "IPC message exceeds bound".into(),
        ));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub(crate) async fn read_message<S, T>(
    stream: &mut S,
    limit: usize,
) -> Result<T, WorkerControlError>
where
    S: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > limit {
        return Err(WorkerControlError::Protocol(
            "IPC message length is empty or exceeds bound".into(),
        ));
    }
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|error| WorkerControlError::Protocol(error.to_string()))
}

pub(crate) fn decode_frame_content(
    frame: &WorkerFrame,
) -> Result<ControlFrameContent, WorkerControlError> {
    let content = BASE64.decode(&frame.content_base64).map_err(|_| {
        WorkerControlError::Protocol("worker response payload is not base64".into())
    })?;
    serde_json::from_slice(&content)
        .map_err(|error| WorkerControlError::Protocol(format!("invalid worker response: {error}")))
}

pub(crate) fn now_ms() -> i128 {
    OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_operations_have_the_worker_wire_shape() {
        assert_eq!(
            serde_json::to_value(ControlOperation::Ping).expect("ping"),
            serde_json::json!({"operation": "ping"})
        );
        assert_eq!(
            serde_json::to_value(ControlOperation::SetApprovalMode {
                approval_mode: WorkerApprovalMode::FullAccess,
            })
            .expect("set mode"),
            serde_json::json!({
                "operation": "set_approval_mode",
                "approval_mode": "full_access",
            })
        );
    }
}
