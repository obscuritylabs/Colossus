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
/// Maximum runs released by one inventory discovery page.
pub const MAX_DISCOVERY_PAGE_SIZE: u32 = 32;
/// Caller-visible runtime metadata with authority derived by its public SDK.
pub type ReleasedRunInventory = colossus_sdk::VisibleRun;

/// Native placement advertised by an enrolled runtime, without machine paths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentKind {
    /// An independently managed installed runtime.
    #[default]
    Cli,
    /// A workspace runtime supervised by Colossus Desktop.
    Desktop,
}

/// Explicit local disclosure posture; enrollment does not imply session sharing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSharing {
    /// Only sessions owned by the independent cloud application are visible.
    #[default]
    CloudOwned,
    /// Local ownership has explicitly released workspace sessions to the connector.
    SharedVisibleSessions,
}

/// Non-secret native inventory. Grouping fields never change application authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInventory {
    /// Runtime-released caller-scoped configuration posture; absent means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<colossus_sdk::RuntimePolicyPosture>,
    /// Installation identity persisted in the owner-private native Colossus home.
    pub host_id: String,
    /// Operator-visible host label, never an automatically discovered hostname.
    pub host_label: String,
    /// Bounded native platform identifier such as linux, windows, or macos.
    pub platform: String,
    /// Independently supervised CLI or Desktop workspace runtime.
    pub deployment_kind: DeploymentKind,
    /// Opaque runtime/workspace identity; never a filesystem path.
    pub workspace_id: String,
    /// Native-sourced workspace presentation label.
    pub workspace_label: String,
    /// Explicit local disclosure posture.
    pub sharing: WorkspaceSharing,
}

impl RuntimeInventory {
    /// Validate grouping labels and opaque identities before persisting inventory.
    pub fn validate(&self) -> Result<(), PayloadError> {
        let identity = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        };
        let label = |value: &str| {
            !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        };
        if self.policy.as_ref().is_none_or(|policy| policy.validate())
            && identity(&self.host_id)
            && identity(&self.workspace_id)
            && label(&self.host_label)
            && label(&self.workspace_label)
            && matches!(
                self.platform.as_str(),
                "linux" | "windows" | "macos" | "freebsd" | "unknown"
            )
        {
            Ok(())
        } else {
            Err(PayloadError)
        }
    }
}

/// Closed runtime operation set. The server assigns all durable idempotency keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Inspect exact caller-owned agent attempts through the shared SDK.
    InspectInboxes {
        /// Fixed local run allocated to the cloud task.
        root_run_id: String,
        /// Optional exact recipient; absent lists attempts only.
        participant_id: Option<String>,
        /// Exclusive inbox sequence.
        after_sequence: u64,
    },
    /// Read canonical released conversation history under local sharing authority.
    History {
        /// Runtime-owned source run already mapped by the controller to this node/session.
        source_run_id: String,
        /// Opaque SDK activity cursor; callers cannot select an application owner.
        page_token: Option<String>,
        /// At most 32 canonical activity records.
        page_size: u32,
    },
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
    /// Released message inspection, independently scoped from execution or sending.
    Inboxes {
        /// Exact attempts in the task's caller-owned collaboration.
        participants: Vec<colossus_sdk::AgentParticipant>,
        /// Optional bounded recipient-ordered page.
        page: Option<colossus_sdk::AgentMessagePage>,
    },
    /// Canonical caller-visible user/assistant activity page; no tools or system lanes.
    History {
        /// Released bounded SDK history and its continuation/projection watermark.
        response: colossus_sdk::ListSessionActivityResponse,
    },
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
