//! Shared session discovery workload against the file-backed embedded journals.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use colossus_contracts::{
    Actor, ActorType, EventEnvelope, ModelMessage, ModelMessageRole, NewEvent, ProjectionWorkItem,
    SessionMessageAppend, SignedCheckpoint, StartupVerificationMode,
};
use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal, StaticKeyProvider};
use colossus_journal_sql::{SqlEventJournal, SqlJournalBackend};
use colossus_ports::{
    EventJournal, KeyProvider, ProjectionStore, SessionRepository, StoreError, VerificationReport,
};
use colossus_projection::{ProjectionWorker, default_handlers};
use colossus_session::EventSourcedSessionRepository;
use serde_json::Value;

const MESSAGE_EVENT: &str = "session.message.appended.v1";
const MESSAGES_PER_SESSION: usize = 8;

#[test]
#[ignore = "explicit acceptance gate for a payload-free session catalog"]
fn redb_catalog_reads_without_decrypting_messages() {
    assert_payload_free(Backend::Redb);
}

#[test]
#[ignore = "explicit acceptance gate for a payload-free session catalog"]
fn sqlite_catalog_reads_without_decrypting_messages() {
    assert_payload_free(Backend::Sql(SqlJournalBackend::Sqlite));
}

#[test]
#[ignore = "explicit acceptance gate for a payload-free session catalog"]
fn turso_catalog_reads_without_decrypting_messages() {
    assert_payload_free(Backend::Sql(SqlJournalBackend::Turso));
}

#[derive(Clone, Copy, Debug)]
enum Backend {
    Redb,
    Sql(SqlJournalBackend),
}

trait FixtureStore: EventJournal + ProjectionStore {}

impl<T: EventJournal + ProjectionStore> FixtureStore for T {}

impl Backend {
    fn open(self, path: &Path, keys: Arc<dyn KeyProvider>) -> Arc<dyn FixtureStore> {
        let signer = Arc::new(Ed25519CheckpointSigner::new("catalog-test", [19; 32]));
        let mode = StartupVerificationMode::Incremental;
        match self {
            Self::Redb => Arc::new(
                RedbEventJournal::open_with_startup_verification(path, keys, signer, mode)
                    .expect("open protected redb"),
            ),
            Self::Sql(backend) => Arc::new(
                SqlEventJournal::open(path, backend, keys, signer, mode)
                    .expect("open protected SQL journal"),
            ),
        }
    }
}

fn assert_payload_free(backend: Backend) {
    let directory = tempfile::tempdir().expect("private fixture directory");
    let path = directory.path().join("journal.db");
    let keys: Arc<dyn KeyProvider> = Arc::new(StaticKeyProvider::new("catalog-fixture", [17; 32]));
    {
        let store = backend.open(&path, keys.clone());
        seed(store.clone());
        let projection_worker =
            ProjectionWorker::new(store.clone(), store.clone(), default_handlers()).unwrap();
        let projection_report = projection_worker.drain(32, 32).unwrap();
        assert!(
            projection_report
                .projections
                .iter()
                .all(|status| status.ready)
        );
        assert!(store.checkpoint().unwrap().is_some(), "{backend:?}");
    }

    let reopened = backend.open(&path, keys);
    assert!(!reopened.is_recovery_mode(), "{backend:?}");
    let journal = Arc::new(ObservedReads::new(reopened));
    let repository: Box<dyn SessionRepository> =
        Box::new(EventSourcedSessionRepository::new(journal.clone()));

    // Exercise each query separately so one implementation cannot hide the other's work.
    let (recent, list_cost) = journal.measure(|| repository.list_sessions(2).unwrap());
    let (latest, latest_cost) = journal.measure(|| repository.latest_session().unwrap());
    let (archived, archive_cost) =
        journal.measure(|| repository.list_archived_sessions(1).unwrap());
    assert_eq!(
        recent
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        ["recent", "older"]
    );
    assert_eq!(latest.as_ref(), recent.first());
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, "saved-archive");
    let recent = &recent[0];
    assert_eq!(recent.message_count, MESSAGES_PER_SESSION as u64);
    assert_eq!(recent.last_run_id.as_deref(), Some("catalog-run"));
    assert_eq!(
        recent.last_user_preview.as_deref(),
        Some("Synthetic catalog fixture")
    );

    // Show that the guard detects a real decrypt on this reopened, protected store.
    let (transcript, transcript_cost) =
        journal.measure(|| repository.list_messages("recent").unwrap());
    assert_eq!(transcript.len(), MESSAGES_PER_SESSION);
    assert_eq!(transcript_cost.message_decryptions, MESSAGES_PER_SESSION);

    eprintln!("{backend:?}: list={list_cost:?}, latest={latest_cost:?}, archived={archive_cost:?}");
    assert_eq!(
        [
            list_cost.message_decryptions,
            latest_cost.message_decryptions,
            archive_cost.message_decryptions,
        ],
        [0, 0, 0],
        "{backend:?}: recent list, latest selection, and archive list must be payload-free"
    );
}

fn seed(journal: Arc<dyn EventJournal>) {
    let repository = EventSourcedSessionRepository::new(journal);
    let actor = Actor {
        actor_type: ActorType::User,
        id: "catalog-test".into(),
    };
    // Ascending IDs match creation order in case appends share the same timestamp.
    for (id, title) in [
        ("older", "Older session"),
        ("recent", "Latest active session"),
        ("saved-archive", "Newest session, archived"),
    ] {
        repository
            .create_session(id, Some(title), actor.clone())
            .unwrap();
        let messages = (0..MESSAGES_PER_SESSION)
            .map(|_| SessionMessageAppend {
                message: ModelMessage {
                    agent_message_origin: None,
                    role: ModelMessageRole::User,
                    content: "Synthetic catalog fixture".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor: actor.clone(),
            })
            .collect();
        repository
            .append_messages(id, "catalog-run", messages)
            .unwrap();
    }
    repository
        .set_session_archived("saved-archive", true, 0, actor)
        .unwrap();
}

#[derive(Debug)]
struct QueryCost {
    complete_stream_reads: usize,
    message_decryptions: usize,
}

struct ObservedReads {
    journal: Arc<dyn EventJournal>,
    complete_stream_reads: AtomicUsize,
    message_decryptions: AtomicUsize,
}

impl ObservedReads {
    fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self {
            journal,
            complete_stream_reads: AtomicUsize::new(0),
            message_decryptions: AtomicUsize::new(0),
        }
    }

    fn measure<T>(&self, query: impl FnOnce() -> T) -> (T, QueryCost) {
        let start = self.snapshot();
        let result = query();
        let end = self.snapshot();
        (
            result,
            QueryCost {
                complete_stream_reads: end.complete_stream_reads - start.complete_stream_reads,
                message_decryptions: end.message_decryptions - start.message_decryptions,
            },
        )
    }

    fn snapshot(&self) -> QueryCost {
        QueryCost {
            complete_stream_reads: self.complete_stream_reads.load(Ordering::Relaxed),
            message_decryptions: self.message_decryptions.load(Ordering::Relaxed),
        }
    }
}

impl EventJournal for ObservedReads {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.journal.append(event)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.journal.append_batch(events)
    }

    fn read_stream(&self, id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.complete_stream_reads.fetch_add(1, Ordering::Relaxed);
        self.journal.read_stream(id)
    }

    fn read_stream_from(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.journal.read_stream_from(id, after, limit)
    }

    fn read_stream_backwards(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.journal.read_stream_backwards(id, before, limit)
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.journal.list_stream_ids(prefix, after, limit)
    }

    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.journal.read_global(from, limit)
    }

    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.journal.read_projection_work(from, limit)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.journal.head()
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        if event.event_type == MESSAGE_EVENT {
            self.message_decryptions.fetch_add(1, Ordering::Relaxed);
        }
        self.journal.decrypt_payload(event)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.journal.verify()
    }

    fn is_recovery_mode(&self) -> bool {
        self.journal.is_recovery_mode()
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.journal.checkpoint()
    }
}
