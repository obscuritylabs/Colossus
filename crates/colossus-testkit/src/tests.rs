use super::{
    InMemoryEventJournal, InMemoryProjectionStore, assert_journal_conformance,
    assert_projection_store_conformance,
};
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
use colossus_ports::{EventJournal, MAX_STREAM_READ_BATCH};

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
fn in_memory_ranged_stream_reads_are_exclusive_and_hard_bounded() {
    let journal = InMemoryEventJournal::default();
    journal
        .append_batch(
            (0..=u64::try_from(MAX_STREAM_READ_BATCH).expect("batch ceiling"))
                .map(|version| event(version, version.saturating_add(1)))
                .collect(),
        )
        .expect("append ranged read fixture");

    let page = journal
        .read_stream_from("in-memory-conformance", 0, usize::MAX)
        .expect("bounded page");
    assert_eq!(page.len(), MAX_STREAM_READ_BATCH);
    assert_eq!(page.first().map(|event| event.stream_version), Some(1));
    assert_eq!(
        page.last().map(|event| event.stream_version),
        Some(u64::try_from(MAX_STREAM_READ_BATCH).expect("batch ceiling"))
    );

    let tail = journal
        .read_stream_from(
            "in-memory-conformance",
            u64::try_from(MAX_STREAM_READ_BATCH).expect("batch ceiling"),
            usize::MAX,
        )
        .expect("exclusive tail");
    assert_eq!(tail.len(), 1);
    assert_eq!(
        tail[0].stream_version,
        u64::try_from(MAX_STREAM_READ_BATCH)
            .expect("batch ceiling")
            .saturating_add(1)
    );

    let backwards = journal
        .read_stream_backwards("in-memory-conformance", None, usize::MAX)
        .expect("bounded backwards page");
    assert_eq!(backwards.len(), MAX_STREAM_READ_BATCH);
    assert_eq!(
        backwards.first().map(|event| event.stream_version),
        Some(
            u64::try_from(MAX_STREAM_READ_BATCH)
                .expect("batch ceiling")
                .saturating_add(1)
        )
    );
    assert_eq!(backwards.last().map(|event| event.stream_version), Some(2));

    let older = journal
        .read_stream_backwards("in-memory-conformance", Some(3), usize::MAX)
        .expect("exclusive backwards cursor");
    assert_eq!(
        older
            .iter()
            .map(|event| event.stream_version)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn in_memory_projection_store_passes_shared_conformance() {
    assert_projection_store_conformance(&InMemoryProjectionStore::default());
}
