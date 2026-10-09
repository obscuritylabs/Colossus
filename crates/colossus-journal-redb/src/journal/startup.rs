use super::*;

impl RedbEventJournal {
    pub(super) fn verify_startup(&self, mode: StartupVerificationMode) -> Result<(), StoreError> {
        let anchor = if self.payload_protection == JournalPayloadProtection::Encrypted {
            self.keys.load_anchor()?
        } else {
            None
        };
        let (head_sequence, _) = self.head()?;
        if head_sequence == 0 {
            if anchor.as_ref().is_some_and(|anchor| anchor.sequence != 0) {
                return Err(StoreError::Verification(
                    "secure anchor is ahead of an empty journal".into(),
                ));
            }
            self.ensure_stream_events_index()?;
            *self.startup_report.lock().map_err(adapter_error)? = StartupVerificationReport {
                configured_mode: mode,
                path: "empty".into(),
                verified_from_sequence: None,
                verified_through_sequence: 0,
                verified_event_count: 0,
                anchor_format_version: anchor.map(|anchor| anchor.format_version),
            };
            return Ok(());
        }

        if self.payload_protection == JournalPayloadProtection::Plaintext
            && mode == StartupVerificationMode::Incremental
        {
            let report = self.verify_plaintext_incremental()?;
            *self.startup_report.lock().map_err(adapter_error)? = report;
            return Ok(());
        }

        let trusted_incremental = mode == StartupVerificationMode::Incremental
            && anchor.as_ref().is_some_and(|anchor| {
                anchor.format_version == SECURE_ANCHOR_FORMAT_VERSION
                    && anchor.verification_profile.as_deref()
                        == Some(INCREMENTAL_VERIFICATION_PROFILE)
                    && anchor.status == SecureAnchorStatus::Verified
            })
            && self.stream_events_index_version()? == Some(STREAM_EVENTS_INDEX_VERSION);
        if trusted_incremental {
            match self.verify_incremental(anchor.as_ref().expect("checked anchor")) {
                Ok(report) => {
                    *self.startup_report.lock().map_err(adapter_error)? = report;
                    return Ok(());
                }
                // An interrupted anchor-before-checkpoint commit is safe to repair only
                // after the complete journal still verifies against that anchor.
                Err(StoreError::Verification(_)) => {}
                Err(error) => return Err(error),
            }
        }

        let report = self.verify_inner()?;
        self.ensure_stream_events_index()?;
        if report.last_sequence > 0
            && self.payload_protection == JournalPayloadProtection::Encrypted
        {
            self.checkpoint()?;
        }
        *self.startup_report.lock().map_err(adapter_error)? = StartupVerificationReport {
            configured_mode: mode,
            path: if mode == StartupVerificationMode::Full {
                "full".into()
            } else {
                "bootstrap_full".into()
            },
            verified_from_sequence: Some(1),
            verified_through_sequence: report.last_sequence,
            verified_event_count: report.event_count,
            anchor_format_version: (report.last_sequence > 0
                && self.payload_protection == JournalPayloadProtection::Encrypted)
                .then_some(SECURE_ANCHOR_FORMAT_VERSION),
        };
        Ok(())
    }

    pub(super) fn verify_plaintext_incremental(
        &self,
    ) -> Result<StartupVerificationReport, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let events = read.open_table(EVENTS).map_err(adapter_error)?;
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let outbox = read.open_table(OUTBOX).map_err(adapter_error)?;
        let projection_positions = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        let head_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
        let head_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| decode_journal_json(value.value()),
            )?;
        if metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .is_some()
        {
            return Err(StoreError::Verification(
                "plaintext journal contains a signed checkpoint".into(),
            ));
        }
        if metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?
            != Some(STREAM_EVENTS_INDEX_VERSION)
        {
            return Err(StoreError::Verification(
                "plaintext journal stream index is unavailable".into(),
            ));
        }
        if events.len().map_err(adapter_error)? != head_sequence
            || outbox.len().map_err(adapter_error)? != head_sequence
            || stream_events.len().map_err(adapter_error)? != head_sequence
        {
            return Err(StoreError::Verification(
                "plaintext journal local indexes do not match its head".into(),
            ));
        }
        if events.get(0).map_err(adapter_error)?.is_some()
            || outbox.get(0).map_err(adapter_error)?.is_some()
            || events
                .range(head_sequence.saturating_add(1)..)
                .map_err(adapter_error)?
                .next()
                .transpose()
                .map_err(adapter_error)?
                .is_some()
            || outbox
                .range(head_sequence.saturating_add(1)..)
                .map_err(adapter_error)?
                .next()
                .transpose()
                .map_err(adapter_error)?
                .is_some()
        {
            return Err(StoreError::Verification(
                "plaintext journal contains records outside its declared sequence range".into(),
            ));
        }
        let bytes = events
            .get(head_sequence)
            .map_err(adapter_error)?
            .ok_or_else(|| StoreError::Verification("plaintext journal head is absent".into()))?;
        let persisted: PersistedEventEnvelope = decode_journal_json(bytes.value())?;
        let envelope: EventEnvelope = decode_journal_json(bytes.value())?;
        if envelope.global_sequence != head_sequence || envelope.record_hash != head_hash {
            return Err(StoreError::Verification(
                "plaintext journal head metadata does not match its record".into(),
            ));
        }
        self.verify_persisted_event(&envelope, &persisted)?;
        if stream_events
            .get(&(envelope.stream_id.as_str(), envelope.stream_version))
            .map_err(adapter_error)?
            .map(|value| value.value())
            != Some(head_sequence)
            || stream_versions
                .get(envelope.stream_id.as_str())
                .map_err(adapter_error)?
                .map(|value| value.value())
                != Some(envelope.stream_version)
        {
            return Err(StoreError::Verification(
                "plaintext journal head stream index is inconsistent".into(),
            ));
        }
        let queued = outbox
            .get(head_sequence)
            .map_err(adapter_error)?
            .ok_or_else(|| {
                StoreError::Verification("plaintext journal head outbox is absent".into())
            })?;
        let queued: Value = decode_journal_json(queued.value())?;
        if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id)
            || queued.get("global_sequence").and_then(Value::as_u64)
                != Some(envelope.global_sequence)
        {
            return Err(StoreError::Verification(
                "plaintext journal head outbox targets a different event".into(),
            ));
        }
        for entry in projection_positions.iter().map_err(adapter_error)? {
            let (projection, position) = entry.map_err(adapter_error)?;
            projection_prefix(projection.value()).map_err(|_| {
                StoreError::Verification("stored projection name is invalid".into())
            })?;
            if position.value() > head_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {} is ahead of plaintext journal head",
                    projection.value()
                )));
            }
        }
        Ok(StartupVerificationReport {
            configured_mode: StartupVerificationMode::Incremental,
            path: "local_integrity".into(),
            verified_from_sequence: Some(head_sequence),
            verified_through_sequence: head_sequence,
            verified_event_count: 1,
            anchor_format_version: None,
        })
    }

    pub(super) fn verify_incremental(
        &self,
        anchor: &SecureAnchor,
    ) -> Result<StartupVerificationReport, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let events = read.open_table(EVENTS).map_err(adapter_error)?;
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let outbox = read.open_table(OUTBOX).map_err(adapter_error)?;
        let projection_positions = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        let head_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
        let head_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| decode_journal_json(value.value()),
            )?;
        let checkpoint: SignedCheckpoint = metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?
            .ok_or_else(|| {
                StoreError::Verification("incremental startup requires a signed checkpoint".into())
            })?;
        if checkpoint.global_sequence != anchor.sequence
            || checkpoint.record_hash != anchor.hash
            || checkpoint.global_sequence > head_sequence
        {
            return Err(StoreError::Verification(
                "secure anchor and signed checkpoint do not identify one journal boundary".into(),
            ));
        }
        self.verify_checkpoint_signature(&checkpoint)?;

        let mut expected_sequence = checkpoint.global_sequence;
        let mut previous_hash = checkpoint.record_hash.clone();
        let mut inspected = 0_u64;
        let mut touched_streams = BTreeMap::<String, u64>::new();
        let boundary_start = checkpoint.global_sequence.max(1);
        for entry in events.range(boundary_start..).map_err(adapter_error)? {
            let (key, value) = entry.map_err(adapter_error)?;
            let sequence = key.value();
            if inspected == 0 && sequence == checkpoint.global_sequence {
                let persisted: PersistedEventEnvelope = decode_journal_json(value.value())?;
                let envelope: EventEnvelope = decode_journal_json(value.value())?;
                if envelope.global_sequence != sequence {
                    return Err(StoreError::Verification(
                        "checkpoint event sequence does not match its journal key".into(),
                    ));
                }
                self.verify_persisted_event(&envelope, &persisted)?;
                if envelope.record_hash != checkpoint.record_hash {
                    return Err(StoreError::Verification(
                        "checkpoint record is absent or has changed".into(),
                    ));
                }
                if stream_events
                    .get(&(envelope.stream_id.as_str(), envelope.stream_version))
                    .map_err(adapter_error)?
                    .map(|indexed| indexed.value())
                    != Some(sequence)
                {
                    return Err(StoreError::Verification(format!(
                        "stream {} version {} checkpoint index mismatch",
                        envelope.stream_id, envelope.stream_version
                    )));
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
                touched_streams.insert(envelope.stream_id, envelope.stream_version);
                inspected = inspected.saturating_add(1);
                continue;
            }
            expected_sequence = expected_sequence.saturating_add(1);
            if sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "incremental journal sequence gap: expected {expected_sequence}, got {sequence}"
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
            self.verify_persisted_event(&envelope, &persisted)?;
            if stream_events
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
            previous_hash.clone_from(&envelope.record_hash);
            touched_streams.insert(envelope.stream_id, envelope.stream_version);
            inspected = inspected.saturating_add(1);
        }
        if inspected == 0 {
            return Err(StoreError::Verification(
                "checkpoint record is absent from the journal".into(),
            ));
        }
        if expected_sequence != head_sequence || previous_hash != head_hash {
            return Err(StoreError::Verification(
                "incremental verification did not reach the journal head".into(),
            ));
        }
        for (stream_id, version) in touched_streams {
            if stream_versions
                .get(stream_id.as_str())
                .map_err(adapter_error)?
                .map(|stored| stored.value())
                != Some(version)
            {
                return Err(StoreError::Verification(format!(
                    "durable stream version for {stream_id} does not match the verified tail"
                )));
            }
        }
        for entry in projection_positions.iter().map_err(adapter_error)? {
            let (projection, position) = entry.map_err(adapter_error)?;
            projection_prefix(projection.value()).map_err(|_| {
                StoreError::Verification("stored projection name is invalid".into())
            })?;
            if position.value() > head_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {} position {} is ahead of journal head {head_sequence}",
                    projection.value(),
                    position.value()
                )));
            }
        }
        drop(projection_positions);
        drop(outbox);
        drop(metadata);
        drop(stream_versions);
        drop(stream_events);
        drop(events);
        drop(read);
        if head_sequence > checkpoint.global_sequence {
            self.checkpoint()?;
        }
        Ok(StartupVerificationReport {
            configured_mode: StartupVerificationMode::Incremental,
            path: "incremental".into(),
            verified_from_sequence: Some(boundary_start),
            verified_through_sequence: head_sequence,
            verified_event_count: inspected,
            anchor_format_version: Some(anchor.format_version),
        })
    }
}
