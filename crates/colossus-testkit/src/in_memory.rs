use super::*;

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Default)]
struct State {
    events: Vec<EventEnvelope>,
    payloads: BTreeMap<String, Value>,
    streams: BTreeMap<String, Vec<EventEnvelope>>,
    stream_versions: BTreeMap<String, u64>,
}

/// Deterministic in-memory journal for application and conformance tests.
#[derive(Default)]
pub struct InMemoryEventJournal {
    state: Mutex<State>,
    reject_global_reads: AtomicBool,
}

impl InMemoryEventJournal {
    /// Build a journal that fails if repository code falls back to a global event scan.
    pub fn rejecting_global_reads() -> Self {
        Self {
            reject_global_reads: AtomicBool::new(true),
            ..Self::default()
        }
    }
}

fn failure(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl EventJournal for InMemoryEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.append_batch(vec![event])?
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        let mut pending_versions = state.stream_versions.clone();
        for event in &events {
            let actual = pending_versions.get(&event.stream_id).copied().unwrap_or(0);
            if event.expected_stream_version != actual {
                return Err(StoreError::Conflict {
                    stream_id: event.stream_id.clone(),
                    expected: event.expected_stream_version,
                    actual,
                });
            }
            pending_versions.insert(event.stream_id.clone(), actual.saturating_add(1));
        }
        let mut persisted = Vec::with_capacity(events.len());
        for event in events {
            let global_sequence = u64::try_from(state.events.len())
                .map_err(failure)?
                .saturating_add(1);
            let stream_version = state
                .stream_versions
                .get(&event.stream_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            let event_id = Uuid::now_v7().to_string();
            let plaintext = serde_json::to_vec(&event.payload).map_err(failure)?;
            let previous_hash = state
                .events
                .last()
                .map_or_else(|| ZERO_HASH.to_owned(), |record| record.record_hash.clone());
            let mut record = EventEnvelope {
                schema_version: 1,
                event_version: event.event_version,
                event_id: event_id.clone(),
                global_sequence,
                stream_id: event.stream_id,
                stream_version,
                classification: event.classification,
                event_type: event.event_type,
                actor: event.actor,
                context: event.context,
                occurred_at: OffsetDateTime::now_utc()
                    .format(&Rfc3339)
                    .map_err(failure)?,
                payload: EncryptedPayload {
                    key_id: "in-memory-test-only".into(),
                    algorithm: "in-memory-test-only".into(),
                    nonce: String::new(),
                    ciphertext: hex::encode(&plaintext),
                    plaintext_hash: hash(&plaintext),
                },
                previous_hash,
                record_hash: String::new(),
            };
            record.record_hash = hash(&serde_json::to_vec(&record).map_err(failure)?);
            state
                .stream_versions
                .insert(record.stream_id.clone(), stream_version);
            state.payloads.insert(event_id, event.payload);
            state.events.push(record.clone());
            state
                .streams
                .entry(record.stream_id.clone())
                .or_default()
                .push(record.clone());
            persisted.push(record);
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .streams
            .get(stream_id)
            .cloned()
            .unwrap_or_default())
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let state = self.state.lock().map_err(failure)?;
        let Some(events) = state.streams.get(stream_id) else {
            return Ok(Vec::new());
        };
        let start = usize::try_from(after_version).unwrap_or(usize::MAX);
        Ok(events.iter().skip(start).take(limit).cloned().collect())
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 || before_version.is_some_and(|version| version <= 1) {
            return Ok(Vec::new());
        }
        let state = self.state.lock().map_err(failure)?;
        let Some(events) = state.streams.get(stream_id) else {
            return Ok(Vec::new());
        };
        let end = before_version.map_or(events.len(), |version| {
            usize::try_from(version.saturating_sub(1))
                .unwrap_or(usize::MAX)
                .min(events.len())
        });
        Ok(events[..end].iter().rev().take(limit).cloned().collect())
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        if prefix.contains('\0')
            || after.is_some_and(|cursor| cursor.contains('\0') || !cursor.starts_with(prefix))
        {
            return Err(StoreError::Adapter(
                "stream prefix and cursor must contain no NUL and share one prefix".into(),
            ));
        }
        let limit = limit.min(MAX_STREAM_LIST_BATCH);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let state = self.state.lock().map_err(failure)?;
        let start = after.unwrap_or(prefix).to_owned();
        let mut ids = Vec::with_capacity(limit);
        for (stream_id, _) in state.stream_versions.range(start..) {
            if ids.len() >= limit {
                break;
            }
            if after == Some(stream_id.as_str()) {
                continue;
            }
            if !stream_id.starts_with(prefix) {
                break;
            }
            ids.push(stream_id.clone());
        }
        Ok(ids)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.reject_global_reads.load(Ordering::Acquire) {
            return Err(StoreError::Adapter(
                "global event reads are disabled for this test journal".into(),
            ));
        }
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .events
            .iter()
            .filter(|event| event.global_sequence >= from_sequence)
            .take(limit)
            .cloned()
            .collect())
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .events
            .iter()
            .filter(|event| event.global_sequence >= from_sequence)
            .take(limit)
            .map(|event| ProjectionWorkItem {
                global_sequence: event.global_sequence,
                event_id: event.event_id.clone(),
            })
            .collect())
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let state = self.state.lock().map_err(failure)?;
        Ok(state.events.last().map_or_else(
            || (0, ZERO_HASH.into()),
            |event| (event.global_sequence, event.record_hash.clone()),
        ))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.state
            .lock()
            .map_err(failure)?
            .payloads
            .get(&event.event_id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(event.event_id.clone()))
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        let state = self.state.lock().map_err(failure)?;
        let last = state.events.last();
        Ok(VerificationReport {
            event_count: u64::try_from(state.events.len()).map_err(failure)?,
            last_sequence: last.map_or(0, |event| event.global_sequence),
            last_hash: last.map_or_else(|| ZERO_HASH.into(), |event| event.record_hash.clone()),
            checkpoint: None,
        })
    }

    fn is_recovery_mode(&self) -> bool {
        false
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        Ok(None)
    }
}

#[derive(Default)]
struct ProjectionState {
    positions: BTreeMap<String, u64>,
    records: BTreeMap<(String, String), Value>,
}

/// Deterministic in-memory projection store for workers and conformance tests.
#[derive(Default)]
pub struct InMemoryProjectionStore {
    state: Mutex<ProjectionState>,
}

impl ProjectionStore for InMemoryProjectionStore {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .positions
            .get(projection)
            .copied()
            .unwrap_or(0))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .records
            .get(&(projection.into(), key.into()))
            .cloned())
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .records
            .iter()
            .filter(|((name, key), _)| name == projection && key.starts_with(key_prefix))
            .take(limit)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect())
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        let actual = state.positions.get(&batch.projection).copied().unwrap_or(0);
        if actual != batch.expected_position {
            return Err(StoreError::Conflict {
                stream_id: format!("projection:{}", batch.projection),
                expected: batch.expected_position,
                actual,
            });
        }
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        for mutation in batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, value } => {
                    state.records.insert((batch.projection.clone(), key), value);
                }
                ProjectionMutation::Delete { key } => {
                    state.records.remove(&(batch.projection.clone(), key));
                }
            }
        }
        state
            .positions
            .insert(batch.projection, batch.through_sequence);
        Ok(())
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        state.positions.remove(projection);
        state.records.retain(|(name, _), _| name != projection);
        Ok(())
    }
}
