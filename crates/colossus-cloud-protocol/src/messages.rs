use colossus_sdk::{
    ApiError, CancelRunResponse, CreateRunRequest, GetRunResponse, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdate,
};
use serde::{Deserialize, Serialize};

/// Supported major of the closed JSON and Protobuf connection contract.
pub const PROTOCOL_MAJOR: u32 = 1;
/// Maximum command, receipt, or released update payload.
pub const MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Maximum frames queued in either direction before applying backpressure.
pub const MAX_QUEUED_FRAMES: usize = 32;
/// Maximum concurrently watched tasks on one enrolled runtime.
pub const MAX_ACTIVE_TASKS: usize = 16;

/// Closed runtime operation set. The server assigns all durable idempotency keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Create or reconcile one exact caller-bound request, then watch its run.
    Create {
        /// Complete SDK request beneath the dedicated local application grant.
        request: Box<CreateRunRequest>,
    },
    /// Resume observing the exact run already assigned to this node.
    Watch {
        /// Local runtime run identity.
        run_id: String,
        /// Observe state/control only after the host's released-output retention bound.
        #[serde(default)]
        snapshot_only: bool,
    },
    /// Cooperatively cancel the assigned run, using a stable mutation identity.
    Cancel {
        /// Local runtime run identity.
        run_id: String,
        /// Cloud-assigned, caller-scoped idempotency key.
        idempotency_key: colossus_sdk::IdempotencyKey,
    },
    /// Answer an exact pending interaction beneath explicit response authority.
    Respond {
        /// Runtime-bound interaction, revision, response, and idempotency key.
        request: Box<RespondInteractionRequest>,
    },
}

/// Receipt for a command. Success is sent only after the local durable result exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CloudReply {
    /// Full caller-visible snapshot, including the exact run identity and revision.
    Run {
        /// Released local run state.
        run: Box<GetRunResponse>,
    },
    /// Durable cooperative cancellation result.
    Cancelled {
        /// Released cancellation state.
        response: CancelRunResponse,
    },
    /// Durable one-use interaction response.
    Responded {
        /// Released response receipt.
        response: RespondInteractionResponse,
    },
    /// Sanitized SDK error; unknown outcomes require reconciliation.
    Failed {
        /// Released categorical error without transport/provider diagnostics.
        error: ApiError,
    },
}

/// Reject empty, oversized, malformed, or unknown-operation payloads before dispatch.
pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, PayloadError> {
    if bytes.is_empty() || bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(PayloadError);
    }
    serde_json::from_slice(bytes).map_err(|_| PayloadError)
}

/// Encode one bounded typed payload, never private worker frames or credentials.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, PayloadError> {
    let bytes = serde_json::to_vec(value).map_err(|_| PayloadError)?;
    if bytes.is_empty() || bytes.len() > MAX_PAYLOAD_BYTES {
        return Err(PayloadError);
    }
    Ok(bytes)
}

/// Verify both identities/cursors before accepting a released runtime update.
pub fn decode_update(bytes: &[u8], run_id: &str, sequence: u64) -> Result<RunUpdate, PayloadError> {
    let update: RunUpdate = decode(bytes)?;
    if sequence == 0 || update.run_id != run_id || update.sequence != sequence {
        return Err(PayloadError);
    }
    Ok(update)
}

/// A categorical error that intentionally does not include untrusted input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("cloud payload is invalid or exceeds its bound")]
pub struct PayloadError;
