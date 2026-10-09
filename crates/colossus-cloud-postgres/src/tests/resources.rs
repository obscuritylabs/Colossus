//! Real PostgreSQL conformance across independent adapter/replica instances.
use super::*;
use colossus_cloud::{CloudCaller, CloudPermission};
use colossus_cloud_protocol::{ResourceOperation, ResourceReply};
use diesel_async::RunQueryDsl;
use std::collections::BTreeSet;
#[tokio::test]
#[ignore = "requires an isolated local PostgreSQL fixture"]
async fn postgres_resource_replica_fencing_ownership_audit_and_retention() {
    let config = config();
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    let replica = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    store.commit(CloudTransaction {entities:vec![write(EntityKind::Project,"acceptance-project",0,json!({"project_id":"acceptance-project","label":"Acceptance"})),write(EntityKind::Node,"runtime",0,json!({"node_id":"runtime","instance_id":"local-instance","label":"Runtime","revoked":false}))],..Default::default()}).await.unwrap();
    let caller = CloudCaller::new(
        "acceptance".into(),
        "acceptance-project".into(),
        BTreeSet::from([
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Control,
        ]),
    )
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lease = store
        .claim_lease("acceptance-project", "runtime", "replica-a", now, 30)
        .await
        .unwrap();
    assert_eq!(
        replica
            .resource_submit(&caller, &lease, ResourceOperation::Context, now)
            .await,
        Err(CloudError::Conflict)
    );
    store.resource_connect(&lease, true).await.unwrap();
    let id = replica
        .resource_submit(&caller, &lease, ResourceOperation::Context, now)
        .await
        .unwrap();
    let pending = store.resource_take(&lease, now).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, id);
    assert!(replica.resource_take(&lease, now).await.unwrap().is_empty());
    let other = CloudCaller::new(
        "other".into(),
        "acceptance-project".into(),
        BTreeSet::from([CloudPermission::Read]),
    )
    .unwrap();
    assert!(matches!(
        replica.resource_read(&other, &id).await,
        Err(CloudError::NotFound)
    ));
    store
        .resource_complete(
            &lease,
            &id,
            ResourceReply::Result {
                value: json!({"capabilities":[]}),
            },
            now,
        )
        .await
        .unwrap();
    assert!(matches!(
        replica.resource_read(&caller, &id).await.unwrap(),
        Some(ResourceReply::Result { .. })
    ));
    if std::env::var_os("COLOSSUS_RESOURCE_TEST_CHECKPOINT").is_some() {
        store
            .export_audit_checkpoint("acceptance-project", "COLOSSUS_RESOURCE_TEST_CHECKPOINT")
            .await
            .unwrap();
    }
    let lost = replica
        .resource_submit(
            &caller,
            &lease,
            ResourceOperation::ListSchedules { after: None },
            now,
        )
        .await
        .unwrap();
    assert_eq!(store.resource_take(&lease, now).await.unwrap().len(), 1);
    store.release_lease(&lease).await.unwrap();
    let replacement = replica
        .claim_lease("acceptance-project", "runtime", "replica-b", now, 30)
        .await
        .unwrap();
    replica.resource_connect(&replacement, true).await.unwrap();
    assert!(
        replica
            .resource_take(&replacement, now)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .resource_complete(
                &lease,
                &lost,
                ResourceReply::Result { value: json!({}) },
                now
            )
            .await
            .is_err()
    );
    assert!(
        replica
            .resource_read(&caller, &lost)
            .await
            .unwrap()
            .is_none()
    );
    let mut conn = replica.pool.get().await.unwrap();
    diesel::sql_query("UPDATE runtime_resource_requests SET reply='{}'::jsonb WHERE request_id=$1")
        .bind::<diesel::sql_types::Text, _>(&id)
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    assert!(matches!(
        store.resource_read(&caller, &id).await,
        Err(CloudError::Storage)
    ));
    // Short-lived payloads expire independently from retained audit evidence.
    let report = store
        .maintain(now + 120, &CloudMaintenancePolicy::default())
        .await
        .unwrap();
    assert!(report.expired_resource_requests >= 2);
    assert!(matches!(
        replica.resource_read(&caller, &id).await,
        Err(CloudError::NotFound)
    ));
    store.remove_fixture_schema(&config.schema).await.unwrap();
}
