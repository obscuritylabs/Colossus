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
