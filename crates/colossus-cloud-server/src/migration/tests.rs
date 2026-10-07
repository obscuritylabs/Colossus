use super::*;
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};

#[tokio::test]
async fn legacy_import_preserves_stream_versions_ids_receipts_and_output_cursors() {
    let journal = Arc::new(
        RedbEventJournal::open_in_memory(
            Arc::new(PlaintextKeyProvider),
            Arc::new(DisabledCheckpointSigner),
        )
        .expect("source"),
    );
    let append = |stream: &str, version: u64, value: Value| {
        journal
            .append(NewEvent {
                stream_id: stream.into(),
                expected_stream_version: version,
                event_type: "cloud.fixture.v1".into(),
                event_version: 1,
                classification: EventClassification::Domain,
                actor: Actor {
                    actor_type: ActorType::Application,
                    id: "source-owner".into(),
                },
                context: ExecutionContext::default(),
                payload: value,
            })
            .expect("source event")
    };
    append(
        "cloud.node:project:node",
        0,
        json!({"node_id":"node","project_id":"project","revision":1}),
    );
    append(
        "cloud.node:project:node",
        1,
        json!({"node_id":"node","project_id":"project","revision":2}),
    );
    append(
        "cloud.task:project:task",
        0,
        json!({"task_id":"task","project_id":"project","node_id":"node","run_id":"run","last_sequence":1,"revision":1}),
    );
    append(
        "cloud.command:project:node:command",
        0,
        json!({"command_id":"command","task_id":"task","node_id":"node","revision":1,"reply":{"kind":"fixture-receipt"}}),
    );
    append("cloud.run:project:node:run", 0, json!("task"));
    append("cloud.node-task:project:node:task", 0, json!("task"));
    let update = colossus_sdk::RunUpdate {
        run_id: "run".into(),
        sequence: 1,
        created_at: "2026-10-05T00:00:00Z".into(),
        update: colossus_sdk::RunUpdateKind::OutputDelta("Released history".into()),
    };
    append(
        "cloud.output:project:task",
        0,
        serde_json::to_value(update).expect("released update"),
    );
    let head = journal.head().expect("head");
    let store: Arc<dyn CloudStore> = Arc::new(MemoryCloudStore::default());
    let report = replay(journal.clone(), store.clone(), head.clone())
        .await
        .expect("import");
    assert_eq!(report.entities, 6);
    assert_eq!(report.released_events, 1);
    let node = store
        .read(&identity("cloud.node:project:node", &json!({})).expect("key"))
        .await
        .expect("node");
    assert_eq!(node.revision, 2);
    assert_eq!(node.value["node_id"], "node");
    let command = store
        .read(&identity("cloud.command:project:node:command", &json!({})).expect("key"))
        .await
        .expect("receipt");
    assert_eq!(command.value["reply"]["kind"], "fixture-receipt");
    assert_eq!(
        store
            .events("project", "task", 0, 16)
            .await
            .expect("events")[0]
            .sequence,
        1
    );
    replay(journal, store.clone(), head)
        .await
        .expect("exact source resumes without duplicates");
    assert_eq!(
        store
            .events("project", "task", 0, 16)
            .await
            .expect("events")
            .len(),
        1
    );
}
