use crate::{repository::*, service::CommunicationService};
use colossus_contracts::{
    Actor, AgentMessage, AgentMessageFailure, AgentMessageReceipt, ExecutionContext, NewEvent,
    SessionMessageAppend,
};
use colossus_ports::{AgentInbox, AgentInboxBatch, RunControl, StoreError, agent_input_messages};

impl CommunicationService {
    pub(crate) fn close_events(
        &self,
        id: &str,
        reason: AgentMessageFailure,
    ) -> Result<Vec<NewEvent>, StoreError> {
        let (mut state, version) = self.participant(id)?;
        if !state.participant.open {
            return Ok(Vec::new());
        }
        let participant = state.participant.clone();
        let (mut scope, scope_version) = self.scope(&participant.root_run_id)?;
        let context = ExecutionContext {
            correlation_id: participant.root_run_id.clone(),
            run_id: participant.run_id.clone(),
            session_id: Some(participant.session_id.clone()),
            subagent_id: participant.subagent_id.clone(),
            ..ExecutionContext::default()
        };
        let mut events = Vec::new();
        let feed_start = self.version(&feed_stream(&scope.root_run_id))?;
        for (feed_version, pending) in (feed_start..).zip(&state.pending) {
            let (mut message, message_version) = self
                .load::<AgentMessage>(&message_stream(&pending.id))?
                .ok_or_else(|| StoreError::Verification("missing pending agent message".into()))?;
            message.receipt = AgentMessageReceipt::NotDelivered { reason };
            events.push(event(
                message_stream(&message.id),
                message_version,
                "receipt",
                &scope.owner,
                &context,
                &message,
            )?);
            events.push(event(
                feed_stream(&scope.root_run_id),
                feed_version,
                "update",
                &scope.owner,
                &context,
                &message,
            )?);
        }
        scope.pending_bytes = scope
            .pending_bytes
            .checked_sub(participant.pending_bytes)
            .ok_or_else(|| {
                StoreError::Verification("invalid communication byte accounting".into())
            })?;
        state.pending.clear();
        state.participant.open = false;
        state.participant.closed_reason = Some(reason);
        state.participant.pending_bytes = 0;
        state.participant.pending_messages = 0;
        events.push(event(
            participant_stream(id),
            version,
            "closed",
            &scope.owner,
            &context,
            &state,
        )?);
        events.push(event(
            root_stream(&scope.root_run_id),
            scope_version,
            "scope",
            &scope.owner,
            &context,
            &scope,
        )?);
        Ok(events)
    }
}

impl AgentInbox for CommunicationService {
    fn initial_origin(
        &self,
        context: &ExecutionContext,
    ) -> Result<Option<colossus_contracts::AgentMessageOrigin>, StoreError> {
        Ok(context
            .run_id
            .as_deref()
            .map(|id| self.participant_for_run(id))
            .transpose()?
            .flatten()
            .and_then(|(state, _)| state.initial_origin))
    }
    fn begin_run(
        &self,
        context: &ExecutionContext,
        owner: &Actor,
        control: RunControl,
    ) -> Result<(), StoreError> {
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("communication execution absent".into()))?;
        if let Some(job) = &context.subagent_id {
            if let Some((index, _)) = self.load::<AttemptIndex>(&job_stream(job))? {
                let (mut state, version) = self.participant(&index.participant_id)?;
                if state.participant.owner != *owner
                    || state.participant.session_id
                        != context.session_id.as_deref().unwrap_or_default()
                    || !state.participant.open
                    || state.participant.run_id.is_some()
                {
                    return Err(StoreError::Adapter(
                        "child communication attempt is unavailable".into(),
                    ));
                }
                state.participant.run_id = Some(run_id.into());
                self.journal.append_batch(vec![
                    event(
                        participant_stream(&state.participant.id),
                        version,
                        "bound",
                        owner,
                        context,
                        &state,
                    )?,
                    event(run_stream(run_id), 0, "run", owner, context, &index)?,
                ])?;
            } else {
                // Legacy queued jobs keep their original execution behavior without messaging.
                return Ok(());
            }
        } else {
            self.register_root(context, owner)?;
        }
        let mut controls = self
            .controls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if controls.len() >= 1024 {
            return Err(StoreError::Adapter(
                "communication live execution limit reached".into(),
            ));
        }
        controls.insert(run_id.into(), control);
        self.wake();
        Ok(())
    }

    fn prepare(&self, context: &ExecutionContext) -> Result<Option<AgentInboxBatch>, StoreError> {
        let Some((state, version)) = context
            .run_id
            .as_deref()
            .map(|id| self.participant_for_run(id))
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        if !state.participant.open || state.pending.is_empty() {
            return Ok(None);
        }
        let mut messages = Vec::new();
        let mut bytes = 0;
        for pending in state.pending.iter().take(8) {
            if bytes + pending.bytes > 32 * 1024 {
                break;
            }
            let (message, _) = self
                .load::<AgentMessage>(&message_stream(&pending.id))?
                .ok_or_else(|| StoreError::Verification("missing pending agent message".into()))?;
            if message.receipt != AgentMessageReceipt::Accepted {
                return Err(StoreError::Verification(
                    "invalid pending agent receipt".into(),
                ));
            }
            bytes += pending.bytes;
            messages.push(message);
        }
        Ok(Some(AgentInboxBatch {
            participant_id: state.participant.id,
            version,
            messages,
        }))
    }

    fn include(
        &self,
        batch: &AgentInboxBatch,
        context: &ExecutionContext,
        turn: u16,
        request_hash: &str,
    ) -> Result<(), StoreError> {
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("communication execution absent".into()))?;
        let (mut state, version) = self.participant(&batch.participant_id)?;
        if version != batch.version {
            return Err(StoreError::Conflict {
                stream_id: participant_stream(&batch.participant_id),
                expected: batch.version,
                actual: version,
            });
        }
        if !state.participant.open
            || state.participant.run_id.as_deref() != Some(run_id)
            || context.session_id.as_deref() != Some(&state.participant.session_id)
            || turn == 0
            || request_hash.len() != 64
            || !request_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || batch.messages.is_empty()
            || batch.messages.len() > 8
            || batch.messages.len() > state.pending.len()
            || !state
                .pending
                .iter()
                .zip(&batch.messages)
                .all(|(pending, message)| pending.id == message.id)
        {
            return Err(StoreError::Adapter(
                "invalid communication input batch".into(),
            ));
        }
        let (mut scope, scope_version) = self.scope(&state.participant.root_run_id)?;
        let mut events = Vec::new();
        let feed_start = self.version(&feed_stream(&scope.root_run_id))?;
        let mut bytes = 0;
        for (feed_version, accepted) in (feed_start..).zip(&batch.messages) {
            let (mut message, message_version) = self
                .load::<AgentMessage>(&message_stream(&accepted.id))?
                .ok_or_else(|| StoreError::Verification("missing included agent message".into()))?;
            if message != *accepted || message.receipt != AgentMessageReceipt::Accepted {
                return Err(StoreError::Verification(
                    "agent input changed before inclusion".into(),
                ));
            }
            bytes += message.text.len();
            message.receipt = AgentMessageReceipt::IncludedInTurn {
                run_id: run_id.into(),
                turn,
                request_hash: request_hash.into(),
            };
            events.push(event(
                message_stream(&message.id),
                message_version,
                "receipt",
                &scope.owner,
                context,
                &message,
            )?);
            events.push(event(
                feed_stream(&scope.root_run_id),
                feed_version,
                "update",
                &scope.owner,
                context,
                &message,
            )?);
        }
        state.pending.drain(..batch.messages.len());
        state.participant.pending_messages = state.pending.len();
        state.participant.pending_bytes = state
            .participant
            .pending_bytes
            .checked_sub(bytes)
            .ok_or_else(|| StoreError::Verification("invalid inbox byte accounting".into()))?;
        scope.pending_bytes = scope
            .pending_bytes
            .checked_sub(bytes)
            .ok_or_else(|| StoreError::Verification("invalid scope byte accounting".into()))?;
        events.push(event(
            participant_stream(&state.participant.id),
            version,
            "included",
            &scope.owner,
            context,
            &state,
        )?);
        events.push(event(
            root_stream(&scope.root_run_id),
            scope_version,
            "scope",
            &scope.owner,
            context,
            &scope,
        )?);
        self.sessions.append_messages_with_events(
            &state.participant.session_id,
            run_id,
            agent_input_messages(batch)
                .into_iter()
                .map(|message| SessionMessageAppend {
                    message,
                    actor: scope.owner.clone(),
                })
                .collect(),
            events,
        )?;
        self.wake();
        Ok(())
    }

    fn try_complete(&self, context: &ExecutionContext) -> Result<bool, StoreError> {
        let Some(run_id) = context.run_id.as_deref() else {
            return Ok(true);
        };
        for _ in 0..8 {
            let Some((state, version)) = self.participant_for_run(run_id)? else {
                return Ok(true);
            };
            if !state.participant.open {
                return Ok(true);
            }
            if !state.pending.is_empty() {
                return Ok(false);
            }
            let events =
                self.close_events(&state.participant.id, AgentMessageFailure::Completed)?;
            if events.iter().any(|event| {
                event.stream_id == participant_stream(&state.participant.id)
                    && event.expected_stream_version != version
            }) {
                continue;
            }
            match self.journal.append_batch(events) {
                Ok(_) => {
                    self.wake();
                    return Ok(true);
                }
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }

    fn close_run(&self, run_id: &str, reason: AgentMessageFailure) -> Result<(), StoreError> {
        for _ in 0..8 {
            let Some((state, _)) = self.participant_for_run(run_id)? else {
                return Ok(());
            };
            let events = self.close_events(&state.participant.id, reason)?;
            match self.journal.append_batch(events) {
                Ok(_) => {
                    self.controls
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(run_id);
                    self.wake();
                    return Ok(());
                }
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(StoreError::Adapter(
            "communication closure remained contended".into(),
        ))
    }
}
