use std::{
    path::Path,
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
};

use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, NewEvent, ProjectionBatch,
    ProjectionMutation, SecureAnchor, SecureAnchorStatus, StartupVerificationMode,
};
use colossus_journal_redb::{
    DisabledCheckpointSigner, Ed25519CheckpointSigner, PlaintextKeyProvider, StaticKeyProvider,
};
use colossus_ports::{
    EventJournal, ExternalWorkQueue, KeyProvider, MAX_STREAM_LIST_BATCH, MAX_STREAM_READ_BATCH,
    ProjectionStore, StoreError,
};
use colossus_projection::JournalExternalWorkQueue;
use colossus_testkit::{
    assert_external_work_queue_conformance, assert_journal_conformance,
    assert_projection_store_conformance,
};
use serde_json::json;

use crate::{
    SqlEventJournal, SqlJournalBackend,
    driver::{Connection, Param},
};

const BACKENDS: [SqlJournalBackend; 2] = [SqlJournalBackend::Sqlite, SqlJournalBackend::Turso];
const INCREMENTAL: StartupVerificationMode = StartupVerificationMode::Incremental;
const CRASH_AT: &str = "COLOSSUS_EMBEDDED_JOURNAL_CRASH_AT";
const CRASH_PATH: &str = "COLOSSUS_EMBEDDED_JOURNAL_CRASH_PATH";
const CRASH_BACKEND: &str = "COLOSSUS_EMBEDDED_JOURNAL_CRASH_BACKEND";
const CRASH_EXIT: i32 = 77;

pub(super) fn crash_at(point: &str) {
    if std::env::var(CRASH_AT).as_deref() == Ok(point) {
        // Exit without Rust destructors, connection close, or a clean WAL checkpoint.
        std::process::exit(CRASH_EXIT);
    }
}

fn event(stream: &str, version: u64) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: stream.into(),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: "embedded-test.appended".into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "embedded-test".into(),
        },
        context: ExecutionContext::default(),
        payload: json!({"version": version, "text": "sensitive fixture"}),
    }
}

fn plaintext(path: &Path, backend: SqlJournalBackend) -> Result<SqlEventJournal, StoreError> {
    SqlEventJournal::open(
        path,
        backend,
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
        INCREMENTAL,
    )
}

fn protected(
    path: &Path,
    backend: SqlJournalBackend,
    keys: Arc<dyn KeyProvider>,
    mode: StartupVerificationMode,
) -> Result<SqlEventJournal, StoreError> {
    SqlEventJournal::open(
        path,
        backend,
        keys,
        Arc::new(Ed25519CheckpointSigner::new("test", [5; 32])),
        mode,
    )
}

#[test]
fn both_adapters_pass_the_canonical_ports_and_reopen() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        {
            let store = Arc::new(plaintext(&path, backend).unwrap());
            assert!(
                !store.is_recovery_mode(),
                "{backend:?}: {:?}",
                store.recovery_reason()
            );
            assert_journal_conformance(store.as_ref(), event("session", 0), event("session", 0));
            store.append(event("session", 1)).unwrap();
            assert_projection_store_conformance(store.as_ref());
            let first = store.read_stream("session").unwrap().remove(0);
            assert_eq!(
                store.decrypt_payload(&first).unwrap(),
                event("session", 0).payload
            );
            assert!(store.checkpoint().unwrap().is_none());
        }
        let store = plaintext(&path, backend).unwrap();
        assert_eq!(store.startup_verification_report().verified_event_count, 1);
        assert_eq!(store.verify().unwrap().event_count, 2);
        assert_eq!(store.read_projection_work(1, 10).unwrap().len(), 2);
        assert!(!store.is_recovery_mode());

        let queue_path = dir.path().join("queue.db");
        let store = Arc::new(plaintext(&queue_path, backend).unwrap());
        let queue = JournalExternalWorkQueue::new(store.clone(), store.clone());
        assert_external_work_queue_conformance(
            store.as_ref(),
            &queue,
            event("q", 0),
            event("q", 1),
        );
        queue
            .record_failure(
                "blocked",
                None,
                "2026-11-25T00:00:00Z",
                false,
                "test",
                "uncertain",
            )
            .unwrap();
        drop(queue);
        drop(store);
        let store = Arc::new(plaintext(&queue_path, backend).unwrap());
        let queue = JournalExternalWorkQueue::new(store.clone(), store.clone());
        assert_eq!(queue.position("conformance.left-v1").unwrap(), 2);
        assert_eq!(queue.pending("conformance.right-v1", 10).unwrap().len(), 2);
        let retry = queue.retry_state("blocked").unwrap().unwrap();
        assert!(!retry.retryable);
        assert!(retry.next_retry_at.is_none());
    }
}

#[test]
fn append_conflict_and_projection_conflict_roll_back_complete_batches() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let journal = plaintext(&path, backend).unwrap();
        journal.append(event("root", 0)).unwrap();
        assert!(matches!(
            journal.append_batch(vec![event("new", 0), event("root", 0)]),
            Err(StoreError::Conflict { actual: 1, .. }),
        ));
        assert!(journal.read_stream("new").unwrap().is_empty());
        assert_eq!(journal.head().unwrap().0, 1);
        assert_eq!(journal.read_projection_work(1, 10).unwrap().len(), 1);
        let good = ProjectionBatch {
            projection: "one".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: vec![ProjectionMutation::Upsert {
                key: "α:1".into(),
                value: json!(1),
            }],
        };
        let stale = ProjectionBatch {
            projection: "two".into(),
            expected_position: 1,
            through_sequence: 2,
            mutations: vec![],
        };
        assert!(matches!(
            journal.apply_all(&[good.clone(), stale]),
            Err(StoreError::Conflict { .. })
        ));
        assert_eq!(journal.position("one").unwrap(), 0);
        assert_eq!(journal.get("one", "α:1").unwrap(), None);
        journal.apply(good).unwrap();
        assert_eq!(
            journal.list_after("one", "α", Some("a"), 10).unwrap().len(),
            1
        );
        assert!(
            journal
                .list_after("one", "α", Some("ω"), 10)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn established_database_rejects_another_engine() {
    for (backend, other) in [
        (SqlJournalBackend::Sqlite, SqlJournalBackend::Turso),
        (SqlJournalBackend::Turso, SqlJournalBackend::Sqlite),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let journal = plaintext(&path, backend).unwrap();
        journal.append(event("s", 0)).unwrap();
        drop(journal);
        assert!(matches!(
            plaintext(&path, other),
            Err(StoreError::Verification(_))
        ));
        let journal = plaintext(&path, backend).unwrap();
        assert_eq!(journal.verify().unwrap().event_count, 1);
    }
}

#[test]
fn ordered_ranges_clamp_limits_and_support_utf8_and_exclusive_cursors() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let journal = plaintext(&dir.path().join("journal.db"), backend).unwrap();
        journal
            .append_batch(
                (0..MAX_STREAM_READ_BATCH as u64 + 1)
                    .map(|i| event("α:tail", i))
                    .collect(),
            )
            .unwrap();
        journal
            .append_batch(
                (0..MAX_STREAM_LIST_BATCH + 1)
                    .map(|i| event(&format!("α:item:{i:05}"), 0))
                    .collect(),
            )
            .unwrap();
        journal.append(event("α;outside", 0)).unwrap();
        journal.append(event("β:item", 0)).unwrap();
        let ids = journal.list_stream_ids("α:", None, usize::MAX).unwrap();
        assert_eq!(ids.len(), MAX_STREAM_LIST_BATCH);
        assert_eq!(ids[0], "α:item:00000");
        let more = journal
            .list_stream_ids("α:", ids.last().map(String::as_str), 10)
            .unwrap();
        assert_eq!(
            more,
            [
                format!("α:item:{MAX_STREAM_LIST_BATCH:05}"),
                "α:tail".into()
            ]
        );
        assert_eq!(
            journal
                .list_stream_ids("α:item:", Some("α:item:00000!"), 1)
                .unwrap(),
            ["α:item:00001"]
        );
        assert!(journal.list_stream_ids("β", Some("α"), 1).is_err());
        assert_eq!(
            journal
                .read_stream_from("α:tail", 0, usize::MAX)
                .unwrap()
                .len(),
            MAX_STREAM_READ_BATCH
        );
        let reverse = journal
            .read_stream_backwards("α:tail", Some(u64::MAX), usize::MAX)
            .unwrap();
        assert_eq!(reverse.len(), MAX_STREAM_READ_BATCH);
        assert_eq!(reverse[0].stream_version, MAX_STREAM_READ_BATCH as u64 + 1);
        assert_eq!(reverse.last().unwrap().stream_version, 2);
        assert!(
            journal
                .read_stream_from("α:tail", u64::MAX, 1)
                .unwrap()
                .is_empty()
        );
        assert!(
            journal
                .read_global(u64::MAX, usize::MAX)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn concurrent_writers_commit_without_gaps_or_optimistic_conflict_loss() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let journal = plaintext(&dir.path().join("journal.db"), backend).unwrap();
        let start = Barrier::new(8);
        let winners = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|worker| {
                    let journal = &journal;
                    let start = &start;
                    scope.spawn(move || {
                        start.wait();
                        let winner = match journal.append(event("shared", 0)) {
                            Ok(_) => true,
                            Err(StoreError::Conflict { actual: 1, .. }) => false,
                            Err(error) => panic!("{backend:?}: {error}"),
                        };
                        let stream = format!("worker:{worker}");
                        journal
                            .append_batch(vec![event(&stream, 0), event(&stream, 1)])
                            .unwrap();
                        winner
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|winner| *winner)
                .count()
        });
        assert_eq!(winners, 1);
        assert_eq!(journal.verify().unwrap().event_count, 17);
    }
}

#[test]
fn protected_reopen_verifies_checkpoint_boundary_outbox() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let keys = Arc::new(StaticKeyProvider::new("test", [7; 32]));
        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        journal.append(event("s", 0)).unwrap();
        journal.checkpoint().unwrap();
        drop(journal);
        let db = Connection::open(&path, backend).unwrap();
        db.execute(
            "UPDATE projection_outbox SET event_id = 'mismatch' WHERE sequence = 1",
            &[],
        )
        .unwrap();
        drop(db);
        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        assert!(journal.is_recovery_mode());
        assert!(
            journal
                .recovery_reason()
                .unwrap()
                .unwrap()
                .contains("outbox")
        );
        assert_eq!(
            keys.load_anchor().unwrap().unwrap().status,
            SecureAnchorStatus::Quarantined
        );
    }
}

#[test]
fn protected_prefix_tampering_quarantines_anchor_and_blocks_writes() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let keys = Arc::new(StaticKeyProvider::new("test", [7; 32]));
        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        let mut first = journal.append(event("session", 0)).unwrap();
        assert_eq!(
            journal.decrypt_payload(&first).unwrap(),
            event("session", 0).payload
        );
        journal.append(event("session", 1)).unwrap();
        assert_eq!(journal.checkpoint().unwrap().unwrap().global_sequence, 2);
        journal.append(event("session", 2)).unwrap();
        drop(journal);

        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        assert_eq!(journal.startup_verification_report().path, "incremental");
        assert_eq!(
            journal.startup_verification_report().verified_event_count,
            2
        );
        assert_eq!(journal.verify().unwrap().event_count, 3);
        assert!(matches!(
            plaintext(&path, backend),
            Err(StoreError::WriterLeaseHeld)
        ));
        drop(journal);
        assert!(matches!(
            plaintext(&path, backend),
            Err(StoreError::Verification(_))
        ));

        let replacement = if first.payload.ciphertext.starts_with("00") {
            "01"
        } else {
            "00"
        };
        first.payload.ciphertext.replace_range(..2, replacement);
        let bytes = serde_json::to_vec(&first).unwrap();
        let db = Connection::open(&path, backend).unwrap();
        db.execute(
            "UPDATE events SET envelope = ?1 WHERE sequence = 1",
            &[Param::Bytes(&bytes)],
        )
        .unwrap();
        drop(db);

        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        assert!(!journal.is_recovery_mode());
        let stored = journal.read_global(1, 1).unwrap().remove(0);
        assert!(matches!(
            journal.decrypt_payload(&stored),
            Err(StoreError::Verification(_))
        ));
        assert!(journal.is_recovery_mode());
        assert!(matches!(
            journal.append(event("session", 3)),
            Err(StoreError::RecoveryMode)
        ));
        assert!(matches!(
            journal.checkpoint(),
            Err(StoreError::RecoveryMode)
        ));
        assert_eq!(
            keys.load_anchor().unwrap().unwrap().status,
            SecureAnchorStatus::Quarantined
        );
        assert!(journal.verify().is_err());
        drop(journal);
        let reopened =
            protected(&path, backend, keys.clone(), StartupVerificationMode::Full).unwrap();
        assert!(reopened.is_recovery_mode());
    }
}

#[test]
fn protected_anchor_rejects_a_consistent_database_rollback() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let keys = Arc::new(StaticKeyProvider::new("test", [7; 32]));
        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        let first = journal.append(event("s", 0)).unwrap();
        journal.append(event("s", 1)).unwrap();
        journal.checkpoint().unwrap();
        drop(journal);
        let db = Connection::open(&path, backend).unwrap();
        db.write(|| {
            db.execute("DELETE FROM events WHERE sequence = 2", &[])?;
            db.execute("DELETE FROM projection_outbox WHERE sequence = 2", &[])?;
            db.execute("DELETE FROM checkpoints", &[])?;
            db.execute("UPDATE streams SET version = 1, tail_sequence = 1", &[])?;
            db.execute(
                "UPDATE journal_state SET sequence = 1, record_hash = ?1",
                &[Param::Text(&first.record_hash)],
            )
        })
        .unwrap();
        drop(db);
        let reopened = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        assert!(reopened.is_recovery_mode());
        assert!(
            reopened
                .recovery_reason()
                .unwrap()
                .unwrap()
                .contains("secure anchor")
        );
        assert_eq!(
            keys.load_anchor().unwrap().unwrap().status,
            SecureAnchorStatus::Quarantined
        );
    }
}

struct FailingAnchor {
    keys: StaticKeyProvider,
    fail: AtomicBool,
}

impl KeyProvider for FailingAnchor {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        self.keys.active_key()
    }
    fn key_by_id(&self, id: &str) -> Result<[u8; 32], StoreError> {
        self.keys.key_by_id(id)
    }
    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        self.keys.load_anchor()
    }
    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(StoreError::Adapter("injected anchor write error".into()));
        }
        self.keys.store_anchor(anchor)
    }
}

#[test]
fn committed_checkpoint_recovers_when_external_anchor_acknowledgement_failed() {
    for backend in BACKENDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.db");
        let keys = Arc::new(FailingAnchor {
            keys: StaticKeyProvider::new("test", [7; 32]),
            fail: AtomicBool::new(false),
        });
        let journal = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        journal.append(event("s", 0)).unwrap();
        journal.checkpoint().unwrap();
        journal.append(event("s", 1)).unwrap();
        keys.fail.store(true, Ordering::Relaxed);
        assert!(matches!(
            journal.checkpoint(),
            Err(StoreError::OutcomeUnknown(_))
        ));
        assert!(journal.is_recovery_mode());
        assert_eq!(keys.load_anchor().unwrap().unwrap().sequence, 1);
        drop(journal);
        keys.fail.store(false, Ordering::Relaxed);
        let reopened = protected(&path, backend, keys.clone(), INCREMENTAL).unwrap();
        assert!(!reopened.is_recovery_mode());
        assert_eq!(
            reopened.startup_verification_report().verified_event_count,
            2
        );
        assert_eq!(keys.load_anchor().unwrap().unwrap().sequence, 2);
        assert_eq!(reopened.verify().unwrap().event_count, 2);
    }
}

#[test]
fn process_interruption_recovers_both_sides_of_the_commit_boundary() {
    for backend in BACKENDS {
        for (point, expected) in [("before_commit", 1), ("after_commit", 3)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.db");
            let journal = plaintext(&path, backend).unwrap();
            journal.append(event("s", 0)).unwrap();
            drop(journal);
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::journal_crash_child"])
                .env(CRASH_AT, point)
                .env(CRASH_PATH, &path)
                .env(CRASH_BACKEND, backend.name())
                .status()
                .unwrap();
            assert_eq!(child.code(), Some(CRASH_EXIT), "{backend:?} {point}");
            let recovered = plaintext(&path, backend).unwrap();
            assert!(
                !recovered.is_recovery_mode(),
                "{backend:?} {point}: {:?}",
                recovered.recovery_reason()
            );
            assert_eq!(recovered.verify().unwrap().event_count, expected);
            assert_eq!(
                recovered.read_projection_work(1, 10).unwrap().len() as u64,
                expected
            );
            recovered.append(event("s", expected)).unwrap();
        }
    }
}

#[test]
fn journal_crash_child() {
    let Some(path) = std::env::var_os(CRASH_PATH) else {
        return;
    };
    let engine = std::env::var(CRASH_BACKEND).unwrap();
    let backend = BACKENDS
        .into_iter()
        .find(|backend| backend.name() == engine)
        .unwrap();
    let journal = plaintext(Path::new(&path), backend).unwrap();
    journal
        .append_batch(vec![event("s", 1), event("s", 2)])
        .unwrap();
    panic!("configured fault did not interrupt the child");
}
