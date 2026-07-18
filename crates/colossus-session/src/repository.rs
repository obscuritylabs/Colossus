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
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, SCAN_BATCH)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(id) = event.stream_id.strip_prefix("session:") {
                    ids.insert(id.to_owned());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < SCAN_BATCH {
                break;
            }
        }
        let mut sessions = ids
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

    fn append_message(
        &self,
        session_id: &str,
        run_id: &str,
        message: ModelMessage,
        actor: Actor,
    ) -> Result<SessionMessage, StoreError> {
        validate_session_id(session_id)?;
        if run_id.is_empty() {
            return Err(StoreError::Adapter("message run id is required".into()));
        }
        validate_message(&message)?;
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events
            .first()
            .is_none_or(|event| event.event_type != SESSION_EVENT)
        {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        let sequence = events
            .iter()
            .filter(|event| event.event_type == MESSAGE_EVENT)
            .count()
            .saturating_add(1);
        let sequence =
            u64::try_from(sequence).map_err(|error| StoreError::Adapter(error.to_string()))?;
        let expected_stream_version = events.last().map_or(0, |event| event.stream_version);
        let envelope = self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: MESSAGE_EVENT.into(),
            actor,
            context: ExecutionContext {
                correlation_id: run_id.into(),
                session_id: Some(session_id.into()),
                run_id: Some(run_id.into()),
                ..ExecutionContext::default()
            },
            payload: json!({
                "run_id": run_id,
                "sequence": sequence,
                "message": message,
            }),
        })?;
        Ok(SessionMessage {
            session_id: session_id.into(),
            run_id: run_id.into(),
            sequence,
            message,
            created_at: envelope.occurred_at,
        })
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
