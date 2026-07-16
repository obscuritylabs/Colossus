//! Shared adapter conformance fixtures.

use colossus_contracts::{
    Actor, ActorType, AuditEvidence, DecisionPriority, DecisionSource, DecisionStatus,
    EncryptedPayload, EventDisplayMode, EventEnvelope, GoalRecord, GoalStatus, IntegrationAuth,
    IntegrationConnection, IntegrationKind, IntegrationOperation, IntegrationStatus, KeyDecision,
    MemoryRecord, MemoryScope, MemoryStatus, ModelMessage, ModelMessageRole, NewEvent,
    PackInstallation, PackManifest, PackStatus, PlanRecord, PlanStatus, PlanStep, ProjectionBatch,
    ProjectionMutation, ProjectionWorkItem, PublisherTrust, ResearchClaim, ResearchDepth,
    ResearchRun, ResearchSource, ResearchSourceKind, ResearchStatus, SignedCheckpoint,
    StreamDisplayMode, SubagentJob, SubagentStatus, TaskRecord, TaskStatus, TerminalPreferences,
    ThemeName, ToolSpec, TranscriptDensity, WorkflowDefinition, WorkflowMetadata, WorkflowSchedule,
    WorkflowScheduleMisfirePolicy, WorkflowStep, WorkflowWebhook,
};
use colossus_ports::{
    AuditExporter, EventJournal, ExtensionRepository, ExternalWorkQueue, MemoryIndex,
    MemoryRepository, PresentationRepository, ProjectionStore, ResearchRepository,
    SessionRepository, StoreError, VerificationReport, WorkRepository, WorkflowRepository,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Default)]
struct State {
    events: Vec<EventEnvelope>,
    payloads: BTreeMap<String, Value>,
    stream_versions: BTreeMap<String, u64>,
}

/// Deterministic in-memory journal for application and conformance tests.
#[derive(Default)]
pub struct InMemoryEventJournal {
    state: Mutex<State>,
}

fn failure(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl EventJournal for InMemoryEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.append_batch(vec![event])?
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        let mut pending_versions = state.stream_versions.clone();
        for event in &events {
            let actual = pending_versions.get(&event.stream_id).copied().unwrap_or(0);
            if event.expected_stream_version != actual {
                return Err(StoreError::Conflict {
                    stream_id: event.stream_id.clone(),
                    expected: event.expected_stream_version,
                    actual,
                });
            }
            pending_versions.insert(event.stream_id.clone(), actual.saturating_add(1));
        }
        let mut persisted = Vec::with_capacity(events.len());
        for event in events {
            let global_sequence = u64::try_from(state.events.len())
                .map_err(failure)?
                .saturating_add(1);
            let stream_version = state
                .stream_versions
                .get(&event.stream_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            let event_id = Uuid::now_v7().to_string();
            let plaintext = serde_json::to_vec(&event.payload).map_err(failure)?;
            let previous_hash = state
                .events
                .last()
                .map_or_else(|| ZERO_HASH.to_owned(), |record| record.record_hash.clone());
            let mut record = EventEnvelope {
                schema_version: 1,
                event_version: event.event_version,
                event_id: event_id.clone(),
                global_sequence,
                stream_id: event.stream_id,
                stream_version,
                classification: event.classification,
                event_type: event.event_type,
                actor: event.actor,
                context: event.context,
                occurred_at: OffsetDateTime::now_utc()
                    .format(&Rfc3339)
                    .map_err(failure)?,
                payload: EncryptedPayload {
                    key_id: "in-memory-test-only".into(),
                    algorithm: "in-memory-test-only".into(),
                    nonce: String::new(),
                    ciphertext: hex::encode(&plaintext),
                    plaintext_hash: hash(&plaintext),
                },
                previous_hash,
                record_hash: String::new(),
            };
            record.record_hash = hash(&serde_json::to_vec(&record).map_err(failure)?);
            state
                .stream_versions
                .insert(record.stream_id.clone(), stream_version);
            state.payloads.insert(event_id, event.payload);
            state.events.push(record.clone());
            persisted.push(record);
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .events
            .iter()
            .filter(|event| event.stream_id == stream_id)
            .cloned()
            .collect())
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .events
            .iter()
            .filter(|event| event.global_sequence >= from_sequence)
            .take(limit)
            .cloned()
            .collect())
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .events
            .iter()
            .filter(|event| event.global_sequence >= from_sequence)
            .take(limit)
            .map(|event| ProjectionWorkItem {
                global_sequence: event.global_sequence,
                event_id: event.event_id.clone(),
            })
            .collect())
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let state = self.state.lock().map_err(failure)?;
        Ok(state.events.last().map_or_else(
            || (0, ZERO_HASH.into()),
            |event| (event.global_sequence, event.record_hash.clone()),
        ))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.state
            .lock()
            .map_err(failure)?
            .payloads
            .get(&event.event_id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(event.event_id.clone()))
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        let state = self.state.lock().map_err(failure)?;
        let last = state.events.last();
        Ok(VerificationReport {
            event_count: u64::try_from(state.events.len()).map_err(failure)?,
            last_sequence: last.map_or(0, |event| event.global_sequence),
            last_hash: last.map_or_else(|| ZERO_HASH.into(), |event| event.record_hash.clone()),
            checkpoint: None,
        })
    }

    fn is_recovery_mode(&self) -> bool {
        false
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        Ok(None)
    }
}

#[derive(Default)]
struct ProjectionState {
    positions: BTreeMap<String, u64>,
    records: BTreeMap<(String, String), Value>,
}

/// Deterministic in-memory projection store for workers and conformance tests.
#[derive(Default)]
pub struct InMemoryProjectionStore {
    state: Mutex<ProjectionState>,
}

impl ProjectionStore for InMemoryProjectionStore {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .positions
            .get(projection)
            .copied()
            .unwrap_or(0))
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .records
            .get(&(projection.into(), key.into()))
            .cloned())
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        Ok(self
            .state
            .lock()
            .map_err(failure)?
            .records
            .iter()
            .filter(|((name, key), _)| name == projection && key.starts_with(key_prefix))
            .take(limit)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect())
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        let actual = state.positions.get(&batch.projection).copied().unwrap_or(0);
        if actual != batch.expected_position {
            return Err(StoreError::Conflict {
                stream_id: format!("projection:{}", batch.projection),
                expected: batch.expected_position,
                actual,
            });
        }
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        for mutation in batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, value } => {
                    state.records.insert((batch.projection.clone(), key), value);
                }
                ProjectionMutation::Delete { key } => {
                    state.records.remove(&(batch.projection.clone(), key));
                }
            }
        }
        state
            .positions
            .insert(batch.projection, batch.through_sequence);
        Ok(())
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(failure)?;
        state.positions.remove(projection);
        state.records.retain(|(name, _), _| name != projection);
        Ok(())
    }
}

/// Run the storage behavior shared by every canonical journal adapter.
pub fn assert_journal_conformance(journal: &dyn EventJournal, first: NewEvent, stale: NewEvent) {
    let stored = journal.append(first).expect("conformance append");
    assert_eq!(stored.global_sequence, 1);
    assert_eq!(stored.stream_version, 1);
    assert_eq!(
        journal.head().expect("conformance head"),
        (1, stored.record_hash.clone())
    );
    assert_eq!(
        journal
            .read_projection_work(1, 10)
            .expect("conformance projection work"),
        vec![ProjectionWorkItem {
            global_sequence: 1,
            event_id: stored.event_id.clone(),
        }]
    );
    assert!(matches!(
        journal.append(stale),
        Err(StoreError::Conflict { .. })
    ));
    assert_eq!(journal.verify().expect("conformance verify").event_count, 1);
}

/// Run the behavior shared by every projection-store adapter.
pub fn assert_projection_store_conformance(store: &dyn ProjectionStore) {
    assert_eq!(store.position("test").expect("initial position"), 0);
    store
        .apply(ProjectionBatch {
            projection: "test".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: vec![ProjectionMutation::Upsert {
                key: "record-1".into(),
                value: serde_json::json!({"value": 1}),
            }],
        })
        .expect("projection apply");
    assert_eq!(store.position("test").expect("position"), 1);
    assert_eq!(
        store.get("test", "record-1").expect("record"),
        Some(serde_json::json!({"value": 1}))
    );
    assert_eq!(
        store.list("test", "record-", 10).expect("list"),
        vec![("record-1".into(), serde_json::json!({"value": 1}))]
    );
    store
        .apply(ProjectionBatch {
            projection: "test".into(),
            expected_position: 1,
            through_sequence: 2,
            mutations: vec![ProjectionMutation::Delete {
                key: "record-1".into(),
            }],
        })
        .expect("projection delete");
    assert!(store.get("test", "record-1").expect("deleted").is_none());
    assert!(matches!(
        store.apply(ProjectionBatch {
            projection: "test".into(),
            expected_position: 1,
            through_sequence: 3,
            mutations: Vec::new(),
        }),
        Err(StoreError::Conflict { actual: 2, .. })
    ));
    store.reset("test").expect("projection reset");
    assert_eq!(store.position("test").expect("reset position"), 0);
    assert!(
        store
            .get("test", "record-1")
            .expect("reset record")
            .is_none()
    );
}

/// Run durable isolation, optimistic acknowledgment, and replay checks shared by
/// every external-work queue adapter.
pub fn assert_external_work_queue_conformance(
    journal: &dyn EventJournal,
    queue: &dyn ExternalWorkQueue,
    first: NewEvent,
    second: NewEvent,
) {
    let first = journal.append(first).expect("first external work append");
    let second = journal.append(second).expect("second external work append");
    let left = queue.pending("conformance.left-v1", 8).expect("left work");
    let right = queue
        .pending("conformance.right-v1", 8)
        .expect("right work");
    assert_eq!(left, right);
    assert_eq!(left.len(), 2);
    assert_eq!(left[0].event_id, first.event_id);
    assert_eq!(left[1].event_id, second.event_id);

    let retry = queue
        .record_failure(
            "conformance.left-v1",
            Some(&left[0]),
            "2026-07-11T00:00:00Z",
            true,
            "external_work.test",
            "bounded test failure",
        )
        .expect("retry state");
    assert_eq!(retry.attempts, 1);
    assert_eq!(retry.next_retry_at.as_deref(), Some("2026-07-11T00:00:01Z"));
    assert_eq!(
        queue
            .retry_state("conformance.left-v1")
            .expect("durable retry state"),
        Some(retry.clone())
    );
    assert!(
        queue
            .retry_state("conformance.right-v1")
            .expect("isolated retry state")
            .is_none()
    );
    let mut capped = retry;
    for _ in 1..10 {
        capped = queue
            .record_failure(
                "conformance.left-v1",
                Some(&left[0]),
                "2026-07-11T00:00:00Z",
                true,
                "external_work.test",
                "bounded test failure",
            )
            .expect("increment retry state");
    }
    assert_eq!(capped.attempts, 10);
    assert_eq!(
        capped.next_retry_at.as_deref(),
        Some("2026-07-11T00:05:00Z")
    );

    queue
        .acknowledge("conformance.left-v1", 0, &left[0])
        .expect("left acknowledge");
    assert_eq!(queue.position("conformance.left-v1").expect("left"), 1);
    assert_eq!(queue.position("conformance.right-v1").expect("right"), 0);
    assert!(matches!(
        queue.acknowledge("conformance.left-v1", 0, &left[0]),
        Err(StoreError::Conflict { actual: 1, .. })
    ));

    queue.reset("conformance.left-v1").expect("left reset");
    assert!(
        queue
            .retry_state("conformance.left-v1")
            .expect("cleared retry state")
            .is_none()
    );
    assert_eq!(
        queue.pending("conformance.left-v1", 8).expect("replay"),
        left
    );
    assert_eq!(
        queue
            .acknowledge_batch("conformance.left-v1", 0, &left)
            .expect("batch acknowledge"),
        2
    );
}

/// Shared creation, append-only ordering, validation, and reconstruction checks for
/// every canonical session repository adapter.
pub fn assert_session_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn SessionRepository>,
{
    let repository = factory();
    assert!(
        repository
            .get_session("session-conformance")
            .expect("missing session")
            .is_none()
    );
    let actor = conformance_actor("session-user");
    let created = repository
        .create_session(
            "session-conformance",
            Some("Conformance session"),
            actor.clone(),
        )
        .expect("create session");
    assert_eq!(created.message_count, 0);
    assert!(
        repository
            .create_session("session-conformance", Some("duplicate"), actor.clone())
            .is_err()
    );
    for (role, content) in [
        (ModelMessageRole::User, "Persist this Rust context."),
        (ModelMessageRole::Assistant, "Context persisted."),
    ] {
        repository
            .append_message(
                "session-conformance",
                "run-conformance",
                ModelMessage {
                    role,
                    content: content.into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor.clone(),
            )
            .expect("append message");
    }
    let reopened = factory();
    let messages = reopened
        .list_messages("session-conformance")
        .expect("reconstructed messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sequence, 1);
    assert_eq!(messages[1].sequence, 2);
    let summary = reopened
        .get_session("session-conformance")
        .expect("session")
        .expect("reconstructed session");
    assert_eq!(summary.message_count, 2);
    assert_eq!(summary.last_run_id.as_deref(), Some("run-conformance"));
    assert_eq!(
        summary.last_user_preview.as_deref(),
        Some("Persist this Rust context.")
    );
    assert_eq!(
        reopened.list_sessions(10).expect("session list")[0].id,
        "session-conformance"
    );
    assert!(
        reopened
            .append_message(
                "missing-session",
                "run-conformance",
                ModelMessage {
                    role: ModelMessageRole::User,
                    content: "invalid".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                actor,
            )
            .is_err()
    );
}

/// Shared lifecycle, filtering, immutable-identity, and reconstruction checks for every
/// canonical work repository adapter.
pub fn assert_work_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn WorkRepository>,
{
    const AT: &str = "2026-07-12T00:00:00Z";
    const LATER: &str = "2026-07-12T00:01:00Z";
    let actor = conformance_actor("work-user");
    let repository = factory();

    let task = TaskRecord {
        id: "task-conformance".into(),
        session_id: "session-conformance".into(),
        title: "Prove repository behavior".into(),
        description: "Shared adapter contract".into(),
        status: TaskStatus::Pending,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_task(task.clone(), actor.clone())
        .expect("create task");
    assert!(repository.create_task(task.clone(), actor.clone()).is_err());
    let mut updated_task = task.clone();
    updated_task.status = TaskStatus::InProgress;
    updated_task.updated_at = LATER.into();
    repository
        .update_task(updated_task.clone(), actor.clone())
        .expect("update task");

    let decision = KeyDecision {
        id: "decision-conformance".into(),
        session_id: "session-conformance".into(),
        goal_id: None,
        plan_id: None,
        source: DecisionSource::User,
        status: DecisionStatus::Active,
        priority: DecisionPriority::Critical,
        title: "Rust cutover".into(),
        decision: "Use canonical Rust repositories.".into(),
        intent: "Complete the transition.".into(),
        applies_when: "Persisting state.".into(),
        rationale: "One auditable runtime.".into(),
        source_excerpt: "Transition to Rust.".into(),
        supersedes: None,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_decision(decision.clone(), actor.clone())
        .expect("create decision");
    let archived = repository
        .archive_decision(&decision.id, actor.clone())
        .expect("archive decision");
    assert_eq!(archived.status, DecisionStatus::Archived);

    let plan = PlanRecord {
        id: "plan-conformance".into(),
        session_id: "session-conformance".into(),
        prompt: "Complete the Rust transition.".into(),
        status: PlanStatus::Draft,
        content: "Execute the shared contract.".into(),
        steps: vec![PlanStep {
            index: 1,
            title: "Verify".into(),
            detail: "Run conformance.".into(),
            requires_mutation: false,
        }],
        created_at: AT.into(),
        updated_at: AT.into(),
        approved_at: None,
        executed_run_id: None,
    };
    repository
        .create_plan(plan.clone(), actor.clone())
        .expect("create plan");
    let mut approved = plan.clone();
    approved.status = PlanStatus::Approved;
    approved.updated_at = LATER.into();
    approved.approved_at = Some(LATER.into());
    repository
        .update_plan(approved.clone(), actor.clone())
        .expect("approve plan");

    let goal = GoalRecord {
        id: "goal-conformance".into(),
        session_id: "session-conformance".into(),
        objective: "Finish the conformance milestone.".into(),
        source_plan_id: None,
        status: GoalStatus::Active,
        summary: String::new(),
        blocked_reason: String::new(),
        iteration_budget: 3,
        iterations_completed: 0,
        created_at: AT.into(),
        updated_at: AT.into(),
    };
    repository
        .create_goal(goal.clone(), actor.clone())
        .expect("create goal");
    let mut completed_goal = goal.clone();
    completed_goal.status = GoalStatus::Complete;
    completed_goal.summary = "Conformance verified.".into();
    completed_goal.updated_at = LATER.into();
    repository
        .update_goal(completed_goal.clone(), actor.clone())
        .expect("complete goal");

    let job = SubagentJob {
        id: "agent-conformance".into(),
        session_id: "session-conformance".into(),
        parent_run_id: "run-conformance".into(),
        parent_call_id: "call-conformance".into(),
        task: "Verify one bounded adapter.".into(),
        role: "subagent_default".into(),
        status: SubagentStatus::Queued,
        child_session_id: "child-session-conformance".into(),
        child_run_id: None,
        final_output: String::new(),
        error: String::new(),
        created_at: AT.into(),
        updated_at: AT.into(),
        started_at: None,
        completed_at: None,
    };
    repository
        .create_subagent(job.clone(), actor.clone())
        .expect("create subagent");
    let mut running = job.clone();
    running.status = SubagentStatus::Running;
    running.started_at = Some(LATER.into());
    running.updated_at = LATER.into();
    repository
        .update_subagent(running.clone(), actor)
        .expect("start subagent");

    let reopened = factory();
    assert_eq!(
        reopened.get_task(&task.id).expect("task"),
        Some(updated_task)
    );
    assert_eq!(
        reopened.get_decision(&decision.id).expect("decision"),
        Some(archived)
    );
    assert_eq!(reopened.get_plan(&plan.id).expect("plan"), Some(approved));
    assert_eq!(
        reopened.get_goal(&goal.id).expect("goal"),
        Some(completed_goal)
    );
    assert_eq!(
        reopened.get_subagent(&job.id).expect("subagent"),
        Some(running)
    );
    assert_eq!(
        reopened
            .list_tasks(
                Some("session-conformance"),
                Some(TaskStatus::InProgress),
                10
            )
            .expect("tasks")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_decisions(
                Some("session-conformance"),
                Some(DecisionStatus::Archived),
                10
            )
            .expect("decisions")
            .len(),
        1
    );
}

/// Shared canonical lifecycle, atomic supersession, filtering, and reconstruction checks
/// for every memory repository adapter.
pub fn assert_memory_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn MemoryRepository>,
{
    const AT: &str = "2026-07-12T00:00:00Z";
    const LATER: &str = "2026-07-12T00:01:00Z";
    let actor = conformance_actor("memory-user");
    let repository = factory();
    let record = MemoryRecord {
        id: "memory-conformance".into(),
        scope: MemoryScope::Global,
        kind: "fact".into(),
        confidence: 0.9,
        source: "user".into(),
        status: MemoryStatus::Active,
        text: "Rust owns canonical state.".into(),
        rationale: "Shared repository contract.".into(),
        created_at: AT.into(),
        updated_at: AT.into(),
        expires_at: None,
        superseded_by: None,
    };
    repository
        .create(record.clone(), actor.clone())
        .expect("create memory");
    assert!(repository.create(record.clone(), actor.clone()).is_err());
    let mut updated = record.clone();
    updated.text = "Rust owns canonical auditable state.".into();
    updated.updated_at = LATER.into();
    repository
        .update(updated.clone(), actor.clone())
        .expect("update memory");
    let replacement = MemoryRecord {
        id: "memory-replacement".into(),
        text: "Rust owns canonical event-sourced state.".into(),
        superseded_by: None,
        ..updated.clone()
    };
    let (old, replacement) = repository
        .supersede(&record.id, replacement, actor.clone())
        .expect("supersede memory");
    assert_eq!(old.status, MemoryStatus::Superseded);
    assert_eq!(old.superseded_by.as_deref(), Some(replacement.id.as_str()));
    assert_eq!(replacement.status, MemoryStatus::Active);

    let archived_seed = MemoryRecord {
        id: "memory-archive".into(),
        text: "Archive this canonical record.".into(),
        ..record
    };
    repository
        .create(archived_seed.clone(), actor.clone())
        .expect("create archived seed");
    let archived = repository
        .archive(&archived_seed.id, actor)
        .expect("archive memory");
    assert_eq!(archived.status, MemoryStatus::Archived);

    let reopened = factory();
    assert_eq!(reopened.get_memory(&old.id).expect("old memory"), Some(old));
    assert_eq!(
        reopened
            .get_memory(&replacement.id)
            .expect("replacement memory"),
        Some(replacement.clone())
    );
    assert_eq!(
        reopened.get_memory(&archived.id).expect("archived memory"),
        Some(archived)
    );
    assert_eq!(
        reopened.list_active(10).expect("active memories"),
        vec![replacement]
    );
    assert_eq!(
        reopened
            .list_memories(Some(MemoryStatus::Superseded), 10)
            .expect("superseded memories")
            .len(),
        1
    );
}

/// Shared definition idempotency, trust invalidation, and reconstruction checks for every
/// workflow repository adapter.
pub fn assert_workflow_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn WorkflowRepository>,
{
    let repository = factory();
    let definition = WorkflowDefinition {
        api_version: "colossus.dev/v1alpha1".into(),
        kind: "Workflow".into(),
        metadata: WorkflowMetadata {
            name: "conformance".into(),
            version: "1.0.0".into(),
            description: "Shared workflow repository contract.".into(),
        },
        inputs: serde_json::json!({"type": "object"}),
        outputs: serde_json::json!({"type": "object"}),
        capabilities: Vec::new(),
        max_concurrency: 1,
        step_budget: 2,
        steps: vec![WorkflowStep::Emit {
            id: "emit".into(),
            value: serde_json::json!({"ok": true}),
        }],
        compensation: Vec::new(),
    };
    repository
        .register(&definition, "hash-one", "repository:test")
        .expect("register definition");
    repository
        .register(&definition, "hash-one", "repository:test")
        .expect("idempotent registration");
    assert_eq!(
        repository
            .definition("conformance", "1.0.0")
            .expect("definition"),
        Some((definition.clone(), "hash-one".into()))
    );
    repository
        .register(&definition, "hash-two", "repository:test-changed")
        .expect("definition change");
    let schedule = WorkflowSchedule {
        schedule_id: "conformance-daily".into(),
        workflow_name: "conformance".into(),
        workflow_version: "1.0.0".into(),
        workflow_hash: "hash-two".into(),
        inputs: serde_json::json!({}),
        cadence_seconds: 86_400,
        misfire_policy: WorkflowScheduleMisfirePolicy::FireOnce,
        enabled: true,
        starts_at: "2026-01-01T00:00:00Z".into(),
        next_fire_at: "2026-01-01T00:00:00Z".into(),
        last_scheduled_at: None,
        last_run_id: None,
        blocked_reason: None,
        created_at: "2025-12-31T00:00:00Z".into(),
        updated_at: "2025-12-31T00:00:00Z".into(),
    };
    repository
        .create_schedule(
            &schedule,
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("create schedule");
    assert!(
        repository
            .create_schedule(
                &schedule,
                Actor {
                    actor_type: ActorType::User,
                    id: "duplicate".into(),
                },
            )
            .is_err(),
        "schedule identifiers must be unique"
    );
    let disabled = repository
        .set_schedule_enabled(
            &schedule.schedule_id,
            false,
            "2026-01-01T01:00:00Z",
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("disable schedule");
    assert!(!disabled.enabled);
    let webhook = WorkflowWebhook {
        webhook_id: "conformance-hook".into(),
        workflow_name: "conformance".into(),
        workflow_version: "1.0.0".into(),
        workflow_hash: "hash-two".into(),
        secret_reference: "env:CONFORMANCE_WEBHOOK_SECRET".into(),
        enabled: true,
        replay_window_seconds: 300,
        max_body_bytes: 4096,
        blocked_reason: None,
        created_at: "2025-12-31T00:00:00Z".into(),
        updated_at: "2025-12-31T00:00:00Z".into(),
    };
    repository
        .create_webhook(
            &webhook,
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("create webhook");
    assert!(
        repository
            .create_webhook(
                &webhook,
                Actor {
                    actor_type: ActorType::User,
                    id: "duplicate".into(),
                },
            )
            .is_err(),
        "webhook identifiers must be unique"
    );
    let disabled_webhook = repository
        .set_webhook_enabled(
            &webhook.webhook_id,
            false,
            "2026-01-01T01:00:00Z",
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("disable webhook");
    assert!(!disabled_webhook.enabled);
    let reopened = factory();
    assert_eq!(
        reopened
            .definition("conformance", "1.0.0")
            .expect("reconstructed definition"),
        Some((definition, "hash-two".into()))
    );
    assert!(reopened.run("missing-run").expect("missing run").is_none());
    assert!(reopened.runs(10).expect("empty runs").is_empty());
    assert_eq!(
        reopened
            .schedule(&schedule.schedule_id)
            .expect("reconstructed schedule"),
        Some(disabled.clone())
    );
    assert_eq!(
        reopened.schedules(10).expect("schedule list"),
        vec![disabled]
    );
    assert_eq!(
        reopened
            .webhook(&webhook.webhook_id)
            .expect("reconstructed webhook"),
        Some(disabled_webhook.clone())
    );
    assert_eq!(
        reopened.webhooks(10).expect("webhook list"),
        vec![disabled_webhook]
    );
    assert!(
        reopened
            .webhook_delivery(&webhook.webhook_id, "missing-delivery")
            .expect("missing webhook delivery")
            .is_none()
    );
}

/// Shared position, event-id idempotency, candidate, status, removal, and rebuild checks
/// for every disposable memory index adapter.
pub async fn assert_memory_index_conformance(index: &dyn MemoryIndex) {
    assert_eq!(index.position().expect("initial index position"), 0);
    index.set_position(17).await.expect("set index position");
    assert_eq!(index.position().expect("index position"), 17);
    index
        .upsert(
            "event-conformance-1",
            "memory-1",
            "Rust audit journal",
            &serde_json::json!({"scope": "global"}),
            None,
        )
        .await
        .expect("index upsert");
    index
        .upsert(
            "event-conformance-1",
            "memory-1",
            "duplicate event must be idempotent",
            &serde_json::json!({"scope": "global"}),
            None,
        )
        .await
        .expect("idempotent index upsert");
    let candidates = index
        .search("audit journal", 4)
        .await
        .expect("index search");
    assert!(
        candidates
            .iter()
            .any(|(id, score)| id == "memory-1" && score.is_finite())
    );
    assert!(index.status().await.expect("index status").is_object());
    index
        .remove("event-conformance-2", "memory-1")
        .await
        .expect("index remove");
    index
        .remove("event-conformance-2", "memory-1")
        .await
        .expect("idempotent index remove");
    index
        .rebuild(&[(
            "memory-2".into(),
            "durable workflow".into(),
            serde_json::json!({"scope": "global"}),
        )])
        .await
        .expect("index rebuild");
}

/// Assert the common stable-kind and idempotent-delivery contract for an audit sink.
pub async fn assert_audit_exporter_conformance(
    exporter: &dyn AuditExporter,
    evidence: &AuditEvidence,
) {
    assert!(!exporter.kind().trim().is_empty());
    exporter
        .export(evidence)
        .await
        .expect("first conformance export");
    exporter
        .export(evidence)
        .await
        .expect("idempotent conformance replay");
}

/// Shared reconstruction and validation checks for presentation repository adapters.
pub fn assert_presentation_repository_conformance(repository: &dyn PresentationRepository) {
    assert_eq!(
        repository.load().expect("default presentation profile"),
        TerminalPreferences::default()
    );
    let expected = TerminalPreferences {
        theme: ThemeName::HighContrast,
        multiline: true,
        stream_mode: StreamDisplayMode::Off,
        events_mode: EventDisplayMode::Verbose,
        show_reasoning: false,
        transcript_density: TranscriptDensity::Compact,
        ..TerminalPreferences::default()
    };
    let saved = repository
        .save(
            expected.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("save presentation profile");
    assert_eq!(saved, expected);
    assert_eq!(repository.load().expect("reconstructed profile"), expected);
    assert!(
        repository
            .list_history(10)
            .expect("empty history")
            .is_empty()
    );
    assert_eq!(
        repository
            .append_history(
                "first prompt".into(),
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .expect("append history"),
        "first prompt"
    );
    repository
        .append_history(
            "first prompt".into(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("deduplicate history");
    repository
        .append_history(
            "second prompt".into(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("append second history");
    assert_eq!(
        repository.list_history(1).expect("bounded history"),
        vec!["second prompt"]
    );
    assert_eq!(
        repository.list_history(10).expect("history"),
        vec!["first prompt", "second prompt"]
    );
    assert!(repository.list_history(0).is_err());
    assert!(
        repository
            .append_history(
                " ".into(),
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .is_err()
    );
    let invalid = TerminalPreferences {
        schema_version: u16::MAX,
        ..TerminalPreferences::default()
    };
    assert!(
        repository
            .save(
                invalid,
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .is_err(),
        "unknown presentation schema must fail closed"
    );
}

fn conformance_actor(id: &str) -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: id.into(),
    }
}

/// Shared lifecycle, citation, validation, and reconstruction checks for research adapters.
pub fn assert_research_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn ResearchRepository>,
{
    let repository = factory();
    assert!(
        repository
            .get_run("research-conformance")
            .expect("missing run")
            .is_none()
    );
    let mut run = ResearchRun {
        id: "research-conformance".into(),
        session_id: "session-conformance".into(),
        question: "What is reconstructed?".into(),
        depth: ResearchDepth::Standard,
        source_kinds: vec![ResearchSourceKind::Repo],
        status: ResearchStatus::Running,
        queries: Vec::new(),
        lanes: Vec::new(),
        progress: Vec::new(),
        limitations: Vec::new(),
        report: String::new(),
        error: String::new(),
        created_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
        completed_at: None,
    };
    assert_eq!(
        repository
            .create_run(run.clone(), conformance_actor("research-user"))
            .expect("create run"),
        run
    );
    assert!(
        repository
            .create_run(run.clone(), conformance_actor("research-user"))
            .is_err(),
        "duplicate creation must fail"
    );
    let mut changed_provenance = run.clone();
    changed_provenance.question = "Changed".into();
    assert!(
        repository
            .update_run(changed_provenance, conformance_actor("research-user"))
            .is_err(),
        "research provenance must be immutable"
    );
    let source = ResearchSource {
        id: "source-conformance".into(),
        run_id: run.id.clone(),
        label: "R1".into(),
        kind: ResearchSourceKind::Repo,
        title: "Architecture".into(),
        uri: "docs/ARCHITECTURE.md".into(),
        content: "The runtime is event sourced.".into(),
        query: "architecture".into(),
        metadata: BTreeMap::new(),
        created_at: "2026-07-11T12:01:00Z".into(),
    };
    let mut skipped_label = source.clone();
    skipped_label.label = "R2".into();
    assert!(
        repository
            .add_source(skipped_label, conformance_actor("research-user"))
            .is_err(),
        "source labels must be sequential"
    );
    repository
        .add_source(source.clone(), conformance_actor("research-user"))
        .expect("add source");
    assert!(
        repository
            .add_source(source, conformance_actor("research-user"))
            .is_err(),
        "source identity and URI must be unique"
    );
    let claim = ResearchClaim {
        id: "claim-conformance".into(),
        run_id: run.id.clone(),
        text: "The runtime is event sourced.".into(),
        source_labels: vec!["R1".into()],
        created_at: "2026-07-11T12:02:00Z".into(),
    };
    let mut dangling = claim.clone();
    dangling.source_labels = vec!["R2".into()];
    assert!(
        repository
            .add_claim(dangling, conformance_actor("research-user"))
            .is_err(),
        "claim labels must resolve"
    );
    repository
        .add_claim(claim.clone(), conformance_actor("research-user"))
        .expect("add claim");
    assert!(
        repository
            .add_claim(claim, conformance_actor("research-user"))
            .is_err(),
        "claim identity must be unique"
    );
    run.status = ResearchStatus::Completed;
    run.report = "The runtime is event sourced [R1].".into();
    run.updated_at = "2026-07-11T12:03:00Z".into();
    run.completed_at = Some(run.updated_at.clone());
    repository
        .update_run(run.clone(), conformance_actor("research-user"))
        .expect("complete run");
    assert!(
        repository
            .update_run(run.clone(), conformance_actor("research-user"))
            .is_err(),
        "terminal runs must be immutable"
    );
    drop(repository);

    let reopened = factory();
    assert_eq!(reopened.get_run(&run.id).expect("reopened run"), Some(run));
    assert_eq!(
        reopened
            .list_sources("research-conformance")
            .expect("sources")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_claims("research-conformance")
            .expect("claims")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_runs(Some("session-conformance"), 10)
            .expect("session runs")
            .len(),
        1
    );
    assert!(
        reopened
            .list_runs(Some("another-session"), 10)
            .expect("filtered runs")
            .is_empty()
    );
}

/// Shared integration, pack, trust, bounds, and reconstruction checks for extension adapters.
pub fn assert_extension_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn ExtensionRepository>,
{
    let repository = factory();
    let operation = IntegrationOperation {
        tool: ToolSpec {
            name: "openapi.demo.read".into(),
            description: "Read a demo record.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            effect_action: Some("openapi.demo.read".into()),
            capability: Some("integration.invoke".into()),
            max_output_bytes: 1024,
        },
        operation_id: "read".into(),
        method: "GET".into(),
        path: "/records".into(),
        path_parameters: Vec::new(),
        query_parameters: Vec::new(),
        accepts_body: false,
    };
    let mut connection = IntegrationConnection {
        name: "demo".into(),
        kind: IntegrationKind::OpenApi,
        status: IntegrationStatus::Connected,
        title: "Demo".into(),
        description: "Conformance connection.".into(),
        base_url: "https://example.com".into(),
        auth: IntegrationAuth::None,
        credential_reference: None,
        credential_references: BTreeMap::new(),
        scopes: Vec::new(),
        operations: vec![operation],
        manifest_sha256: "0".repeat(64),
        connected_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    assert!(
        repository
            .get_integration("demo")
            .expect("missing integration")
            .is_none()
    );
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("save integration");
    connection.description = "Updated connection.".into();
    connection.updated_at = "2026-07-11T12:01:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("update integration");
    let mut changed_identity = connection.clone();
    changed_identity.connected_at = "2026-07-12T00:00:00Z".into();
    assert!(
        repository
            .save_integration(changed_identity, conformance_actor("extension-user"))
            .is_err(),
        "connected_at must be immutable"
    );
    let disconnected = repository
        .disconnect_integration(
            "demo",
            conformance_actor("extension-user"),
            "2026-07-11T12:02:00Z",
        )
        .expect("disconnect integration");
    assert_eq!(disconnected.status, IntegrationStatus::Disconnected);
    connection.updated_at = "2026-07-11T12:03:00Z".into();
    repository
        .save_integration(connection.clone(), conformance_actor("extension-user"))
        .expect("reconnect integration");
    assert!(repository.list_integrations(0).is_err());
    assert!(repository.list_integrations(1_001).is_err());

    let manifest = PackManifest {
        format_version: 1,
        name: "demo-pack".into(),
        version: "1.0.0".into(),
        description: "Conformance pack.".into(),
        publisher: "example".into(),
        license: "Apache-2.0".into(),
        homepage: String::new(),
        capabilities: Vec::new(),
        permissions: Vec::new(),
        files: Vec::new(),
        integrations: Vec::new(),
        skills: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        binaries: Vec::new(),
        docker: Vec::new(),
        docs: Vec::new(),
        tests: Vec::new(),
        dependencies: Vec::new(),
        signatures: Vec::new(),
    };
    let mut installation = PackInstallation {
        manifest,
        status: PackStatus::Enabled,
        source: "conformance".into(),
        installed_path: "/tmp/colossus-conformance-pack".into(),
        manifest_sha256: "1".repeat(64),
        trust_key_id: None,
        installed_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    repository
        .install_pack(installation.clone(), conformance_actor("extension-user"))
        .expect("install pack");
    assert!(
        repository
            .install_pack(installation.clone(), conformance_actor("extension-user"))
            .is_err(),
        "installed pack cannot be overwritten"
    );
    assert_eq!(
        repository
            .set_pack_status(
                "demo-pack",
                PackStatus::Disabled,
                conformance_actor("extension-user"),
                "2026-07-11T12:01:00Z",
            )
            .expect("disable pack")
            .status,
        PackStatus::Disabled
    );
    repository
        .set_pack_status(
            "demo-pack",
            PackStatus::Uninstalled,
            conformance_actor("extension-user"),
            "2026-07-11T12:02:00Z",
        )
        .expect("uninstall pack");
    assert!(
        repository
            .set_pack_status(
                "demo-pack",
                PackStatus::Enabled,
                conformance_actor("extension-user"),
                "2026-07-11T12:03:00Z",
            )
            .is_err(),
        "uninstalled pack cannot transition without reinstall"
    );
    installation.updated_at = "2026-07-11T12:04:00Z".into();
    repository
        .install_pack(installation.clone(), conformance_actor("extension-user"))
        .expect("reinstall pack");
    assert!(repository.list_packs(0).is_err());
    assert!(repository.list_packs(1_001).is_err());

    let trust = PublisherTrust {
        publisher: "example".into(),
        key_id: "2".repeat(64),
        public_key: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
        added_at: "2026-07-11T12:00:00Z".into(),
    };
    repository
        .add_publisher_trust(trust.clone(), conformance_actor("extension-user"))
        .expect("add publisher trust");
    assert!(
        repository
            .add_publisher_trust(trust.clone(), conformance_actor("extension-user"))
            .is_err(),
        "publisher/key trust binding is immutable"
    );
    assert!(repository.list_publisher_trust(0).is_err());
    assert!(repository.list_publisher_trust(1_001).is_err());
    drop(repository);

    let reopened = factory();
    assert_eq!(
        reopened
            .get_integration("demo")
            .expect("reopened integration"),
        Some(connection)
    );
    assert_eq!(
        reopened.list_integrations(10).expect("integrations").len(),
        1
    );
    assert!(reopened.get("demo").expect("aggregate get").is_some());
    assert_eq!(reopened.list(10).expect("aggregate list").len(), 1);
    assert_eq!(
        reopened.get_pack("demo-pack").expect("reopened pack"),
        Some(installation)
    );
    assert_eq!(reopened.list_packs(10).expect("packs").len(), 1);
    assert_eq!(
        reopened
            .get_publisher_trust(&trust.publisher, &trust.key_id)
            .expect("publisher trust"),
        Some(trust)
    );
    assert_eq!(
        reopened
            .list_publisher_trust(10)
            .expect("publisher trust list")
            .len(),
        1
    );
}

#[cfg(test)]
mod tests {
    use super::{
        InMemoryEventJournal, InMemoryProjectionStore, assert_journal_conformance,
        assert_projection_store_conformance,
    };
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};

    fn event(expected_stream_version: u64, value: u64) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: "in-memory-conformance".into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: "conformance.recorded.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "conformance".into(),
            },
            context: ExecutionContext {
                correlation_id: "in-memory-conformance".into(),
                ..ExecutionContext::default()
            },
            payload: serde_json::json!({"value": value}),
        }
    }

    #[test]
    fn in_memory_journal_passes_shared_conformance() {
        assert_journal_conformance(&InMemoryEventJournal::default(), event(0, 1), event(0, 2));
    }

    #[test]
    fn in_memory_projection_store_passes_shared_conformance() {
        assert_projection_store_conformance(&InMemoryProjectionStore::default());
    }
}
