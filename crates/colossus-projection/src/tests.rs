use super::{
    EffectRecoveryProjection, JournalExternalWorkQueue, ProjectedSessionRepository,
    ProjectionHandler, ProjectionWorker, default_handlers, pending_effects,
};
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, NewEvent, ProjectionBatch,
    ProjectionMutation,
};
use colossus_ports::{
    AggregateRepository, EventJournal, ExternalWorkQueue, ProjectionStore, StoreError,
};
use colossus_testkit::{InMemoryEventJournal, InMemoryProjectionStore};
use serde_json::{Value, json};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Default)]
struct RecordingProjectionStore {
    inner: InMemoryProjectionStore,
    direct_applies: AtomicUsize,
    grouped_applies: Mutex<Vec<Vec<ProjectionBatch>>>,
}

impl ProjectionStore for RecordingProjectionStore {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        self.inner.position(projection)
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        self.inner.get(projection, key)
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.inner.list(projection, key_prefix, limit)
    }

    fn list_after(
        &self,
        projection: &str,
        key_prefix: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.inner
            .list_after(projection, key_prefix, after_key, limit)
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        self.direct_applies.fetch_add(1, Ordering::Relaxed);
        self.inner.apply(batch)
    }

    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        self.grouped_applies
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?
            .push(batches.to_vec());
        for batch in batches {
            self.inner.apply(batch.clone())?;
        }
        Ok(())
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        self.inner.reset(projection)
    }
}

fn event(stream: &str, version: u64, event_type: &str, payload: Value) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: stream.into(),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "test".into(),
        },
        context: ExecutionContext {
            correlation_id: "test".into(),
            ..ExecutionContext::default()
        },
        payload,
    }
}

fn worker(
    journal: Arc<InMemoryEventJournal>,
    store: Arc<InMemoryProjectionStore>,
) -> ProjectionWorker {
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store;
    ProjectionWorker::new(journal_port, store_port, default_handlers()).expect("worker")
}

#[test]
fn passive_projection_checkpoints_are_grouped() {
    let journal = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event(
            "session:one",
            0,
            "session.created.v1",
            json!({"title": "Session"}),
        ))
        .expect("append");
    let store = Arc::new(RecordingProjectionStore::default());
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store.clone();
    let worker =
        ProjectionWorker::new(journal_port, store_port, default_handlers()).expect("worker");

    let report = worker.run_once(8).expect("projection run");

    assert_eq!(report.applied, 6);
    assert!(report.projections.iter().all(|status| status.position == 1));
    assert_eq!(store.direct_applies.load(Ordering::Relaxed), 1);
    let grouped = store.grouped_applies.lock().expect("grouped applies");
    assert_eq!(grouped.len(), 1);
    assert_eq!(
        grouped[0]
            .iter()
            .map(|batch| batch.projection.as_str())
            .collect::<Vec<_>>(),
        vec![
            "work-v1",
            "memory-v1",
            "workflows-v1",
            "effects-recovery-v1",
            "session-activity-v5"
        ]
    );
}

#[test]
fn external_work_consumers_advance_independently_and_replay_after_reset() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let store = Arc::new(InMemoryProjectionStore::default());
    journal
        .append(event("memory:one", 0, "memory.created.v1", json!({})))
        .expect("first append");
    journal
        .append(event("memory:two", 0, "memory.created.v1", json!({})))
        .expect("second append");
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store;
    let queue = JournalExternalWorkQueue::new(journal_port, store_port);

    let lexical = queue.pending("memory.tantivy-v1", 8).expect("lexical work");
    let semantic = queue.pending("memory.chroma-v1", 8).expect("semantic work");
    assert_eq!(lexical, semantic);
    assert_eq!(lexical.len(), 2);

    assert_eq!(
        queue
            .acknowledge("memory.tantivy-v1", 0, &lexical[0])
            .expect("lexical acknowledge"),
        1
    );
    assert_eq!(queue.position("memory.tantivy-v1").expect("lexical"), 1);
    assert_eq!(queue.position("memory.chroma-v1").expect("semantic"), 0);
    assert!(matches!(
        queue.acknowledge("memory.tantivy-v1", 0, &lexical[1]),
        Err(StoreError::Adapter(_))
    ));

    queue.reset("memory.tantivy-v1").expect("reset");
    assert_eq!(
        queue.pending("memory.tantivy-v1", 8).expect("replay"),
        lexical
    );
}

#[test]
fn lag_catches_up_idempotently_and_rebuilds() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let store = Arc::new(InMemoryProjectionStore::default());
    journal
        .append(event(
            "session:one",
            0,
            "session.created.v1",
            json!({"title": "First"}),
        ))
        .expect("append");
    let worker = worker(Arc::clone(&journal), Arc::clone(&store));
    assert!(
        worker
            .status()
            .expect("status")
            .iter()
            .all(|item| item.lag == 1)
    );
    assert_eq!(worker.drain(8, 8).expect("drain").applied, 6);
    assert_eq!(worker.run_once(8).expect("rerun").applied, 0);
    assert_eq!(
        store
            .get("sessions-v1", "one")
            .expect("get")
            .expect("session")["title"],
        json!("First")
    );
    store
        .apply(colossus_contracts::ProjectionBatch {
            projection: "unrelated-v1".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: Vec::new(),
        })
        .expect("unrelated");
    assert!(worker.rebuild("sessions-v1").expect("rebuild").projections[0].ready);
}

#[test]
fn effect_recovery_projection_tracks_nonterminal_tail_and_removes_terminal_effects() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let store = Arc::new(InMemoryProjectionStore::default());
    for (version, event_type) in [
        (0, "effect.requested.v1"),
        (1, "effect.started.v1"),
        (2, "effect.chunk_released.v1"),
    ] {
        journal
            .append(event("effect:one", version, event_type, json!({})))
            .expect("append effect event");
    }
    let journal_port: Arc<dyn EventJournal> = journal.clone();
    let store_port: Arc<dyn ProjectionStore> = store.clone();
    let worker = ProjectionWorker::new(
        journal_port,
        store_port,
        vec![Arc::new(EffectRecoveryProjection)],
    )
    .expect("worker");

    worker.drain(8, 8).expect("drain pending effect");
    let pending = pending_effects(store.as_ref(), 8).expect("pending effects");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stream_id, "effect:one");
    assert_eq!(pending[0].started_stream_version, 2);
    assert_eq!(pending[0].latest_stream_version, 3);

    journal
        .append(event(
            "effect:one",
            3,
            "effect.outcome_unknown.v1",
            json!({}),
        ))
        .expect("append terminal event");
    worker.drain(8, 8).expect("drain terminal effect");
    assert!(
        pending_effects(store.as_ref(), 8)
            .expect("pending effects")
            .is_empty()
    );
}

struct FailingProjection;

impl ProjectionHandler for FailingProjection {
    fn name(&self) -> &'static str {
        "failing-v1"
    }

    fn project(
        &self,
        _store: &dyn ProjectionStore,
        _event: &colossus_contracts::EventEnvelope,
        _payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        Err(StoreError::Adapter("injected failure".into()))
    }
}

struct NoopProjection;

impl ProjectionHandler for NoopProjection {
    fn name(&self) -> &'static str {
        "noop-v1"
    }

    fn project(
        &self,
        _store: &dyn ProjectionStore,
        _event: &colossus_contracts::EventEnvelope,
        _payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        Ok(Vec::new())
    }
}

struct SlowProjection {
    active: AtomicUsize,
    max_active: AtomicUsize,
}

impl SlowProjection {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
        }
    }
}

impl ProjectionHandler for SlowProjection {
    fn name(&self) -> &'static str {
        "slow-v1"
    }

    fn project(
        &self,
        _store: &dyn ProjectionStore,
        _event: &colossus_contracts::EventEnvelope,
        _payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

#[test]
fn concurrent_drains_are_serialized() {
    const CALLERS: usize = 8;

    let journal = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event("session:one", 0, "session.created.v1", json!({})))
        .expect("append");
    let store = Arc::new(InMemoryProjectionStore::default());
    let projection = Arc::new(SlowProjection::new());
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store.clone();
    let worker = Arc::new(
        ProjectionWorker::new(journal_port, store_port, vec![projection.clone()]).expect("worker"),
    );
    let start = Arc::new(Barrier::new(CALLERS));
    let callers = (0..CALLERS)
        .map(|_| {
            let worker = worker.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                worker.drain(1, 1)
            })
        })
        .collect::<Vec<_>>();

    for caller in callers {
        caller.join().expect("caller").expect("drain");
    }

    assert_eq!(projection.max_active.load(Ordering::SeqCst), 1);
    assert_eq!(store.position("slow-v1").expect("position"), 1);
}

#[test]
fn one_projection_round_honors_its_batch_and_round_bounds() {
    let journal = Arc::new(InMemoryEventJournal::default());
    for index in 0..3 {
        journal
            .append(event(
                &format!("audit:{index}"),
                0,
                "worker.ipc.accepted.v1",
                json!({}),
            ))
            .expect("append");
    }
    let store = Arc::new(InMemoryProjectionStore::default());
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store;
    let worker = ProjectionWorker::new(journal_port, store_port, vec![Arc::new(NoopProjection)])
        .expect("worker");

    let report = worker.drain(1, 1).expect("bounded drain");
    assert_eq!(report.applied, 1);
    assert_eq!(report.projections[0].position, 1);
    assert_eq!(report.projections[0].lag, 2);
    assert!(!report.projections[0].ready);
}

#[test]
fn handler_failure_never_advances_position() {
    let journal = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event("session:one", 0, "session.created.v1", json!({})))
        .expect("append");
    let store = Arc::new(InMemoryProjectionStore::default());
    let journal_port: Arc<dyn EventJournal> = journal;
    let store_port: Arc<dyn ProjectionStore> = store.clone();
    let worker = ProjectionWorker::new(journal_port, store_port, vec![Arc::new(FailingProjection)])
        .expect("worker");
    assert!(worker.run_once(1).is_err());
    assert_eq!(store.position("failing-v1").expect("position"), 0);
}

#[test]
fn projected_repositories_serve_replayed_state() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let store = Arc::new(InMemoryProjectionStore::default());
    journal
        .append(event(
            "session:one",
            0,
            "session.created.v1",
            json!({"title": "Session"}),
        ))
        .expect("append");
    journal
        .append(event(
            "session:one",
            1,
            "session.message.appended.v1",
            json!({
                "run_id": "run-1",
                "sequence": 1,
                "message": {
                    "role": "user",
                    "content": "private message",
                    "tool_call_id": null,
                    "tool_calls": [],
                }
            }),
        ))
        .expect("message");
    worker(journal, Arc::clone(&store))
        .drain(8, 8)
        .expect("drain");
    let store_port: Arc<dyn ProjectionStore> = store;
    let repository = ProjectedSessionRepository::new(store_port);
    assert_eq!(repository.list(10).expect("list").len(), 1);
    let record = repository.get("one").expect("get").expect("record");
    assert_eq!(record["title"], json!("Session"));
    assert_eq!(record["message_count"], json!(1));
    assert_eq!(record["last_user_preview"], json!("private message"));
    assert!(record.get("message").is_none());
}

#[test]
fn work_memory_and_workflow_reducers_reconstruct_current_views() {
    let journal = Arc::new(InMemoryEventJournal::default());
    let store = Arc::new(InMemoryProjectionStore::default());
    journal
        .append(event(
            "task:one",
            0,
            "task.created.v1",
            json!({"record": {
                "id": "one",
                "session_id": "session-1",
                "title": "Task",
                "description": "",
                "status": "pending",
                "created_at": "2026-07-09T00:00:00Z",
                "updated_at": "2026-07-09T00:00:00Z"
            }}),
        ))
        .expect("task");
    journal
        .append(event(
            "task:one",
            1,
            "task.updated.v1",
            json!({"record": {
                "id": "one",
                "session_id": "session-1",
                "title": "Task",
                "description": "done",
                "status": "completed",
                "created_at": "2026-07-09T00:00:00Z",
                "updated_at": "2026-07-10T00:00:00Z"
            }}),
        ))
        .expect("task updated");
    journal
        .append(event(
            "memory:one",
            0,
            "memory.created.v1",
            json!({"record": {"id": "one", "status": "active"}}),
        ))
        .expect("memory created");
    journal
        .append(event(
            "memory:one",
            1,
            "memory.updated.v1",
            json!({"record": {"id": "one", "status": "active", "text": "updated"}}),
        ))
        .expect("memory updated");
    journal
        .append(event(
            "memory:one",
            2,
            "memory.archived.v1",
            json!({"updated_at": "2026-07-09T00:00:00Z"}),
        ))
        .expect("memory archived");
    journal
        .append(event(
            "workflow-run:one",
            0,
            "workflow.run.started.v1",
            json!({
                "workflow_name": "example",
                "workflow_version": "1.0.0",
                "workflow_hash": "hash",
                "inputs": {},
            }),
        ))
        .expect("workflow started");
    journal
        .append(event(
            "workflow-run:one",
            1,
            "workflow.run.completed.v1",
            json!({"outputs": {"done": true}}),
        ))
        .expect("workflow completed");
    worker(journal, Arc::clone(&store))
        .drain(16, 16)
        .expect("drain");

    assert_eq!(
        store
            .get("work-v1", "task:one")
            .expect("work")
            .expect("task")["title"],
        json!("Task")
    );
    assert_eq!(
        store
            .get("work-v1", "task:one")
            .expect("work")
            .expect("task")["status"],
        json!("completed")
    );
    assert_eq!(
        store
            .get("memory-v1", "one")
            .expect("memory")
            .expect("record")["status"],
        json!("archived")
    );
    assert_eq!(
        store
            .get("memory-v1", "one")
            .expect("memory")
            .expect("record")["text"],
        json!("updated")
    );
    let run = store
        .get("workflows-v1", "run:one")
        .expect("workflow")
        .expect("run");
    assert_eq!(run["status"], json!("completed"));
    assert_eq!(run["outputs"], json!({"done": true}));
}
