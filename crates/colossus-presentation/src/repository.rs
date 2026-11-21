use super::*;
use colossus_contracts::EventEnvelope;

/// Immutable-journal implementation of the presentation preference port.
pub struct EventSourcedPresentationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPresentationRepository {
    /// Bind the global terminal presentation profile to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn preferences_from_event(
        &self,
        event: Option<&EventEnvelope>,
    ) -> Result<TerminalPreferences, StoreError> {
        let Some(event) = event else {
            return Ok(TerminalPreferences::default());
        };
        if event.event_type != PREFERENCES_UPDATED {
            return Err(StoreError::Verification(
                "presentation stream contains an unknown event".into(),
            ));
        }
        let payload = self.journal.decrypt_payload(event)?;
        let preferences: TerminalPreferences = serde_json::from_value(
            payload
                .get("preferences")
                .cloned()
                .ok_or_else(|| StoreError::Verification("preferences payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        validate_preferences(&preferences)?;
        Ok(preferences)
    }

    fn append_preferences(
        &self,
        preferences: TerminalPreferences,
        actor: Actor,
        stream_length: usize,
    ) -> Result<TerminalPreferences, StoreError> {
        let expected_stream_version =
            u64::try_from(stream_length).map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: PREFERENCES_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: PREFERENCES_UPDATED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: PREFERENCES_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"preferences": &preferences}),
        })?;
        Ok(preferences)
    }
}

impl PresentationRepository for EventSourcedPresentationRepository {
    fn load(&self) -> Result<TerminalPreferences, StoreError> {
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        self.preferences_from_event(events.last())
    }

    fn save(
        &self,
        preferences: TerminalPreferences,
        actor: Actor,
    ) -> Result<TerminalPreferences, StoreError> {
        validate_preferences(&preferences)?;
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        self.append_preferences(preferences, actor, events.len())
    }

    fn save_if_current(
        &self,
        expected: &TerminalPreferences,
        preferences: TerminalPreferences,
        actor: Actor,
    ) -> Result<TerminalPreferences, StoreError> {
        validate_preferences(&preferences)?;
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        if &self.preferences_from_event(events.last())? != expected {
            return Err(StoreError::Adapter(
                "Terminal settings changed while acknowledgement was pending. Reopen settings and try again."
                    .into(),
            ));
        }
        // Use the version of the same snapshot we just compared, so another writer
        // cannot change the profile between the comparison and the append.
        self.append_preferences(preferences, actor, events.len())
    }

    fn list_history(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        if !(1..=MAX_HISTORY_ENTRIES).contains(&limit) {
            return Err(StoreError::Adapter(format!(
                "history limit must be between 1 and {MAX_HISTORY_ENTRIES}"
            )));
        }
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        let skip = events.len().saturating_sub(limit);
        events
            .iter()
            .skip(skip)
            .map(|event| {
                if event.event_type != HISTORY_APPENDED {
                    return Err(StoreError::Verification(
                        "presentation history contains an unknown event".into(),
                    ));
                }
                let payload = self.journal.decrypt_payload(event)?;
                let entry = payload
                    .get("entry")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::Verification("presentation history entry is absent".into())
                    })?;
                validate_history_entry(entry)?;
                Ok(entry.into())
            })
            .collect()
    }

    fn append_history(&self, entry: String, actor: Actor) -> Result<String, StoreError> {
        validate_history_entry(&entry)?;
        let events = self.journal.read_stream(HISTORY_STREAM)?;
        if let Some(event) = events.last() {
            if event.event_type != HISTORY_APPENDED {
                return Err(StoreError::Verification(
                    "presentation history contains an unknown event".into(),
                ));
            }
            let payload = self.journal.decrypt_payload(event)?;
            if payload.get("entry").and_then(Value::as_str) == Some(entry.as_str()) {
                return Ok(entry);
            }
        }
        let expected_stream_version =
            u64::try_from(events.len()).map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: HISTORY_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: HISTORY_APPENDED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: HISTORY_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"entry": &entry}),
        })?;
        Ok(entry)
    }
}

fn validate_history_entry(entry: &str) -> Result<(), StoreError> {
    if entry.trim().is_empty() || entry.len() > MAX_HISTORY_ENTRY_BYTES {
        return Err(StoreError::Adapter(format!(
            "history entry must be nonempty and at most {MAX_HISTORY_ENTRY_BYTES} bytes"
        )));
    }
    Ok(())
}
