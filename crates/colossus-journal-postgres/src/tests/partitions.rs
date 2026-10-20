use super::*;
use crate::PostgresJournalFence;
use colossus_contracts::{ProjectionBatch, ProjectionMutation};

#[tokio::test]
async fn live_journal_sync_ports_are_safe_inside_a_tokio_runtime() {
    let Some(mut config) = live_config() else {
        return;
    };
    config.runtime_namespace = "tokio-user".into();
    let journal = open(&config);
    journal.append(event("async-session", 0, 10)).unwrap();
    assert_eq!(journal.read_stream("async-session").unwrap().len(), 1);
    journal.checkpoint().unwrap();
    journal
        .apply(ProjectionBatch {
            projection: "async-projection".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: vec![],
        })
        .unwrap();
    assert_eq!(journal.position("async-projection").unwrap(), 1);
    assert_eq!(journal.verify().unwrap().event_count, 1);
    drop(journal);
    assert_eq!(open(&config).head().unwrap().0, 1);
}

#[test]
fn namespace_validation_rejects_unbounded_or_ambiguous_identity() {
    let mut config =
        PostgresJournalConfig::new("UNUSED_DATABASE", "journal", PostgresTlsConfig::Disabled)
            .unwrap();
    for value in [
        "../alice".to_owned(),
        "alice\0bob".to_owned(),
        "a".repeat(257),
    ] {
        config.runtime_namespace = value;
        assert!(config.validate().is_err());
    }
    config.runtime_namespace = "project-alice.v1".into();
    config.validate().unwrap();
}

#[test]
fn live_partitions_isolate_identical_stream_projection_and_outbox_keys() {
    let Some(mut first_config) = live_config() else {
        return;
    };
    first_config.runtime_namespace = "project-alice".into();
    let first = open(&first_config);
    let second_config = PostgresJournalConfig {
        runtime_namespace: "project-bob".into(),
        ..first_config.clone()
    };
    let second = open(&second_config);
    first.append(event("session:same-id", 0, 11)).unwrap();
    second.append(event("session:same-id", 0, 22)).unwrap();
    for (journal, value) in [(&first, 11), (&second, 22)] {
        journal
            .apply(ProjectionBatch {
                projection: "same-projection".into(),
                expected_position: 0,
                through_sequence: 1,
                mutations: vec![ProjectionMutation::Upsert {
                    key: "same-key".into(),
                    value: json!({"value":value}),
                }],
            })
            .unwrap();
        assert_eq!(journal.head().unwrap().0, 1);
        let events = journal.read_stream("session:same-id").unwrap();
        assert_eq!(
            journal.decrypt_payload(&events[0]).unwrap(),
            json!({"value":value})
        );
        assert_eq!(journal.read_global(1, 10).unwrap().len(), 1);
        assert_eq!(journal.read_projection_work(1, 10).unwrap().len(), 1);
        assert_eq!(
            journal.get("same-projection", "same-key").unwrap(),
            Some(json!({"value":value}))
        );
        assert_eq!(
            journal.list_stream_ids("session:", None, 10).unwrap(),
            vec!["session:same-id"]
        );
        journal.verify().unwrap();
    }
    first.reset("same-projection").unwrap();
    assert_eq!(second.position("same-projection").unwrap(), 1);
    let legacy = open(&PostgresJournalConfig {
        runtime_namespace: String::new(),
        ..first_config
    });
    assert_eq!(legacy.head().unwrap().0, 0);
    assert!(legacy.read_stream("session:same-id").unwrap().is_empty());
    assert!(legacy.list("same-projection", "", 10).unwrap().is_empty());
    assert_eq!(open(&second_config).head().unwrap().0, 1);
}

#[test]
fn live_expired_and_superseded_fences_cannot_append_checkpoint_or_project() {
    let Some(mut config) = live_config() else {
        return;
    };
    config.runtime_namespace = "project-alice".into();
    let seed = open(&config);
    let mut client = seed.connect().unwrap();
    let lease_schema = format!("{}_leases", config.schema);
    client.batch_execute(&format!("CREATE SCHEMA \"{lease_schema}\"; CREATE TABLE \"{lease_schema}\".hosted_chat_leases(namespace TEXT PRIMARY KEY, owner_id TEXT NOT NULL, generation BIGINT NOT NULL, expires_at BIGINT NOT NULL)")).unwrap();
    client.execute(&format!("INSERT INTO \"{lease_schema}\".hosted_chat_leases VALUES($1,'owner-a',1,EXTRACT(EPOCH FROM clock_timestamp())::BIGINT+60)"),&[&config.runtime_namespace]).unwrap();
    let make = |owner: &str, generation| {
        PostgresEventJournal::open_fenced(
            config.clone(),
            Arc::new(StaticKeyProvider::new("test-key", [7; 32])),
            Arc::new(Ed25519CheckpointSigner::new("test-signing", [8; 32])),
            PostgresJournalFence {
                lease_schema: lease_schema.clone(),
                owner_id: owner.into(),
                generation,
            },
        )
    };
    let first = make("owner-a", 1).unwrap();
    first.append(event("same", 0, 1)).unwrap();
    client
        .execute(
            &format!(
                "UPDATE \"{lease_schema}\".hosted_chat_leases SET expires_at=0 WHERE namespace=$1"
            ),
            &[&config.runtime_namespace],
        )
        .unwrap();
    assert!(matches!(
        first.append(event("same", 1, 2)),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(matches!(
        first.checkpoint(),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(matches!(
        first.apply(ProjectionBatch {
            projection: "p".into(),
            expected_position: 0,
            through_sequence: 1,
            mutations: vec![]
        }),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert!(
        make("owner-a", 1).is_err(),
        "expired owner cannot open/recover"
    );
    client.execute(&format!("UPDATE \"{lease_schema}\".hosted_chat_leases SET owner_id='owner-b',generation=2,expires_at=EXTRACT(EPOCH FROM clock_timestamp())::BIGINT+60 WHERE namespace=$1"),&[&config.runtime_namespace]).unwrap();
    let second = make("owner-b", 2).unwrap();
    second.append(event("same", 1, 3)).unwrap();
    assert!(matches!(
        first.append(event("same", 2, 4)),
        Err(StoreError::WriterLeaseHeld)
    ));
    assert_eq!(second.head().unwrap().0, 2);
}
