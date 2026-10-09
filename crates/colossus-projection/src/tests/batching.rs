use super::*;

struct AccumulatingProjection {
    fail_at: Option<u64>,
}

impl ProjectionHandler for AccumulatingProjection {
    fn name(&self) -> &'static str {
        "counter-v1"
    }
    fn requires_payload(&self) -> bool {
        false
    }
    fn project(
        &self,
        store: &dyn ProjectionStore,
        event: &EventEnvelope,
        payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        assert_eq!(*payload, Value::Null);
        assert_eq!(store.position(self.name())?, event.global_sequence - 1);
        let count = store
            .get(self.name(), "count")?
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
            + 1;
        if self.fail_at == Some(count) {
            return Err(StoreError::Adapter("reducer failed".into()));
        }
        let mut mutations = vec![ProjectionMutation::Upsert {
            key: "count".into(),
            value: json!(count),
        }];
        match count {
            1 => mutations.extend([
                ProjectionMutation::Delete {
                    key: "item:a".into(),
                },
                ProjectionMutation::Delete {
                    key: "item:b".into(),
                },
                ProjectionMutation::Upsert {
                    key: "item:c".into(),
                    value: json!(3),
                },
            ]),
            2 => {
                assert_eq!(store.get(self.name(), "item:a")?, None);
                assert_eq!(store.get(self.name(), "item:c")?, Some(json!(3)));
                assert_eq!(
                    store.list(self.name(), "item:", 3)?,
                    vec![
                        ("item:c".into(), json!(3)),
                        ("item:d".into(), json!(4)),
                        ("item:f".into(), json!(6))
                    ]
                );
                assert_eq!(
                    store.list_after(self.name(), "item:", Some("item:c"), 2)?,
                    vec![("item:d".into(), json!(4)), ("item:f".into(), json!(6))]
                );
                assert_eq!(
                    store.list_after(self.name(), "item:", Some("a"), 1)?,
                    vec![("item:c".into(), json!(3))]
                );
                assert!(store.list(self.name(), "item:", 0)?.is_empty());
                mutations.extend([
                    ProjectionMutation::Delete {
                        key: "item:c".into(),
                    },
                    ProjectionMutation::Upsert {
                        key: "item:a".into(),
                        value: json!(8),
                    },
                ]);
            }
            3 => {
                assert_eq!(store.get(self.name(), "item:c")?, None);
                assert_eq!(store.get(self.name(), "item:a")?, Some(json!(8)));
                assert_eq!(
                    store.list_after(self.name(), "item:", Some("item:b"), 2)?,
                    vec![("item:d".into(), json!(4)), ("item:f".into(), json!(6))]
                );
            }
            _ => {}
        }
        Ok(mutations)
    }
}

struct ObservingProjection;

impl ProjectionHandler for ObservingProjection {
    fn name(&self) -> &'static str {
        "observer-v1"
    }
    fn requires_payload(&self) -> bool {
        false
    }
    fn project(
        &self,
        store: &dyn ProjectionStore,
        _event: &EventEnvelope,
        _payload: &Value,
    ) -> Result<Vec<ProjectionMutation>, StoreError> {
        let count = store.get("counter-v1", "count")?.unwrap_or(json!(0));
        Ok(vec![ProjectionMutation::Upsert {
            key: "observed".into(),
            value: count,
        }])
    }
}

fn fixture() -> (Arc<InMemoryEventJournal>, Arc<RecordingProjectionStore>) {
    let journal = Arc::new(InMemoryEventJournal::default());
    journal
        .append(event("seed", 0, "seed.v1", json!({})))
        .expect("seed journal");
    for version in 0..3 {
        journal
            .append(event("counter", version, "counter.v1", json!({})))
            .expect("append");
    }
    let store = Arc::new(RecordingProjectionStore::default());
    store
        .apply_all(&[
            ProjectionBatch {
                projection: "counter-v1".into(),
                expected_position: 0,
                through_sequence: 1,
                mutations: [("item:a", 1), ("item:b", 2), ("item:d", 4), ("item:f", 6)]
                    .map(|(key, value)| ProjectionMutation::Upsert {
                        key: key.into(),
                        value: json!(value),
                    })
                    .into(),
            },
            ProjectionBatch {
                projection: "observer-v1".into(),
                expected_position: 0,
                through_sequence: 1,
                mutations: Vec::new(),
            },
        ])
        .expect("seed records");
    store
        .grouped_applies
        .lock()
        .expect("recorded commits")
        .clear();
    (journal, store)
}

#[test]
fn bounded_round_preserves_staged_reads_and_matches_single_event_replay() {
    let mut outcomes = Vec::new();
    for limit in [1, 3] {
        let (journal, store) = fixture();
        let worker = ProjectionWorker::new(
            journal,
            store.clone(),
            vec![
                Arc::new(AccumulatingProjection { fail_at: None }),
                Arc::new(ObservingProjection),
            ],
        )
        .expect("worker");
        let report = worker.drain(limit, 3).expect("replay");
        assert_eq!(report.applied, 6);
        assert!(
            report
                .projections
                .iter()
                .all(|status| status.ready && status.position == 4)
        );
        let idle = worker.run_once(limit).expect("already caught up");
        assert_eq!(idle.applied, 0);
        assert_eq!(idle.projections, report.projections);
        assert_eq!(
            store.get("counter-v1", "count").expect("count"),
            Some(json!(3))
        );
        assert_eq!(
            store.get("observer-v1", "observed").expect("observed"),
            Some(json!(3))
        );
        assert_eq!(store.direct_applies.load(Ordering::Relaxed), 0);
        let commits = store.grouped_applies.lock().expect("commits");
        assert_eq!(commits.len(), if limit == 1 { 3 } else { 1 });
        assert!(commits.iter().all(|batches| batches.len() == 2));
        outcomes.push((
            store.list("counter-v1", "", 20).expect("counter records"),
            store.list("observer-v1", "", 20).expect("observer records"),
        ));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}

#[test]
fn reducer_failure_discards_the_round_without_advancing_any_checkpoint() {
    let (journal, store) = fixture();
    let worker = ProjectionWorker::new(
        journal,
        store.clone(),
        vec![
            Arc::new(ObservingProjection),
            Arc::new(AccumulatingProjection { fail_at: Some(2) }),
        ],
    )
    .expect("worker");
    assert!(matches!(worker.run_once(3), Err(StoreError::Adapter(_))));
    assert_eq!(store.position("counter-v1").expect("counter checkpoint"), 1);
    assert_eq!(
        store.position("observer-v1").expect("observer checkpoint"),
        1
    );
    assert_eq!(store.get("counter-v1", "count").expect("count"), None);
    assert_eq!(
        store.get("counter-v1", "item:a").expect("original record"),
        Some(json!(1))
    );
    assert_eq!(
        store.get("observer-v1", "observed").expect("observed"),
        None
    );
    assert!(store.grouped_applies.lock().expect("commits").is_empty());
}
