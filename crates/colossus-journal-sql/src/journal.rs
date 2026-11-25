use std::{
    collections::BTreeMap,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use colossus_contracts::{
    EventEnvelope, NewEvent, ProjectionWorkItem, SignedCheckpoint, StartupVerificationMode,
    StartupVerificationReport,
};
use colossus_journal_format::seal_event;
use colossus_ports::{
    CheckpointSigner, EventJournal, JournalPayloadProtection, KeyProvider, StoreError,
    VerificationReport,
};
use serde_json::Value;

use crate::{
    driver::{Connection, Param, SqlJournalBackend},
    lease, reads,
    schema::{self, MAX_SQL_SEQUENCE, encode, head, invalid, sql_sequence},
};

const CHECKPOINT_INTERVAL: u64 = 100;
const CHECKPOINT_MAX_AGE: Duration = Duration::from_secs(60);

/// File-backed implementation of the canonical journal and disposable projection store.
///
/// A journal owns an exclusive writer lease and serializes its connection. All database
/// paths and sidecars must live in an owner-private directory controlled by the caller.
/// A protected journal needs its own independent anchor namespace in the key provider.
pub struct SqlEventJournal {
    state: Mutex<JournalState>,
    pub(crate) keys: Arc<dyn KeyProvider>,
    pub(crate) signer: Arc<dyn CheckpointSigner>,
    pub(crate) protection: JournalPayloadProtection,
    startup_report: StartupVerificationReport,
    // Close the engine before releasing the writer lease.
    _lease: File,
}

pub(crate) struct JournalState {
    pub(crate) db: Connection,
    pub(crate) checkpoint_sequence: u64,
    last_checkpoint: Instant,
    recovery_reason: Option<String>,
}

impl SqlEventJournal {
    /// Open or initialize a local journal and verify it before admitting writes.
    ///
    /// An established journal is bound to its engine and key protection mode.
    /// Deterministic integrity failures encountered during verification allow inspection
    /// in read-only recovery mode.
    pub fn open(
        path: impl AsRef<Path>,
        backend: SqlJournalBackend,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        let (path, lease) = lease::acquire(path.as_ref())?;
        let db = Connection::open(&path, backend)?;
        let protection = keys.payload_protection();
        schema::initialize(&db, backend, protection)?;
        let mut journal = Self {
            state: Mutex::new(JournalState {
                db,
                checkpoint_sequence: 0,
                last_checkpoint: Instant::now(),
                recovery_reason: None,
            }),
            keys,
            signer,
            protection,
            startup_report: StartupVerificationReport {
                configured_mode: mode,
                path: "recovery".into(),
                verified_from_sequence: None,
                verified_through_sequence: 0,
                verified_event_count: 0,
                anchor_format_version: None,
            },
            _lease: lease,
        };
        let result = journal.with_state(false, |state| journal.startup(state, mode));
        match result {
            Ok(report) => journal.startup_report = report,
            Err(error) => {
                let state = journal.state.get_mut().map_err(|_| lock_error())?;
                state
                    .recovery_reason
                    .get_or_insert_with(|| error.to_string());
            }
        }
        Ok(journal)
    }

    /// Work performed by startup verification for this open.
    pub fn startup_verification_report(&self) -> &StartupVerificationReport {
        &self.startup_report
    }

    /// Safe diagnostic explaining why writes are disabled, when applicable.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        self.with_state(false, |state| Ok(state.recovery_reason.clone()))
    }

    pub(crate) fn with_state<T>(
        &self,
        writing: bool,
        operation: impl FnOnce(&mut JournalState) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        if writing && state.recovery_reason.is_some() {
            return Err(StoreError::RecoveryMode);
        }
        let result = operation(&mut state);
        if let Err(error) = &result {
            match error {
                StoreError::Verification(_) | StoreError::KeyUnavailable(_) => {
                    if state.recovery_reason.is_none() {
                        self.quarantine_anchor();
                    }
                    state.recovery_reason = Some(error.to_string());
                }
                StoreError::OutcomeUnknown(_) => {
                    state.recovery_reason = Some(error.to_string());
                }
                _ => {}
            }
        }
        result
    }

    fn append_transaction(
        &self,
        db: &Connection,
        events: Vec<NewEvent>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        db.write(|| {
            let (mut sequence, mut previous_hash) = head(db)?;
            let mut stream_tails = BTreeMap::<String, (u64, u64)>::new();
            let mut envelopes = Vec::with_capacity(events.len());
            for event in events {
                if event.stream_id.contains('\0') {
                    return Err(StoreError::Adapter("stream ID must contain no NUL".into()));
                }
                let durable = match stream_tails.get(&event.stream_id) {
                    Some((version, _)) => *version,
                    None => reads::stream_version(db, &event.stream_id)?,
                };
                if event.expected_stream_version != durable {
                    return Err(StoreError::Conflict {
                        stream_id: event.stream_id,
                        expected: event.expected_stream_version,
                        actual: durable,
                    });
                }
                sequence += 1;
                if sequence > MAX_SQL_SEQUENCE || durable >= MAX_SQL_SEQUENCE {
                    return Err(invalid(
                        "embedded journal sequence or stream version is exhausted",
                    ));
                }
                let event = seal_event(
                    event,
                    sequence,
                    durable + 1,
                    &previous_hash,
                    self.protection,
                    self.keys.as_ref(),
                )?;
                insert_event(db, &event)?;
                stream_tails.insert(event.stream_id.clone(), (event.stream_version, sequence));
                previous_hash.clone_from(&event.record_hash);
                envelopes.push(event);
            }
            for (stream, (version, sequence)) in &stream_tails {
                db.execute(
                    "INSERT INTO streams (stream_id, version, tail_sequence) VALUES (?1, ?2, ?3)
                     ON CONFLICT (stream_id) DO UPDATE
                     SET version = excluded.version, tail_sequence = excluded.tail_sequence",
                    &[
                        Param::Bytes(stream.as_bytes()),
                        sql_sequence(*version)?,
                        sql_sequence(*sequence)?,
                    ],
                )?;
            }
            db.execute(
                "UPDATE journal_state SET sequence = ?1, record_hash = ?2 WHERE id = 1",
                &[sql_sequence(sequence)?, Param::Text(&previous_hash)],
            )?;
            #[cfg(test)]
            super::tests::crash_at("before_commit");
            Ok(envelopes)
        })
    }

    fn checkpoint_locked(
        &self,
        state: &mut JournalState,
    ) -> Result<Option<SignedCheckpoint>, StoreError> {
        let checkpoint = self.persist_checkpoint(&state.db)?;
        if let Some(checkpoint) = &checkpoint {
            state.checkpoint_sequence = checkpoint.global_sequence;
            state.last_checkpoint = Instant::now();
        }
        Ok(checkpoint)
    }
}

fn insert_event(db: &Connection, event: &EventEnvelope) -> Result<(), StoreError> {
    let bytes = encode(event)?;
    db.execute(
        "INSERT INTO events (sequence, stream_id, stream_version, event_id, record_hash, envelope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        &[
            sql_sequence(event.global_sequence)?,
            Param::Bytes(event.stream_id.as_bytes()),
            sql_sequence(event.stream_version)?,
            Param::Text(&event.event_id),
            Param::Text(&event.record_hash),
            Param::Bytes(&bytes),
        ],
    )?;
    db.execute(
        "INSERT INTO projection_outbox (sequence, event_id) VALUES (?1, ?2)",
        &[
            sql_sequence(event.global_sequence)?,
            Param::Text(&event.event_id),
        ],
    )
}

fn lock_error() -> StoreError {
    StoreError::Adapter("embedded journal connection lock is poisoned".into())
}

impl EventJournal for SqlEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.append_batch(vec![event])?
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.with_state(true, |state| {
            if events.is_empty() {
                return Ok(Vec::new());
            }
            let persisted = self.append_transaction(&state.db, events)?;
            #[cfg(test)]
            super::tests::crash_at("after_commit");
            let due = persisted.last().is_some_and(|event| {
                event
                    .global_sequence
                    .saturating_sub(state.checkpoint_sequence)
                    >= CHECKPOINT_INTERVAL
            }) || state.last_checkpoint.elapsed() >= CHECKPOINT_MAX_AGE;
            if self.protection == JournalPayloadProtection::Encrypted
                && due
                && let Err(error) = self.checkpoint_locked(state)
            {
                if matches!(
                    error,
                    StoreError::Verification(_) | StoreError::KeyUnavailable(_)
                ) {
                    self.quarantine_anchor();
                }
                return Err(StoreError::OutcomeUnknown(
                    "events committed but their scheduled checkpoint could not be confirmed".into(),
                ));
            }
            Ok(persisted)
        })
    }

    fn read_stream(&self, stream: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.with_state(false, |state| {
            reads::stream(&state.db, stream, 0, usize::MAX)
        })
    }

    fn read_stream_from(
        &self,
        stream: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.with_state(false, |state| {
            reads::stream(
                &state.db,
                stream,
                after,
                limit.min(colossus_ports::MAX_STREAM_READ_BATCH),
            )
        })
    }

    fn read_stream_backwards(
        &self,
        stream: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.with_state(false, |state| {
            reads::stream_backwards(&state.db, stream, before, limit)
        })
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.with_state(false, |state| {
            reads::list_streams(&state.db, prefix, after, limit)
        })
    }

    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.with_state(false, |state| reads::global(&state.db, from, limit))
    }

    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.with_state(false, |state| reads::outbox(&state.db, from, limit))
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.with_state(false, |state| head(&state.db))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.with_state(false, |state| self.decrypt_stored(&state.db, event))
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.with_state(false, |state| self.verify_full(&state.db))
    }

    fn is_recovery_mode(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |state| state.recovery_reason.is_some())
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        if self.protection == JournalPayloadProtection::Plaintext {
            return Ok(None);
        }
        self.with_state(true, |state| self.checkpoint_locked(state))
    }
}
