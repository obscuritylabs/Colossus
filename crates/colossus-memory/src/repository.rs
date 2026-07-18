use super::*;

pub(super) const MAX_MEMORY_TEXT_BYTES: usize = 64 * 1024;
pub(super) const MAX_METADATA_BYTES: usize = 64 * 1024;
pub(super) const MAX_LIST: usize = 1_000;
pub(super) const POSITION_DOCUMENT_ID: &str = "__position__";

pub(super) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

pub(super) fn validate_record(record: &MemoryRecord) -> Result<(), StoreError> {
    if record.id.is_empty()
        || record.kind.is_empty()
        || !matches!(record.source.as_str(), "user" | "agent")
        || record.text.is_empty()
        || record.text.len() > MAX_MEMORY_TEXT_BYTES
        || record.rationale.len() > MAX_MEMORY_TEXT_BYTES
        || !record.confidence.is_finite()
        || !(0.0..=1.0).contains(&record.confidence)
        || record.status != MemoryStatus::Active
        || record.superseded_by.is_some()
        || matches!(&record.scope, MemoryScope::Repository(id) | MemoryScope::Session(id) if id.trim().is_empty())
    {
        return Err(StoreError::Adapter(
            "memory id/kind/source/text/confidence/status is invalid".into(),
        ));
    }
    if record
        .expires_at
        .as_deref()
        .is_some_and(|value| OffsetDateTime::parse(value, &Rfc3339).is_err())
    {
        return Err(StoreError::Adapter(
            "memory expiry must be UTC RFC3339 when supplied".into(),
        ));
    }
    let normalized = format!("{} {}", record.text, record.rationale).to_ascii_lowercase();
    if normalized.contains("private key-----")
        || normalized.contains("authorization: bearer ")
        || normalized.contains(" bearer ")
        || normalized.contains("password=")
        || normalized.contains("password:")
        || normalized.contains("api_key=")
        || normalized.contains("api-key:")
        || normalized.contains("apikey=")
        || normalized.contains("secret=")
        || normalized.contains("token=")
        || normalized.contains("github_pat_")
        || normalized.contains("ghp_")
        || normalized.contains(" sk-")
    {
        return Err(StoreError::Adapter(
            "memory text resembles prohibited secret material".into(),
        ));
    }
    Ok(())
}

/// Memory lifecycle repository backed only by immutable journal events.
pub struct EventSourcedMemoryRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedMemoryRepository {
    /// Bind canonical memory streams to the authoritative journal.
    /// Runtime mutation surfaces still place this adapter behind the effect gateway.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(id: &str) -> String {
        format!("memory:{id}")
    }

    fn event(
        id: &str,
        expected_stream_version: u64,
        event_type: &str,
        actor: Actor,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: Self::stream(id),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("memory:{id}"),
                ..ExecutionContext::default()
            },
            payload,
        }
    }
}

impl MemoryRepository for EventSourcedMemoryRepository {
    fn create(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError> {
        validate_record(&record)?;
        self.journal.append(Self::event(
            &record.id,
            0,
            "memory.created.v1",
            actor,
            json!({"record": &record}),
        ))?;
        Ok(record)
    }

    fn get_memory(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        let events = self.journal.read_stream(&Self::stream(id))?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        let payload = self.journal.decrypt_payload(first)?;
        let mut record: MemoryRecord = serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("memory record is absent".into()))?,
        )
        .map_err(adapter)?;
        validate_record(&record).map_err(|error| {
            StoreError::Verification(format!("invalid canonical memory creation event: {error}"))
        })?;
        for event in events.iter().skip(1) {
            let payload = self.journal.decrypt_payload(event)?;
            match event.event_type.as_str() {
                "memory.updated.v1" => {
                    let updated: MemoryRecord =
                        serde_json::from_value(payload.get("record").cloned().ok_or_else(
                            || StoreError::Verification("updated memory record is absent".into()),
                        )?)
                        .map_err(adapter)?;
                    validate_record(&updated).map_err(|error| {
                        StoreError::Verification(format!(
                            "invalid canonical memory update event: {error}"
                        ))
                    })?;
                    if updated.id != record.id
                        || updated.scope != record.scope
                        || updated.source != record.source
                        || updated.created_at != record.created_at
                    {
                        return Err(StoreError::Verification(
                            "memory update changed immutable identity or provenance".into(),
                        ));
                    }
                    record = updated;
                }
                "memory.archived.v1" => {
                    record.status = MemoryStatus::Archived;
                    record.updated_at = string(&payload, "updated_at")?;
                }
                "memory.superseded.v1" => {
                    record.status = MemoryStatus::Superseded;
                    record.updated_at = string(&payload, "updated_at")?;
                    record.superseded_by = Some(string(&payload, "replacement_id")?);
                }
                _ => {}
            }
        }
        Ok(Some(record))
    }

    fn update(&self, record: MemoryRecord, actor: Actor) -> Result<MemoryRecord, StoreError> {
        validate_record(&record)?;
        let current = self
            .get_memory(&record.id)?
            .ok_or_else(|| StoreError::NotFound(format!("memory {}", record.id)))?;
        if current.status != MemoryStatus::Active
            || record.scope != current.scope
            || record.source != current.source
            || record.created_at != current.created_at
        {
            return Err(StoreError::Adapter(
                "memory update requires an active record and immutable identity, scope, source, and creation time"
                    .into(),
            ));
        }
        let stream = Self::stream(&record.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            &record.id,
            expected,
            "memory.updated.v1",
            actor,
            json!({"record": &record}),
        ))?;
        Ok(record)
    }

    fn list_active(&self, limit: usize) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut records = self.list_memories(Some(MemoryStatus::Active), MAX_LIST)?;
        let current_time = OffsetDateTime::now_utc();
        records.retain(|record| !expired(record, current_time));
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn list_memories(
        &self,
        status: Option<MemoryStatus>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if event.event_type == "memory.created.v1"
                    && let Some(id) = event.stream_id.strip_prefix("memory:")
                {
                    ids.insert(id.to_owned());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 {
                break;
            }
        }
        let mut records = ids
            .into_iter()
            .filter_map(|id| self.get_memory(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| status.is_none_or(|status| record.status == status));
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn archive(&self, id: &str, actor: Actor) -> Result<MemoryRecord, StoreError> {
        let record = self
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        if record.status != MemoryStatus::Active {
            return Err(StoreError::Conflict {
                stream_id: Self::stream(id),
                expected: 1,
                actual: u64::try_from(self.journal.read_stream(&Self::stream(id))?.len())
                    .map_err(adapter)?,
            });
        }
        let events = self.journal.read_stream(&Self::stream(id))?;
        self.journal.append(Self::event(
            id,
            u64::try_from(events.len()).map_err(adapter)?,
            "memory.archived.v1",
            actor,
            json!({"updated_at": now()?}),
        ))?;
        self.get_memory(id)?
            .ok_or_else(|| StoreError::Verification("archived memory disappeared".into()))
    }

    fn supersede(
        &self,
        id: &str,
        replacement: MemoryRecord,
        actor: Actor,
    ) -> Result<(MemoryRecord, MemoryRecord), StoreError> {
        let current = self
            .get_memory(id)?
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        if current.status != MemoryStatus::Active || replacement.id == id {
            return Err(StoreError::Adapter(
                "only an active memory can be superseded by a different id".into(),
            ));
        }
        validate_record(&replacement)?;
        if self.get_memory(&replacement.id)?.is_some() {
            return Err(StoreError::Conflict {
                stream_id: Self::stream(&replacement.id),
                expected: 0,
                actual: 1,
            });
        }
        let old_events = self.journal.read_stream(&Self::stream(id))?;
        let timestamp = now()?;
        let replacement_id = replacement.id.clone();
        self.journal.append_batch(vec![
            Self::event(
                id,
                u64::try_from(old_events.len()).map_err(adapter)?,
                "memory.superseded.v1",
                actor.clone(),
                json!({"updated_at": timestamp, "replacement_id": replacement_id}),
            ),
            Self::event(
                &replacement.id,
                0,
                "memory.created.v1",
                actor,
                json!({"record": &replacement}),
            ),
        ])?;
        Ok((
            self.get_memory(id)?
                .ok_or_else(|| StoreError::Verification("superseded memory disappeared".into()))?,
            self.get_memory(&replacement.id)?
                .ok_or_else(|| StoreError::Verification("replacement memory disappeared".into()))?,
        ))
    }
}

pub(super) fn string(value: &Value, field: &str) -> Result<String, StoreError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Verification(format!("memory field {field} is absent")))
}

pub(super) fn expired(record: &MemoryRecord, now: OffsetDateTime) -> bool {
    record.expires_at.as_deref().is_some_and(|value| {
        OffsetDateTime::parse(value, &Rfc3339).is_ok_and(|expiry| expiry <= now)
    })
}
