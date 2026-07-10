//! Shared adapter conformance fixtures.

use colossus_contracts::{EncryptedPayload, EventEnvelope, NewEvent, SignedCheckpoint};
use colossus_ports::{EventJournal, StoreError, VerificationReport};
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

/// Run the storage behavior shared by every canonical journal adapter.
pub fn assert_journal_conformance(journal: &dyn EventJournal, first: NewEvent, stale: NewEvent) {
    let stored = journal.append(first).expect("conformance append");
    assert_eq!(stored.global_sequence, 1);
    assert_eq!(stored.stream_version, 1);
    assert!(matches!(
        journal.append(stale),
        Err(StoreError::Conflict { .. })
    ));
    assert_eq!(journal.verify().expect("conformance verify").event_count, 1);
}
