use super::*;

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
    assert_eq!(
        journal
            .read_stream_from(&stored.stream_id, 0, MAX_STREAM_READ_BATCH + 1)
            .expect("conformance ranged stream"),
        vec![stored.clone()]
    );
    assert!(
        journal
            .read_stream_from(&stored.stream_id, stored.stream_version, 1)
            .expect("conformance exclusive cursor")
            .is_empty()
    );
    assert!(
        journal
            .read_stream_from(&stored.stream_id, 0, 0)
            .expect("conformance zero limit")
            .is_empty()
    );
    assert_eq!(
        journal
            .read_stream_backwards(&stored.stream_id, None, MAX_STREAM_READ_BATCH + 1)
            .expect("conformance backwards stream"),
        vec![stored.clone()]
    );
    assert!(
        journal
            .read_stream_backwards(&stored.stream_id, Some(1), 1)
            .expect("conformance backwards exclusive cursor")
            .is_empty()
    );
    assert!(
        journal
            .read_stream_backwards(&stored.stream_id, None, 0)
            .expect("conformance backwards zero limit")
            .is_empty()
    );
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
