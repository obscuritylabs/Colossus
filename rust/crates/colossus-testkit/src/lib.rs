//! Shared adapter conformance fixtures.

use colossus_contracts::{
    Actor, ActorType, EncryptedPayload, EventDisplayMode, EventEnvelope, IntegrationAuth,
    IntegrationConnection, IntegrationKind, IntegrationOperation, IntegrationStatus, NewEvent,
    PackInstallation, PackManifest, PackStatus, ProjectionBatch, ProjectionMutation,
    ProjectionWorkItem, PublisherTrust, ReplPreferences, ResearchClaim, ResearchDepth, ResearchRun,
    ResearchSource, ResearchSourceKind, ResearchStatus, SignedCheckpoint, StreamDisplayMode,
    ThemeName, ToolSpec, TranscriptDensity,
};
use colossus_ports::{
    EventJournal, ExtensionRepository, ExternalWorkQueue, PresentationRepository, ProjectionStore,
    ResearchRepository, StoreError, VerificationReport,
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

/// Shared reconstruction and validation checks for presentation repository adapters.
pub fn assert_presentation_repository_conformance(repository: &dyn PresentationRepository) {
    assert_eq!(
        repository.load().expect("default presentation profile"),
        ReplPreferences::default()
    );
    let expected = ReplPreferences {
        theme: ThemeName::HighContrast,
        multiline: true,
        stream_mode: StreamDisplayMode::Off,
        events_mode: EventDisplayMode::Verbose,
        show_reasoning: false,
        transcript_density: TranscriptDensity::Compact,
        ..ReplPreferences::default()
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
    let invalid = ReplPreferences {
        schema_version: u16::MAX,
        ..ReplPreferences::default()
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
