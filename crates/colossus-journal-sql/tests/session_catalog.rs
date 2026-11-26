//! Shared session discovery workload against the file-backed embedded journals.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use colossus_contracts::{
    Actor, ActorType, EventEnvelope, ModelContent, ModelContentPart, ModelMessage,
    ModelMessageRole, NewEvent, ProjectionWorkItem, SessionMessageAppend, SignedCheckpoint,
    StartupVerificationMode,
};
use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal, StaticKeyProvider};
use colossus_journal_sql::{SqlEventJournal, SqlJournalBackend};
use colossus_ports::{
    EventJournal, KeyProvider, ProjectionStore, SessionRepository, StoreError, VerificationReport,
};
use colossus_projection::{ProjectionWorker, SESSION_SUMMARY_PROJECTION, default_handlers};
use colossus_session::{
    EventSourcedSessionRepository, IndexedSessionCatalog, SESSION_CATALOG_PROJECTION,
    SessionCatalogProjection,
};
use serde_json::Value;

const MESSAGE_EVENT: &str = "session.message.appended.v1";
const MESSAGES_PER_SESSION: usize = 128;
const PAGE_LIMIT: usize = 5;
const PAGE_BYTES: usize = 16 * 1024;
const MULTIPART_TEXT: &str = "Multipart fixture kept in canonical message";

#[test]
fn redb_session_reads_respect_the_budget() {
    assert_read_budgets(Backend::Redb);
}

#[test]
fn sqlite_session_reads_respect_the_budget() {
    assert_read_budgets(Backend::Sql(SqlJournalBackend::Sqlite));
}

#[test]
fn turso_session_reads_respect_the_budget() {
    assert_read_budgets(Backend::Sql(SqlJournalBackend::Turso));
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

fn assert_read_budgets(backend: Backend) {
    let directory = tempfile::tempdir().expect("private fixture directory");
    let path = directory.path().join("journal.db");
    let keys: Arc<dyn KeyProvider> = Arc::new(StaticKeyProvider::new("catalog-fixture", [17; 32]));
    {
        let store = backend.open(&path, keys.clone());
        seed(store.clone());
        let mut handlers = default_handlers();
        handlers.push(Arc::new(SessionCatalogProjection));
        let projection_worker =
            ProjectionWorker::new(store.clone(), store.clone(), handlers).unwrap();
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
    let journal = Arc::new(ObservedReads::new(reopened.clone()));
    let legacy = EventSourcedSessionRepository::new(journal.clone());
    let index = Arc::new(IndexedSessionCatalog::new(reopened.clone()));
    let repository: Box<dyn SessionRepository> =
        Box::new(EventSourcedSessionRepository::new(journal.clone()).with_catalog(index.clone()));

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
    assert_eq!(recent, legacy.list_sessions(2).unwrap());
    let mut canonical_archive = legacy.list_archived_sessions(1).unwrap();
    assert_eq!(
        canonical_archive[0].last_user_preview.take().as_deref(),
        Some(MULTIPART_TEXT)
    );
    assert_eq!(archived, canonical_archive);

    // Exercise the same metadata fallback used on upgrade with a durable legacy
    // summary projection and a missing ordered catalog.
    reopened.reset(SESSION_CATALOG_PROJECTION).unwrap();
    for (include_archived, limit, mut expected) in [
        (false, 2, recent.clone()),
        (false, 1, vec![latest.clone().unwrap()]),
        (true, 1, archived.clone()),
    ] {
        for summary in &mut expected {
            summary.last_user_preview = None;
        }
        let (fallback, cost) = journal.measure(|| {
            index
                .recent_from_projected_summaries(
                    journal.as_ref(),
                    SESSION_SUMMARY_PROJECTION,
                    include_archived,
                    limit,
                    None,
                )
                .unwrap()
        });
        assert_eq!(fallback, expected, "{backend:?}");
        assert_eq!(cost.complete_stream_reads, 0, "{backend:?}: {cost:?}");
        assert_eq!(cost.global_reads, 0, "{backend:?}: {cost:?}");
        assert_eq!(cost.outbox_reads, 0, "{backend:?}: {cost:?}");
        assert_eq!(cost.message_decryptions, 0, "{backend:?}: {cost:?}");
        assert_eq!(cost.stream_enumerations, 1, "{backend:?}: {cost:?}");
        assert!(cost.largest_range <= 1, "{backend:?}: {cost:?}");
    }
    assert_eq!(reopened.position(SESSION_CATALOG_PROJECTION).unwrap(), 0);
    let mut handlers = default_handlers();
    handlers.push(Arc::new(SessionCatalogProjection));
    assert!(
        ProjectionWorker::new(reopened.clone(), reopened, handlers)
            .unwrap()
            .drain(32, 32)
            .unwrap()
            .projections
            .iter()
            .all(|status| status.ready)
    );

    let archived_page = repository
        .list_messages_page("saved-archive", None, 1, PAGE_BYTES)
        .unwrap();
    assert_eq!(
        archived_page.messages[0].message.content.plain_text(),
        MULTIPART_TEXT
    );
    let recent = &recent[0];
    assert_eq!(recent.message_count, MESSAGES_PER_SESSION as u64);
    assert_eq!(recent.title.as_deref(), Some("Renamed session"));
    assert_eq!(recent.last_run_id.as_deref(), Some("catalog-run"));
    assert_eq!(
        recent.last_user_preview.as_deref(),
        Some("Synthetic catalog fixture")
    );

    let (first, first_cost) = journal.measure(|| {
        repository
            .list_messages_page("recent", None, PAGE_LIMIT, PAGE_BYTES)
            .unwrap()
    });
    let (older, older_cost) = journal.measure(|| {
        repository
            .list_messages_page("recent", first.before_sequence, PAGE_LIMIT, PAGE_BYTES)
            .unwrap()
    });
    let (legacy_first, legacy_cost) = journal.measure(|| {
        legacy
            .list_messages_page("recent", None, PAGE_LIMIT, PAGE_BYTES)
            .unwrap()
    });
    assert_eq!(legacy_first, first);
    assert!(first.has_more && older.has_more);

    // Measure a real complete read as a positive control.
    let (transcript, transcript_cost) =
        journal.measure(|| repository.list_messages("recent").unwrap());
    assert_eq!(transcript.len(), MESSAGES_PER_SESSION);
    assert_eq!(transcript_cost.complete_stream_reads, 1);
    assert_eq!(transcript_cost.message_decryptions, MESSAGES_PER_SESSION);
    let boundary = MESSAGES_PER_SESSION - PAGE_LIMIT;
    assert_eq!(first.messages, transcript[boundary..]);
    assert_eq!(older.messages, transcript[boundary - PAGE_LIMIT..boundary]);

    eprintln!("{backend:?}: list={list_cost:?}, latest={latest_cost:?}, archived={archive_cost:?}");
    for cost in [&list_cost, &latest_cost, &archive_cost] {
        assert_eq!(
            *cost,
            QueryCost::default(),
            "{backend:?}: catalog query read the journal"
        );
    }
    eprintln!(
        "{backend:?}: first={first_cost:?}, older={older_cost:?}, legacy_first={legacy_cost:?}"
    );
    for cost in [&first_cost, &older_cost, &legacy_cost] {
        assert_eq!(cost.complete_stream_reads, 0, "{backend:?}: {cost:?}");
        assert_eq!(cost.stream_enumerations, 0, "{backend:?}: {cost:?}");
        assert_eq!(
            cost.message_decryptions,
            PAGE_LIMIT + 1,
            "{backend:?}: {cost:?}"
        );
        assert!(
            cost.ranged_envelopes <= PAGE_LIMIT + 3,
            "{backend:?}: {cost:?}"
        );
        assert!(cost.largest_range <= 8, "{backend:?}: {cost:?}");
    }
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
        let mut messages: Vec<_> = (0..MESSAGES_PER_SESSION)
            .map(|index| SessionMessageAppend {
                message: ModelMessage {
                    agent_message_origin: None,
                    role: ModelMessageRole::User,
                    content: if id == "saved-archive" && index + 1 == MESSAGES_PER_SESSION {
                        ModelContent::Parts(vec![ModelContentPart::Text {
                            text: MULTIPART_TEXT.into(),
                        }])
                    } else {
                        "Synthetic catalog fixture".into()
                    },
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor: actor.clone(),
            })
            .collect();
        let tail = if id == "recent" {
            messages.split_off(MESSAGES_PER_SESSION - 6)
        } else {
            Vec::new()
        };
        repository
            .append_messages(id, "catalog-run", messages)
            .unwrap();
        if !tail.is_empty() {
            repository
                .set_title(id, "Renamed session", actor.clone())
                .unwrap();
            repository.append_messages(id, "catalog-run", tail).unwrap();
            repository
                .set_title(id, "Renamed session", actor.clone())
                .unwrap();
        }
    }
    repository
        .set_session_archived("saved-archive", true, 0, actor)
        .unwrap();
}

#[derive(Debug, Default, PartialEq, Eq)]
struct QueryCost {
    complete_stream_reads: usize,
    stream_enumerations: usize,
    global_reads: usize,
    outbox_reads: usize,
    ranged_envelopes: usize,
    largest_range: usize,
    message_decryptions: usize,
}

struct ObservedReads {
    journal: Arc<dyn EventJournal>,
    complete_stream_reads: AtomicUsize,
    stream_enumerations: AtomicUsize,
    global_reads: AtomicUsize,
    outbox_reads: AtomicUsize,
    ranged_envelopes: AtomicUsize,
    largest_range: AtomicUsize,
    message_decryptions: AtomicUsize,
}

impl ObservedReads {
    fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self {
            journal,
            complete_stream_reads: AtomicUsize::new(0),
            stream_enumerations: AtomicUsize::new(0),
            global_reads: AtomicUsize::new(0),
            outbox_reads: AtomicUsize::new(0),
            ranged_envelopes: AtomicUsize::new(0),
            largest_range: AtomicUsize::new(0),
            message_decryptions: AtomicUsize::new(0),
        }
    }

    fn measure<T>(&self, query: impl FnOnce() -> T) -> (T, QueryCost) {
        self.snapshot_and_reset();
        let result = query();
        (result, self.snapshot_and_reset())
    }

    fn snapshot_and_reset(&self) -> QueryCost {
        QueryCost {
            complete_stream_reads: self.complete_stream_reads.swap(0, Ordering::Relaxed),
            stream_enumerations: self.stream_enumerations.swap(0, Ordering::Relaxed),
            global_reads: self.global_reads.swap(0, Ordering::Relaxed),
            outbox_reads: self.outbox_reads.swap(0, Ordering::Relaxed),
            ranged_envelopes: self.ranged_envelopes.swap(0, Ordering::Relaxed),
            largest_range: self.largest_range.swap(0, Ordering::Relaxed),
            message_decryptions: self.message_decryptions.swap(0, Ordering::Relaxed),
        }
    }

    fn observe_range(&self, count: usize) {
        self.ranged_envelopes.fetch_add(count, Ordering::Relaxed);
        self.largest_range.fetch_max(count, Ordering::Relaxed);
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
        let events = self.journal.read_stream_from(id, after, limit)?;
        self.observe_range(events.len());
        Ok(events)
    }

    fn read_stream_backwards(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let events = self.journal.read_stream_backwards(id, before, limit)?;
        self.observe_range(events.len());
        Ok(events)
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.stream_enumerations.fetch_add(1, Ordering::Relaxed);
        self.journal.list_stream_ids(prefix, after, limit)
    }

    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.global_reads.fetch_add(1, Ordering::Relaxed);
        let events = self.journal.read_global(from, limit)?;
        self.observe_range(events.len());
        Ok(events)
    }

    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.outbox_reads.fetch_add(1, Ordering::Relaxed);
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
