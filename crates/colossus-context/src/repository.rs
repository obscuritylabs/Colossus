use super::*;

/// Journal-backed immutable context snapshot repository.
pub struct EventSourcedContextRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedContextRepository {
    /// Bind snapshots to the authoritative encrypted journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(session_id: &str) -> String {
        format!("session:{session_id}")
    }
}

impl ContextRepository for EventSourcedContextRepository {
    fn create(
        &self,
        mut snapshot: ContextSnapshot,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError> {
        validate_snapshot(&snapshot)?;
        let stream_id = Self::stream(&snapshot.session_id);
        let events = self.journal.read_stream(&stream_id)?;
        if events.is_empty() {
            return Err(StoreError::NotFound(format!(
                "session {}",
                snapshot.session_id
            )));
        }
        if self
            .list(&snapshot.session_id)?
            .iter()
            .any(|existing| existing.id == snapshot.id)
        {
            return Err(StoreError::Adapter(format!(
                "context snapshot already exists: {}",
                snapshot.id
            )));
        }
        snapshot.created_at.clear();
        let expected = events.last().map_or(0, |event| event.stream_version);
        let context = ExecutionContext {
            correlation_id: snapshot.id.clone(),
            session_id: Some(snapshot.session_id.clone()),
            ..ExecutionContext::default()
        };
        let envelopes = self.journal.append_batch(vec![
            NewEvent {
                event_version: 1,
                stream_id: stream_id.clone(),
                expected_stream_version: expected,
                classification: EventClassification::Domain,
                event_type: SNAPSHOT_CREATED.into(),
                actor: actor.clone(),
                context: context.clone(),
                payload: serde_json::to_value(&snapshot)
                    .map_err(|error| StoreError::Adapter(error.to_string()))?,
            },
            NewEvent {
                event_version: 1,
                stream_id,
                expected_stream_version: expected.saturating_add(1),
                classification: EventClassification::Domain,
                event_type: SNAPSHOT_ACTIVATED.into(),
                actor,
                context,
                payload: json!({"snapshot_id": snapshot.id}),
            },
        ])?;
        snapshot.created_at = envelopes
            .first()
            .map_or_else(String::new, |event| event.occurred_at.clone());
        Ok(snapshot)
    }

    fn list(&self, session_id: &str) -> Result<Vec<ContextSnapshot>, StoreError> {
        let events = self.journal.read_stream(&Self::stream(session_id))?;
        if events.is_empty() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        events
            .iter()
            .filter(|event| event.event_type == SNAPSHOT_CREATED)
            .map(|event| {
                let mut snapshot: ContextSnapshot =
                    serde_json::from_value(self.journal.decrypt_payload(event)?)
                        .map_err(|error| StoreError::Verification(error.to_string()))?;
                snapshot.created_at.clone_from(&event.occurred_at);
                validate_snapshot(&snapshot)?;
                Ok(snapshot)
            })
            .collect()
    }

    fn active(&self, session_id: &str) -> Result<Option<ContextSnapshot>, StoreError> {
        let events = self.journal.read_stream(&Self::stream(session_id))?;
        if events.is_empty() {
            return Err(StoreError::NotFound(format!("session {session_id}")));
        }
        let active_id = events
            .iter()
            .rev()
            .find(|event| event.event_type == SNAPSHOT_ACTIVATED)
            .map(|event| self.journal.decrypt_payload(event))
            .transpose()?
            .and_then(|payload| {
                payload
                    .get("snapshot_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        let snapshots = self.list(session_id)?;
        active_id.map_or(Ok(None), |id| {
            snapshots
                .into_iter()
                .find(|snapshot| snapshot.id == id)
                .map(Some)
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "active context snapshot does not exist: {id}"
                    ))
                })
        })
    }

    fn activate(
        &self,
        session_id: &str,
        snapshot_id: &str,
        actor: Actor,
    ) -> Result<ContextSnapshot, StoreError> {
        let snapshot = self
            .list(session_id)?
            .into_iter()
            .find(|snapshot| snapshot.id == snapshot_id)
            .ok_or_else(|| StoreError::NotFound(format!("context snapshot {snapshot_id}")))?;
        let stream_id = Self::stream(session_id);
        let events = self.journal.read_stream(&stream_id)?;
        let expected_stream_version = events.last().map_or(0, |event| event.stream_version);
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: SNAPSHOT_ACTIVATED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: snapshot_id.into(),
                session_id: Some(session_id.into()),
                ..ExecutionContext::default()
            },
            payload: json!({"snapshot_id": snapshot_id}),
        })?;
        Ok(snapshot)
    }
}
