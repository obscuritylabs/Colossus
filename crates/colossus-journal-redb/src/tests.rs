use super::{
    DisabledCheckpointSigner, EVENTS, Ed25519CheckpointSigner, METADATA, OUTBOX,
    PAYLOAD_PROTECTION_KEY, PROJECTION_POSITIONS, PersistedEventEnvelope, PlaintextKeyProvider,
    RedbEventJournal, RedbWriterLease, STREAM_EVENTS, STREAM_EVENTS_INDEX_KEY, STREAM_VERSIONS,
    StaticKeyProvider, adapter_error, cached_platform_secret, persisted_associated_data,
    persisted_record_hash,
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, NewEvent, PLAINTEXT_PAYLOAD_ALGORITHM,
    ProjectionBatch, ProjectionMutation, SecureAnchor, SecureAnchorStatus, StartupVerificationMode,
};
use colossus_memory::EventSourcedMemoryRepository;
use colossus_ports::{EventJournal, ExternalWorkQueue, KeyProvider, ProjectionStore, StoreError};
use colossus_projection::{JournalExternalWorkQueue, ProjectionWorker, default_handlers};
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::{
    assert_external_work_queue_conformance, assert_journal_conformance,
    assert_memory_repository_conformance, assert_projection_store_conformance,
    assert_session_repository_conformance, assert_work_repository_conformance,
    assert_workflow_repository_conformance,
};
use colossus_work::EventSourcedWorkRepository;
use colossus_workflow::EventSourcedWorkflowRepository;
use redb::{Database, ReadableDatabase, TableDefinition};
use serde_json::json;
use std::{
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use tempfile::tempdir;

fn event(stream: &str, version: u64, value: u64) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: stream.into(),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: "test.recorded.v1".into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "test".into(),
        },
        context: ExecutionContext {
            correlation_id: "correlation".into(),
            ..ExecutionContext::default()
        },
        payload: json!({"value": value}),
    }
}

fn journal(path: &std::path::Path) -> RedbEventJournal {
    journal_with_keys(
        path,
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
    )
}

fn journal_with_keys(path: &std::path::Path, keys: Arc<StaticKeyProvider>) -> RedbEventJournal {
    RedbEventJournal::open(
        path,
        keys,
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    )
    .expect("open journal")
}

fn plaintext_journal(path: &std::path::Path) -> RedbEventJournal {
    RedbEventJournal::open(
        path,
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
    )
    .expect("open plaintext journal")
}

fn ephemeral_journal() -> RedbEventJournal {
    RedbEventJournal::open_in_memory(
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
    )
    .expect("open ephemeral journal")
}

#[test]
fn established_schema_uses_read_only_fast_path() {
    let directory = tempdir().expect("tempdir");
    let database = Database::create(directory.path().join("schema.redb")).expect("database");

    assert!(RedbEventJournal::ensure_schema(&database).expect("create schema"));
    assert!(!RedbEventJournal::ensure_schema(&database).expect("reuse schema"));
}

#[test]
fn established_schema_rejects_incompatible_table_definition() {
    const MISMATCHED_PROJECTION_RECORDS: TableDefinition<&str, u64> =
        TableDefinition::new("projection_records");

    let directory = tempdir().expect("tempdir");
    let database = Database::create(directory.path().join("mismatch.redb")).expect("database");
    {
        let write = database.begin_write().expect("write transaction");
        write.open_table(EVENTS).expect("events");
        write.open_table(STREAM_EVENTS).expect("stream events");
        write.open_table(STREAM_VERSIONS).expect("stream versions");
        write.open_table(METADATA).expect("metadata");
        write.open_table(OUTBOX).expect("outbox");
        write
            .open_table(PROJECTION_POSITIONS)
            .expect("projection positions");
        write
            .open_table(MISMATCHED_PROJECTION_RECORDS)
            .expect("mismatched projection records");
        write.commit().expect("commit");
    }

    let error =
        RedbEventJournal::ensure_schema(&database).expect_err("reject incompatible definition");
    assert!(
        matches!(error, StoreError::Adapter(ref message) if message.contains("projection_records")),
        "unexpected schema error: {error}"
    );
}

#[test]
fn plaintext_journal_preserves_integrity_without_keys_or_checkpoints() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("plaintext.redb");
    {
        let journal = plaintext_journal(&path);
        let stored = journal.append(event("stream-1", 0, 7)).expect("append");
        assert_eq!(stored.payload.algorithm, PLAINTEXT_PAYLOAD_ALGORITHM);
        assert_eq!(stored.payload.key_id, "none");
        assert!(stored.payload.nonce.is_empty());
        assert_eq!(
            journal.decrypt_payload(&stored).expect("decode payload"),
            json!({"value": 7})
        );
        assert!(journal.checkpoint().expect("disabled checkpoint").is_none());
        let report = journal.verify().expect("full audit");
        assert_eq!(report.event_count, 1);
        assert!(report.checkpoint.is_none());
    }

    let reopened = plaintext_journal(&path);
    let startup = reopened
        .startup_verification_report()
        .expect("startup report");
    assert_eq!(startup.path, "local_integrity");
    assert_eq!(startup.verified_event_count, 1);
    assert_eq!(startup.anchor_format_version, None);
}

#[test]
fn plaintext_explicit_audit_replays_and_detects_historical_tampering() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("plaintext.redb");
    {
        let journal = plaintext_journal(&path);
        journal
            .append_batch(vec![event("stream-1", 0, 1), event("stream-1", 1, 2)])
            .expect("append plaintext history");
    }

    let database = Database::create(&path).expect("database");
    let read = database.begin_read().expect("read");
    let table = read.open_table(EVENTS).expect("events");
    let bytes = table.get(1).expect("get").expect("event").value().to_vec();
    drop(table);
    drop(read);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    value["event_type"] = json!("tampered.v1");
    let bytes = serde_json::to_vec(&value).expect("encode");
    let write = database.begin_write().expect("write");
    {
        let mut table = write.open_table(EVENTS).expect("events");
        table.insert(1, bytes.as_slice()).expect("tamper event");
    }
    write.commit().expect("commit tamper");
    drop(database);

    let reopened = plaintext_journal(&path);
    assert!(!reopened.is_recovery_mode());
    assert!(matches!(
        reopened.verify(),
        Err(StoreError::Verification(_))
    ));
    assert!(reopened.is_recovery_mode());
    drop(reopened);

    let full = RedbEventJournal::open_with_startup_verification(
        &path,
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
        StartupVerificationMode::Full,
    )
    .expect("open plaintext journal for full verification");
    assert!(full.is_recovery_mode());
}

#[test]
fn journal_protection_mode_cannot_change_in_place() {
    let directory = tempdir().expect("tempdir");
    let encrypted_path = directory.path().join("encrypted.redb");
    journal(&encrypted_path)
        .append(event("stream-1", 0, 1))
        .expect("encrypted append");
    let error = RedbEventJournal::open(
        &encrypted_path,
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
    )
    .err()
    .expect("encrypted to plaintext must fail");
    assert!(error.to_string().contains("in-place protection changes"));

    let plaintext_path = directory.path().join("plaintext.redb");
    plaintext_journal(&plaintext_path)
        .append(event("stream-1", 0, 1))
        .expect("plaintext append");
    let error = RedbEventJournal::open(
        &plaintext_path,
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    )
    .err()
    .expect("plaintext to encrypted must fail");
    assert!(error.to_string().contains("in-place protection changes"));
}

#[test]
fn markerless_nonempty_journal_is_classified_as_encrypted() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("legacy.redb");
    journal(&path)
        .append(event("stream-1", 0, 1))
        .expect("encrypted append");
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    {
        let mut metadata = write.open_table(METADATA).expect("metadata");
        metadata
            .remove(PAYLOAD_PROTECTION_KEY)
            .expect("remove protection marker");
    }
    write.commit().expect("commit");
    drop(database);

    assert!(
        RedbEventJournal::open(
            &path,
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )
        .is_err()
    );
    let reopened = journal(&path);
    assert!(!reopened.is_recovery_mode());
}

struct FileAnchorKeyProvider {
    path: std::path::PathBuf,
}

impl KeyProvider for FileAnchorKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok(("test-key".into(), [7_u8; 32]))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        if key_id == "test-key" {
            Ok([7_u8; 32])
        } else {
            Err(StoreError::KeyUnavailable(key_id.into()))
        }
    }

    fn store_anchor(&self, anchor: &SecureAnchor) -> Result<(), StoreError> {
        std::fs::write(
            &self.path,
            serde_json::to_vec(anchor).map_err(adapter_error)?,
        )
        .map_err(adapter_error)
    }

    fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.path).map_err(adapter_error)?)
                .map_err(adapter_error)?;
        Ok(Some(SecureAnchor {
            format_version: value
                .get("format_version")
                .and_then(serde_json::Value::as_u64)
                .map_or(Ok(1_u16), |version| {
                    u16::try_from(version).map_err(adapter_error)
                })?,
            sequence: value
                .get("sequence")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| StoreError::Verification("test anchor sequence is absent".into()))?,
            hash: value
                .get("hash")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Verification("test anchor hash is absent".into()))?
                .into(),
            verification_profile: value
                .get("verification_profile")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            status: value
                .get("status")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(adapter_error)?
                .unwrap_or_default(),
        }))
    }
}

fn journal_with_file_anchor(
    path: &std::path::Path,
    anchor_path: &std::path::Path,
) -> RedbEventJournal {
    RedbEventJournal::open(
        path,
        Arc::new(FileAnchorKeyProvider {
            path: anchor_path.into(),
        }),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    )
    .expect("open journal with file anchor")
}

#[test]
fn stream_discovery_uses_the_durable_prefix_index() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("stream-discovery.redb"));
    journal
        .append_batch(vec![
            event("indexed:b", 0, 1),
            event("other:a", 0, 2),
            event("indexed:a", 0, 3),
            event("indexed:c", 0, 4),
            event("indexed:a", 1, 5),
        ])
        .expect("append indexed streams");

    assert_eq!(
        journal
            .list_stream_ids("indexed:", None, usize::MAX)
            .expect("indexed streams"),
        ["indexed:a", "indexed:b", "indexed:c"]
    );
    assert_eq!(
        journal
            .list_stream_ids("indexed:", Some("indexed:a"), 1)
            .expect("exclusive indexed stream cursor"),
        ["indexed:b"]
    );
    assert!(
        journal
            .list_stream_ids("indexed:", Some("other:a"), 1)
            .is_err()
    );
}

#[test]
fn startup_builds_and_uses_the_verified_legacy_stream_index() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("legacy-stream-index.redb");
    {
        let journal = journal(&path);
        journal
            .append_batch(vec![
                event("stream-a", 0, 1),
                event("stream-b", 0, 2),
                event("stream-a", 1, 3),
                event("stream-b", 1, 4),
                event("stream-a", 2, 5),
            ])
            .expect("interleaved append");
    }

    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .delete_table(STREAM_EVENTS)
        .expect("delete modern stream index");
    {
        let mut metadata = write.open_table(METADATA).expect("metadata");
        metadata
            .remove(STREAM_EVENTS_INDEX_KEY)
            .expect("remove stream index version");
    }
    write.commit().expect("commit legacy shape");
    drop(database);

    let reopened = journal(&path);
    assert!(!reopened.is_recovery_mode());
    let page = reopened
        .read_stream_from("stream-a", 1, 2)
        .expect("indexed ranged read");
    assert_eq!(
        page.iter()
            .map(|event| (event.stream_version, event.global_sequence))
            .collect::<Vec<_>>(),
        [(2, 3), (3, 5)]
    );
    reopened.verify().expect("verify rebuilt stream index");
}

#[test]
fn crash_append_child() {
    let Ok(path) = std::env::var("COLOSSUS_REDB_TEST_CRASH_PATH") else {
        return;
    };
    let expected_version = std::env::var("COLOSSUS_REDB_TEST_EXPECTED_VERSION")
        .unwrap_or_else(|_| "1".into())
        .parse::<u64>()
        .expect("expected stream version");
    let journal = if let Ok(anchor_path) = std::env::var("COLOSSUS_REDB_TEST_ANCHOR_PATH") {
        journal_with_file_anchor(
            std::path::Path::new(&path),
            std::path::Path::new(&anchor_path),
        )
    } else {
        journal(std::path::Path::new(&path))
    };
    journal
        .append(event(
            "crash-stream",
            expected_version,
            expected_version.saturating_add(1),
        ))
        .expect("fault point must terminate before append returns");
    if std::env::var("COLOSSUS_REDB_TEST_FORCE_CHECKPOINT").as_deref() == Ok("true") {
        journal
            .checkpoint()
            .expect("fault point must terminate before checkpoint returns");
    }
    panic!("configured journal crash point did not terminate the child process");
}

#[test]
fn process_crash_preserves_atomic_journal_head_stream_and_outbox() {
    for (point, expected_after_crash) in [("before_commit", 1_u64), ("after_commit", 2_u64)] {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(format!("{point}.redb"));
        {
            let journal = journal(&path);
            journal
                .append(event("crash-stream", 0, 1))
                .expect("baseline append");
        }

        let child = Command::new(std::env::current_exe().expect("current test executable"))
            .args(["--exact", "tests::crash_append_child", "--nocapture"])
            .env("COLOSSUS_REDB_TEST_CRASH_PATH", &path)
            .env("COLOSSUS_REDB_TEST_CRASH_POINT", point)
            .status()
            .expect("spawn crash child");
        assert!(!child.success(), "crash child unexpectedly succeeded");

        let journal = journal(&path);
        let report = journal.verify().expect("verify recovered journal");
        assert_eq!(report.event_count, expected_after_crash);
        assert_eq!(report.last_sequence, expected_after_crash);
        assert_eq!(
            journal
                .read_stream("crash-stream")
                .expect("recovered stream")
                .len(),
            usize::try_from(expected_after_crash).expect("event count")
        );
        assert_eq!(
            journal
                .read_projection_work(1, 8)
                .expect("recovered outbox")
                .len(),
            usize::try_from(expected_after_crash).expect("outbox count")
        );
        journal
            .append(event(
                "crash-stream",
                expected_after_crash,
                expected_after_crash.saturating_add(1),
            ))
            .expect("append after crash recovery");
        assert_eq!(
            journal.verify().expect("post-recovery verify").event_count,
            expected_after_crash.saturating_add(1)
        );
    }
}

#[test]
fn startup_repairs_checkpoint_interrupted_after_interval_commit() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("checkpoint-crash.redb");
    {
        let journal = journal(&path);
        journal
            .append_batch(
                (0_u64..99)
                    .map(|version| event("crash-stream", version, version.saturating_add(1)))
                    .collect(),
            )
            .expect("baseline batch");
        assert!(
            journal
                .verify()
                .expect("verify baseline")
                .checkpoint
                .is_none()
        );
    }

    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "tests::crash_append_child", "--nocapture"])
        .env("COLOSSUS_REDB_TEST_CRASH_PATH", &path)
        .env("COLOSSUS_REDB_TEST_EXPECTED_VERSION", "99")
        .env("COLOSSUS_REDB_TEST_CRASH_POINT", "after_commit")
        .status()
        .expect("spawn checkpoint crash child");
    assert!(
        !child.success(),
        "checkpoint crash child unexpectedly succeeded"
    );

    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    let journal = journal_with_keys(&path, Arc::clone(&keys));
    let report = journal.verify().expect("verify repaired checkpoint");
    let checkpoint = report.checkpoint.expect("startup checkpoint repair");
    assert_eq!(checkpoint.global_sequence, 100);
    let anchor = keys
        .load_anchor()
        .expect("secure anchor")
        .expect("anchor record");
    assert_eq!(anchor.sequence, 100);
    assert_eq!(anchor.hash, checkpoint.record_hash);
    assert_eq!(anchor.format_version, 2);
}

#[test]
fn startup_repairs_checkpoint_after_crash_between_anchor_and_metadata() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("checkpoint-anchor-crash.redb");
    let anchor_path = directory.path().join("anchor.json");
    {
        let journal = journal_with_file_anchor(&path, &anchor_path);
        journal
            .append_batch(
                (0_u64..99)
                    .map(|version| event("crash-stream", version, version.saturating_add(1)))
                    .collect(),
            )
            .expect("baseline batch");
    }

    let child = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "tests::crash_append_child", "--nocapture"])
        .env("COLOSSUS_REDB_TEST_CRASH_PATH", &path)
        .env("COLOSSUS_REDB_TEST_ANCHOR_PATH", &anchor_path)
        .env("COLOSSUS_REDB_TEST_EXPECTED_VERSION", "99")
        .env(
            "COLOSSUS_REDB_TEST_CRASH_POINT",
            "after_anchor_before_checkpoint_commit",
        )
        .env("COLOSSUS_REDB_TEST_CRASH_SEQUENCE", "100")
        .env("COLOSSUS_REDB_TEST_FORCE_CHECKPOINT", "true")
        .status()
        .expect("spawn secure-anchor crash child");
    assert!(
        !child.success(),
        "secure-anchor crash child unexpectedly succeeded"
    );

    let journal = journal_with_file_anchor(&path, &anchor_path);
    let report = journal.verify().expect("verify repaired checkpoint");
    let checkpoint = report.checkpoint.expect("startup checkpoint repair");
    assert_eq!(checkpoint.global_sequence, 100);
    let anchor = FileAnchorKeyProvider { path: anchor_path }
        .load_anchor()
        .expect("secure anchor")
        .expect("anchor record");
    assert_eq!(anchor.sequence, 100);
    assert_eq!(anchor.hash, checkpoint.record_hash);
    assert_eq!(anchor.format_version, 2);
}

#[test]
fn encrypted_append_round_trip_and_concurrency() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("state.redb"));
    let stored = journal.append(event("stream-1", 0, 42)).expect("append");
    assert_ne!(stored.payload.ciphertext, hex::encode(br#"{"value":42}"#));
    assert_eq!(
        journal.decrypt_payload(&stored).expect("decrypt"),
        json!({"value": 42})
    );
    let conflict = journal.append(event("stream-1", 0, 43));
    assert!(matches!(
        conflict,
        Err(StoreError::Conflict { actual: 1, .. })
    ));
    let report = journal.verify().expect("verify");
    assert_eq!(report.event_count, 1);
}

#[test]
fn shared_journal_conformance_suite_passes() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("state.redb"));
    assert_journal_conformance(
        &journal,
        event("conformance", 0, 1),
        event("conformance", 0, 2),
    );
}

#[test]
fn ephemeral_redb_passes_shared_journal_conformance_suite() {
    let journal = ephemeral_journal();
    assert_journal_conformance(
        &journal,
        event("ephemeral-conformance", 0, 1),
        event("ephemeral-conformance", 0, 2),
    );
}

#[test]
fn ephemeral_redb_rejects_protected_keys() {
    let result = RedbEventJournal::open_in_memory(
        Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32])),
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
    );
    assert!(
        matches!(result, Err(StoreError::Adapter(ref message)) if message.contains("plaintext payload protection"))
    );
}

#[test]
fn shared_projection_store_conformance_suite_passes() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("state.redb"));
    assert_projection_store_conformance(&journal);
}

#[test]
fn ephemeral_redb_passes_shared_projection_store_conformance_suite() {
    assert_projection_store_conformance(&ephemeral_journal());
}

#[test]
fn projection_batch_group_rolls_back_on_conflict() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("state.redb"));
    journal
        .apply(ProjectionBatch {
            projection: "existing-v1".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: Vec::new(),
        })
        .expect("seed projection");

    let error = journal
        .apply_all(&[
            ProjectionBatch {
                projection: "new-v1".into(),
                expected_position: 0,
                through_sequence: 1,
                mutations: vec![ProjectionMutation::Upsert {
                    key: "record".into(),
                    value: json!({"value": 1}),
                }],
            },
            ProjectionBatch {
                projection: "existing-v1".into(),
                expected_position: 0,
                through_sequence: 2,
                mutations: Vec::new(),
            },
        ])
        .expect_err("second batch conflicts");

    assert!(matches!(error, StoreError::Conflict { actual: 1, .. }));
    assert_eq!(journal.position("new-v1").expect("position"), 0);
    assert!(journal.get("new-v1", "record").expect("record").is_none());
    assert_eq!(journal.position("existing-v1").expect("position"), 1);
}

#[test]
fn shared_external_work_queue_conformance_suite_passes() {
    let directory = tempdir().expect("tempdir");
    let journal = Arc::new(journal(&directory.path().join("state.redb")));
    let journal_port: Arc<dyn EventJournal> = journal.clone();
    let store_port: Arc<dyn ProjectionStore> = journal;
    let queue = JournalExternalWorkQueue::new(journal_port.clone(), store_port);
    assert_external_work_queue_conformance(
        journal_port.as_ref(),
        &queue,
        event("external-one", 0, 1),
        event("external-two", 0, 2),
    );
}

#[test]
fn canonical_repositories_pass_shared_conformance_over_encrypted_redb() {
    let directory = tempdir().expect("tempdir");
    let session_journal: Arc<dyn EventJournal> =
        Arc::new(journal(&directory.path().join("session.redb")));
    assert_session_repository_conformance(|| {
        Box::new(EventSourcedSessionRepository::new(Arc::clone(
            &session_journal,
        )))
    });

    let work_journal: Arc<dyn EventJournal> =
        Arc::new(journal(&directory.path().join("work.redb")));
    assert_work_repository_conformance(|| {
        Box::new(EventSourcedWorkRepository::new(Arc::clone(&work_journal)))
    });

    let memory_journal: Arc<dyn EventJournal> =
        Arc::new(journal(&directory.path().join("memory.redb")));
    assert_memory_repository_conformance(|| {
        Box::new(EventSourcedMemoryRepository::new(Arc::clone(
            &memory_journal,
        )))
    });

    let workflow_journal: Arc<dyn EventJournal> =
        Arc::new(journal(&directory.path().join("workflow.redb")));
    assert_workflow_repository_conformance(|| {
        Box::new(EventSourcedWorkflowRepository::new(Arc::clone(
            &workflow_journal,
        )))
    });
}

#[test]
fn external_work_checkpoint_survives_restart_without_blocking_other_consumers() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    let first_event_id;
    {
        let journal = Arc::new(journal_with_keys(&path, Arc::clone(&keys)));
        journal
            .append(event("restart-one", 0, 1))
            .expect("first append");
        journal
            .append(event("restart-two", 0, 2))
            .expect("second append");
        let journal_port: Arc<dyn EventJournal> = journal.clone();
        let store_port: Arc<dyn ProjectionStore> = journal;
        let queue = JournalExternalWorkQueue::new(journal_port, store_port);
        let pending = queue.pending("memory.tantivy-v1", 8).expect("pending");
        first_event_id = pending[0].event_id.clone();
        queue
            .acknowledge("memory.tantivy-v1", 0, &pending[0])
            .expect("acknowledge");
        assert_eq!(queue.position("memory.chroma-v1").expect("chroma"), 0);
        queue
            .record_failure(
                "memory.chroma-v1",
                Some(&pending[0]),
                "2026-07-11T00:00:00Z",
                true,
                "external_work.test",
                "temporary Chroma failure",
            )
            .expect("retry state");
    }

    let journal = Arc::new(journal_with_keys(&path, keys));
    let journal_port: Arc<dyn EventJournal> = journal.clone();
    let store_port: Arc<dyn ProjectionStore> = journal;
    let queue = JournalExternalWorkQueue::new(journal_port, store_port);
    assert_eq!(queue.position("memory.tantivy-v1").expect("position"), 1);
    assert_eq!(
        queue.pending("memory.tantivy-v1", 8).expect("remaining")[0].global_sequence,
        2
    );
    assert_eq!(
        queue.pending("memory.chroma-v1", 8).expect("chroma")[0].event_id,
        first_event_id
    );
    let retry = queue
        .retry_state("memory.chroma-v1")
        .expect("retry state")
        .expect("persisted retry state");
    assert_eq!(retry.attempts, 1);
    assert_eq!(retry.error_code, "external_work.test");
}

#[test]
fn writer_lease_is_exclusive_and_reacquirable() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let first = RedbWriterLease::acquire(&path).expect("first lease");
    assert!(matches!(
        RedbWriterLease::acquire(&path),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(first.path().ends_with("state.redb.writer.lock"));
    drop(first);
    RedbWriterLease::acquire(&path).expect("reacquired lease");
}

#[test]
fn platform_secret_cache_loads_once_for_concurrent_callers() {
    let service = format!("test-service-{}", uuid::Uuid::now_v7());
    let account = "shared-account".to_owned();
    let loads = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let service = service.clone();
            let account = account.clone();
            let loads = Arc::clone(&loads);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                cached_platform_secret(&service, &account, || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    Ok([42_u8; 32])
                })
                .expect("cached secret")
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        assert_eq!(handle.join().expect("caller"), [42_u8; 32]);
    }
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

#[test]
fn platform_secret_cache_does_not_cache_load_failures() {
    let service = format!("test-service-{}", uuid::Uuid::now_v7());
    let account = "retry-account";
    let loads = AtomicUsize::new(0);

    let first = cached_platform_secret(&service, account, || {
        loads.fetch_add(1, Ordering::SeqCst);
        Err(StoreError::KeyUnavailable("operator denied access".into()))
    });
    assert!(matches!(first, Err(StoreError::KeyUnavailable(_))));
    assert_eq!(
        cached_platform_secret(&service, account, || {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok([24_u8; 32])
        })
        .expect("retried secret"),
        [24_u8; 32]
    );
    assert_eq!(loads.load(Ordering::SeqCst), 2);
}

#[test]
fn projection_worker_catches_up_after_journal_only_restart() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append(NewEvent {
                event_type: "session.created.v1".into(),
                stream_id: "session:restarted".into(),
                payload: json!({"title": "Recovered"}),
                ..event("unused", 0, 1)
            })
            .expect("journal append before crash");
    }
    let journal = Arc::new(journal_with_keys(&path, keys));
    let journal_port: Arc<dyn EventJournal> = journal.clone();
    let store_port: Arc<dyn ProjectionStore> = journal.clone();
    let worker =
        ProjectionWorker::new(journal_port, store_port, default_handlers()).expect("worker");
    assert!(
        worker
            .status()
            .expect("lag")
            .iter()
            .all(|item| item.lag == 1)
    );
    worker.drain(16, 16).expect("catch up");
    assert_eq!(
        journal
            .get("sessions-v1", "restarted")
            .expect("record")
            .expect("session")["title"],
        json!("Recovered")
    );
}

#[test]
fn concurrent_appends_are_serialized_without_lost_events() {
    let directory = tempdir().expect("tempdir");
    let journal = Arc::new(journal(&directory.path().join("state.redb")));
    let handles = (0_u64..8)
        .map(|index| {
            let journal = Arc::clone(&journal);
            thread::spawn(move || {
                journal
                    .append(event(&format!("stream-{index}"), 0, index))
                    .expect("concurrent append")
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(journal.verify().expect("verify").event_count, 8);
}

#[test]
fn historical_keys_remain_usable_after_rotation() {
    let directory = tempdir().expect("tempdir");
    let keys = Arc::new(StaticKeyProvider::new("key-v1", [1_u8; 32]));
    let journal = journal_with_keys(&directory.path().join("state.redb"), Arc::clone(&keys));
    let first = journal.append(event("stream", 0, 1)).expect("first");
    keys.rotate("key-v2", [2_u8; 32]).expect("rotate");
    let second = journal.append(event("stream", 1, 2)).expect("second");
    assert_eq!(first.payload.key_id, "key-v1");
    assert_eq!(second.payload.key_id, "key-v2");
    assert_eq!(
        journal.decrypt_payload(&first).expect("old key"),
        json!({"value": 1})
    );
    assert_eq!(
        journal.decrypt_payload(&second).expect("new key"),
        json!({"value": 2})
    );
    journal.verify().expect("rotation verification");
}

#[test]
fn signed_checkpoint_and_secure_anchor_verify() {
    let directory = tempdir().expect("tempdir");
    let journal = journal(&directory.path().join("state.redb"));
    journal.append(event("stream-1", 0, 1)).expect("append");
    let checkpoint = journal
        .checkpoint()
        .expect("checkpoint")
        .expect("nonempty checkpoint");
    assert_eq!(checkpoint.global_sequence, 1);
    assert_eq!(
        journal.verify().expect("verify").checkpoint,
        Some(checkpoint)
    );
}

#[test]
fn incremental_startup_verifies_only_the_checkpoint_boundary_and_tail() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append_batch(
                (0_u64..100)
                    .map(|version| event("stream-1", version, version))
                    .collect(),
            )
            .expect("checkpointed history");
        journal
            .append(event("stream-1", 100, 100))
            .expect("unchecked tail");
    }

    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        let report = journal
            .startup_verification_report()
            .expect("startup report");
        assert_eq!(report.path, "incremental");
        assert_eq!(report.verified_from_sequence, Some(100));
        assert_eq!(report.verified_through_sequence, 101);
        assert_eq!(report.verified_event_count, 2);
    }

    let journal = journal_with_keys(&path, Arc::clone(&keys));
    let report = journal
        .startup_verification_report()
        .expect("second startup report");
    assert_eq!(report.path, "incremental");
    assert_eq!(report.verified_from_sequence, Some(101));
    assert_eq!(report.verified_event_count, 1);
    drop(journal);

    let journal = RedbEventJournal::open_with_startup_verification(
        &path,
        keys,
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        StartupVerificationMode::Full,
    )
    .expect("full startup");
    let report = journal
        .startup_verification_report()
        .expect("full startup report");
    assert_eq!(report.path, "full");
    assert_eq!(report.verified_from_sequence, Some(1));
    assert_eq!(report.verified_event_count, 101);
}

#[test]
fn legacy_anchor_bootstraps_once_and_is_replaced_by_version_two() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let anchor_path = directory.path().join("anchor.json");
    let legacy_anchor;
    {
        let journal = journal_with_file_anchor(&path, &anchor_path);
        journal.append(event("stream-1", 0, 1)).expect("append");
        journal.checkpoint().expect("checkpoint");
        let anchor = FileAnchorKeyProvider {
            path: anchor_path.clone(),
        }
        .load_anchor()
        .expect("anchor")
        .expect("anchor record");
        legacy_anchor = json!({"sequence": anchor.sequence, "hash": anchor.hash});
    }
    std::fs::write(
        &anchor_path,
        serde_json::to_vec(&legacy_anchor).expect("legacy anchor"),
    )
    .expect("write legacy anchor");

    let journal = journal_with_file_anchor(&path, &anchor_path);
    let report = journal
        .startup_verification_report()
        .expect("startup report");
    assert_eq!(report.path, "bootstrap_full");
    assert_eq!(report.verified_event_count, 1);
    let anchor = FileAnchorKeyProvider { path: anchor_path }
        .load_anchor()
        .expect("anchor")
        .expect("anchor record");
    assert_eq!(anchor.format_version, 2);
    assert_eq!(anchor.status, SecureAnchorStatus::Verified);
}

#[test]
fn persisted_legacy_context_shape_remains_verifiable_and_decryptable() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal.append(event("stream-1", 0, 1)).expect("append");
    }

    let database = Database::create(&path).expect("database");
    let read = database.begin_read().expect("read");
    let table = read.open_table(EVENTS).expect("events");
    let bytes = table.get(1).expect("get").expect("event").value().to_vec();
    drop(table);
    drop(read);

    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let context = value["context"].as_object_mut().expect("context object");
    for field in ["goal_id", "plan_id", "subagent_id", "skill_ids"] {
        context.remove(field);
    }

    let persisted: PersistedEventEnvelope =
        serde_json::from_value(value.clone()).expect("legacy persisted envelope");
    let aad = serde_json::to_vec(&persisted_associated_data(&persisted)).expect("aad");
    let nonce = hex::decode(value["payload"]["nonce"].as_str().expect("nonce")).expect("hex");
    let nonce: [u8; 24] = nonce.try_into().expect("nonce length");
    let plaintext = serde_json::to_vec(&json!({"value": 1})).expect("plaintext");
    let ciphertext = XChaCha20Poly1305::new((&[7_u8; 32]).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .expect("encrypt legacy payload");
    value["payload"]["ciphertext"] = json!(hex::encode(ciphertext));

    let persisted: PersistedEventEnvelope =
        serde_json::from_value(value.clone()).expect("updated persisted envelope");
    let record_hash = persisted_record_hash(&persisted).expect("legacy record hash");
    value["record_hash"] = json!(record_hash);
    let bytes = serde_json::to_vec(&value).expect("encode legacy envelope");
    let hash_bytes = serde_json::to_vec(&record_hash).expect("encode head hash");

    let write = database.begin_write().expect("write");
    {
        let mut events = write.open_table(EVENTS).expect("events");
        events.insert(1, bytes.as_slice()).expect("replace event");
        let mut metadata = write.open_table(METADATA).expect("metadata");
        metadata
            .insert("last_hash", hash_bytes.as_slice())
            .expect("replace head hash");
    }
    write.commit().expect("commit");
    drop(database);

    let reopened = journal_with_keys(&path, keys);
    assert!(!reopened.is_recovery_mode());
    reopened.verify().expect("verify legacy envelope");
    let stored = reopened
        .read_global(1, 1)
        .expect("read legacy event")
        .pop()
        .expect("legacy event");
    assert_eq!(
        reopened
            .decrypt_payload(&stored)
            .expect("decrypt legacy event"),
        json!({"value": 1})
    );
}

#[test]
fn tampering_enters_recovery_mode_on_reopen() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal.append(event("stream-1", 0, 1)).expect("append");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let read = database.begin_read().expect("read");
    let table = read.open_table(EVENTS).expect("events");
    let bytes = table.get(1).expect("get").expect("event").value().to_vec();
    drop(table);
    drop(read);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    value["event_type"] = json!("tampered.v1");
    let bytes = serde_json::to_vec(&value).expect("encode");
    let write = database.begin_write().expect("write");
    {
        let mut table = write.open_table(EVENTS).expect("events");
        table
            .insert(1, bytes.as_slice())
            .map_err(adapter_error)
            .expect("insert");
    }
    write.commit().expect("commit");
    drop(database);

    let reopened = journal_with_keys(&path, Arc::clone(&keys));
    assert!(reopened.is_recovery_mode());
    assert_eq!(
        keys.load_anchor()
            .expect("anchor")
            .expect("anchor record")
            .status,
        SecureAnchorStatus::Quarantined
    );
    assert!(matches!(
        reopened.append(event("stream-1", 1, 2)),
        Err(StoreError::RecoveryMode)
    ));
}

#[test]
fn incremental_startup_defers_anchored_history_checks_until_access() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append_batch(
                (0_u64..3)
                    .map(|version| event("stream-1", version, version))
                    .collect(),
            )
            .expect("append history");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let read = database.begin_read().expect("read");
    let table = read.open_table(EVENTS).expect("events");
    let bytes = table.get(1).expect("get").expect("event").value().to_vec();
    drop(table);
    drop(read);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    value["event_type"] = json!("tampered.v1");
    let bytes = serde_json::to_vec(&value).expect("encode");
    let write = database.begin_write().expect("write");
    {
        let mut table = write.open_table(EVENTS).expect("events");
        table.insert(1, bytes.as_slice()).expect("tamper event");
    }
    write.commit().expect("commit tamper");
    drop(database);

    let reopened = journal_with_keys(&path, Arc::clone(&keys));
    assert!(!reopened.is_recovery_mode());
    assert_eq!(
        reopened
            .startup_verification_report()
            .expect("startup report")
            .verified_event_count,
        1
    );
    let first = reopened
        .read_stream("stream-1")
        .expect("read anchored stream")
        .remove(0);
    assert!(matches!(
        reopened.decrypt_payload(&first),
        Err(StoreError::Verification(_))
    ));
    assert!(reopened.is_recovery_mode());
    assert_eq!(
        keys.load_anchor()
            .expect("anchor")
            .expect("anchor record")
            .status,
        SecureAnchorStatus::Quarantined
    );
}

#[test]
fn incremental_global_read_detects_an_anchored_prefix_gap() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append_batch(vec![event("stream-1", 0, 1), event("stream-1", 1, 2)])
            .expect("append history");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .open_table(EVENTS)
        .expect("events")
        .remove(1)
        .expect("remove old event");
    write.commit().expect("commit gap");
    drop(database);

    let reopened = journal_with_keys(&path, keys);
    assert!(!reopened.is_recovery_mode());
    assert!(matches!(
        reopened.read_global(1, 8),
        Err(StoreError::Verification(_))
    ));
    assert!(reopened.is_recovery_mode());
}

#[test]
fn incremental_outbox_read_detects_an_anchored_prefix_gap() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append_batch(vec![event("stream-1", 0, 1), event("stream-1", 1, 2)])
            .expect("append history");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    write
        .open_table(OUTBOX)
        .expect("outbox")
        .remove(1)
        .expect("remove old outbox record");
    write.commit().expect("commit gap");
    drop(database);

    let reopened = journal_with_keys(&path, keys);
    assert!(!reopened.is_recovery_mode());
    assert!(matches!(
        reopened.read_projection_work(1, 8),
        Err(StoreError::Verification(_))
    ));
    assert!(reopened.is_recovery_mode());
}

#[test]
fn full_startup_audit_detects_corruption_before_the_checkpoint() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal
            .append_batch(
                (0_u64..3)
                    .map(|version| event("stream-1", version, version))
                    .collect(),
            )
            .expect("append history");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let read = database.begin_read().expect("read");
    let table = read.open_table(EVENTS).expect("events");
    let bytes = table.get(1).expect("get").expect("event").value().to_vec();
    drop(table);
    drop(read);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    value["event_type"] = json!("tampered.v1");
    let bytes = serde_json::to_vec(&value).expect("encode");
    let write = database.begin_write().expect("write");
    {
        let mut table = write.open_table(EVENTS).expect("events");
        table.insert(1, bytes.as_slice()).expect("tamper event");
    }
    write.commit().expect("commit tamper");
    drop(database);

    let reopened = RedbEventJournal::open_with_startup_verification(
        &path,
        keys,
        Arc::new(Ed25519CheckpointSigner::new("test-signing", [8_u8; 32])),
        StartupVerificationMode::Full,
    )
    .expect("open in recovery");
    assert!(reopened.is_recovery_mode());
    assert!(
        reopened
            .recovery_reason()
            .expect("reason")
            .expect("recovery reason")
            .contains("record hash mismatch")
    );
}

#[test]
fn secure_anchor_detects_consistent_tail_truncation() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    let first_hash;
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        first_hash = journal
            .append(event("stream-1", 0, 1))
            .expect("first")
            .record_hash;
        journal.append(event("stream-1", 1, 2)).expect("second");
        journal.checkpoint().expect("checkpoint");
    }
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    {
        let mut events = write.open_table(EVENTS).expect("events");
        events.remove(2).expect("truncate event");
        let mut streams = write.open_table(STREAM_VERSIONS).expect("streams");
        streams.insert("stream-1", 1).expect("rewind stream");
        let mut outbox = write.open_table(OUTBOX).expect("outbox");
        outbox.remove(2).expect("truncate outbox");
        let mut metadata = write.open_table(METADATA).expect("metadata");
        let one = serde_json::to_vec(&1_u64).expect("sequence");
        let hash = serde_json::to_vec(&first_hash).expect("hash");
        metadata
            .insert("last_sequence", one.as_slice())
            .expect("rewind sequence");
        metadata
            .insert("last_hash", hash.as_slice())
            .expect("rewind hash");
        metadata
            .remove("latest_checkpoint")
            .expect("remove checkpoint");
    }
    write.commit().expect("commit truncation");
    drop(database);

    let reopened = journal_with_keys(&path, keys);
    assert!(reopened.is_recovery_mode());
    assert!(
        reopened
            .recovery_reason()
            .expect("reason")
            .expect("recovery reason")
            .contains("secure anchor")
    );
}

#[test]
fn projection_position_ahead_of_journal_enters_recovery_mode() {
    let directory = tempdir().expect("tempdir");
    let path = directory.path().join("state.redb");
    let keys = Arc::new(StaticKeyProvider::new("test-key", [7_u8; 32]));
    {
        let journal = journal_with_keys(&path, Arc::clone(&keys));
        journal.append(event("stream", 0, 1)).expect("append");
    }
    let database = Database::create(&path).expect("database");
    let write = database.begin_write().expect("write");
    {
        let mut positions = write
            .open_table(PROJECTION_POSITIONS)
            .expect("projection positions");
        positions
            .insert("sessions-v1", 2)
            .expect("corrupt position");
    }
    write.commit().expect("commit corruption");
    drop(database);

    let reopened = journal_with_keys(&path, keys);
    assert!(reopened.is_recovery_mode());
    assert!(
        reopened
            .recovery_reason()
            .expect("reason")
            .expect("recovery reason")
            .contains("ahead of journal head")
    );
}
