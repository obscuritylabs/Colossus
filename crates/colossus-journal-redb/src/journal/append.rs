use super::*;

impl RedbEventJournal {
    pub(super) fn append_locked(
        &self,
        events: Vec<NewEvent>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let write = self.database.begin_write().map_err(adapter_error)?;
        let mut persisted = Vec::with_capacity(events.len());
        {
            let mut event_table = write.open_table(EVENTS).map_err(adapter_error)?;
            let mut stream_events = write.open_table(STREAM_EVENTS).map_err(adapter_error)?;
            let mut stream_table = write.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            let mut outbox = write.open_table(OUTBOX).map_err(adapter_error)?;
            let mut sequence = metadata
                .get("last_sequence")
                .map_err(adapter_error)?
                .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
            let mut previous_hash = metadata
                .get("last_hash")
                .map_err(adapter_error)?
                .map_or_else(
                    || Ok::<String, StoreError>(ZERO_HASH.into()),
                    |value| decode_journal_json(value.value()),
                )?;
            let mut batch_versions = BTreeMap::<String, u64>::new();

            for event in events {
                let durable_version = if let Some(version) = batch_versions.get(&event.stream_id) {
                    *version
                } else {
                    stream_table
                        .get(event.stream_id.as_str())
                        .map_err(adapter_error)?
                        .map_or(0, |value| value.value())
                };
                if event.expected_stream_version != durable_version {
                    return Err(StoreError::Conflict {
                        stream_id: event.stream_id,
                        expected: event.expected_stream_version,
                        actual: durable_version,
                    });
                }

                sequence = sequence.checked_add(1).ok_or_else(|| {
                    StoreError::Verification("journal sequence is exhausted".into())
                })?;
                let stream_version = durable_version.checked_add(1).ok_or_else(|| {
                    StoreError::Verification("stream version is exhausted".into())
                })?;
                let mut envelope = EventEnvelope {
                    schema_version: 1,
                    event_version: event.event_version,
                    event_id: Uuid::now_v7().to_string(),
                    global_sequence: sequence,
                    stream_id: event.stream_id,
                    stream_version,
                    classification: event.classification,
                    event_type: event.event_type,
                    actor: event.actor,
                    context: event.context,
                    occurred_at: utc_now()?,
                    payload: EncryptedPayload {
                        key_id: String::new(),
                        algorithm: String::new(),
                        nonce: String::new(),
                        ciphertext: String::new(),
                        plaintext_hash: String::new(),
                    },
                    previous_hash: previous_hash.clone(),
                    record_hash: String::new(),
                };
                let plaintext = serde_json::to_vec(&event.payload).map_err(adapter_error)?;
                envelope.payload = self.encrypt_payload(&envelope, &plaintext)?;
                envelope.record_hash = record_hash(&envelope)?;
                previous_hash.clone_from(&envelope.record_hash);
                let encoded = serde_json::to_vec(&envelope).map_err(adapter_error)?;
                event_table
                    .insert(sequence, encoded.as_slice())
                    .map_err(adapter_error)?;
                if stream_events
                    .insert(&(envelope.stream_id.as_str(), stream_version), &sequence)
                    .map_err(adapter_error)?
                    .is_some()
                {
                    return Err(StoreError::Verification(format!(
                        "stream {} version {stream_version} is already indexed",
                        envelope.stream_id
                    )));
                }
                stream_table
                    .insert(envelope.stream_id.as_str(), stream_version)
                    .map_err(adapter_error)?;
                let outbox_record = serde_json::to_vec(&json!({
                    "event_id": envelope.event_id,
                    "global_sequence": sequence,
                    "status": "pending"
                }))
                .map_err(adapter_error)?;
                outbox
                    .insert(sequence, outbox_record.as_slice())
                    .map_err(adapter_error)?;
                batch_versions.insert(envelope.stream_id.clone(), stream_version);
                persisted.push(envelope);
            }
            let sequence_bytes = serde_json::to_vec(&sequence).map_err(adapter_error)?;
            let hash_bytes = serde_json::to_vec(&previous_hash).map_err(adapter_error)?;
            metadata
                .insert("last_sequence", sequence_bytes.as_slice())
                .map_err(adapter_error)?;
            metadata
                .insert("last_hash", hash_bytes.as_slice())
                .map_err(adapter_error)?;
        }
        #[cfg(test)]
        crash_at_test_fault("before_commit");
        write.commit().map_err(adapter_error)?;
        #[cfg(test)]
        crash_at_test_fault("after_commit");
        Ok(persisted)
    }
}
