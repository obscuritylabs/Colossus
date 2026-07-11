//! Shared adapter conformance fixtures.

use colossus_contracts::{
    Actor, ActorType, EncryptedPayload, EventDisplayMode, EventEnvelope, NewEvent, ProjectionBatch,
    ProjectionMutation, ProjectionWorkItem, ReplPreferences, SignedCheckpoint, StreamDisplayMode,
    ThemeName, TranscriptDensity,
};
use colossus_ports::{
    EventJournal, PresentationRepository, ProjectionStore, StoreError, VerificationReport,
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
