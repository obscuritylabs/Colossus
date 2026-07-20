use super::*;

/// Canonical redb journal adapter.
pub struct RedbEventJournal {
    database: Database,
    keys: Arc<dyn KeyProvider>,
    signer: Arc<dyn CheckpointSigner>,
    writer: Mutex<()>,
    last_checkpoint: Mutex<Instant>,
    recovery_mode: AtomicBool,
    recovery_reason: Mutex<Option<String>>,
}

impl RedbEventJournal {
    /// Open or create a journal, then verify it before enabling writes.
    pub fn open(
        path: impl AsRef<Path>,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        let database = Database::create(path).map_err(adapter_error)?;
        let write = database.begin_write().map_err(adapter_error)?;
        write.open_table(EVENTS).map_err(adapter_error)?;
        write.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        write.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        write.open_table(METADATA).map_err(adapter_error)?;
        write.open_table(OUTBOX).map_err(adapter_error)?;
        write
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        write
            .open_table(PROJECTION_RECORDS)
            .map_err(adapter_error)?;
        write.commit().map_err(adapter_error)?;
        let journal = Self {
            database,
            keys,
            signer,
            writer: Mutex::new(()),
            last_checkpoint: Mutex::new(Instant::now()),
            recovery_mode: AtomicBool::new(false),
            recovery_reason: Mutex::new(None),
        };
        let startup = journal
            .verify_inner()
            .and_then(|_| journal.ensure_stream_events_index())
            .and_then(|_| journal.verify_inner())
            .and_then(|report| {
                let checkpoint_sequence = report
                    .checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.global_sequence);
                if report.last_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL {
                    journal.checkpoint()?;
                }
                Ok(())
            });
        if let Err(error) = startup {
            journal.recovery_mode.store(true, Ordering::Release);
            *journal.recovery_reason.lock().map_err(adapter_error)? = Some(error.to_string());
        }
        Ok(journal)
    }

    /// Bounded reason startup entered recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        Ok(self.recovery_reason.lock().map_err(adapter_error)?.clone())
    }

    fn ensure_stream_events_index(&self) -> Result<(), StoreError> {
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        let index_version = {
            let metadata = write.open_table(METADATA).map_err(adapter_error)?;
            metadata
                .get(STREAM_EVENTS_INDEX_KEY)
                .map_err(adapter_error)?
                .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
                .transpose()?
        };
        if index_version == Some(STREAM_EVENTS_INDEX_VERSION) {
            return Ok(());
        }
        if index_version.is_some() {
            return Err(StoreError::Verification(
                "stream event index version is unsupported".into(),
            ));
        }
        {
            let event_table = write.open_table(EVENTS).map_err(adapter_error)?;
            let mut stream_events = write.open_table(STREAM_EVENTS).map_err(adapter_error)?;
            if !stream_events.is_empty().map_err(adapter_error)? {
                return Err(StoreError::Verification(
                    "unversioned stream event index is not empty".into(),
                ));
            }
            for entry in event_table.iter().map_err(adapter_error)? {
                let (sequence, value) = entry.map_err(adapter_error)?;
                let sequence = sequence.value();
                let event: EventEnvelope =
                    serde_json::from_slice(value.value()).map_err(adapter_error)?;
                if event.global_sequence != sequence {
                    return Err(StoreError::Verification(format!(
                        "event {} global sequence does not match its key",
                        event.event_id
                    )));
                }
                if stream_events
                    .insert(&(event.stream_id.as_str(), event.stream_version), &sequence)
                    .map_err(adapter_error)?
                    .is_some()
                {
                    return Err(StoreError::Verification(format!(
                        "stream {} has duplicate version {}",
                        event.stream_id, event.stream_version
                    )));
                }
            }
        }
        {
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            let version =
                serde_json::to_vec(&STREAM_EVENTS_INDEX_VERSION).map_err(adapter_error)?;
            metadata
                .insert(STREAM_EVENTS_INDEX_KEY, version.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }

    fn read_indexed_stream(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: Option<usize>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        if limit == Some(0) || after_version == u64::MAX {
            return Ok(Vec::new());
        }
        let start_version = after_version.saturating_add(1);
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?;
        if index_version != Some(STREAM_EVENTS_INDEX_VERSION) {
            return Err(StoreError::Verification(
                "stream event index is unavailable".into(),
            ));
        }
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit.unwrap_or(0).min(MAX_STREAM_READ_BATCH));
        for entry in stream_events
            .range((stream_id, start_version)..=(stream_id, u64::MAX))
            .map_err(adapter_error)?
        {
            if limit.is_some_and(|limit| events.len() >= limit) {
                break;
            }
            let (key, sequence) = entry.map_err(adapter_error)?;
            let (indexed_stream, indexed_version) = key.value();
            let sequence = sequence.value();
            let persisted = event_table
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "stream event index references absent event {sequence}"
                    ))
                })?;
            let event: EventEnvelope =
                serde_json::from_slice(persisted.value()).map_err(adapter_error)?;
            if event.global_sequence != sequence
                || event.stream_id != indexed_stream
                || event.stream_version != indexed_version
            {
                return Err(StoreError::Verification(format!(
                    "stream event index entry {indexed_stream}/{indexed_version} is invalid"
                )));
            }
            events.push(event);
        }
        Ok(events)
    }

    fn read_indexed_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 || before_version.is_some_and(|version| version <= 1) {
            return Ok(Vec::new());
        }
        let last_version = before_version.map_or(u64::MAX, |version| version.saturating_sub(1));
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?;
        if index_version != Some(STREAM_EVENTS_INDEX_VERSION) {
            return Err(StoreError::Verification(
                "stream event index is unavailable".into(),
            ));
        }
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit);
        for entry in stream_events
            .range((stream_id, 1)..=(stream_id, last_version))
            .map_err(adapter_error)?
            .rev()
        {
            if events.len() >= limit {
                break;
            }
            let (key, sequence) = entry.map_err(adapter_error)?;
            let (indexed_stream, indexed_version) = key.value();
            let sequence = sequence.value();
            let persisted = event_table
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "stream event index references absent event {sequence}"
                    ))
                })?;
            let event: EventEnvelope =
                serde_json::from_slice(persisted.value()).map_err(adapter_error)?;
            if event.global_sequence != sequence
                || event.stream_id != indexed_stream
                || event.stream_version != indexed_version
            {
                return Err(StoreError::Verification(format!(
                    "stream event index entry {indexed_stream}/{indexed_version} is invalid"
                )));
            }
            events.push(event);
        }
        Ok(events)
    }

    fn encrypt_payload(
        &self,
        envelope: &EventEnvelope,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, StoreError> {
        let (key_id, key) = self.keys.active_key()?;
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).map_err(adapter_error)?;
        let aad = serde_json::to_vec(&associated_data(envelope)).map_err(adapter_error)?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(adapter_error)?;
        Ok(EncryptedPayload {
            key_id,
            algorithm: "XChaCha20-Poly1305".into(),
            nonce: hex::encode(nonce),
            ciphertext: hex::encode(ciphertext),
            plaintext_hash: sha256_hex(plaintext),
        })
    }

    fn append_locked(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
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
                .map_or(Ok(0_u64), |value| {
                    serde_json::from_slice(value.value()).map_err(adapter_error)
                })?;
            let mut previous_hash = metadata
                .get("last_hash")
                .map_err(adapter_error)?
                .map_or_else(
                    || Ok::<String, StoreError>(ZERO_HASH.into()),
                    |value| serde_json::from_slice(value.value()).map_err(adapter_error),
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

                sequence = sequence.saturating_add(1);
                let stream_version = durable_version.saturating_add(1);
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

    fn decrypt_persisted(
        &self,
        event: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Vec<u8>, StoreError> {
        if event.payload.algorithm != "XChaCha20-Poly1305" {
            return Err(StoreError::Verification(format!(
                "unsupported payload algorithm {}",
                event.payload.algorithm
            )));
        }
        let key = self.keys.key_by_id(&event.payload.key_id)?;
        let nonce = hex::decode(&event.payload.nonce).map_err(adapter_error)?;
        let nonce: [u8; 24] = nonce
            .try_into()
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce length".into()))?;
        let ciphertext = hex::decode(&event.payload.ciphertext).map_err(adapter_error)?;
        let aad =
            serde_json::to_vec(&persisted_associated_data(persisted)).map_err(adapter_error)?;
        XChaCha20Poly1305::new((&key).into())
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                StoreError::Verification(format!(
                    "event {} payload authentication failed",
                    event.event_id
                ))
            })
    }

    fn load_persisted(&self, event: &EventEnvelope) -> Result<PersistedEventEnvelope, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let bytes = table
            .get(event.global_sequence)
            .map_err(adapter_error)?
            .ok_or_else(|| {
                StoreError::Verification(format!(
                    "event {} is absent from the journal",
                    event.event_id
                ))
            })?
            .value()
            .to_vec();
        let stored: EventEnvelope = serde_json::from_slice(&bytes).map_err(adapter_error)?;
        if stored != *event {
            return Err(StoreError::Verification(format!(
                "event {} does not match its persisted envelope",
                event.event_id
            )));
        }
        serde_json::from_slice(&bytes).map_err(adapter_error)
    }

    fn checkpoint_sequence(&self) -> Result<u64, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| {
                serde_json::from_slice::<SignedCheckpoint>(value.value())
                    .map(|checkpoint| checkpoint.global_sequence)
                    .map_err(adapter_error)
            })
            .transpose()
            .map(|sequence| sequence.unwrap_or(0))
    }

    fn verify_checkpoint(
        &self,
        checkpoint: &SignedCheckpoint,
        event_hashes: &BTreeMap<u64, String>,
    ) -> Result<(), StoreError> {
        if checkpoint.algorithm != "Ed25519" || checkpoint.key_id != self.signer.key_id() {
            return Err(StoreError::Verification(
                "checkpoint signer identity or algorithm mismatch".into(),
            ));
        }
        if event_hashes.get(&checkpoint.global_sequence) != Some(&checkpoint.record_hash) {
            return Err(StoreError::Verification(
                "checkpoint does not match journal record".into(),
            ));
        }
        let signature = hex::decode(&checkpoint.signature).map_err(adapter_error)?;
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &signature,
        )
    }

    fn verify_inner(&self) -> Result<VerificationReport, StoreError> {
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
        let mut event_hashes = BTreeMap::<u64, String>::new();
        let stream_index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
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
            let persisted: PersistedEventEnvelope =
                serde_json::from_slice(value.value()).map_err(adapter_error)?;
            let envelope: EventEnvelope =
                serde_json::from_slice(value.value()).map_err(adapter_error)?;
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
            let computed_hash = persisted_record_hash(&persisted)?;
            if computed_hash != persisted.record_hash
                || envelope.record_hash != persisted.record_hash
            {
                return Err(StoreError::Verification(format!(
                    "event {} record hash mismatch",
                    envelope.event_id
                )));
            }
            let plaintext = self.decrypt_persisted(&envelope, &persisted)?;
            if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
                return Err(StoreError::Verification(format!(
                    "event {} plaintext hash mismatch",
                    envelope.event_id
                )));
            }
            serde_json::from_slice::<Value>(&plaintext).map_err(adapter_error)?;
            previous_hash.clone_from(&envelope.record_hash);
            event_hashes.insert(sequence, envelope.record_hash);
            let queued = outbox
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {sequence} is absent"
                    ))
                })?;
            let queued: Value = serde_json::from_slice(queued.value()).map_err(adapter_error)?;
            if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id) {
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
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let metadata_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
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
            projection_prefix(projection.value())?;
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
            projection_record_key(projection, record_key)?;
            serde_json::from_slice::<Value>(value.value()).map_err(|error| {
                StoreError::Verification(format!(
                    "projection record {} is invalid JSON: {error}",
                    key.value()
                ))
            })?;
        }

        let checkpoint = metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?;
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &event_hashes)?;
        }
        if let Some((anchor_sequence, anchor_hash)) = self.keys.load_anchor()?
            && event_hashes.get(&anchor_sequence) != Some(&anchor_hash)
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
            self.append_locked(events)?
        };
        let checkpoint_sequence = if persisted.is_empty() {
            0
        } else {
            self.checkpoint_sequence()?
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
        self.read_indexed_stream(stream_id, 0, None)
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.read_indexed_stream(
            stream_id,
            after_version,
            Some(limit.min(MAX_STREAM_READ_BATCH)),
        )
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.read_indexed_stream_backwards(stream_id, before_version, limit)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit.min(1024));
        for entry in table.range(from_sequence..).map_err(adapter_error)? {
            if events.len() >= limit {
                break;
            }
            let (_, value) = entry.map_err(adapter_error)?;
            events.push(serde_json::from_slice(value.value()).map_err(adapter_error)?);
        }
        Ok(events)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(OUTBOX).map_err(adapter_error)?;
        let mut work = Vec::with_capacity(limit.min(1024));
        for entry in table.range(from_sequence..).map_err(adapter_error)? {
            if work.len() >= limit {
                break;
            }
            let (sequence, value) = entry.map_err(adapter_error)?;
            let record: Value = serde_json::from_slice(value.value()).map_err(adapter_error)?;
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
        }
        Ok(work)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
            )?;
        Ok((sequence, hash))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let persisted = self.load_persisted(event)?;
        serde_json::from_slice(&self.decrypt_persisted(event, &persisted)?).map_err(adapter_error)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.verify_inner()
    }

    fn is_recovery_mode(&self) -> bool {
        self.recovery_mode.load(Ordering::Acquire)
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        if sequence == 0 {
            return Ok(None);
        }
        let hash: String = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()?
            .ok_or_else(|| StoreError::Verification("journal head hash is absent".into()))?;
        drop(metadata);
        drop(read);
        let signature = self.signer.sign(&checkpoint_message(sequence, &hash))?;
        let checkpoint = SignedCheckpoint {
            global_sequence: sequence,
            record_hash: hash.clone(),
            key_id: self.signer.key_id().to_owned(),
            algorithm: "Ed25519".into(),
            signature: hex::encode(signature),
            created_at: utc_now()?,
        };
        self.keys.store_anchor(sequence, &hash)?;
        #[cfg(test)]
        crash_at_test_fault("after_anchor_before_checkpoint_commit");
        let bytes = serde_json::to_vec(&checkpoint).map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            metadata
                .insert("latest_checkpoint", bytes.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)?;
        *self.last_checkpoint.lock().map_err(adapter_error)? = Instant::now();
        Ok(Some(checkpoint))
    }
}

impl ProjectionStore for RedbEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        projection_prefix(projection)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read
            .open_table(PROJECTION_POSITIONS)
            .map_err(adapter_error)?;
        Ok(table
            .get(projection)
            .map_err(adapter_error)?
            .map_or(0, |position| position.value()))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        let namespaced = projection_record_key(projection, key)?;
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        table
            .get(namespaced.as_str())
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        let namespace = projection_prefix(projection)?;
        if key_prefix.contains('\0') {
            return Err(StoreError::Adapter(
                "projection key prefix may not contain NUL".into(),
            ));
        }
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(PROJECTION_RECORDS).map_err(adapter_error)?;
        let mut records = Vec::with_capacity(limit.min(1024));
        for entry in table.iter().map_err(adapter_error)? {
            if records.len() >= limit {
                break;
            }
            let (stored_key, value) = entry.map_err(adapter_error)?;
            let Some(key) = stored_key.value().strip_prefix(&namespace) else {
                continue;
            };
            if !key.starts_with(key_prefix) {
                continue;
            }
            records.push((
                key.to_owned(),
                serde_json::from_slice(value.value()).map_err(adapter_error)?,
            ));
        }
        Ok(records)
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        projection_prefix(&batch.projection)?;
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        let encoded = batch
            .mutations
            .into_iter()
            .map(|mutation| match mutation {
                ProjectionMutation::Upsert { key, value } => Ok((
                    projection_record_key(&batch.projection, &key)?,
                    Some(serde_json::to_vec(&value).map_err(adapter_error)?),
                )),
                ProjectionMutation::Delete { key } => {
                    Ok((projection_record_key(&batch.projection, &key)?, None))
                }
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            let actual = positions
                .get(batch.projection.as_str())
                .map_err(adapter_error)?
                .map_or(0, |position| position.value());
            if actual != batch.expected_position {
                return Err(StoreError::Conflict {
                    stream_id: format!("projection:{}", batch.projection),
                    expected: batch.expected_position,
                    actual,
                });
            }
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            for (key, value) in &encoded {
                if let Some(value) = value {
                    records
                        .insert(key.as_str(), value.as_slice())
                        .map_err(adapter_error)?;
                } else {
                    records.remove(key.as_str()).map_err(adapter_error)?;
                }
            }
            positions
                .insert(batch.projection.as_str(), batch.through_sequence)
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let namespace = projection_prefix(projection)?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            let mut keys = Vec::new();
            for entry in records.iter().map_err(adapter_error)? {
                let (key, _) = entry.map_err(adapter_error)?;
                if key.value().starts_with(&namespace) {
                    keys.push(key.value().to_owned());
                }
            }
            for key in keys {
                records.remove(key.as_str()).map_err(adapter_error)?;
            }
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            positions.remove(projection).map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)
    }
}
