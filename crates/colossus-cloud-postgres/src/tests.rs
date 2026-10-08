use crate::{CloudDatabaseConfig, CloudDatabaseTls, CloudPostgresStore};
use colossus_cloud::{CloudError, storage::*};
use colossus_network::AdditionalRootCertificates;
use colossus_ports::StoreError;
use diesel_async::SimpleAsyncConnection;
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
mod fixtures;
mod identity;
mod normalized;

fn key(kind: EntityKind, id: &str) -> EntityKey {
    EntityKey {
        kind,
        project_id: "acceptance-project".into(),
        parent_id: None,
        id: id.into(),
    }
}
fn write(kind: EntityKind, id: &str, expected: u64, value: serde_json::Value) -> EntityMutation {
    EntityMutation {
        key: key(kind, id),
        expected_revision: expected,
        value: fixtures::value(kind, id, value),
        actor: "acceptance".into(),
        operation: "cloud.acceptance.v1".into(),
    }
}
fn config() -> CloudDatabaseConfig {
    CloudDatabaseConfig {
        connection_variable: "COLOSSUS_CLOUD_TEST_DATABASE_URL".into(),
        schema: format!("cloud_acceptance_{}", uuid::Uuid::now_v7().simple()),
        tls: CloudDatabaseTls::Disabled,
        max_connections: 4,
        connection_timeout_ms: 5000,
        statement_timeout_ms: 15000,
    }
}

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_atomic_recovery_and_replica_conformance() {
    let config = config();
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    store.commit(CloudTransaction{entities:vec![write(EntityKind::Project,"acceptance-project",0,json!({"project_id":"acceptance-project","label":"Acceptance"})),write(EntityKind::Node,"runtime",0,json!({"node_id":"runtime","instance_id":"local-instance","label":"Runtime","revoked":false}))],..Default::default()}).await.unwrap();
    let host = write(
        EntityKind::Host,
        "host",
        0,
        json!({"label":"Acceptance host"}),
    );
    store
        .commit(CloudTransaction {
            entities: vec![host.clone()],
            ..Default::default()
        })
        .await
        .unwrap();
    let results = tokio::join!(
        store.commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Host,
                "host",
                1,
                json!({"label":"Writer one"})
            )],
            ..Default::default()
        }),
        store.commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Host,
                "host",
                1,
                json!({"label":"Writer two"})
            )],
            ..Default::default()
        })
    );
    assert_ne!(results.0.is_ok(), results.1.is_ok());
    assert_eq!(store.read(&host.key).await.unwrap().revision, 2);
    // A conflict after another domain write rolls the complete transaction back.
    assert!(matches!(
        store
            .commit(CloudTransaction {
                entities: vec![
                    write(
                        EntityKind::Node,
                        "must-rollback",
                        0,
                        json!({"label":"Rollback"})
                    ),
                    write(EntityKind::Host, "host", 0, json!({"label":"Invalid"}))
                ],
                ..Default::default()
            })
            .await,
        Err(StoreError::Conflict { .. })
    ));
    assert_eq!(
        store
            .read(&key(EntityKind::Node, "must-rollback"))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    let event = ReleasedEvent {
        project_id: "acceptance-project".into(),
        scope_id: "scope".into(),
        sequence: 1,
        value: json!({"released":"first"}),
    };
    store
        .commit(CloudTransaction {
            events: vec![event.clone()],
            cursors: vec![CursorMutation {
                project_id: event.project_id.clone(),
                source_id: "runtime".into(),
                scope_id: event.scope_id.clone(),
                expected_sequence: 0,
                sequence: 1,
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .commit(CloudTransaction {
            events: vec![event.clone()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .cursor("acceptance-project", "runtime", "scope")
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .commit(CloudTransaction {
                events: vec![ReleasedEvent {
                    sequence: 3,
                    ..event.clone()
                }],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        store
            .commit(CloudTransaction {
                events: vec![ReleasedEvent {
                    value: json!({"released":"changed"}),
                    ..event.clone()
                }],
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        store
            .events("other-project", "scope", 0, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .events("acceptance-project", "scope", 0, 100)
            .await
            .unwrap(),
        vec![event.clone()]
    );
    store.commit(CloudTransaction{entities:vec![write(EntityKind::Workspace,"workspace",0,json!({"node_id":"runtime","label":"Workspace"})),write(EntityKind::Thread,"thread",0,json!({"node_id":"runtime","workspace_id":"workspace","title":"Conversation","created_at":"2026-10-05T12:00:00Z","updated_at":"2026-10-05T12:00:00Z"}))],..Default::default()}).await.unwrap();
    store
        .commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Task,
                "message-task",
                0,
                json!({"thread_id":"thread"}),
            )],
            ..Default::default()
        })
        .await
        .unwrap();
    let mut older = write(
        EntityKind::ThreadMessage,
        "z-older",
        0,
        json!({"role":"user","created_at":"2026-10-05T12:00:01Z"}),
    );
    older.key.parent_id = Some("thread".into());
    let mut newer = write(
        EntityKind::ThreadMessage,
        "a-newer",
        0,
        json!({"role":"assistant","created_at":"2026-10-05T12:00:02Z"}),
    );
    newer.key.parent_id = Some("thread".into());
    store
        .commit(CloudTransaction {
            entities: vec![older, newer],
            ..Default::default()
        })
        .await
        .unwrap();
    let query = EntityQuery {
        kind: EntityKind::ThreadMessage,
        project_id: "acceptance-project".into(),
        parent_id: Some("thread".into()),
        order: EntityOrder::CreatedDesc,
        limit: 1,
        ..Default::default()
    };
    let page = store.list(&query).await.unwrap();
    assert_eq!(page[0].key.id, "a-newer");
    let page = store
        .list(&EntityQuery {
            after: page[0].page_cursor.clone(),
            ..query
        })
        .await
        .unwrap();
    assert_eq!(page[0].key.id, "z-older");
    assert_stable_page_boundary(&store).await;
    assert!(
        !store
            .thread_incomplete("acceptance-project", "thread")
            .await
            .unwrap()
    );
    let mut turns = Vec::new();
    turns.push(write(EntityKind::Task,"old-incomplete",0,json!({"node_id":"runtime","thread_id":"thread","subject":"human","last_sequence":0,"output_limited":true,"created_at":"2025-01-01T00:00:00Z"})));
    for index in 0..105 {
        turns.push(write(EntityKind::Task,&format!("new-complete-{index:03}"),0,json!({"node_id":"runtime","thread_id":"thread","subject":"human","last_sequence":0,"output_limited":false,"created_at":"2026-01-01T00:00:00Z"})));
    }
    store
        .commit(CloudTransaction {
            entities: turns,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        store
            .thread_incomplete("acceptance-project", "thread")
            .await
            .unwrap()
    );
    assert!(
        !store
            .thread_incomplete("other-project", "thread")
            .await
            .unwrap()
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let session = AuthSession {
        security_epoch: 0,
        session_hash: "a".repeat(64),
        subject: "acceptance-user".into(),
        csrf_hash: "b".repeat(64),
        created_at: now,
        expires_at: now + 120,
    };
    store.put_session(session.clone()).await.unwrap();
    let lease = store
        .claim_lease("acceptance-project", "runtime", "replica-one", now, 30)
        .await
        .unwrap();
    assert_eq!(
        store
            .claim_lease("acceptance-project", "runtime", "replica-two", now, 30)
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    let fenced = store
        .claim_lease("acceptance-project", "runtime", "replica-one", now, 30)
        .await
        .unwrap();
    assert!(fenced.generation > lease.generation);
    assert_eq!(
        store.verify_lease(&lease, now).await.unwrap_err(),
        CloudError::Conflict
    );
    assert!(matches!(
        store
            .commit(CloudTransaction {
                entities: vec![write(
                    EntityKind::Host,
                    "stale-owner",
                    0,
                    json!({"label":"Stale"})
                )],
                lease: Some(lease),
                ..Default::default()
            })
            .await,
        Err(StoreError::WriterLeaseHeld)
    ));
    store.release_lease(&fenced).await.unwrap();
    assert_eq!(
        store
            .claim_lease("acceptance-project", "missing-node", "replica", now, 30)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let revoked_lease = store
        .claim_lease("acceptance-project", "runtime", "revocation-case", now, 30)
        .await
        .unwrap();
    let current = store.read(&key(EntityKind::Node, "runtime")).await.unwrap();
    let mut revoked = colossus_cloud::CloudNode::try_from(current.value).unwrap();
    revoked.revoked = true;
    store
        .commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Node,
                "runtime",
                current.revision,
                serde_json::to_value(revoked).unwrap(),
            )],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(
        store
            .commit(CloudTransaction {
                entities: vec![write(
                    EntityKind::Host,
                    "revoked-owner",
                    0,
                    json!({"label":"Must not persist"})
                )],
                lease: Some(revoked_lease),
                ..Default::default()
            })
            .await,
        Err(StoreError::WriterLeaseHeld)
    ));
    assert_eq!(
        store
            .read(&key(EntityKind::Host, "revoked-owner"))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        store
            .read_lease("acceptance-project", "runtime", now)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    let replica = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    assert_eq!(
        replica
            .read_session(&session.session_hash, now)
            .await
            .unwrap(),
        session
    );
    assert_eq!(
        replica
            .events("acceptance-project", "scope", 0, 100)
            .await
            .unwrap(),
        vec![event]
    );
    let mut notifications = replica.subscribe();
    // Dedicated LISTEN connections connect asynchronously; this is acceptance synchronization.
    tokio::time::sleep(Duration::from_millis(100)).await;
    store
        .commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Host,
                "notification",
                0,
                json!({"label":"Notification"}),
            )],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), notifications.recv())
            .await
            .unwrap()
            .unwrap(),
        "acceptance-project"
    );
    store
        .delete_entity(&key(EntityKind::Host, "notification"), 1)
        .await
        .unwrap();
    assert_eq!(
        store
            .read(&key(EntityKind::Host, "notification"))
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert!(
        store
            .commit(CloudTransaction {
                entities: vec![write(
                    EntityKind::Host,
                    "notification",
                    0,
                    json!({"label":"Cannot reuse"})
                )],
                ..Default::default()
            })
            .await
            .is_err()
    );
    let mut conn = store.pool.get().await.unwrap();
    conn.batch_execute("UPDATE hosts SET host_name='changed without an audit' WHERE id='host'")
        .await
        .unwrap();
    assert_eq!(
        store.read(&host.key).await.unwrap_err(),
        CloudError::Storage
    );
    // Only the generated acceptance namespace is removed.
    assert!(config.schema.starts_with("cloud_acceptance_"));
    conn.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}

#[test]
fn config_rejects_identifiers_and_remote_plaintext() {
    let mut config = config();
    config.schema = "cloud;DROP TABLE".into();
    assert_eq!(config.validate().unwrap_err(), CloudError::InvalidArgument);
    config.schema = "cloud".into();
    assert_eq!(
        crate::connection::parse("postgres://fixture:fixture@example.com/cloud", &config)
            .unwrap_err(),
        CloudError::InvalidArgument
    );
}

#[test]
fn stored_hashes_ignore_object_order_without_losing_large_integers() {
    let first: serde_json::Value =
        serde_json::from_str(r#"{"z":{"long":18446744073709551615,"a":[{"z":true,"a":1}]},"a":0}"#)
            .unwrap();
    let reordered: serde_json::Value =
        serde_json::from_str(r#"{"a":0,"z":{"a":[{"a":1,"z":true}],"long":18446744073709551615}}"#)
            .unwrap();
    // Exercise the exact feature union that previously made JSONB reads reject audits.
    assert_eq!(first.as_object().unwrap().keys().next().unwrap(), "z");
    let expected = br#"{"a":0,"z":{"a":[{"a":1,"z":true}],"long":18446744073709551615}}"#;
    assert_eq!(crate::canonical::bytes(&first).unwrap(), expected);
    assert_eq!(
        crate::entities::digest(&first).unwrap(),
        crate::entities::digest(&reordered).unwrap()
    );
    let changed: serde_json::Value =
        serde_json::from_str(r#"{"a":0,"z":{"a":[{"a":1,"z":true}],"long":18446744073709551614}}"#)
            .unwrap();
    assert_ne!(
        crate::entities::digest(&first).unwrap(),
        crate::entities::digest(&changed).unwrap()
    );
    let negative_zero: serde_json::Value =
        serde_json::from_str(r#"{"z":-0.0,"a":0,"nested":[0.0,-0.0]}"#).unwrap();
    assert_eq!(
        crate::canonical::bytes(&negative_zero).unwrap(),
        br#"{"a":0,"nested":[0.0,0.0],"z":0.0}"#
    );
}

async fn assert_stable_page_boundary(store: &dyn CloudStore) {
    let value = |time: &str| json!({"node_id":"runtime","workspace_id":"workspace","title":"CursorRegression","created_at":"2026-01-01T00:00:00Z","updated_at":time});
    store
        .commit(CloudTransaction {
            entities: vec![
                write(
                    EntityKind::Thread,
                    "page-first",
                    0,
                    value("2026-10-05T12:00:03Z"),
                ),
                write(
                    EntityKind::Thread,
                    "page-boundary",
                    0,
                    value("2026-10-05T12:00:02Z"),
                ),
                write(
                    EntityKind::Thread,
                    "page-older",
                    0,
                    value("2026-10-05T12:00:01Z"),
                ),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let query = EntityQuery {
        kind: EntityKind::Thread,
        project_id: "acceptance-project".into(),
        query: Some("CursorRegression".into()),
        order: EntityOrder::UpdatedDesc,
        limit: 2,
        ..Default::default()
    };
    let first = store.list(&query).await.unwrap();
    assert_eq!(
        first
            .iter()
            .map(|record| record.key.id.as_str())
            .collect::<Vec<_>>(),
        vec!["page-first", "page-boundary"]
    );
    let after = first[1].page_cursor.clone();
    assert!(after.as_ref().unwrap().starts_with("cp1_"));
    store
        .commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Thread,
                "page-boundary",
                1,
                value("2026-10-05T12:00:05Z"),
            )],
            ..Default::default()
        })
        .await
        .unwrap();
    let second = store
        .list(&EntityQuery {
            after: after.clone(),
            ..query.clone()
        })
        .await
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|record| record.key.id.as_str())
            .collect::<Vec<_>>(),
        vec!["page-older"]
    );
    assert_eq!(
        store
            .read(&key(EntityKind::Thread, "page-boundary"))
            .await
            .unwrap()
            .page_cursor,
        None
    );
    assert_eq!(
        store
            .list(&EntityQuery {
                project_id: "other-project".into(),
                after,
                ..query
            })
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
}

#[tokio::test]
async fn memory_page_cursor_preserves_position_when_boundary_changes() {
    let store = MemoryCloudStore::default();
    assert_stable_page_boundary(&store).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and a separate synthetic audit signing reference"]
async fn postgres_checkpoint_and_maintenance_conformance() {
    let config = config();
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    let body:serde_json::Value=serde_json::from_str(r#"{"verifier":"synthetic-opaque-pkce","nonce":"synthetic-nonce","expires_at":9999999999,"metadata":{"z":18446744073709551615,"a":[1,2,3],"zero":[-0.0,0.0]}}"#).unwrap();
    let body_key = EntityKey {
        kind: EntityKind::AuthFlow,
        project_id: "__auth".into(),
        parent_id: None,
        id: "preserve-order-proof".into(),
    };
    store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: body_key.clone(),
                expected_revision: 0,
                value: EntityValue::AuthFlow(body.clone()),
                actor: "acceptance".into(),
                operation: "cloud.auth.flow-created.v2".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .read(&body_key)
            .await
            .unwrap()
            .value
            .auth_flow()
            .unwrap(),
        &body
    );
    store
        .commit(CloudTransaction {
            entities: vec![
                write(
                    EntityKind::Project,
                    "acceptance-project",
                    0,
                    json!({"project_id":"acceptance-project","label":"Acceptance"}),
                ),
                write(
                    EntityKind::Host,
                    "anchor-host",
                    0,
                    json!({"label":"Anchored host"}),
                ),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .commit(CloudTransaction {
            entities: vec![write(
                EntityKind::Host,
                "anchor-host",
                1,
                json!({"label":"Anchored revision two"}),
            )],
            ..Default::default()
        })
        .await
        .unwrap();
    let anchor = store
        .export_audit_checkpoint(
            "acceptance-project",
            "COLOSSUS_CLOUD_TEST_AUDIT_SIGNING_SEED",
        )
        .await
        .unwrap();
    crate::verify_checkpoint_signature(&anchor, &anchor.public_key).unwrap();
    store
        .verify_audit_checkpoint(&anchor, &anchor.public_key)
        .await
        .unwrap();
    let mut changed = anchor.clone();
    changed.checkpoint.heads[0].chain_hash = "0".repeat(64);
    assert!(crate::verify_checkpoint_signature(&changed, &anchor.public_key).is_err());
    assert!(crate::verify_checkpoint_signature(&anchor, &"0".repeat(64)).is_err());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    store
        .put_session(AuthSession {
            security_epoch: 0,
            session_hash: "d".repeat(64),
            subject: "expired".into(),
            csrf_hash: String::new(),
            created_at: now - 60,
            expires_at: now - 1,
        })
        .await
        .unwrap();
    let expired = EntityMutation {
        key: EntityKey {
            kind: EntityKind::AuthFlow,
            project_id: "__auth".into(),
            parent_id: None,
            id: "expired-flow".into(),
        },
        expected_revision: 0,
        value: EntityValue::AuthFlow(
            json!({"nonce":"synthetic","ciphertext":"opaque-synthetic-ciphertext","expires_at":now-1}),
        ),
        actor: "acceptance".into(),
        operation: "cloud.auth-flow.created.v1".into(),
    };
    let marker = EntityMutation {
        key: EntityKey {
            kind: EntityKind::AuthFlow,
            project_id: "__migration".into(),
            parent_id: None,
            id: "retained-marker".into(),
        },
        expected_revision: 0,
        value: EntityValue::AuthFlow(json!({"status":"running"})),
        actor: "acceptance".into(),
        operation: "cloud.import.started.v1".into(),
    };
    store
        .commit(CloudTransaction {
            entities: vec![expired.clone(), marker.clone()],
            ..Default::default()
        })
        .await
        .unwrap();
    let report = store
        .maintain(now, &CloudMaintenancePolicy::default())
        .await
        .unwrap();
    assert!(report.published_outbox >= 1);
    assert_eq!(report.expired_sessions, 1);
    assert_eq!(report.expired_auth_flows, 1);
    assert_eq!(
        store.read(&expired.key).await.unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(store.read(&marker.key).await.unwrap().value, marker.value);
    let report = store
        .maintain(now + 86402, &CloudMaintenancePolicy::default())
        .await
        .unwrap();
    assert!(report.removed_outbox >= 1);
    store
        .verify_audit_checkpoint(&anchor, &anchor.public_key)
        .await
        .unwrap();
    let mut conn = store.pool.get().await.unwrap();
    conn.batch_execute("UPDATE hosts SET revision=1 WHERE id='anchor-host'")
        .await
        .unwrap();
    assert_eq!(
        store
            .verify_audit_checkpoint(&anchor, &anchor.public_key)
            .await
            .unwrap_err(),
        CloudError::Storage
    );
    conn.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}
