use super::*;

/// Canonical redb journal adapter.
pub struct RedbEventJournal {
    database: Database,
    payload_protection: JournalPayloadProtection,
    keys: Arc<dyn KeyProvider>,
    signer: Arc<dyn CheckpointSigner>,
    writer: Mutex<()>,
    last_checkpoint: Mutex<Instant>,
    recovery_mode: AtomicBool,
    recovery_reason: Mutex<Option<String>>,
    startup_report: Mutex<StartupVerificationReport>,
}

impl RedbEventJournal {
    /// Open a fresh process-local journal backed only by memory.
    pub fn open_in_memory(
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        Self::open_in_memory_with_startup_verification(
            keys,
            signer,
            StartupVerificationMode::Incremental,
        )
    }

    /// Open a fresh process-local journal with one explicit startup verification policy.
    pub fn open_in_memory_with_startup_verification(
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        if keys.payload_protection() != JournalPayloadProtection::Plaintext {
            return Err(StoreError::Adapter(
                "an in-memory redb journal requires plaintext payload protection".into(),
            ));
        }
        let database = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(adapter_error)?;
        Self::open_database(database, keys, signer, mode)
    }

    /// Open or create a journal, then verify it before enabling writes.
    pub fn open(
        path: impl AsRef<Path>,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        Self::open_with_startup_verification(
            path,
            keys,
            signer,
            StartupVerificationMode::Incremental,
        )
    }

    /// Open with one explicit startup verification policy.
    pub fn open_with_startup_verification(
        path: impl AsRef<Path>,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        let database = Database::create(path).map_err(adapter_error)?;
        Self::open_database(database, keys, signer, mode)
    }

    /// Open from an already no-follow, owner-validated read/write file.
    pub fn open_file_with_startup_verification(
        file: File,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        let database = Database::builder()
            .create_file(file)
            .map_err(adapter_error)?;
        Self::open_database(database, keys, signer, mode)
    }

    fn open_database(
        database: Database,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        Self::ensure_schema(&database)?;
        let payload_protection = keys.payload_protection();
        Self::initialize_payload_protection(&database, payload_protection)?;
        let journal = Self {
            database,
            payload_protection,
            keys,
            signer,
            writer: Mutex::new(()),
            last_checkpoint: Mutex::new(Instant::now()),
            recovery_mode: AtomicBool::new(false),
            recovery_reason: Mutex::new(None),
            startup_report: Mutex::new(StartupVerificationReport {
                configured_mode: mode,
                path: "empty".into(),
                verified_from_sequence: None,
                verified_through_sequence: 0,
                verified_event_count: 0,
                anchor_format_version: None,
            }),
        };
        let startup = journal.quarantine_result(journal.verify_startup(mode));
        if let Err(error) = startup {
            journal.recovery_mode.store(true, Ordering::Release);
            *journal.recovery_reason.lock().map_err(adapter_error)? = Some(error.to_string());
        }
        Ok(journal)
    }

    pub(super) fn ensure_schema(database: &Database) -> Result<bool, StoreError> {
        let read = database.begin_read().map_err(adapter_error)?;
        let established = Self::established_schema(&read)?;
        drop(read);
        if established {
            return Ok(false);
        }

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
        Ok(true)
    }

    /// Report whether every required table already exists with its expected typed definition.
    ///
    /// A missing table means the schema still has to be created, while an incompatible
    /// key/value definition is rejected here instead of surfacing during a later operation.
    fn established_schema(read: &ReadTransaction) -> Result<bool, StoreError> {
        macro_rules! established_table {
            ($definition:expr) => {
                match read.open_table($definition) {
                    Ok(_) => {}
                    Err(TableError::TableDoesNotExist(_)) => return Ok(false),
                    Err(error) => return Err(adapter_error(error)),
                }
            };
        }

        established_table!(EVENTS);
        established_table!(STREAM_EVENTS);
        established_table!(STREAM_VERSIONS);
        established_table!(METADATA);
        established_table!(OUTBOX);
        established_table!(PROJECTION_POSITIONS);
        established_table!(PROJECTION_RECORDS);
        Ok(true)
    }

    /// Bounded reason startup entered recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        Ok(self.recovery_reason.lock().map_err(adapter_error)?.clone())
    }

    /// Stable metadata describing the startup verification path.
    pub fn startup_verification_report(&self) -> Result<StartupVerificationReport, StoreError> {
        Ok(self.startup_report.lock().map_err(adapter_error)?.clone())
    }

    fn initialize_payload_protection(
        database: &Database,
        configured: JournalPayloadProtection,
    ) -> Result<(), StoreError> {
        let read = database.begin_read().map_err(adapter_error)?;
        let events = read.open_table(EVENTS).map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let marker = metadata
            .get(PAYLOAD_PROTECTION_KEY)
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice::<String>(value.value()).map_err(adapter_error))
            .transpose()?;
        let head_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let nonempty = head_sequence > 0 || !events.is_empty().map_err(adapter_error)?;
        let effective = match marker.as_deref() {
            Some("encrypted") => JournalPayloadProtection::Encrypted,
            Some("plaintext") => JournalPayloadProtection::Plaintext,
            Some(_) => {
                return Err(StoreError::Verification(
                    "journal payload-protection marker is unsupported".into(),
                ));
            }
            None if nonempty => JournalPayloadProtection::Encrypted,
            None => configured,
        };
        drop(metadata);
        drop(events);
        drop(read);
        if effective != configured {
            return Err(StoreError::Verification(format!(
                "journal payload protection is {}, but configuration requests {}; use a fresh storage path because in-place protection changes are unsupported",
                effective.as_str(),
                configured.as_str()
            )));
        }
        if marker.is_none() {
            let bytes = serde_json::to_vec(configured.as_str()).map_err(adapter_error)?;
            let write = database.begin_write().map_err(adapter_error)?;
            {
                let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
                metadata
                    .insert(PAYLOAD_PROTECTION_KEY, bytes.as_slice())
                    .map_err(adapter_error)?;
            }
            write.commit().map_err(adapter_error)?;
        }
        Ok(())
    }

    fn verify_startup(&self, mode: StartupVerificationMode) -> Result<(), StoreError> {
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

    fn verify_plaintext_incremental(&self) -> Result<StartupVerificationReport, StoreError> {
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
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let head_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
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
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
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
        let persisted: PersistedEventEnvelope =
            serde_json::from_slice(bytes.value()).map_err(adapter_error)?;
        let envelope: EventEnvelope =
            serde_json::from_slice(bytes.value()).map_err(adapter_error)?;
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
        let queued: Value = serde_json::from_slice(queued.value()).map_err(adapter_error)?;
        if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id) {
            return Err(StoreError::Verification(
                "plaintext journal head outbox targets a different event".into(),
            ));
        }
        for entry in projection_positions.iter().map_err(adapter_error)? {
            let (projection, position) = entry.map_err(adapter_error)?;
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

    fn verify_incremental(
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
            .map_or(Ok(0_u64), |value| {
                serde_json::from_slice(value.value()).map_err(adapter_error)
            })?;
        let head_hash = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map_or_else(
                || Ok::<String, StoreError>(ZERO_HASH.into()),
                |value| serde_json::from_slice(value.value()).map_err(adapter_error),
            )?;
        let checkpoint: SignedCheckpoint = metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
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
                let persisted: PersistedEventEnvelope =
                    serde_json::from_slice(value.value()).map_err(adapter_error)?;
                let envelope: EventEnvelope =
                    serde_json::from_slice(value.value()).map_err(adapter_error)?;
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
                let queued: Value =
                    serde_json::from_slice(queued.value()).map_err(adapter_error)?;
                if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id) {
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
            let queued: Value = serde_json::from_slice(queued.value()).map_err(adapter_error)?;
            if queued.get("event_id").and_then(Value::as_str) != Some(&envelope.event_id) {
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
            projection_prefix(projection.value())?;
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

    fn stream_events_index_version(&self) -> Result<Option<u64>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| serde_json::from_slice(value.value()).map_err(adapter_error))
            .transpose()
    }

    fn ensure_stream_events_index(&self) -> Result<bool, StoreError> {
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
            return Ok(false);
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
        write.commit().map_err(adapter_error)?;
        Ok(true)
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
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
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
        let mut expected_version = start_version;
        for event in &events {
            if event.stream_version != expected_version {
                return Err(StoreError::Verification(format!(
                    "stream {stream_id} index has a version gap at {expected_version}"
                )));
            }
            expected_version = expected_version.saturating_add(1);
        }
        let durable_version = stream_versions
            .get(stream_id)
            .map_err(adapter_error)?
            .map_or(0, |version| version.value());
        if limit.is_none_or(|limit| events.len() < limit)
            && events
                .last()
                .map_or(after_version.min(durable_version), |event| {
                    event.stream_version
                })
                != durable_version
        {
            return Err(StoreError::Verification(format!(
                "stream {stream_id} index does not reach durable version {durable_version}"
            )));
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
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
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
        let durable_version = stream_versions
            .get(stream_id)
            .map_err(adapter_error)?
            .map_or(0, |version| version.value());
        let expected_first = before_version.map_or(durable_version, |version| {
            version.saturating_sub(1).min(durable_version)
        });
        if events.first().map(|event| event.stream_version)
            != (expected_first > 0).then_some(expected_first)
        {
            return Err(StoreError::Verification(format!(
                "stream {stream_id} reverse index does not begin at version {expected_first}"
            )));
        }
        for pair in events.windows(2) {
            if pair[0].stream_version != pair[1].stream_version.saturating_add(1) {
                return Err(StoreError::Verification(format!(
                    "stream {stream_id} reverse index has a version gap"
                )));
            }
        }
        Ok(events)
    }

    fn encrypt_payload(
        &self,
        envelope: &EventEnvelope,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, StoreError> {
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(EncryptedPayload {
                key_id: "none".into(),
                algorithm: PLAINTEXT_PAYLOAD_ALGORITHM.into(),
                nonce: String::new(),
                ciphertext: hex::encode(plaintext),
                plaintext_hash: sha256_hex(plaintext),
            });
        }
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
            algorithm: ENCRYPTED_PAYLOAD_ALGORITHM.into(),
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
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            if event.payload.algorithm != PLAINTEXT_PAYLOAD_ALGORITHM
                || event.payload.key_id != "none"
                || !event.payload.nonce.is_empty()
            {
                return Err(StoreError::Verification(format!(
                    "event {} payload does not match plaintext journal protection",
                    event.event_id
                )));
            }
            return hex::decode(&event.payload.ciphertext).map_err(|_| {
                StoreError::Verification(format!(
                    "event {} plaintext payload encoding is invalid",
                    event.event_id
                ))
            });
        }
        if event.payload.algorithm != ENCRYPTED_PAYLOAD_ALGORITHM {
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
        if event_hashes.get(&checkpoint.global_sequence) != Some(&checkpoint.record_hash) {
            return Err(StoreError::Verification(
                "checkpoint does not match journal record".into(),
            ));
        }
        self.verify_checkpoint_signature(checkpoint)
    }

    fn verify_checkpoint_signature(&self, checkpoint: &SignedCheckpoint) -> Result<(), StoreError> {
        if checkpoint.algorithm != "Ed25519" || checkpoint.key_id != self.signer.key_id() {
            return Err(StoreError::Verification(
                "checkpoint signer identity or algorithm mismatch".into(),
            ));
        }
        let signature = hex::decode(&checkpoint.signature).map_err(adapter_error)?;
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &signature,
        )
    }

    fn verify_persisted_event(
        &self,
        envelope: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Vec<u8>, StoreError> {
        let computed_hash = persisted_record_hash(persisted)?;
        if computed_hash != persisted.record_hash || envelope.record_hash != persisted.record_hash {
            return Err(StoreError::Verification(format!(
                "event {} record hash mismatch",
                envelope.event_id
            )));
        }
        let plaintext = self.decrypt_persisted(envelope, persisted)?;
        if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
            return Err(StoreError::Verification(format!(
                "event {} plaintext hash mismatch",
                envelope.event_id
            )));
        }
        serde_json::from_slice::<Value>(&plaintext).map_err(adapter_error)?;
        Ok(plaintext)
    }

    fn quarantine_result<T>(&self, result: Result<T, StoreError>) -> Result<T, StoreError> {
        if let Err(StoreError::Verification(reason)) = &result {
            self.recovery_mode.store(true, Ordering::Release);
            if let Ok(mut recovery_reason) = self.recovery_reason.lock() {
                *recovery_reason = Some(reason.clone());
            }
            if let Ok(Some(mut anchor)) = self.keys.load_anchor() {
                anchor.status = SecureAnchorStatus::Quarantined;
                let _ = self.keys.store_anchor(&anchor);
            }
        }
        result
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
            self.verify_persisted_event(&envelope, &persisted)?;
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
        if self.payload_protection == JournalPayloadProtection::Plaintext && checkpoint.is_some() {
            return Err(StoreError::Verification(
                "plaintext journal contains a signed checkpoint".into(),
            ));
        }
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &event_hashes)?;
        }
        if let Some(anchor) = self.keys.load_anchor()?
            && event_hashes.get(&anchor.sequence) != Some(&anchor.hash)
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
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(persisted);
        }
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
                .map_or(Ok(0_u64), |value| {
                    serde_json::from_slice(value.value()).map_err(adapter_error)
                })?;
            let mut events = Vec::with_capacity(limit.min(1024));
            let mut expected_sequence = from_sequence.max(1);
            for entry in table.range(expected_sequence..).map_err(adapter_error)? {
                if events.len() >= limit {
                    break;
                }
                let (key, value) = entry.map_err(adapter_error)?;
                let event: EventEnvelope =
                    serde_json::from_slice(value.value()).map_err(adapter_error)?;
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
                .map_or(Ok(0_u64), |value| {
                    serde_json::from_slice(value.value()).map_err(adapter_error)
                })?;
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
        let result = (|| {
            let persisted = self.load_persisted(event)?;
            let plaintext = self.verify_persisted_event(event, &persisted)?;
            serde_json::from_slice(&plaintext).map_err(adapter_error)
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
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(None);
        }
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
        self.keys.store_anchor(&SecureAnchor {
            format_version: SECURE_ANCHOR_FORMAT_VERSION,
            sequence,
            hash: hash.clone(),
            verification_profile: Some(INCREMENTAL_VERIFICATION_PROFILE.into()),
            status: SecureAnchorStatus::Verified,
        })?;
        #[cfg(test)]
        if std::env::var("COLOSSUS_REDB_TEST_CRASH_SEQUENCE")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .is_none_or(|fault_sequence| fault_sequence == sequence)
        {
            crash_at_test_fault("after_anchor_before_checkpoint_commit");
        }
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

struct EncodedProjectionBatch {
    projection: String,
    expected_position: u64,
    through_sequence: u64,
    mutations: Vec<(String, Option<Vec<u8>>)>,
}

fn encode_projection_batch(batch: &ProjectionBatch) -> Result<EncodedProjectionBatch, StoreError> {
    projection_prefix(&batch.projection)?;
    if batch.through_sequence <= batch.expected_position {
        return Err(StoreError::Adapter(
            "projection position must advance".into(),
        ));
    }
    let mutations = batch
        .mutations
        .iter()
        .map(|mutation| match mutation {
            ProjectionMutation::Upsert { key, value } => Ok((
                projection_record_key(&batch.projection, key)?,
                Some(serde_json::to_vec(value).map_err(adapter_error)?),
            )),
            ProjectionMutation::Delete { key } => {
                Ok((projection_record_key(&batch.projection, key)?, None))
            }
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(EncodedProjectionBatch {
        projection: batch.projection.clone(),
        expected_position: batch.expected_position,
        through_sequence: batch.through_sequence,
        mutations,
    })
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
        for entry in table.range(namespace.as_str()..).map_err(adapter_error)? {
            if records.len() >= limit {
                break;
            }
            let (stored_key, value) = entry.map_err(adapter_error)?;
            let Some(key) = stored_key.value().strip_prefix(&namespace) else {
                break;
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
        self.apply_all(std::slice::from_ref(&batch))
    }

    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        if batches.is_empty() {
            return Ok(());
        }
        let encoded = batches
            .iter()
            .map(encode_projection_batch)
            .collect::<Result<Vec<_>, StoreError>>()?;
        let _guard = self.writer.lock().map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut positions = write
                .open_table(PROJECTION_POSITIONS)
                .map_err(adapter_error)?;
            let mut records = write
                .open_table(PROJECTION_RECORDS)
                .map_err(adapter_error)?;
            for batch in &encoded {
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
                for (key, value) in &batch.mutations {
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
