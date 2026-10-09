use super::*;

mod append;
mod checkpoint;
mod event_journal;
mod payload;
mod projections;
mod startup;
mod streams;
mod verification;

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
            .map(|value| decode_journal_json::<String>(value.value()))
            .transpose()?;
        let head_sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
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
}
