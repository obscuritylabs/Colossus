use super::{
    EventSourcedMemoryRepository, MemoryIndexRegistration, MemoryService, TantivyMemoryIndex,
    UnavailableMemoryIndex,
};
use colossus_contracts::{Actor, ActorType, MemoryRecord, MemoryScope, MemoryStatus};
use colossus_ports::{
    EventJournal, ExternalWorkQueue, MemoryIndex, MemoryRepository, ProjectionStore,
    SessionRepository,
};
use colossus_projection::JournalExternalWorkQueue;
use colossus_session::EventSourcedSessionRepository;
use colossus_testkit::{
    InMemoryEventJournal, InMemoryProjectionStore, assert_memory_index_conformance,
    assert_memory_repository_conformance,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tempfile::tempdir;

fn work_queue(journal: Arc<dyn EventJournal>) -> Arc<dyn ExternalWorkQueue> {
    let store: Arc<dyn ProjectionStore> = Arc::new(InMemoryProjectionStore::default());
    Arc::new(JournalExternalWorkQueue::new(journal, store))
}

fn actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "test-user".into(),
    }
}

fn memory(id: &str, text: &str) -> MemoryRecord {
    MemoryRecord {
        id: id.into(),
        scope: MemoryScope::Global,
        kind: "preference".into(),
        confidence: 0.9,
        source: "user".into(),
        status: MemoryStatus::Active,
        text: text.into(),
        rationale: "test".into(),
        created_at: "2026-07-09T00:00:00Z".into(),
        updated_at: "2026-07-09T00:00:00Z".into(),
        expires_at: None,
        superseded_by: None,
    }
}

#[test]
fn event_sourced_memory_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    assert_memory_repository_conformance(|| {
        Box::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)))
    });
}

#[tokio::test]
async fn tantivy_index_passes_shared_conformance() {
    let directory = tempdir().expect("tempdir");
    let index = TantivyMemoryIndex::open(directory.path()).expect("index");
    assert_memory_index_conformance(&index).await;
}

#[test]
fn canonical_lifecycle_is_reconstructed_and_supersession_is_atomic() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedMemoryRepository::new(Arc::clone(&journal));
    repository
        .create(memory("old", "Use Rust"), actor())
        .expect("create");
    let mut updated = repository.get_memory("old").expect("get").expect("memory");
    updated.text = "Use auditable Rust".into();
    updated.rationale = "updated test".into();
    updated.confidence = 0.95;
    updated.updated_at = "2026-07-10T00:00:00Z".into();
    let updated = repository.update(updated, actor()).expect("update");
    let reopened = EventSourcedMemoryRepository::new(Arc::clone(&journal));
    assert_eq!(
        reopened.get_memory("old").expect("reconstruct"),
        Some(updated.clone())
    );
    assert_eq!(updated.scope, MemoryScope::Global);
    assert_eq!(updated.source, "user");
    assert_eq!(updated.created_at, "2026-07-09T00:00:00Z");
    let (old, replacement) = repository
        .supersede("old", memory("new", "Use Rust 1.96"), actor())
        .expect("supersede");
    assert_eq!(old.status, MemoryStatus::Superseded);
    assert_eq!(old.superseded_by.as_deref(), Some("new"));
    assert_eq!(replacement.status, MemoryStatus::Active);
    assert_eq!(
        repository.list_active(10).expect("active"),
        vec![replacement]
    );
}

#[tokio::test]
async fn tantivy_index_is_idempotent_disposable_and_returns_candidate_ids() {
    let directory = tempdir().expect("tempdir");
    let index = TantivyMemoryIndex::open(directory.path()).expect("index");
    index
        .upsert(
            "event-1",
            "memory-1",
            "Rust workflow runtime",
            &json!({}),
            None,
        )
        .await
        .expect("upsert");
    index
        .upsert("event-1", "memory-1", "ignored duplicate", &json!({}), None)
        .await
        .expect("idempotent upsert");
    assert_eq!(
        index.search("Rust", 10).await.expect("search")[0].0,
        "memory-1"
    );
    index.remove("event-2", "memory-1").await.expect("remove");
    assert!(
        index
            .search("Rust", 10)
            .await
            .expect("removed search")
            .is_empty()
    );
    index
        .rebuild(&[("memory-2".into(), "Chroma candidate".into(), json!({}))])
        .await
        .expect("rebuild");
    assert_eq!(
        index.search("Chroma", 10).await.expect("rebuilt")[0].0,
        "memory-2"
    );
}

#[tokio::test]
async fn service_replays_index_events_without_resurrecting_archived_records() {
    let directory = tempdir().expect("tempdir");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    sessions
        .create_session("session-1", None, actor())
        .expect("session");
    let repository: Arc<dyn MemoryRepository> =
        Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
    let index: Arc<dyn MemoryIndex> =
        Arc::new(TantivyMemoryIndex::open(directory.path()).expect("index"));
    let queue = work_queue(Arc::clone(&journal));
    let service = MemoryService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        Arc::clone(&queue),
        Arc::clone(&index),
        Arc::clone(&sessions),
    )
    .expect("service");
    let created = service
        .create(
            MemoryScope::Session("session-1".into()),
            "preference",
            1.0,
            "Run Rust tests before completion",
            "user preference",
            None,
            actor(),
        )
        .await
        .expect("create");
    assert_eq!(
        service
            .search("Rust tests", Some("session-1"), None, 8)
            .await
            .expect("search"),
        vec![created.clone()]
    );
    let updated = service
        .update(
            &created.id,
            Some("Run Rust Clippy before completion"),
            Some("updated preference"),
            Some(0.95),
            actor(),
        )
        .await
        .expect("update");
    assert_eq!(updated.scope, created.scope);
    assert_eq!(updated.source, created.source);
    assert!(
        service
            .search("tests", Some("session-1"), None, 8)
            .await
            .expect("old text removed")
            .is_empty()
    );
    assert_eq!(
        service
            .search("Clippy", Some("session-1"), None, 8)
            .await
            .expect("updated text indexed"),
        vec![updated.clone()]
    );
    assert!(
        service
            .search("Rust tests", Some("other-session"), None, 8)
            .await
            .expect("scope filtered")
            .is_empty()
    );
    service
        .archive(&created.id, actor())
        .await
        .expect("archive");

    let reopened =
        MemoryService::new(journal, repository, queue, index, sessions).expect("reopened service");
    let persisted_status = reopened.index_status().await.expect("persisted status");
    assert_eq!(persisted_status["ready"], true);
    assert_eq!(persisted_status["lag"], 0);
    reopened.sync_index().await.expect("replay");
    assert!(
        reopened
            .search("Clippy", Some("session-1"), None, 8)
            .await
            .expect("search after restart")
            .is_empty()
    );
    assert_eq!(
        reopened
            .get(&created.id)
            .expect("get")
            .expect("record")
            .status,
        MemoryStatus::Archived
    );
}

#[tokio::test]
async fn unavailable_index_leaves_canonical_memory_usable_and_lag_visible() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    sessions
        .create_session("session-1", None, actor())
        .expect("session");
    let repository: Arc<dyn MemoryRepository> =
        Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
    let index: Arc<dyn MemoryIndex> = Arc::new(UnavailableMemoryIndex::new("index unavailable"));
    let queue = work_queue(Arc::clone(&journal));
    let service = MemoryService::new(journal, repository, queue, index, sessions).expect("service");
    assert_eq!(
        service.index_status().await.expect("empty status")["ready"],
        false
    );
    let record = service
        .create(
            MemoryScope::Global,
            "warning",
            0.8,
            "Canonical memory remains available",
            "fallback test",
            None,
            actor(),
        )
        .await
        .expect("canonical create");
    assert_eq!(
        service
            .search("Canonical", Some("session-1"), None, 8)
            .await
            .expect("fallback search"),
        vec![record]
    );
    let status = service.index_status().await.expect("status");
    assert_eq!(status["ready"], false);
    assert!(status["lag"].as_u64().is_some_and(|lag| lag > 0));
    assert!(status["last_error"].as_str().is_some());
}

#[tokio::test]
async fn failed_semantic_consumer_does_not_block_lexical_index_progress() {
    let directory = tempdir().expect("tempdir");
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let repository: Arc<dyn MemoryRepository> =
        Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
    let queue = work_queue(Arc::clone(&journal));
    let lexical: Arc<dyn MemoryIndex> =
        Arc::new(TantivyMemoryIndex::open(directory.path()).expect("index"));
    let semantic: Arc<dyn MemoryIndex> =
        Arc::new(UnavailableMemoryIndex::new("semantic adapter offline"));
    let service = MemoryService::with_indexes(
        Arc::clone(&journal),
        repository,
        queue,
        vec![
            MemoryIndexRegistration::new("memory.tantivy-v1", lexical)
                .expect("lexical registration"),
            MemoryIndexRegistration::new("memory.chroma-v1", semantic)
                .expect("semantic registration"),
        ],
        sessions,
    )
    .expect("service");

    let record = service
        .create(
            MemoryScope::Global,
            "fact",
            0.9,
            "Independent lexical progress",
            "consumer isolation test",
            None,
            actor(),
        )
        .await
        .expect("create");
    assert_eq!(
        service
            .search("lexical progress", None, None, 8)
            .await
            .expect("search"),
        vec![record]
    );
    let status = service.index_status().await.expect("status");
    assert_eq!(status["ready"], false);
    let consumers = status["consumers"].as_array().expect("consumers");
    let lexical = consumers
        .iter()
        .find(|item| item["consumer"] == "memory.tantivy-v1")
        .expect("lexical status");
    let semantic = consumers
        .iter()
        .find(|item| item["consumer"] == "memory.chroma-v1")
        .expect("semantic status");
    assert_eq!(lexical["ready"], true);
    assert_eq!(lexical["lag"], 0);
    assert_eq!(semantic["ready"], false);
    assert!(semantic["lag"].as_u64().is_some_and(|lag| lag > 0));
}

struct CountingFailureIndex {
    position: AtomicU64,
    attempts: AtomicU64,
    outcome_unknown: bool,
}

#[async_trait::async_trait]
impl MemoryIndex for CountingFailureIndex {
    fn position(&self) -> Result<u64, colossus_ports::StoreError> {
        Ok(self.position.load(Ordering::Acquire))
    }

    async fn set_position(&self, position: u64) -> Result<(), colossus_ports::StoreError> {
        self.position.store(position, Ordering::Release);
        Ok(())
    }

    async fn upsert(
        &self,
        _event_id: &str,
        _memory_id: &str,
        _text: &str,
        _metadata: &serde_json::Value,
        _embedding: Option<&[f32]>,
    ) -> Result<(), colossus_ports::StoreError> {
        self.attempts.fetch_add(1, Ordering::AcqRel);
        if self.outcome_unknown {
            Err(colossus_ports::StoreError::OutcomeUnknown(
                "fixture mutation outcome is unknown".into(),
            ))
        } else {
            Err(colossus_ports::StoreError::Adapter(
                "fixture adapter is temporarily unavailable".into(),
            ))
        }
    }

    async fn remove(
        &self,
        _event_id: &str,
        _memory_id: &str,
    ) -> Result<(), colossus_ports::StoreError> {
        self.upsert("remove", "remove", "remove", &json!({}), None)
            .await
    }

    async fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<(String, f32)>, colossus_ports::StoreError> {
        Ok(Vec::new())
    }

    async fn status(&self) -> Result<serde_json::Value, colossus_ports::StoreError> {
        Ok(json!({"ready": false, "kind": "counting-failure-fixture"}))
    }

    async fn rebuild(
        &self,
        _records: &[(String, String, serde_json::Value)],
    ) -> Result<(), colossus_ports::StoreError> {
        Ok(())
    }
}

async fn failure_service(outcome_unknown: bool) -> (MemoryService, Arc<CountingFailureIndex>) {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let repository: Arc<dyn MemoryRepository> =
        Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
    let queue = work_queue(Arc::clone(&journal));
    let fixture = Arc::new(CountingFailureIndex {
        position: AtomicU64::new(0),
        attempts: AtomicU64::new(0),
        outcome_unknown,
    });
    let index: Arc<dyn MemoryIndex> = fixture.clone();
    let service =
        MemoryService::new(journal, repository, queue, index, sessions).expect("failure service");
    service
        .create(
            MemoryScope::Global,
            "fact",
            0.9,
            "Retry telemetry fixture",
            "retry test",
            None,
            actor(),
        )
        .await
        .expect("canonical create");
    (service, fixture)
}

#[tokio::test]
async fn retryable_failure_is_durable_and_immediate_retry_is_suppressed() {
    let (service, fixture) = failure_service(false).await;
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    let error = service.sync_index().await.expect_err("backoff");
    assert!(error.to_string().contains("retry is deferred until"));
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    let status = service.index_status().await.expect("status");
    assert_eq!(status["consumers"][0]["retry"]["attempts"], 1);
    assert_eq!(status["consumers"][0]["retry"]["retryable"], true);
    assert!(status["consumers"][0]["retry"]["next_retry_at"].is_string());
}

#[tokio::test]
async fn unknown_outcome_is_durably_blocked_without_automatic_retry() {
    let (service, fixture) = failure_service(true).await;
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    assert!(matches!(
        service.sync_index().await,
        Err(colossus_ports::StoreError::OutcomeUnknown(_))
    ));
    assert_eq!(fixture.attempts.load(Ordering::Acquire), 1);
    let status = service.index_status().await.expect("status");
    assert_eq!(status["consumers"][0]["retry"]["retryable"], false);
    assert!(status["consumers"][0]["retry"]["next_retry_at"].is_null());
}

struct FailingRebuildIndex {
    position: AtomicU64,
    fail_rebuild: AtomicBool,
}

#[async_trait::async_trait]
impl MemoryIndex for FailingRebuildIndex {
    fn position(&self) -> Result<u64, colossus_ports::StoreError> {
        Ok(self.position.load(Ordering::Acquire))
    }

    async fn set_position(&self, position: u64) -> Result<(), colossus_ports::StoreError> {
        self.position.store(position, Ordering::Release);
        Ok(())
    }

    async fn upsert(
        &self,
        _event_id: &str,
        _memory_id: &str,
        _text: &str,
        _metadata: &serde_json::Value,
        _embedding: Option<&[f32]>,
    ) -> Result<(), colossus_ports::StoreError> {
        Ok(())
    }

    async fn remove(
        &self,
        _event_id: &str,
        _memory_id: &str,
    ) -> Result<(), colossus_ports::StoreError> {
        Ok(())
    }

    async fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<(String, f32)>, colossus_ports::StoreError> {
        Ok(Vec::new())
    }

    async fn status(&self) -> Result<serde_json::Value, colossus_ports::StoreError> {
        Ok(json!({"ready": true, "kind": "failing-rebuild-fixture"}))
    }

    async fn rebuild(
        &self,
        _records: &[(String, String, serde_json::Value)],
    ) -> Result<(), colossus_ports::StoreError> {
        if self.fail_rebuild.load(Ordering::Acquire) {
            Err(colossus_ports::StoreError::Adapter(
                "fixture rebuild failed after reset".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn failed_destructive_rebuild_resets_cursor_for_complete_journal_replay() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let repository: Arc<dyn MemoryRepository> =
        Arc::new(EventSourcedMemoryRepository::new(Arc::clone(&journal)));
    let fixture = Arc::new(FailingRebuildIndex {
        position: AtomicU64::new(0),
        fail_rebuild: AtomicBool::new(true),
    });
    let index: Arc<dyn MemoryIndex> = fixture.clone();
    let queue = work_queue(Arc::clone(&journal));
    let service = MemoryService::new(journal, repository, queue, index, sessions).expect("service");
    let record = service
        .create(
            MemoryScope::Global,
            "fact",
            0.9,
            "Replay every canonical event",
            "rebuild recovery test",
            None,
            actor(),
        )
        .await
        .expect("create");
    assert!(fixture.position.load(Ordering::Acquire) > 0);
    assert!(service.rebuild_index().await.is_err());
    assert_eq!(fixture.position.load(Ordering::Acquire), 0);
    assert_eq!(
        service.index_status().await.expect("lag status")["ready"],
        false
    );
    assert_eq!(
        service
            .search("canonical event", None, None, 8)
            .await
            .expect("canonical fallback during failed rebuild"),
        vec![record]
    );
    fixture.fail_rebuild.store(false, Ordering::Release);
    service.sync_index().await.expect("full replay");
    assert_eq!(
        service.index_status().await.expect("ready status")["ready"],
        true
    );
}
