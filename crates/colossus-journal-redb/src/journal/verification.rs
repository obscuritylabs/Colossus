use super::*;

impl RedbEventJournal {
    pub(super) fn quarantine_result<T>(
        &self,
        result: Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        if let Err(StoreError::Verification(reason)) = &result {
            self.recovery_mode.store(true, Ordering::Release);
            if let Ok(mut recovery_reason) = self.recovery_reason.lock() {
                *recovery_reason = Some(reason.clone());
            }
            // Serialize anchor changes with checkpoints so quarantine cannot be overwritten.
            if let Ok(_guard) = self.writer.lock()
                && let Ok(Some(mut anchor)) = self.keys.load_anchor()
            {
                anchor.status = SecureAnchorStatus::Quarantined;
                let _ = self.keys.store_anchor(&anchor);
            }
        }
        result
    }

    pub(super) fn verify_inner(&self) -> Result<VerificationReport, StoreError> {
        // Load the external anchor before pinning the database snapshot. A checkpoint
        // may advance concurrently, but every earlier anchor must remain in that snapshot.
        let anchor = self.keys.load_anchor()?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let stream_event_table = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let durable_stream_table = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let outbox = read.open_table(OUTBOX).map_err(adapter_error)?;
        let projection_positions = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        let projection_records = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        let mut expected_sequence = 1_u64;
        let mut previous_hash = ZERO_HASH.to_owned();
        let mut stream_versions = BTreeMap::<String, u64>::new();
        let checkpoint: Option<SignedCheckpoint> = metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?;
        let mut checkpoint_hash = None;
        let mut anchor_hash = None;
        let stream_index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?;
        let verify_stream_index = match stream_index_version {
            None => {
                if !stream_event_table.is_empty().map_err(adapter_error)? {
                    return Err(StoreError::Verification(
                        "unversioned stream event index is not empty".into(),
                    ));
                }
                false
            }
            Some(STREAM_EVENTS_INDEX_VERSION) => true,
            Some(_) => {
                return Err(StoreError::Verification(
                    "stream event index version is unsupported".into(),
                ));
            }
        };

        for entry in event_table.iter().map_err(adapter_error)? {
            let (key, value) = entry.map_err(adapter_error)?;
            let sequence = key.value();
            if sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "global sequence gap: expected {expected_sequence}, got {sequence}"
                )));
            }
            let persisted: PersistedEventEnvelope = decode_journal_json(value.value())?;
            let envelope: EventEnvelope = decode_journal_json(value.value())?;
            if envelope.global_sequence != sequence || envelope.previous_hash != previous_hash {
                return Err(StoreError::Verification(format!(
                    "event {} sequence or previous hash mismatch",
                    envelope.event_id
                )));
            }
            let expected_stream = stream_versions
                .get(&envelope.stream_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            if envelope.stream_version != expected_stream {
                return Err(StoreError::Verification(format!(
                    "stream {} version mismatch",
                    envelope.stream_id
                )));
            }
            if verify_stream_index
                && stream_event_table
                    .get(&(envelope.stream_id.as_str(), envelope.stream_version))
                    .map_err(adapter_error)?
                    .map(|indexed| indexed.value())
                    != Some(sequence)
            {
                return Err(StoreError::Verification(format!(
                    "stream {} version {} index mismatch",
                    envelope.stream_id, envelope.stream_version
                )));
            }
            self.verify_persisted_event(&envelope, &persisted)?;
            previous_hash.clone_from(&envelope.record_hash);
            if checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.global_sequence == sequence)
            {
                checkpoint_hash = Some(envelope.record_hash.clone());
            }
            if anchor
                .as_ref()
                .is_some_and(|anchor| anchor.sequence == sequence)
            {
                anchor_hash = Some(envelope.record_hash.clone());
            }
            let queued = outbox
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {sequence} is absent"
                    ))
                })?;
            let queued: Value = decode_journal_json(queued.value())?;
            if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id)
                || queued.get("global_sequence").and_then(Value::as_u64)
                    != Some(envelope.global_sequence)
            {
                return Err(StoreError::Verification(format!(
                    "projection outbox record {sequence} targets a different event"
                )));
            }
            stream_versions.insert(envelope.stream_id, envelope.stream_version);
            expected_sequence = expected_sequence.saturating_add(1);
        }

        let last_sequence = expected_sequence.saturating_sub(1);
        let metadata_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
        let metadata_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| decode_journal_json(value.value()),
            )?;
        if metadata_sequence != last_sequence || metadata_hash != previous_hash {
            return Err(StoreError::Verification(
                "journal head metadata does not match event chain".into(),
            ));
        }
        if outbox.len().map_err(adapter_error)? != last_sequence {
            return Err(StoreError::Verification(
                "projection outbox position does not match journal head".into(),
            ));
        }
        let mut durable_stream_versions = BTreeMap::new();
        for entry in durable_stream_table.iter().map_err(adapter_error)? {
            let (stream_id, version) = entry.map_err(adapter_error)?;
            durable_stream_versions.insert(stream_id.value().to_owned(), version.value());
        }
        if durable_stream_versions != stream_versions {
            return Err(StoreError::Verification(
                "durable stream versions do not match journal replay".into(),
            ));
        }
        for entry in projection_positions.iter().map_err(adapter_error)? {
            let (projection, position) = entry.map_err(adapter_error)?;
            projection_prefix(projection.value()).map_err(|_| {
                StoreError::Verification("stored projection name is invalid".into())
            })?;
            if position.value() > last_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {} position {} is ahead of journal head {last_sequence}",
                    projection.value(),
                    position.value()
                )));
            }
        }
        for entry in projection_records.iter().map_err(adapter_error)? {
            let (key, value) = entry.map_err(adapter_error)?;
            let Some((projection, record_key)) = key.value().split_once('\0') else {
                return Err(StoreError::Verification(
                    "projection record key has no namespace delimiter".into(),
                ));
            };
            projection_record_key(projection, record_key).map_err(|_| {
                StoreError::Verification("stored projection record key is invalid".into())
            })?;
            decode_journal_json::<Value>(value.value()).map_err(|error| {
                StoreError::Verification(format!(
                    "projection record {} is invalid JSON: {error}",
                    key.value()
                ))
            })?;
        }

        if self.payload_protection == JournalPayloadProtection::Plaintext && checkpoint.is_some() {
            return Err(StoreError::Verification(
                "plaintext journal contains a signed checkpoint".into(),
            ));
        }
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, checkpoint_hash.as_deref())?;
        }
        if let Some(anchor) = anchor
            && anchor_hash.as_deref() != Some(anchor.hash.as_str())
        {
            return Err(StoreError::Verification(
                "secure anchor is missing or differs from journal".into(),
            ));
        }
        if verify_stream_index && stream_event_table.len().map_err(adapter_error)? != last_sequence
        {
            return Err(StoreError::Verification(
                "stream event index position does not match journal head".into(),
            ));
        }
        Ok(VerificationReport {
            event_count: last_sequence,
            last_sequence,
            last_hash: previous_hash,
            checkpoint,
        })
    }
}
