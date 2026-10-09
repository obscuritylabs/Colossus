use super::*;
use crate::PROJECTION_RECORDS;
use colossus_contracts::EventEnvelope;
use std::sync::Mutex;

fn anchored_history() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<StaticKeyProvider>,
    EventEnvelope,
) {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7; 32]));
    let first = {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        let events = journal
            .append_batch(
                (0..3)
                    .map(|version| event("stream", version, version))
                    .collect(),
            )
            .expect("append history");
        journal.checkpoint().expect("anchor history");
        events[0].clone()
    };
    (directory, path, keys, first)
}

#[test]
fn reverse_stream_read_detects_a_missing_anchored_prefix() {
    let (_directory, path, keys, _) = anchored_history();
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .open_table(STREAM_EVENTS)
        .expect("index")
        .remove(&("stream", 1))
        .expect("remove prefix");
    write.commit().expect("commit");
    drop(database);
    let journal = journal_with_keys(&path, keys);
    assert!(!journal.is_recovery_mode());
    assert_eq!(
        journal
            .read_stream_backwards("stream", None, 1)
            .expect("bounded tail")[0]
            .stream_version,
        3
    );
    assert_eq!(
        journal
            .read_stream_backwards("stream", Some(3), 1)
            .expect("next page")[0]
            .stream_version,
        2
    );
    assert!(matches!(
        journal.read_stream_backwards("stream", None, 8),
        Err(StoreError::Verification(_))
    ));
    assert!(journal.is_recovery_mode());
}

#[test]
fn malformed_anchored_envelopes_quarantine_every_access_path() {
    for access in ["global", "forward", "reverse", "payload"] {
        let (_directory, path, keys, first) = anchored_history();
        let database = Database::create(&path).expect("database");
        let write = database.begin_write().expect("write");
        write
            .open_table(EVENTS)
            .expect("events")
            .insert(1, b"{".as_slice())
            .expect("malformed envelope");
        write.commit().expect("commit");
        drop(database);
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        assert!(!journal.is_recovery_mode());
        let result = match access {
            "global" => journal.read_global(1, 1).map(|_| ()),
            "forward" => journal.read_stream_from("stream", 0, 1).map(|_| ()),
            "reverse" => journal
                .read_stream_backwards("stream", Some(2), 1)
                .map(|_| ()),
            _ => journal.decrypt_payload(&first).map(|_| ()),
        };
        assert!(
            matches!(result, Err(StoreError::Verification(_))),
            "{access}: {result:?}"
        );
        assert!(journal.is_recovery_mode());
        assert!(matches!(
            journal.append(event("other", 0, 0)),
            Err(StoreError::RecoveryMode)
        ));
        assert_eq!(
            keys.load_anchor().expect("anchor").expect("present").status,
            SecureAnchorStatus::Quarantined
        );
    }
}

#[test]
fn malformed_outbox_and_mismatched_sequence_are_integrity_failures() {
    for malformed in [true, false] {
        let (_directory, path, keys, first) = anchored_history();
        let database = Database::create(&path).expect("database");
        let write = database.begin_write().expect("write");
        let bytes = if malformed {
            b"{".to_vec()
        } else {
            serde_json::to_vec(
                &json!({"event_id": first.event_id, "global_sequence": 99, "status": "pending"}),
            )
            .expect("outbox")
        };
        write
            .open_table(OUTBOX)
            .expect("outbox")
            .insert(1, bytes.as_slice())
            .expect("tamper outbox");
        write.commit().expect("commit");
        drop(database);
        let journal = journal_with_keys(&path, keys);
        assert!(!journal.is_recovery_mode());
        let result = if malformed {
            journal.read_projection_work(1, 1).map(|_| ())
        } else {
            journal.verify().map(|_| ())
        };
        assert!(matches!(result, Err(StoreError::Verification(_))));
        assert!(journal.is_recovery_mode());
    }
}

#[test]
fn malformed_projection_namespace_quarantines_a_full_audit() {
    let (_directory, path, keys, _) = anchored_history();
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .open_table(PROJECTION_RECORDS)
        .expect("records")
        .insert("projection\0bad\0record", b"{}".as_slice())
        .expect("malformed record key");
    write.commit().expect("commit");
    drop(database);
    let journal = journal_with_keys(&path, keys);
    assert!(!journal.is_recovery_mode());
    assert!(matches!(journal.verify(), Err(StoreError::Verification(_))));
    assert!(journal.is_recovery_mode());
}

struct GatedAnchorKeys {
    inner: Arc<StaticKeyProvider>,
    gate: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
    store_gate: Mutex<Option<(Arc<Barrier>, Arc<Barrier>)>>,
}

impl KeyProvider for GatedAnchorKeys {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        self.inner.active_key()
    }
    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        self.inner.key_by_id(key_id)
    }
    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        let gate = self.store_gate.lock().map_err(adapter_error)?.take();
        if let Some((entered, resume)) = gate {
            entered.wait();
            resume.wait();
        }
        self.inner.store_anchor(anchor)
    }
    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        let gate = self.gate.lock().map_err(adapter_error)?.take();
        if let Some((entered, resume)) = gate {
            entered.wait();
            resume.wait();
        }
        self.inner.load_anchor()
    }
}

#[test]
fn full_audit_accepts_an_anchor_advanced_by_a_concurrent_checkpoint() {
    let directory = tempdir().expect("tempdir");
    let keys = Arc::new(GatedAnchorKeys {
        inner: Arc::new(StaticKeyProvider::new("test-key", [7; 32])),
        gate: Mutex::new(None),
        store_gate: Mutex::new(None),
    });
    let journal = Arc::new(
        RedbEventJournal::open(
            directory.path().join("state.redb"),
            keys.clone(),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8; 32])),
        )
        .expect("journal"),
    );
    journal.append(event("stream", 0, 1)).expect("first");
    journal.checkpoint().expect("first checkpoint");
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *keys.gate.lock().expect("gate") = Some((entered.clone(), resume.clone()));
    let auditing = Arc::clone(&journal);
    let audit = thread::spawn(move || auditing.verify());
    entered.wait();
    journal
        .append(event("stream", 1, 2))
        .expect("concurrent append");
    journal.checkpoint().expect("concurrent checkpoint");
    resume.wait();
    let report = audit
        .join()
        .expect("audit thread")
        .expect("consistent audit");
    assert_eq!(report.event_count, 2);
    assert!(!journal.is_recovery_mode());
}

#[test]
fn concurrent_checkpoint_cannot_overwrite_a_quarantined_anchor() {
    let (_directory, path, retained_keys, _) = anchored_history();
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .open_table(EVENTS)
        .expect("events")
        .insert(1, b"{".as_slice())
        .expect("malformed history");
    write.commit().expect("commit");
    drop(database);
    let keys = Arc::new(GatedAnchorKeys {
        inner: retained_keys,
        gate: Mutex::new(None),
        store_gate: Mutex::new(None),
    });
    let journal = Arc::new(
        RedbEventJournal::open(
            &path,
            keys.clone(),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8; 32])),
        )
        .expect("journal"),
    );
    assert!(!journal.is_recovery_mode());
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    *keys.store_gate.lock().expect("store gate") = Some((entered.clone(), resume.clone()));
    let checkpointing = journal.clone();
    let checkpoint = thread::spawn(move || checkpointing.checkpoint());
    entered.wait();
    let reading = journal.clone();
    let reader = thread::spawn(move || reading.read_global(1, 1));
    let started = std::time::Instant::now();
    while !journal.is_recovery_mode() && started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(1));
    }
    let quarantined_during_checkpoint = journal.is_recovery_mode();
    resume.wait();
    checkpoint
        .join()
        .expect("checkpoint thread")
        .expect("checkpoint already in progress");
    assert!(matches!(
        reader.join().expect("reader thread"),
        Err(StoreError::Verification(_))
    ));
    assert!(quarantined_during_checkpoint);
    assert_eq!(
        keys.load_anchor().expect("anchor").expect("present").status,
        SecureAnchorStatus::Quarantined
    );
    assert!(matches!(
        journal.checkpoint(),
        Err(StoreError::RecoveryMode)
    ));
}
