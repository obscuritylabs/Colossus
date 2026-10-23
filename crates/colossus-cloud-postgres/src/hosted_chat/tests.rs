use super::*;
use crate::{CloudDatabaseTls, CloudPostgresStore};
use diesel_async::SimpleAsyncConnection;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn shared_custody_cas_placement_and_at_most_once_management() {
    let config = CloudDatabaseConfig {
        connection_variable: "COLOSSUS_CLOUD_TEST_DATABASE_URL".into(),
        schema: format!("hosted_sql_{}", uuid::Uuid::now_v7().simple()),
        tls: CloudDatabaseTls::Disabled,
        max_connections: 2,
        connection_timeout_ms: 5000,
        statement_timeout_ms: 15000,
    };
    let store = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    let a = std::sync::Arc::new(ChatSqlStore::open(config.clone()).unwrap());
    let b = std::sync::Arc::new(ChatSqlStore::open(config.clone()).unwrap());
    let barrier = std::sync::Barrier::new(2);
    let (first, second) = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            a.claim_lease("partition", "project", "alice", "owner-a", 30)
        });
        let two = scope.spawn(|| {
            barrier.wait();
            b.claim_lease("partition", "project", "alice", "owner-b", 30)
        });
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_ne!(first.is_ok(), second.is_ok());
    let lease = first.or(second).unwrap();
    assert_eq!(b.read_lease("partition").unwrap(), lease);
    a.write_records(&[ChatRecordWrite {
        namespace: "personal-alice",
        record_id: "credential",
        expected_revision: 0,
        ciphertext: Some(b"opaque-encrypted-one"),
        ttl_seconds: None,
    }])
    .unwrap();
    assert_eq!(
        b.read_record("personal-alice", "credential")
            .unwrap()
            .ciphertext
            .as_deref(),
        Some(b"opaque-encrypted-one".as_slice())
    );
    assert!(
        b.read_record("personal-bob", "credential")
            .unwrap()
            .ciphertext
            .is_none()
    );
    assert_eq!(
        b.write_records(&[ChatRecordWrite {
            namespace: "personal-alice",
            record_id: "credential",
            expected_revision: 0,
            ciphertext: Some(b"stale"),
            ttl_seconds: None
        }])
        .unwrap_err(),
        CloudError::Conflict
    );
    b.write_records(&[ChatRecordWrite {
        namespace: "personal-alice",
        record_id: "credential",
        expected_revision: 1,
        ciphertext: Some(b"opaque-encrypted-two"),
        ttl_seconds: None,
    }])
    .unwrap();
    assert_eq!(
        a.read_record("personal-alice", "credential")
            .unwrap()
            .revision,
        2
    );
    // A later batch conflict rolls back its otherwise valid first update.
    assert_eq!(
        a.write_records(&[
            ChatRecordWrite {
                namespace: "personal-alice",
                record_id: "credential",
                expected_revision: 2,
                ciphertext: None,
                ttl_seconds: None
            },
            ChatRecordWrite {
                namespace: "personal-bob",
                record_id: "missing",
                expected_revision: 1,
                ciphertext: Some(b"never-committed"),
                ttl_seconds: None
            },
        ])
        .unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        b.read_record("personal-alice", "credential")
            .unwrap()
            .revision,
        2
    );
    a.submit_management(&lease, "request-one", b"opaque-request")
        .unwrap();
    let requests = b.take_management(&lease).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].id, "request-one");
    assert!(a.take_management(&lease).unwrap().is_empty());
    let handoff = b
        .complete_management(
            &lease,
            "request-one",
            b"opaque-response",
            Some("administrator"),
        )
        .unwrap()
        .unwrap();
    assert!(handoff.generation > lease.generation);
    assert_eq!(a.verify_lease(&lease).unwrap_err(), CloudError::Conflict);
    assert_eq!(
        a.management_response("request-one").unwrap().as_deref(),
        Some(b"opaque-response".as_slice())
    );
    a.release_lease(&lease).unwrap();
    assert_eq!(b.read_lease("partition").unwrap(), handoff);
    assert_eq!(
        a.write_records_fenced(
            &[ChatRecordWrite {
                namespace: "personal-alice",
                record_id: "credential",
                expected_revision: 2,
                ciphertext: Some(b"stale-owner"),
                ttl_seconds: None
            }],
            Some(&lease)
        )
        .unwrap_err(),
        CloudError::Conflict
    );
    a.claim_project_gate("project", "admin", 30).unwrap();
    assert_eq!(
        b.claim_lease("other-partition", "project", "bob", "owner-c", 30)
            .unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        a.write_records_guarded(
            &[ChatRecordWrite {
                namespace: "settings",
                record_id: "configuration",
                expected_revision: 0,
                ciphertext: Some(b"ciphertext"),
                ttl_seconds: None
            }],
            None,
            Some(("project", "wrong-admin"))
        )
        .unwrap_err(),
        CloudError::Conflict
    );
    a.release_project_gate("project", "wrong-admin").unwrap();
    assert_eq!(
        b.claim_project_gate("project", "another-admin", 30)
            .unwrap_err(),
        CloudError::Conflict
    );
    a.release_project_gate("project", "admin").unwrap();
    // Expiry permits takeover, and an old delayed renewal cannot resurrect its epoch.
    a.with_client(|client| {
        client
            .execute(
                "UPDATE hosted_chat_leases SET expires_at=0 WHERE namespace='partition'",
                &[],
            )
            .map(|_| ())
            .map_err(|_| CloudError::Storage)
    })
    .unwrap();
    let replacement = b
        .claim_lease("partition", "project", "alice", "replacement", 30)
        .unwrap();
    assert!(replacement.generation > handoff.generation);
    assert_eq!(
        a.renew_lease(&handoff, 30).unwrap_err(),
        CloudError::Conflict
    );
    for i in 0..4 {
        a.create_oauth_flow(&format!("state-{i}"), b"opaque-flow", "alice")
            .unwrap();
    }
    assert_eq!(
        b.create_oauth_flow("state-too-many", b"opaque-flow", "alice")
            .unwrap_err(),
        CloudError::ResourceExhausted
    );
    assert_eq!(
        b.take_record("oauth-browser", "state-0", 2).unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        b.take_record("oauth-browser", "state-0", 1).unwrap(),
        b"opaque-flow"
    );
    assert_eq!(
        a.take_record("oauth-browser", "state-0", 1).unwrap_err(),
        CloudError::Conflict
    );
    a.create_oauth_flow("state-new", b"opaque-flow", "alice")
        .unwrap();
    // Instance administration has real instance custody, independent of project identities.
    a.claim_instance_gate("instance-admin-a", 30).unwrap();
    assert_eq!(
        b.claim_instance_gate("instance-admin-b", 30).unwrap_err(),
        CloudError::Conflict
    );
    let write = [ChatRecordWrite {
        namespace: "instance-settings",
        record_id: "catalog",
        expected_revision: 0,
        ciphertext: Some(b"encrypted-instance-catalog"),
        ttl_seconds: None,
    }];
    assert_eq!(
        b.write_instance_records(&write, "wrong-owner", "alice", "instance.catalog.replace")
            .unwrap_err(),
        CloudError::Conflict
    );
    a.write_instance_records(
        &write,
        "instance-admin-a",
        "alice",
        "instance.catalog.replace",
    )
    .unwrap();
    assert_eq!(
        b.read_record("instance-settings", "catalog")
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        b.write_instance_records(
            &write,
            "instance-admin-a",
            "alice",
            "instance.catalog.replace"
        )
        .unwrap_err(),
        CloudError::Conflict
    );
    a.release_instance_gate("wrong-owner").unwrap();
    assert_eq!(
        b.claim_instance_gate("instance-admin-b", 30).unwrap_err(),
        CloudError::Conflict
    );
    a.with_client(|client| {
        let row=client.query_one("SELECT COUNT(*),MIN(actor_id),MIN(operation),MIN(length(ciphertext_sha256)) FROM hosted_instance_audit",&[]).map_err(|_|CloudError::Storage)?;
        assert_eq!(row.get::<_,i64>(0),1);
        assert_eq!(row.get::<_,Option<String>>(1).as_deref(),Some("alice"));
        assert_eq!(row.get::<_,Option<String>>(2).as_deref(),Some("instance.catalog.replace"));
        assert_eq!(row.get::<_,Option<i32>>(3),Some(64));
        Ok(())
    }).unwrap();
    a.release_instance_gate("instance-admin-a").unwrap();
    b.claim_instance_gate("instance-admin-b", 30).unwrap();
    assert_eq!(
        a.write_instance_records(
            &write,
            "instance-admin-a",
            "alice",
            "instance.catalog.replace"
        )
        .unwrap_err(),
        CloudError::Conflict
    );
    b.release_instance_gate("instance-admin-b").unwrap();
    drop(a);
    drop(b);
    let mut connection = store.pool.get().await.unwrap();
    connection
        .batch_execute(&format!("DROP SCHEMA \"{}\" CASCADE", config.schema))
        .await
        .unwrap();
}
