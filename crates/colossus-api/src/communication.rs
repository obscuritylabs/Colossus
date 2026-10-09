//! Optional caller-bound agent communication, independent of run display updates.
use crate::{ApiError, ApiErrorReason, ApiResult, CallerContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

pub use colossus_contracts::{
    AgentCommunicationUpdate, AgentMessage, AgentMessageFailure, AgentMessagePage,
    AgentMessageReceipt, AgentMessageSender, AgentParticipant, MAX_AGENT_MESSAGE_BYTES,
    SendAgentMessage,
};

/// Authenticated read capability for message and inbox inspection.
pub const AGENT_COMMUNICATION_READ_CAPABILITY: &str = "agent_messages.read.v1";
/// Authenticated send capability, independently granted from read access.
pub const AGENT_COMMUNICATION_SEND_CAPABILITY: &str = "agent_messages.send.v1";

/// Inspect the attempts associated with one caller-owned root execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAgentParticipantsRequest {
    /// Public root run identifier; knowledge of it never grants access.
    pub root_run_id: String,
}

/// Inspect bounded messages for one exact attempt, including terminal inboxes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAgentMessagesRequest {
    /// Exact opaque address.
    pub participant_id: String,
    /// Exclusive committed recipient sequence, zero initially.
    pub after_sequence: u64,
    /// From one through sixteen messages per page.
    pub limit: u32,
}

/// Inspect one caller-owned message and its durable receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAgentMessageRequest {
    /// Canonical server-assigned identity.
    pub message_id: String,
}

/// Replay and tail a collaboration's independent released communication feed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchAgentMessagesRequest {
    /// Exact caller-owned public root run.
    pub root_run_id: String,
    /// Exclusive durable scope feed cursor, zero initially.
    pub after_sequence: u64,
}

/// Ordered released communication updates; drop closes only the subscription.
pub type AgentCommunicationStream =
    Pin<Box<dyn futures::Stream<Item = ApiResult<AgentCommunicationUpdate>> + Send>>;

/// Optional application service used by every host and the A2A edge.
#[async_trait]
pub trait AgentCommunicationApi: Send + Sync {
    /// Submit globally idempotent peer text through a fixed execution profile.
    async fn submit_task_message(
        &self,
        _caller: &CallerContext,
        _request: crate::SubmitAgentTaskMessageRequest,
    ) -> ApiResult<crate::AgentTaskSnapshot> {
        Err(agent_communication_unavailable())
    }
    /// Inspect a caller-owned task and bounded released input history.
    async fn get_task(
        &self,
        _caller: &CallerContext,
        _request: crate::GetAgentTaskRequest,
    ) -> ApiResult<crate::AgentTaskSnapshot> {
        Err(agent_communication_unavailable())
    }
    /// Query a canonical snapshot by last lifecycle update, independent of ListRuns ordering.
    async fn list_tasks(
        &self,
        _caller: &CallerContext,
        _request: crate::ListAgentTasksRequest,
    ) -> ApiResult<crate::ListAgentTasksResponse> {
        Err(agent_communication_unavailable())
    }

    /// Discover participants through immutable caller ownership.
    async fn list_participants(
        &self,
        caller: &CallerContext,
        request: ListAgentParticipantsRequest,
    ) -> ApiResult<Vec<AgentParticipant>>;
    /// Admit already-disclosed text idempotently as the authenticated application.
    async fn send_message(
        &self,
        caller: &CallerContext,
        request: SendAgentMessage,
    ) -> ApiResult<AgentMessage>;
    /// Inspect one message and its receipt.
    async fn get_message(
        &self,
        caller: &CallerContext,
        request: GetAgentMessageRequest,
    ) -> ApiResult<AgentMessage>;
    /// Inspect a bounded inbox page.
    async fn list_messages(
        &self,
        caller: &CallerContext,
        request: ListAgentMessagesRequest,
    ) -> ApiResult<AgentMessagePage>;
    /// Replay and tail durable accepted/input/undelivered updates with independent budgets.
    async fn watch_messages(
        &self,
        caller: &CallerContext,
        request: WatchAgentMessagesRequest,
    ) -> ApiResult<AgentCommunicationStream>;
}

/// Safe optional-capability error for hosts connected to older runtimes.
pub fn agent_communication_unavailable() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "agent communication is unavailable",
    )
}
