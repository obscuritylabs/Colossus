//! Transport-neutral local collaboration records. Peer input never grants authority.
use crate::Actor;
use serde::{Deserialize, Serialize};

/// Maximum UTF-8 bytes in one released agent message.
pub const MAX_AGENT_MESSAGE_BYTES: usize = 16 * 1024;
/// Maximum queued messages for one exact attempt.
pub const MAX_AGENT_INBOX_MESSAGES: usize = 64;
/// Maximum queued content bytes for one exact attempt.
pub const MAX_AGENT_INBOX_BYTES: usize = 256 * 1024;

/// Address of an execution attempt, allocated before delegated work is runnable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentParticipant {
    /// Opaque address; never reused for another attempt.
    pub id: String,
    /// Root execution owning this collaboration.
    pub root_run_id: String,
    /// Authenticated immutable execution owner.
    pub owner: Actor,
    /// Isolated canonical conversation.
    pub session_id: String,
    /// Exact active run, absent while queued.
    pub run_id: Option<String>,
    /// Parent participant, absent for the root execution.
    pub parent_id: Option<String>,
    /// Durable local delegated job, absent for the root execution.
    pub subagent_id: Option<String>,
    /// One-based job attempt generation.
    pub generation: u64,
    /// Whether new messages may be accepted.
    pub open: bool,
    /// Categorical terminal reason, without private diagnostics.
    pub closed_reason: Option<AgentMessageFailure>,
    /// Number of admitted messages still awaiting input inclusion.
    pub pending_messages: usize,
    /// Bytes of admitted content still awaiting input inclusion.
    pub pending_bytes: usize,
    /// UTC admission time.
    pub created_at: String,
}

/// Proven sender. A peer role in a transport is never proof of this identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMessageSender {
    /// A registered local execution.
    Participant {
        /// Exact attempt address derived from execution context.
        participant_id: String,
    },
    /// The authenticated application controlling this execution.
    Application {
        /// Identity derived from verified credentials.
        application_id: String,
    },
}

/// Categorical reasons an accepted message could not be included.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageFailure {
    /// Execution completed and cannot take another turn.
    Completed,
    /// Cooperative cancellation or explicit job stop.
    Cancelled,
    /// Execution failed; private diagnostics are excluded.
    Failed,
    /// Process loss interrupted the original attempt.
    Interrupted,
    /// The execution's existing turn budget was exhausted.
    BudgetExhausted,
    /// A deliberate requeue replaced the attempt.
    Superseded,
}

/// Durable evidence, never a claim that the model understood or acted on text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMessageReceipt {
    /// Admission was committed before acknowledging the sender.
    Accepted,
    /// Input was committed with a structurally settled session transcript.
    IncludedInTurn {
        /// Exact execution consuming this message.
        run_id: String,
        /// One-based provider turn.
        turn: u16,
        /// SHA-256 of the prepared provider-neutral request.
        request_hash: String,
    },
    /// Terminal delivery failure for this exact attempt.
    NotDelivered {
        /// Safe reason for failure.
        reason: AgentMessageFailure,
    },
}

/// Canonical bounded, policy-released local message and its latest receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessage {
    /// Server-assigned opaque identity.
    pub id: String,
    /// Canonical root execution.
    pub root_run_id: String,
    /// Verified sender.
    pub sender: AgentMessageSender,
    /// Exact recipient attempt.
    pub recipient_id: String,
    /// One-based committed order within the recipient inbox.
    pub sequence: u64,
    /// Released UTF-8 text.
    pub text: String,
    /// Optional reference to a message visible in the same collaboration.
    pub reply_to: Option<String>,
    /// UTC acceptance time.
    pub accepted_at: String,
    /// Latest canonical delivery evidence.
    pub receipt: AgentMessageReceipt,
}

/// Trusted canonical origin retained separately from the provider's conversation role.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageOrigin {
    /// Canonical accepted input identity.
    pub message_id: String,
    /// Verified sender address or application identity.
    pub sender: AgentMessageSender,
    /// Exact receiving attempt.
    pub recipient_id: String,
}

/// Message send input. Trusted callers establish sender, owner and workspace separately.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendAgentMessage {
    /// Exact opaque recipient address.
    pub recipient_id: String,
    /// Bounded text passing the ordinary disclosure boundary before admission.
    pub text: String,
    /// Stable sender-scoped operation identity, independent of transport request IDs.
    pub idempotency_key: String,
    /// Optional same-scope reply reference.
    pub reply_to: Option<String>,
}

/// Bounded page of messages in committed recipient order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessagePage {
    /// Messages with current durable receipts.
    pub messages: Vec<AgentMessage>,
    /// Exclusive recipient sequence to use for the next page.
    pub next_sequence: u64,
    /// Whether more admitted messages exist after this page.
    pub has_more: bool,
}

/// Durable scope feed update, suitable for independent application inspectors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCommunicationUpdate {
    /// Exclusive replay cursor within this root execution's feed.
    pub sequence: u64,
    /// Exact record as committed at this cursor.
    pub message: AgentMessage,
}
