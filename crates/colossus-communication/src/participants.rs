use crate::{repository::*, service::CommunicationService};
use colossus_contracts::{
    Actor, AgentMessageFailure, AgentParticipant, ExecutionContext, NewEvent, SubagentJob,
    SubagentStatus,
};
use colossus_ports::{ChildCommunication, StoreError};

const MAX_SCOPE_PARTICIPANTS: usize = 128;

impl CommunicationService {
    pub(crate) fn register_root(
        &self,
        context: &ExecutionContext,
        owner: &Actor,
    ) -> Result<(), StoreError> {
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("communication requires an execution".into()))?;
        if self.participant_for_run(run_id)?.is_none() {
            self.journal
                .append_batch(self.stage_root(context, owner)?)?;
        }
        let (mut state, version) = self
            .participant_for_run(run_id)?
            .ok_or_else(|| StoreError::Verification("root inbox missing".into()))?;
        if state.participant.owner != *owner
            || state.participant.session_id != context.session_id.as_deref().unwrap_or_default()
            || !state.participant.open
            || state.participant.run_id.is_some()
        {
            return Err(StoreError::Adapter(
                "root communication attempt is unavailable".into(),
            ));
        }
        state.participant.run_id = Some(run_id.into());
        self.journal.append(event(
            participant_stream(&state.participant.id),
            version,
            "bound",
            owner,
            context,
            &state,
        )?)?;
        self.wake();
        Ok(())
    }

    /// Stage the root inbox before public execution becomes runnable.
    pub fn stage_root(
        &self,
        context: &ExecutionContext,
        owner: &Actor,
    ) -> Result<Vec<NewEvent>, StoreError> {
        self.stage_root_with_peer_input(context, owner, None)
    }

    /// Retain the verified origin of initial peer input in the allocation transaction.
    pub fn stage_root_with_peer_input(
        &self,
        context: &ExecutionContext,
        owner: &Actor,
        peer_message_id: Option<&str>,
    ) -> Result<Vec<NewEvent>, StoreError> {
        let run_id = context
            .run_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("communication requires an execution".into()))?;
        let session_id = context
            .session_id
            .as_deref()
            .ok_or_else(|| StoreError::Adapter("communication requires a session".into()))?;
        let id = uuid::Uuid::now_v7().to_string();
        let initial_origin =
            peer_message_id.map(|message_id| colossus_contracts::AgentMessageOrigin {
                message_id: message_id.into(),
                sender: colossus_contracts::AgentMessageSender::Application {
                    application_id: owner.id.clone(),
                },
                recipient_id: id.clone(),
            });
        let participant = AgentParticipant {
            id: id.clone(),
            root_run_id: run_id.into(),
            owner: owner.clone(),
            session_id: session_id.into(),
            run_id: None,
            parent_id: None,
            subagent_id: None,
            generation: 1,
            open: true,
            closed_reason: None,
            pending_messages: 0,
            pending_bytes: 0,
            created_at: timestamp()?,
        };
        let scope = ScopeState {
            root_run_id: run_id.into(),
            owner: owner.clone(),
            participants: vec![id.clone()],
            accepted: 0,
            pending_bytes: 0,
        };
        Ok(vec![
            event(root_stream(run_id), 0, "scope", owner, context, &scope)?,
            event(
                participant_stream(&id),
                0,
                "participant",
                owner,
                context,
                &ParticipantState {
                    participant,
                    pending: Vec::new(),
                    accepted: 0,
                    initial_origin,
                },
            )?,
            event(
                run_stream(run_id),
                0,
                "run",
                owner,
                context,
                &AttemptIndex {
                    participant_id: id,
                    generation: 1,
                },
            )?,
        ])
    }

    /// Recover started attempts as interrupted, retaining queued inputs for unstarted jobs.
    /// Recovery never adopts a process or replays provider/effect work.
    pub fn recover(&self) -> Result<usize, StoreError> {
        let mut after = None;
        let mut recovered = 0;
        loop {
            let page = self
                .journal
                .list_stream_ids("agent-participant:", after.as_deref(), 128)?;
            if page.is_empty() {
                break;
            }
            for stream in &page {
                let (state, _) = self.load::<ParticipantState>(stream)?.ok_or_else(|| {
                    StoreError::Verification("missing communication participant".into())
                })?;
                if state.participant.open
                    && let Some(run_id) = &state.participant.run_id
                {
                    colossus_ports::AgentInbox::close_run(
                        self,
                        run_id,
                        AgentMessageFailure::Interrupted,
                    )?;
                    recovered += 1;
                    if recovered > 1024 {
                        return Err(StoreError::Adapter(
                            "communication recovery bound exceeded".into(),
                        ));
                    }
                }
            }
            after = page.last().cloned();
        }
        Ok(recovered)
    }
}

impl ChildCommunication for CommunicationService {
    fn register_child(&self, job: &SubagentJob) -> Result<Vec<NewEvent>, StoreError> {
        let Some((parent, _)) = self.participant_for_run(&job.parent_run_id)? else {
            // Legacy/private jobs do not gain an address without verified parent lineage.
            return Ok(Vec::new());
        };
        let parent = parent.participant;
        let previous = self.load::<AttemptIndex>(&job_stream(&job.id))?;
        if (!parent.open && previous.is_none())
            || parent.subagent_id.is_some()
            || parent.session_id != job.session_id
            || job.status != SubagentStatus::Queued
        {
            return Err(StoreError::Adapter(
                "invalid child communication lineage".into(),
            ));
        }
        let (mut scope, scope_version) = self.scope(&parent.root_run_id)?;
        if scope.participants.len() >= MAX_SCOPE_PARTICIPANTS {
            return Err(StoreError::Adapter(
                "communication participant limit reached".into(),
            ));
        }
        if let Some((index, _)) = &previous
            && self.participant(&index.participant_id)?.0.participant.open
        {
            return Err(StoreError::Adapter(
                "previous child inbox must be closed before requeue".into(),
            ));
        }
        let generation = previous
            .as_ref()
            .map_or(1, |(index, _)| index.generation + 1);
        let job_version = previous.map_or(0, |(_, version)| version);
        let id = uuid::Uuid::now_v7().to_string();
        let participant = AgentParticipant {
            id: id.clone(),
            root_run_id: parent.root_run_id.clone(),
            owner: parent.owner.clone(),
            session_id: job.child_session_id.clone(),
            run_id: None,
            parent_id: Some(parent.id.clone()),
            subagent_id: Some(job.id.clone()),
            generation,
            open: true,
            closed_reason: None,
            pending_messages: 0,
            pending_bytes: 0,
            created_at: timestamp()?,
        };
        scope.participants.push(id.clone());
        let context = ExecutionContext {
            correlation_id: scope.root_run_id.clone(),
            session_id: Some(job.session_id.clone()),
            run_id: Some(job.parent_run_id.clone()),
            subagent_id: Some(job.id.clone()),
            ..ExecutionContext::default()
        };
        Ok(vec![
            event(
                root_stream(&scope.root_run_id),
                scope_version,
                "scope",
                &parent.owner,
                &context,
                &scope,
            )?,
            event(
                participant_stream(&id),
                0,
                "participant",
                &parent.owner,
                &context,
                &ParticipantState {
                    participant,
                    pending: Vec::new(),
                    accepted: 0,
                    initial_origin: Some(colossus_contracts::AgentMessageOrigin {
                        message_id: format!("delegation:{}:{generation}", job.id),
                        sender: colossus_contracts::AgentMessageSender::Participant {
                            participant_id: parent.id,
                        },
                        recipient_id: id.clone(),
                    }),
                },
            )?,
            event(
                job_stream(&job.id),
                job_version,
                "attempt",
                &parent.owner,
                &context,
                &AttemptIndex {
                    participant_id: id,
                    generation,
                },
            )?,
        ])
    }

    fn close_child(
        &self,
        job: &SubagentJob,
        reason: AgentMessageFailure,
    ) -> Result<Vec<NewEvent>, StoreError> {
        let Some((index, _)) = self.load::<AttemptIndex>(&job_stream(&job.id))? else {
            return Ok(Vec::new());
        };
        self.close_events(&index.participant_id, reason)
    }

    fn child_committed(&self, job: &SubagentJob) {
        self.wake();
        if matches!(
            job.status,
            SubagentStatus::Cancelled | SubagentStatus::Interrupted
        ) && let Ok(Some((index, _))) = self.load::<AttemptIndex>(&job_stream(&job.id))
            && let Ok((state, _)) = self.participant(&index.participant_id)
            && let Some(run_id) = state.participant.run_id
            && let Some(control) = self
                .controls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&run_id)
        {
            control.cancel();
        }
    }
}
