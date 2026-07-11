//! Deterministic, restartable projections over the authoritative event journal.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{EventEnvelope, ProjectionBatch, ProjectionMutation, ProjectionStatus};
use colossus_ports::{AggregateRepository, EventJournal, ProjectionStore, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Pure event-to-projection reducer.
pub trait ProjectionHandler: Send + Sync {
    /// Stable name containing a schema version.
    fn name(&self) -> &'static str;

    /// Produce record mutations for one journal event.
    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError>;
}

/// Result of one bounded worker or drain operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRunReport {
    /// Number of event-handler applications committed.
    pub applied: u64,
    /// Current state of every registered projection.
    pub projections: Vec<ProjectionStatus>,
}

/// Replays journal outbox entries into disposable projections.
pub struct ProjectionWorker {
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn ProjectionStore>,
    handlers: Vec<Arc<dyn ProjectionHandler>>,
}

impl ProjectionWorker {
    /// Build a worker with an explicit journal, store, and handler set.
    pub fn new(
        journal: Arc<dyn EventJournal>,
        store: Arc<dyn ProjectionStore>,
        handlers: Vec<Arc<dyn ProjectionHandler>>,
    ) -> Result<Self, StoreError> {
        let mut names = handlers
            .iter()
            .map(|handler| handler.name())
            .collect::<Vec<_>>();
        names.sort_unstable();
        if names.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(StoreError::Adapter(
                "projection handler names must be unique".into(),
            ));
        }
        Ok(Self {
            journal,
            store,
            handlers,
        })
    }

    /// Apply up to `limit_per_projection` pending events for every handler.
    pub fn run_once(&self, limit_per_projection: usize) -> Result<ProjectionRunReport, StoreError> {
        let mut applied = 0_u64;
        for handler in &self.handlers {
            let mut position = self.store.position(handler.name())?;
            let work = self
                .journal
                .read_projection_work(position.saturating_add(1), limit_per_projection)?;
            for item in work {
                let expected_sequence = position.saturating_add(1);
                if item.global_sequence != expected_sequence {
                    return Err(StoreError::Verification(format!(
                        "projection {} expected outbox sequence {expected_sequence}, got {}",
                        handler.name(),
                        item.global_sequence
                    )));
                }
                let event = self
                    .journal
                    .read_global(item.global_sequence, 1)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "projection outbox sequence {} has no journal event",
                            item.global_sequence
                        ))
                    })?;
                if event.global_sequence != item.global_sequence || event.event_id != item.event_id
                {
                    return Err(StoreError::Verification(format!(
                        "projection outbox sequence {} does not match its journal event",
                        item.global_sequence
                    )));
                }
                let payload = self.journal.decrypt_payload(&event)?;
                let mutations = handler.project(self.store.as_ref(), &event, &payload)?;
                self.store.apply(ProjectionBatch {
                    projection: handler.name().into(),
                    expected_position: position,
                    through_sequence: item.global_sequence,
                    mutations,
                })?;
                position = item.global_sequence;
                applied = applied.saturating_add(1);
            }
        }
        Ok(ProjectionRunReport {
            applied,
            projections: self.status()?,
        })
    }

    /// Replay bounded batches until every projection is current.
    pub fn drain(
        &self,
        batch_limit: usize,
        max_rounds: usize,
    ) -> Result<ProjectionRunReport, StoreError> {
        if batch_limit == 0 || max_rounds == 0 {
            return Err(StoreError::Adapter(
                "projection drain bounds must be greater than zero".into(),
            ));
        }
        let mut applied = 0_u64;
        for _ in 0..max_rounds {
            let report = self.run_once(batch_limit)?;
            applied = applied.saturating_add(report.applied);
            if report.projections.iter().all(|status| status.ready) {
                return Ok(ProjectionRunReport {
                    applied,
                    projections: report.projections,
                });
            }
            if report.applied == 0 {
                break;
            }
        }
        Ok(ProjectionRunReport {
            applied,
            projections: self.status()?,
        })
    }

    /// Delete one named projection and rebuild it from sequence one.
    pub fn rebuild(&self, name: &str) -> Result<ProjectionRunReport, StoreError> {
        if !self.handlers.iter().any(|handler| handler.name() == name) {
            return Err(StoreError::NotFound(format!("projection {name}")));
        }
        self.store.reset(name)?;
        self.drain(256, 16_384)
    }

    /// Delete and rebuild every registered projection.
    pub fn rebuild_all(&self) -> Result<ProjectionRunReport, StoreError> {
        for handler in &self.handlers {
            self.store.reset(handler.name())?;
        }
        self.drain(256, 16_384)
    }

    /// Report current journal head, position, lag, and readiness.
    pub fn status(&self) -> Result<Vec<ProjectionStatus>, StoreError> {
        let (head, _) = self.journal.head()?;
        self.handlers
            .iter()
            .map(|handler| {
                let position = self.store.position(handler.name())?;
                Ok(ProjectionStatus {
                    projection: handler.name().into(),
                    position,
                    journal_head: head,
                    lag: head.saturating_sub(position),
                    ready: !self.journal.is_recovery_mode() && position == head,
                })
            })
            .collect()
    }
}

fn upsert(key: impl Into<String>, value: Value) -> Vec<ProjectionMutation> {
    vec![ProjectionMutation::Upsert {
        key: key.into(),
        value,
    }]
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn merge_event(
    store: &dyn ProjectionStore,
    projection: &str,
    key: &str,
    event: &EventEnvelope,
    payload: &Value,
) -> Result<Value, StoreError> {
    let mut current = store.get(projection, key)?.map(object).unwrap_or_default();
    if let Some(fields) = payload.as_object() {
        current.extend(fields.clone());
    } else {
        current.insert("payload".into(), payload.clone());
    }
    current.insert("id".into(), Value::String(key.into()));
    current.insert("stream_id".into(), Value::String(event.stream_id.clone()));
    current.insert("stream_version".into(), json!(event.stream_version));
    current.insert(
        "last_event_type".into(),
        Value::String(event.event_type.clone()),
    );
    current.insert(
        "updated_at".into(),
        Value::String(event.occurred_at.clone()),
    );
    Ok(Value::Object(current))
}

/// Session aggregate reducer.
pub struct SessionProjection;

impl ProjectionHandler for SessionProjection {
    fn name(&self) -> &'static str {
        "sessions-v1"
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let Some(id) = event.stream_id.strip_prefix("session:") else {
            return Ok(Vec::new());
        };
        let mut current = store.get(self.name(), id)?.map(object).unwrap_or_default();
        match event.event_type.as_str() {
            "session.created.v1" => {
                current.insert(
                    "title".into(),
                    payload.get("title").cloned().unwrap_or(Value::Null),
                );
                current.insert("created_at".into(), json!(event.occurred_at));
                current.insert("message_count".into(), json!(0));
                current.insert("last_run_id".into(), Value::Null);
                current.insert("last_user_preview".into(), Value::Null);
            }
            "session.message.appended.v1" => {
                let message_count = current
                    .get("message_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_add(1);
                current.insert("message_count".into(), json!(message_count));
                current.insert(
                    "last_run_id".into(),
                    payload.get("run_id").cloned().unwrap_or(Value::Null),
                );
                if payload.pointer("/message/role").and_then(Value::as_str) == Some("user") {
                    let preview = payload
                        .pointer("/message/content")
                        .and_then(Value::as_str)
                        .map(|content| content.chars().take(160).collect::<String>());
                    current.insert(
                        "last_user_preview".into(),
                        preview.map_or(Value::Null, Value::String),
                    );
                }
            }
            _ => {}
        }
        current.insert("id".into(), Value::String(id.into()));
        current.insert("stream_id".into(), Value::String(event.stream_id.clone()));
        current.insert("stream_version".into(), json!(event.stream_version));
        current.insert(
            "last_event_type".into(),
            Value::String(event.event_type.clone()),
        );
        current.insert("updated_at".into(), json!(event.occurred_at));
        Ok(upsert(id, Value::Object(current)))
    }
}

/// Task, decision, plan, and goal reducer.
pub struct WorkProjection;

impl ProjectionHandler for WorkProjection {
    fn name(&self) -> &'static str {
        "work-v1"
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        if !["task:", "decision:", "plan:", "goal:"]
            .iter()
            .any(|prefix| event.stream_id.starts_with(prefix))
        {
            return Ok(Vec::new());
        }
        if let Some(record) = payload.get("record") {
            let mut record = object(record.clone());
            record.insert("stream_id".into(), Value::String(event.stream_id.clone()));
            record.insert("stream_version".into(), json!(event.stream_version));
            record.insert(
                "last_event_type".into(),
                Value::String(event.event_type.clone()),
            );
            return Ok(upsert(&event.stream_id, Value::Object(record)));
        }
        Ok(upsert(
            &event.stream_id,
            merge_event(store, self.name(), &event.stream_id, event, payload)?,
        ))
    }
}

/// Canonical memory lifecycle reducer.
pub struct MemoryProjection;

impl ProjectionHandler for MemoryProjection {
    fn name(&self) -> &'static str {
        "memory-v1"
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let Some(id) = event.stream_id.strip_prefix("memory:") else {
            return Ok(Vec::new());
        };
        let value = match event.event_type.as_str() {
            "memory.created.v1" | "memory.updated.v1" => payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("memory record is absent".into()))?,
            "memory.archived.v1" | "memory.superseded.v1" => {
                let mut current = store.get(self.name(), id)?.ok_or_else(|| {
                    StoreError::Verification(format!("memory {id} was not created"))
                })?;
                let status = if event.event_type == "memory.archived.v1" {
                    "archived"
                } else {
                    "superseded"
                };
                current["status"] = Value::String(status.into());
                if let Some(updated_at) = payload.get("updated_at") {
                    current["updated_at"] = updated_at.clone();
                }
                if let Some(replacement) = payload.get("replacement_id") {
                    current["superseded_by"] = replacement.clone();
                }
                current
            }
            _ => return Ok(Vec::new()),
        };
        Ok(upsert(id, value))
    }
}

/// Workflow definition and run reducer.
pub struct WorkflowProjection;

impl ProjectionHandler for WorkflowProjection {
    fn name(&self) -> &'static str {
        "workflows-v1"
    }

    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        if let Some(id) = event.stream_id.strip_prefix("workflow-definition:") {
            return Ok(upsert(
                format!("definition:{id}"),
                merge_event(
                    store,
                    self.name(),
                    &format!("definition:{id}"),
                    event,
                    payload,
                )?,
            ));
        }
        let Some(run_id) = event.stream_id.strip_prefix("workflow-run:") else {
            return Ok(Vec::new());
        };
        let key = format!("run:{run_id}");
        let mut run = store.get(self.name(), &key)?.unwrap_or_else(|| json!({}));
        match event.event_type.as_str() {
            "workflow.run.started.v1" => {
                run = json!({
                    "run_id": run_id,
                    "workflow_name": payload.get("workflow_name"),
                    "workflow_version": payload.get("workflow_version"),
                    "workflow_hash": payload.get("workflow_hash"),
                    "inputs": payload.get("inputs"),
                    "outputs": null,
                    "completed_steps": 0,
                    "status": "running",
                });
            }
            "workflow.step.completed.v1" => {
                let completed = payload
                    .get("root_index")
                    .and_then(Value::as_u64)
                    .map_or(0, |index| index.saturating_add(1));
                run["completed_steps"] = json!(completed);
            }
            "workflow.run.waiting.v1" => run["status"] = json!("waiting"),
            "workflow.run.resumed.v1" => run["status"] = json!("running"),
            "workflow.run.completed.v1" => {
                run["status"] = json!("completed");
                run["outputs"] = payload.get("outputs").cloned().unwrap_or(Value::Null);
            }
            "workflow.run.failed.v1" => run["status"] = json!("failed"),
            "workflow.run.cancelled.v1" => run["status"] = json!("cancelled"),
            "workflow.run.interrupted.v1" => run["status"] = json!("interrupted"),
            _ => return Ok(Vec::new()),
        }
        run["stream_version"] = json!(event.stream_version);
        run["updated_at"] = Value::String(event.occurred_at.clone());
        Ok(upsert(key, run))
    }
}

/// Built-in projections required for the P0/P1 runtime state.
pub fn default_handlers() -> Vec<Arc<dyn ProjectionHandler>> {
    vec![
        Arc::new(SessionProjection),
        Arc::new(WorkProjection),
        Arc::new(MemoryProjection),
        Arc::new(WorkflowProjection),
    ]
}

/// Session repository served from a rebuildable projection.
pub struct ProjectedSessionRepository {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedSessionRepository {
    /// Bind the repository to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }
}

impl AggregateRepository for ProjectedSessionRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.store.get("sessions-v1", id)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("sessions-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

/// Work repository served from task/decision/plan/goal projections.
pub struct ProjectedWorkRepository {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedWorkRepository {
    /// Bind the repository to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }
}

impl AggregateRepository for ProjectedWorkRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        if id.contains(':') {
            return self.store.get("work-v1", id);
        }
        for prefix in ["task:", "decision:", "plan:", "goal:"] {
            if let Some(record) = self.store.get("work-v1", &format!("{prefix}{id}"))? {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("work-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

/// Read-only projected view used after canonical memory authorization.
pub struct ProjectedMemoryReader {
    store: Arc<dyn ProjectionStore>,
}

impl ProjectedMemoryReader {
    /// Bind the reader to a projection store.
    pub fn new(store: Arc<dyn ProjectionStore>) -> Self {
        Self { store }
    }

    /// Load one canonical memory snapshot.
    pub fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.store.get("memory-v1", id)
    }

    /// List bounded canonical memory snapshots.
    pub fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        Ok(self
            .store
            .list("memory-v1", "", limit)?
            .into_iter()
            .map(|(_, value)| value)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProjectedSessionRepository, ProjectionHandler, ProjectionWorker, default_handlers,
    };
    use colossus_contracts::{
        Actor, ActorType, EventClassification, ExecutionContext, NewEvent, ProjectionMutation,
    };
    use colossus_ports::{AggregateRepository, EventJournal, ProjectionStore, StoreError};
    use colossus_testkit::{InMemoryEventJournal, InMemoryProjectionStore};
    use serde_json::{Value, json};
    use std::sync::Arc;

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
        assert_eq!(worker.drain(8, 8).expect("drain").applied, 4);
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

    #[test]
    fn handler_failure_never_advances_position() {
        let journal = Arc::new(InMemoryEventJournal::default());
        journal
            .append(event("session:one", 0, "session.created.v1", json!({})))
            .expect("append");
        let store = Arc::new(InMemoryProjectionStore::default());
        let journal_port: Arc<dyn EventJournal> = journal;
        let store_port: Arc<dyn ProjectionStore> = store.clone();
        let worker =
            ProjectionWorker::new(journal_port, store_port, vec![Arc::new(FailingProjection)])
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
}
