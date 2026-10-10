//! Safe-boundary input and delegated participant registration ports.
use crate::{RunControl, StoreError};
use colossus_contracts::{
    Actor, AgentMessage, AgentMessageFailure, ExecutionContext, ModelMessage, NewEvent, SubagentJob,
};

/// A bounded inbox snapshot. Its version guards against completion and concurrent sends.
#[derive(Clone, Debug)]
pub struct AgentInboxBatch {
    /// Exact attempt address.
    pub participant_id: String,
    /// Observed participant stream version.
    pub version: u64,
    /// Ordered accepted inputs, bounded in count and bytes by the implementation.
    pub messages: Vec<AgentMessage>,
}

/// Injectable agent input boundary. The loop has no dependency on a concrete service.
pub trait AgentInbox: Send + Sync {
    /// Trusted origin of an initial peer task, derived from durable registration.
    fn initial_origin(
        &self,
        _context: &ExecutionContext,
    ) -> Result<Option<colossus_contracts::AgentMessageOrigin>, StoreError> {
        Ok(None)
    }
    /// Allocate or bind an attempt before its first provider turn.
    fn begin_run(
        &self,
        context: &ExecutionContext,
        owner: &Actor,
        control: RunControl,
    ) -> Result<(), StoreError>;
    /// Read bounded pending input without consuming it.
    fn prepare(&self, context: &ExecutionContext) -> Result<Option<AgentInboxBatch>, StoreError>;
    /// Atomically commit input, inbox consumption, and prepared-turn evidence.
    fn include(
        &self,
        batch: &AgentInboxBatch,
        context: &ExecutionContext,
        turn: u16,
        request_hash: &str,
    ) -> Result<(), StoreError>;
    /// Close only if admission has not won the completion race. False requests another turn.
    fn try_complete(&self, context: &ExecutionContext) -> Result<bool, StoreError>;
    /// Settle queued input explicitly on termination, without implying effects stopped.
    fn close_run(&self, run_id: &str, reason: AgentMessageFailure) -> Result<(), StoreError>;
}

/// Registration transaction participant owned by the communication application.
pub trait ChildCommunication: Send + Sync {
    /// Stage a fresh attempt and scope linkage for the same atomic transaction as the job.
    fn register_child(&self, job: &SubagentJob) -> Result<Vec<NewEvent>, StoreError>;
    /// Stage terminal inbox receipts for the same transaction as a job transition.
    fn close_child(
        &self,
        job: &SubagentJob,
        reason: AgentMessageFailure,
    ) -> Result<Vec<NewEvent>, StoreError>;
    /// Wake readers and signal cooperative cancellation after the job transaction commits.
    fn child_committed(&self, job: &SubagentJob);
}

/// Project admitted text as peer input, retaining origin and never using a system role.
pub fn agent_input_messages(batch: &AgentInboxBatch) -> Vec<ModelMessage> {
    batch
        .messages
        .iter()
        .map(|message| ModelMessage {
            role: colossus_contracts::ModelMessageRole::User,
            content: format!(
                "[Peer input; message {}; sender {}; recipient {}]\n{}",
                message.id,
                match &message.sender {
                    colossus_contracts::AgentMessageSender::Participant { participant_id } =>
                        participant_id,
                    colossus_contracts::AgentMessageSender::Application { application_id } =>
                        application_id,
                },
                message.recipient_id,
                message.text
            )
            .into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            agent_message_origin: Some(colossus_contracts::AgentMessageOrigin {
                message_id: message.id.clone(),
                sender: message.sender.clone(),
                recipient_id: message.recipient_id.clone(),
            }),
        })
        .collect()
}
