use super::*;

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
