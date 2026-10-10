use crate::repository::*;
use colossus_contracts::{
    Actor, AgentCommunicationUpdate, AgentMessage, AgentMessagePage, AgentMessageReceipt,
    AgentMessageSender, AgentParticipant, ExecutionContext, MAX_AGENT_INBOX_BYTES,
    MAX_AGENT_INBOX_MESSAGES, MAX_AGENT_MESSAGE_BYTES, NewEvent, SendAgentMessage,
};
use colossus_ports::{EventJournal, RunControl, SessionRepository, StoreError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

const MAX_SCOPE_MESSAGES: u64 = 4096;
const MAX_SCOPE_PENDING_BYTES: usize = 1024 * 1024;
const COMMIT_ATTEMPTS: usize = 8;

/// Typed communication failures with no private content in diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum CommunicationError {
    /// Requested data is outside the authenticated collaboration.
    #[error("agent communication is not authorized")]
    Forbidden,
    /// Invalid or oversized input.
    #[error("invalid agent message")]
    Invalid,
    /// Exact receiving attempt is closed.
    #[error("agent recipient is closed")]
    Closed,
    /// Bounded queue or scope admission is exhausted.
    #[error("agent inbox admission limit reached")]
    Full,
    /// Same sender-scoped key was used for different content or routing.
    #[error("agent message idempotency conflict")]
    IdempotencyConflict,
    /// Canonical storage failure retains uncertainty and concurrency classification.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Shared application owner of durable participant, inbox and receipt operations.
pub struct CommunicationService {
    pub(crate) journal: Arc<dyn EventJournal>,
    pub(crate) sessions: Arc<dyn SessionRepository>,
    pub(crate) changed: watch::Sender<u64>,
    pub(crate) controls: Mutex<HashMap<String, RunControl>>,
}

impl CommunicationService {
    /// Compose with the runtime's existing journal and transactional session port.
    pub fn new(journal: Arc<dyn EventJournal>, sessions: Arc<dyn SessionRepository>) -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            journal,
            sessions,
            changed,
            controls: Mutex::new(HashMap::new()),
        }
    }

    /// Subscribe to retained wake generations. Durable reads always establish availability.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Inspect all bounded attempts belonging to a caller-owned root execution.
    pub fn list_participants(
        &self,
        owner: &Actor,
        root_run_id: &str,
    ) -> Result<Vec<AgentParticipant>, CommunicationError> {
        let (scope, _) = self.scope(root_run_id)?;
        if &scope.owner != owner {
            return Err(CommunicationError::Forbidden);
        }
        scope
            .participants
            .iter()
            .map(|id| {
                self.participant(id)
                    .map(|(state, _)| state.participant)
                    .map_err(Into::into)
            })
            .collect()
    }

    /// Discover a model's exact collaboration from trusted execution context.
    pub fn list_for_run(
        &self,
        context: &ExecutionContext,
    ) -> Result<Vec<AgentParticipant>, CommunicationError> {
        let (state, _) = self
            .participant_for_run(
                context
                    .run_id
                    .as_deref()
                    .ok_or(CommunicationError::Forbidden)?,
            )?
            .ok_or(CommunicationError::Forbidden)?;
        if context.session_id.as_deref() != Some(&state.participant.session_id) {
            return Err(CommunicationError::Forbidden);
        }
        let mut participants =
            self.list_participants(&state.participant.owner, &state.participant.root_run_id)?;
        if let Some(parent) = &state.participant.parent_id {
            participants.retain(|participant| {
                participant.id == state.participant.id || &participant.id == parent
            });
        }
        Ok(participants)
    }

    /// Send as a registered execution; request parameters cannot nominate the sender.
    pub fn send_from_run(
        &self,
        context: &ExecutionContext,
        request: SendAgentMessage,
    ) -> Result<AgentMessage, CommunicationError> {
        let (state, _) = self
            .participant_for_run(
                context
                    .run_id
                    .as_deref()
                    .ok_or(CommunicationError::Forbidden)?,
            )?
            .ok_or(CommunicationError::Forbidden)?;
        if context.session_id.as_deref() != Some(&state.participant.session_id) {
            return Err(CommunicationError::Forbidden);
        }
        self.send(
            &state.participant.owner,
            AgentMessageSender::Participant {
                participant_id: state.participant.id,
            },
            request,
            &|_| Ok(Vec::new()),
        )
    }

    /// Send as the authenticated application owning the exact recipient.
    pub fn send_from_application(
        &self,
        owner: &Actor,
        request: SendAgentMessage,
    ) -> Result<AgentMessage, CommunicationError> {
        if owner.actor_type != colossus_contracts::ActorType::Application {
            return Err(CommunicationError::Forbidden);
        }
        self.send(
            owner,
            AgentMessageSender::Application {
                application_id: owner.id.clone(),
            },
            request,
            &|_| Ok(Vec::new()),
        )
    }

    /// Stage application-owned input identity/history in the same admission transaction.
    /// This trusted callback is never constructed from client-supplied journal events.
    pub fn send_from_application_with_events(
        &self,
        owner: &Actor,
        request: SendAgentMessage,
        additional: &dyn Fn(&AgentMessage) -> Result<Vec<NewEvent>, StoreError>,
    ) -> Result<AgentMessage, CommunicationError> {
        if owner.actor_type != colossus_contracts::ActorType::Application {
            return Err(CommunicationError::Forbidden);
        }
        self.send(
            owner,
            AgentMessageSender::Application {
                application_id: owner.id.clone(),
            },
            request,
            additional,
        )
    }

    fn send(
        &self,
        owner: &Actor,
        sender: AgentMessageSender,
        request: SendAgentMessage,
        additional: &dyn Fn(&AgentMessage) -> Result<Vec<NewEvent>, StoreError>,
    ) -> Result<AgentMessage, CommunicationError> {
        if request.text.is_empty()
            || request.text.len() > MAX_AGENT_MESSAGE_BYTES
            || !valid_token(&request.recipient_id)
            || !valid_token(&request.idempotency_key)
            || request.reply_to.as_ref().is_some_and(|id| !valid_token(id))
        {
            return Err(CommunicationError::Invalid);
        }
        let sender_bytes =
            serde_json::to_vec(&(owner, &sender)).map_err(|_| CommunicationError::Invalid)?;
        let claim_stream = format!(
            "agent-message-key:{}",
            hash(
                &serde_json::to_vec(&(sender_bytes, &request.idempotency_key))
                    .map_err(|_| CommunicationError::Invalid)?
            )
        );
        let fingerprint =
            hash(&serde_json::to_vec(&request).map_err(|_| CommunicationError::Invalid)?);
        for _ in 0..COMMIT_ATTEMPTS {
            let (mut recipient, recipient_version) = self.participant(&request.recipient_id)?;
            if &recipient.participant.owner != owner {
                return Err(CommunicationError::Forbidden);
            }
            let source = if let AgentMessageSender::Participant { participant_id } = &sender {
                let (source, _) = self.participant(participant_id)?;
                if source.participant.owner != *owner
                    || source.participant.root_run_id != recipient.participant.root_run_id
                    || !(recipient.participant.parent_id.as_ref() == Some(participant_id)
                        || source.participant.parent_id.as_ref() == Some(&recipient.participant.id))
                {
                    return Err(CommunicationError::Forbidden);
                }
                Some(source)
            } else {
                None
            };
            if let Some((claim, _)) = self.load::<MessageClaim>(&claim_stream)? {
                if claim.fingerprint != fingerprint {
                    return Err(CommunicationError::IdempotencyConflict);
                }
                return self.get_message(owner, &claim.message_id);
            }
            if !recipient.participant.open
                || source
                    .as_ref()
                    .is_some_and(|source| !source.participant.open)
            {
                return Err(CommunicationError::Closed);
            }
            let (mut scope, scope_version) = self.scope(&recipient.participant.root_run_id)?;
            if recipient.pending.len() >= MAX_AGENT_INBOX_MESSAGES
                || recipient
                    .participant
                    .pending_bytes
                    .saturating_add(request.text.len())
                    > MAX_AGENT_INBOX_BYTES
                || scope.accepted >= MAX_SCOPE_MESSAGES
                || scope.pending_bytes.saturating_add(request.text.len()) > MAX_SCOPE_PENDING_BYTES
            {
                return Err(CommunicationError::Full);
            }
            if let Some(reply) = &request.reply_to {
                let referenced = self.get_message(owner, reply)?;
                if referenced.root_run_id != recipient.participant.root_run_id {
                    return Err(CommunicationError::Forbidden);
                }
            }
            let message = AgentMessage {
                id: uuid::Uuid::now_v7().to_string(),
                root_run_id: recipient.participant.root_run_id.clone(),
                sender: sender.clone(),
                recipient_id: recipient.participant.id.clone(),
                sequence: recipient.accepted + 1,
                text: request.text.clone(),
                reply_to: request.reply_to.clone(),
                accepted_at: timestamp()?,
                receipt: AgentMessageReceipt::Accepted,
            };
            recipient.pending.push(PendingInput {
                id: message.id.clone(),
                bytes: message.text.len(),
            });
            recipient.accepted += 1;
            recipient.participant.pending_messages = recipient.pending.len();
            recipient.participant.pending_bytes += message.text.len();
            scope.accepted += 1;
            scope.pending_bytes += message.text.len();
            let context = ExecutionContext {
                correlation_id: message.root_run_id.clone(),
                session_id: Some(recipient.participant.session_id.clone()),
                run_id: recipient.participant.run_id.clone(),
                ..ExecutionContext::default()
            };
            let mut events = vec![
                event(
                    participant_stream(&message.recipient_id),
                    recipient_version,
                    "inbox",
                    owner,
                    &context,
                    &recipient,
                )?,
                event(
                    root_stream(&message.root_run_id),
                    scope_version,
                    "scope",
                    owner,
                    &context,
                    &scope,
                )?,
                event(
                    message_stream(&message.id),
                    0,
                    "accepted",
                    owner,
                    &context,
                    &message,
                )?,
                event(
                    inbox_stream(&message.recipient_id),
                    message.sequence - 1,
                    "indexed",
                    owner,
                    &context,
                    &message.id,
                )?,
                event(
                    claim_stream.clone(),
                    0,
                    "idempotency",
                    owner,
                    &context,
                    &MessageClaim {
                        fingerprint: fingerprint.clone(),
                        message_id: message.id.clone(),
                    },
                )?,
                event(
                    feed_stream(&message.root_run_id),
                    self.version(&feed_stream(&message.root_run_id))?,
                    "update",
                    owner,
                    &context,
                    &message,
                )?,
            ];
            events.extend(additional(&message)?);
            match self.journal.append_batch(events) {
                Ok(_) => {
                    self.wake();
                    return Ok(message);
                }
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(CommunicationError::Full)
    }

    /// Inspect one admitted message, including terminal receipt, under canonical ownership.
    pub fn get_message(&self, owner: &Actor, id: &str) -> Result<AgentMessage, CommunicationError> {
        if !valid_token(id) {
            return Err(CommunicationError::Invalid);
        }
        let (message, _) = self
            .load::<AgentMessage>(&message_stream(id))?
            .ok_or_else(|| StoreError::NotFound("agent message".into()))?;
        if self.scope(&message.root_run_id)?.0.owner != *owner {
            return Err(CommunicationError::Forbidden);
        }
        Ok(message)
    }

    /// Inspect a bounded page, including closed inboxes, in committed recipient order.
    pub fn list_messages(
        &self,
        owner: &Actor,
        participant_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<AgentMessagePage, CommunicationError> {
        if !valid_token(participant_id) {
            return Err(CommunicationError::Invalid);
        }
        let (state, _) = self.participant(participant_id)?;
        if state.participant.owner != *owner {
            return Err(CommunicationError::Forbidden);
        }
        if after > state.accepted {
            return Err(CommunicationError::Invalid);
        }
        let events = self.journal.read_stream_from(
            &inbox_stream(participant_id),
            after,
            limit.clamp(1, 16),
        )?;
        let mut messages = Vec::new();
        let mut next_sequence = after;
        for envelope in events {
            let id: String = serde_json::from_value(self.journal.decrypt_payload(&envelope)?)
                .map_err(|_| StoreError::Verification("invalid inbox index".into()))?;
            messages.push(self.get_message(owner, &id)?);
            next_sequence = envelope.stream_version;
        }
        Ok(AgentMessagePage {
            messages,
            next_sequence,
            has_more: next_sequence < state.accepted,
        })
    }

    /// Read the independent durable communication feed after an exclusive scope cursor.
    pub fn updates(
        &self,
        owner: &Actor,
        root_run_id: &str,
        after: u64,
    ) -> Result<Vec<AgentCommunicationUpdate>, CommunicationError> {
        if self.scope(root_run_id)?.0.owner != *owner {
            return Err(CommunicationError::Forbidden);
        }
        let stream = feed_stream(root_run_id);
        if after > self.version(&stream)? {
            return Err(CommunicationError::Invalid);
        }
        self.journal
            .read_stream_from(&stream, after, 16)?
            .into_iter()
            .map(|envelope| {
                let message = serde_json::from_value(self.journal.decrypt_payload(&envelope)?)
                    .map_err(|_| StoreError::Verification("invalid communication feed".into()))?;
                Ok(AgentCommunicationUpdate {
                    sequence: envelope.stream_version,
                    message,
                })
            })
            .collect()
    }
}

pub(crate) fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_: .".contains(&byte))
        && !value.contains(' ')
}
