use super::*;

/// Journal-backed canonical session repository.
pub struct EventSourcedSessionRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedSessionRepository {
    /// Bind the repository to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(id: &str) -> String {
        format!("session:{id}")
    }
}

impl SessionRepository for EventSourcedSessionRepository {
    fn create_session(
        &self,
        id: &str,
        title: Option<&str>,
        actor: Actor,
    ) -> Result<SessionSummary, StoreError> {
        validate_session_id(id)?;
        let title = title.map(str::trim).filter(|title| !title.is_empty());
        if title.is_some_and(|title| title.len() > MAX_TITLE_BYTES) {
            return Err(StoreError::Adapter(format!(
                "session title exceeds {MAX_TITLE_BYTES} bytes"
            )));
        }
        let envelope = self.journal.append(NewEvent {
            event_version: 1,
            stream_id: Self::stream(id),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: SESSION_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: id.into(),
                session_id: Some(id.into()),
                ..ExecutionContext::default()
            },
            payload: json!({"title": title}),
        })?;
        Ok(SessionSummary {
            id: id.into(),
            title: title.map(str::to_owned),
            created_at: envelope.occurred_at.clone(),
            updated_at: envelope.occurred_at,
            message_count: 0,
            last_run_id: None,
            last_user_preview: None,
        })
    }

    fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, StoreError> {
        validate_session_id(id)?;
        let events = self.journal.read_stream(&Self::stream(id))?;
        if events.is_empty() {
            return Ok(None);
        }
        reconstruct_summary(self.journal.as_ref(), id, &events).map(Some)
    }

    fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let limit = limit.clamp(1, LIST_LIMIT_MAX);
        let mut sessions = collect_stream_ids(self.journal.as_ref(), "session:")?
            .into_iter()
            .map(|stream_id| {
                stream_id
                    .strip_prefix("session:")
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "indexed stream {stream_id} is not a session stream"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|id| self.get_session(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        sessions.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        sessions.truncate(limit);
        Ok(sessions)
    }

    fn append_messages(
        &self,
        session_id: &str,
        run_id: &str,
        messages: Vec<SessionMessageAppend>,
    ) -> Result<Vec<SessionMessage>, StoreError> {
        validate_session_id(session_id)?;
        if run_id.is_empty() {
            return Err(StoreError::Adapter("message run id is required".into()));
        }
        for message in &messages {
            validate_message(&message.message)?;
        }
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        if pending_tool_turn_from_events(self.journal.as_ref(), &events)?.is_some() {
            return Err(StoreError::Adapter(
                "session has an unsettled provider tool turn".into(),
            ));
        }
        let first_sequence = events
            .iter()
            .filter(|event| event.event_type == MESSAGE_EVENT)
            .count()
            .saturating_add(1);
        let first_sequence = u64::try_from(first_sequence)
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let first_stream_version = events.last().map_or(0, |event| event.stream_version);
        let pending = messages
            .iter()
            .enumerate()
            .map(|(index, append)| {
                let offset =
                    u64::try_from(index).map_err(|error| StoreError::Adapter(error.to_string()))?;
                let sequence = first_sequence.saturating_add(offset);
                Ok(NewEvent {
                    event_version: 1,
                    stream_id: stream_id.clone(),
                    expected_stream_version: first_stream_version.saturating_add(offset),
                    classification: EventClassification::Domain,
                    event_type: MESSAGE_EVENT.into(),
                    actor: append.actor.clone(),
                    context: ExecutionContext {
                        correlation_id: run_id.into(),
                        session_id: Some(session_id.into()),
                        run_id: Some(run_id.into()),
                        ..ExecutionContext::default()
                    },
                    payload: json!({
                        "run_id": run_id,
                        "sequence": sequence,
                        "message": append.message,
                    }),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let envelopes = self.journal.append_batch(pending)?;
        if envelopes.len() != messages.len() {
            return Err(StoreError::Adapter(
                "session batch append returned an unexpected event count".into(),
            ));
        }
        messages
            .into_iter()
            .zip(envelopes)
            .enumerate()
            .map(|(index, (append, envelope))| {
                let offset =
                    u64::try_from(index).map_err(|error| StoreError::Adapter(error.to_string()))?;
                Ok(SessionMessage {
                    session_id: session_id.into(),
                    run_id: run_id.into(),
                    sequence: first_sequence.saturating_add(offset),
                    message: append.message,
                    created_at: envelope.occurred_at,
                })
            })
            .collect()
    }

    fn pending_tool_turn(
        &self,
        session_id: &str,
    ) -> Result<Option<PendingSessionToolTurn>, StoreError> {
        validate_session_id(session_id)?;
        let events = self.journal.read_stream(&Self::stream(session_id))?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        pending_tool_turn_from_events(self.journal.as_ref(), &events)
    }

    fn begin_tool_turn(
        &self,
        session_id: &str,
        pending: PendingSessionToolTurn,
        actor: Actor,
    ) -> Result<(), StoreError> {
        validate_session_id(session_id)?;
        validate_pending_tool_turn(&pending)?;
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        if pending_tool_turn_from_events(self.journal.as_ref(), &events)?.is_some() {
            return Err(StoreError::Adapter(
                "session already has an unsettled provider tool turn".into(),
            ));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: events.last().map_or(0, |event| event.stream_version),
            classification: EventClassification::Domain,
            event_type: TOOL_TURN_PENDING_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: pending.run_id.clone(),
                session_id: Some(session_id.into()),
                run_id: Some(pending.run_id.clone()),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(pending)
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
        })?;
        Ok(())
    }

    fn complete_tool_turn(
        &self,
        session_id: &str,
        pending: &PendingSessionToolTurn,
        messages: Vec<SessionMessageAppend>,
        actor: Actor,
    ) -> Result<Vec<SessionMessage>, StoreError> {
        validate_session_id(session_id)?;
        validate_pending_tool_turn(pending)?;
        for message in &messages {
            validate_message(&message.message)?;
        }
        let transcript = messages
            .iter()
            .map(|append| append.message.clone())
            .collect::<Vec<_>>();
        validate_model_transcript(&transcript)
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let call_ids = transcript
            .first()
            .filter(|message| message.role == ModelMessageRole::Assistant)
            .map(|message| {
                message
                    .tool_calls
                    .iter()
                    .map(|call| call.call_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if call_ids != pending.call_ids {
            return Err(StoreError::Adapter(
                "tool-turn messages do not match the pending call ids".into(),
            ));
        }
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        if pending_tool_turn_from_events(self.journal.as_ref(), &events)?.as_ref() != Some(pending)
        {
            return Err(StoreError::Adapter(
                "session pending tool turn changed before completion".into(),
            ));
        }
        let first_sequence = u64::try_from(
            events
                .iter()
                .filter(|event| event.event_type == MESSAGE_EVENT)
                .count()
                .saturating_add(1),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let first_stream_version = events.last().map_or(0, |event| event.stream_version);
        let mut batch = messages
            .iter()
            .enumerate()
            .map(|(index, append)| {
                let offset =
                    u64::try_from(index).map_err(|error| StoreError::Adapter(error.to_string()))?;
                Ok(NewEvent {
                    event_version: 1,
                    stream_id: stream_id.clone(),
                    expected_stream_version: first_stream_version.saturating_add(offset),
                    classification: EventClassification::Domain,
                    event_type: MESSAGE_EVENT.into(),
                    actor: append.actor.clone(),
                    context: ExecutionContext {
                        correlation_id: pending.run_id.clone(),
                        session_id: Some(session_id.into()),
                        run_id: Some(pending.run_id.clone()),
                        ..ExecutionContext::default()
                    },
                    payload: json!({
                        "run_id": &pending.run_id,
                        "sequence": first_sequence.saturating_add(offset),
                        "message": append.message,
                    }),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let completion_offset = u64::try_from(messages.len())
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        batch.push(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: first_stream_version.saturating_add(completion_offset),
            classification: EventClassification::Domain,
            event_type: TOOL_TURN_COMPLETED_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: pending.run_id.clone(),
                session_id: Some(session_id.into()),
                run_id: Some(pending.run_id.clone()),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(pending)
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
        });
        let mut envelopes = self.journal.append_batch(batch)?;
        let completion = envelopes.pop().ok_or_else(|| {
            StoreError::Adapter("tool-turn append returned no completion marker".into())
        })?;
        if completion.event_type != TOOL_TURN_COMPLETED_EVENT || envelopes.len() != messages.len() {
            return Err(StoreError::Adapter(
                "tool-turn append returned an unexpected event batch".into(),
            ));
        }
        messages
            .into_iter()
            .zip(envelopes)
            .enumerate()
            .map(|(index, (append, envelope))| {
                let offset =
                    u64::try_from(index).map_err(|error| StoreError::Adapter(error.to_string()))?;
                Ok(SessionMessage {
                    session_id: session_id.into(),
                    run_id: pending.run_id.clone(),
                    sequence: first_sequence.saturating_add(offset),
                    message: append.message,
                    created_at: envelope.occurred_at,
                })
            })
            .collect()
    }

    fn list_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, StoreError> {
        validate_session_id(session_id)?;
        let events = self.journal.read_stream(&Self::stream(session_id))?;
        if events.is_empty() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        events
            .iter()
            .filter(|event| event.event_type == MESSAGE_EVENT)
            .map(|event| message_from_event(self.journal.as_ref(), session_id, event))
            .collect()
    }
}
