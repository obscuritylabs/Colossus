use super::*;

impl EventJournal for RedbEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        let mut events = self.append_batch(vec![event])?;
        events
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let persisted = {
            let _guard = self.writer.lock().map_err(adapter_error)?;
            self.append_locked(events)
        };
        let persisted = self.quarantine_result(persisted)?;
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(persisted);
        }
        let checkpoint_sequence = if persisted.is_empty() {
            0
        } else {
            self.quarantine_result(self.checkpoint_sequence())?
        };
        let count_due = persisted.last().is_some_and(|event| {
            event.global_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL
        });
        let age_due = self
            .last_checkpoint
            .lock()
            .map_err(adapter_error)?
            .elapsed()
            >= CHECKPOINT_MAX_AGE;
        if count_due || age_due {
            self.checkpoint()?;
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.quarantine_result(self.read_indexed_stream(stream_id, 0, None))
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let result = self.read_indexed_stream(
            stream_id,
            after_version,
            Some(limit.min(MAX_STREAM_READ_BATCH)),
        );
        self.quarantine_result(result)
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let result = self.read_indexed_stream_backwards(stream_id, before_version, limit);
        self.quarantine_result(result)
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        let result = (|| {
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
            let read = self.database.begin_read().map_err(adapter_error)?;
            let streams = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
            let start = after.unwrap_or(prefix);
            let mut ids = Vec::with_capacity(limit);
            for entry in streams.range(start..).map_err(adapter_error)? {
                if ids.len() >= limit {
                    break;
                }
                let (stream_id, _) = entry.map_err(adapter_error)?;
                let stream_id = stream_id.value();
                if after == Some(stream_id) {
                    continue;
                }
                if !stream_id.starts_with(prefix) {
                    break;
                }
                ids.push(stream_id.to_owned());
            }
            Ok(ids)
        })();
        self.quarantine_result(result)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let result = (|| {
            let read = self.database.begin_read().map_err(adapter_error)?;
            let table = read.open_table(EVENTS).map_err(adapter_error)?;
            let metadata = read.open_table(METADATA).map_err(adapter_error)?;
            let head_sequence = metadata
                .get("last_sequence")
                .map_err(adapter_error)?
                .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
            let mut events = Vec::with_capacity(limit.min(1024));
            let mut expected_sequence = from_sequence.max(1);
            for entry in table.range(expected_sequence..).map_err(adapter_error)? {
                if events.len() >= limit {
                    break;
                }
                let (key, value) = entry.map_err(adapter_error)?;
                let event: EventEnvelope = decode_journal_json(value.value())?;
                if key.value() != expected_sequence || event.global_sequence != expected_sequence {
                    return Err(StoreError::Verification(format!(
                        "global journal read expected sequence {expected_sequence}"
                    )));
                }
                expected_sequence = expected_sequence.saturating_add(1);
                events.push(event);
            }
            if limit > 0 && events.len() < limit && expected_sequence <= head_sequence {
                return Err(StoreError::Verification(format!(
                    "global journal read expected sequence {expected_sequence}"
                )));
            }
            Ok(events)
        })();
        self.quarantine_result(result)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        let result = (|| {
            let read = self.database.begin_read().map_err(adapter_error)?;
            let table = read.open_table(OUTBOX).map_err(adapter_error)?;
            let metadata = read.open_table(METADATA).map_err(adapter_error)?;
            let head_sequence = metadata
                .get("last_sequence")
                .map_err(adapter_error)?
                .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
            let mut work = Vec::with_capacity(limit.min(1024));
            let mut expected_sequence = from_sequence.max(1);
            for entry in table.range(expected_sequence..).map_err(adapter_error)? {
                if work.len() >= limit {
                    break;
                }
                let (sequence, value) = entry.map_err(adapter_error)?;
                if sequence.value() != expected_sequence {
                    return Err(StoreError::Verification(format!(
                        "projection outbox expected sequence {expected_sequence}"
                    )));
                }
                let record: Value = decode_journal_json(value.value())?;
                let event_id = record
                    .get("event_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "projection outbox record {} has no event_id",
                            sequence.value()
                        ))
                    })?;
                if record.get("global_sequence").and_then(Value::as_u64) != Some(sequence.value()) {
                    return Err(StoreError::Verification(format!(
                        "projection outbox record {} has a mismatched sequence",
                        sequence.value()
                    )));
                }
                work.push(ProjectionWorkItem {
                    global_sequence: sequence.value(),
                    event_id: event_id.to_owned(),
                });
                expected_sequence = expected_sequence.saturating_add(1);
            }
            if limit > 0 && work.len() < limit && expected_sequence <= head_sequence {
                return Err(StoreError::Verification(format!(
                    "projection outbox expected sequence {expected_sequence}"
                )));
            }
            Ok(work)
        })();
        self.quarantine_result(result)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.quarantine_result((|| {
            let read = self.database.begin_read().map_err(adapter_error)?;
            let metadata = read.open_table(METADATA).map_err(adapter_error)?;
            let sequence = metadata
                .get("last_sequence")
                .map_err(adapter_error)?
                .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
            let hash = metadata
                .get("last_hash")
                .map_err(adapter_error)?
                .map_or_else(
                    || Ok::<String, StoreError>(ZERO_HASH.into()),
                    |value| decode_journal_json(value.value()),
                )?;
            Ok((sequence, hash))
        })())
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let result = (|| {
            let persisted = self.load_persisted(event)?;
            self.verify_persisted_event(event, &persisted)
        })();
        self.quarantine_result(result)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        let result = self.verify_inner();
        self.quarantine_result(result)
    }

    fn is_recovery_mode(&self) -> bool {
        self.recovery_mode.load(Ordering::Acquire)
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.quarantine_result(self.checkpoint_inner())
    }
}
