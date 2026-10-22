//! Nullable title provenance preserves old digests while surviving a schema upgrade.
use super::*;
use colossus_cloud::{CloudCaller, CloudPermission, CloudRepository};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_title_provenance_migration_and_roundtrip_preserve_legacy_hashes() {
    let config = config();
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    store
        .commit(CloudTransaction {
            entities: vec![
                write(EntityKind::Project, "acceptance-project", 0, json!({})),
                write(EntityKind::Node, "runtime", 0, json!({})),
                write(
                    EntityKind::Thread,
                    "legacy",
                    0,
                    json!({"title":"Legacy name"}),
                ),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let before = store
        .read(&key(EntityKind::Thread, "legacy"))
        .await
        .unwrap();
    // Simulate the pre-title-provenance schema with an already audited row.
    let mut conn = store.pool.get().await.unwrap();
    conn.batch_execute("ALTER TABLE conversation_threads DROP COLUMN title_overridden, DROP COLUMN runtime_title_revision; DELETE FROM cloud_schema_migrations WHERE version=8;").await.unwrap();
    drop(conn);
    let restarted = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    assert_eq!(
        restarted
            .read(&key(EntityKind::Thread, "legacy"))
            .await
            .unwrap(),
        before
    );
    let legacy: colossus_cloud::CloudThread = before.value.try_into().unwrap();
    assert!(legacy.title_overridden.is_none());
    assert!(legacy.runtime_title_revision.is_none());
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("title_overridden")
            .is_none()
    );
    for (id, overridden) in [("automatic", false), ("manual", true)] {
        restarted
            .commit(CloudTransaction {
                entities: vec![write(
                    EntityKind::Thread,
                    id,
                    0,
                    json!({"title_overridden":overridden,"runtime_title_revision":7}),
                )],
                ..Default::default()
            })
            .await
            .unwrap();
        let thread: colossus_cloud::CloudThread = restarted
            .read(&key(EntityKind::Thread, id))
            .await
            .unwrap()
            .value
            .try_into()
            .unwrap();
        assert_eq!(thread.title_overridden, Some(overridden));
        assert_eq!(thread.runtime_title_revision, Some(7));
    }
    let caller = CloudCaller::new(
        "alice".into(),
        "acceptance-project".into(),
        [CloudPermission::Read, CloudPermission::Control].into(),
    )
    .unwrap();
    let repo = CloudRepository::new(restarted.clone()).unwrap();
    let renamed = repo
        .update_thread(
            &caller,
            "legacy",
            legacy.revision,
            Some("Human rename".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(renamed.title_overridden, Some(true));
    let reopened = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    let saved: colossus_cloud::CloudThread = reopened
        .read(&key(EntityKind::Thread, "legacy"))
        .await
        .unwrap()
        .value
        .try_into()
        .unwrap();
    assert_eq!(saved, renamed);
    // Recovery selects the first allocated run, skipping an earlier failed or
    // queued submission before applying its one-per-session SDK read bound.
    reopened.commit(CloudTransaction {
        entities: vec![
            write(EntityKind::Task, "unbound-first", 0, json!({"thread_id":"legacy","created_at":"2000-01-01T00:00:00Z"})),
            write(EntityKind::Task, "bound-later", 0, json!({"thread_id":"legacy","run_id":"retained-run","created_at":"2000-01-02T00:00:00Z"})),
        ],
        ..Default::default()
    }).await.unwrap();
    let first_bound = reopened
        .list(&EntityQuery {
            kind: EntityKind::Task,
            project_id: "acceptance-project".into(),
            node_id: Some("runtime".into()),
            parent_id: Some("legacy".into()),
            status: Some("bound".into()),
            order: EntityOrder::CreatedAsc,
            limit: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(first_bound.len(), 1);
    assert_eq!(first_bound[0].key.id, "bound-later");
    let mut conn = reopened.pool.get().await.unwrap();
    conn.batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}
