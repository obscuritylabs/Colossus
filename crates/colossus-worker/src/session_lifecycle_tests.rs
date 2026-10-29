use super::*;

#[test]
fn session_lifecycle_operations_preserve_authenticated_request_shape() {
    for document in [
        json!({"operation":"session_catalog","archived":false,"limit":100}),
        json!({"operation":"session_catalog","archived":true,"limit":100}),
        json!({"operation":"session_lifecycle","session_id":"exact-session"}),
        json!({"operation":"session_set_archived","session_id":"exact-session","archived":true}),
        json!({"operation":"session_set_archived","session_id":"exact-session","archived":false}),
    ] {
        let operation: WorkerOperation =
            serde_json::from_value(document.clone()).expect("native lifecycle request");
        assert_eq!(
            canonical_authentication_bytes(&operation).unwrap(),
            canonical_authentication_bytes(&document).unwrap()
        );
    }
    assert!(
        serde_json::from_value::<WorkerOperation>(json!({
            "operation":"session_set_archived","session_id":"exact-session","archived":true,
            "application_id":"caller-supplied-owner"
        }))
        .is_err(),
        "native session controls must not accept nominated application ownership"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_worker_archives_and_restores_without_deleting_the_session() {
    let fixture = hosted_worker_fixture().await;
    let created = fixture
        .client
        .call(WorkerOperation::SessionCreate {
            title: Some("Retained local conversation".into()),
        })
        .await
        .expect("create native session");
    let session_id = created["id"].as_str().expect("session identity").to_owned();
    let initial = fixture
        .client
        .call(WorkerOperation::SessionLifecycle {
            session_id: session_id.clone(),
        })
        .await
        .expect("initial lifecycle");
    assert_eq!(initial["archived"], false);
    assert_eq!(initial["revision"], 0);
    let archived = fixture
        .client
        .call(WorkerOperation::SessionSetArchived {
            session_id: session_id.clone(),
            archived: true,
        })
        .await
        .expect("archive native session");
    assert_eq!(archived["session_id"], session_id);
    assert_eq!(archived["archived"], true);
    let read = fixture
        .client
        .call(WorkerOperation::SessionLifecycle {
            session_id: session_id.clone(),
        })
        .await
        .expect("read archived lifecycle");
    assert_eq!(read, archived);
    let repeated = fixture
        .client
        .call(WorkerOperation::SessionSetArchived {
            session_id: session_id.clone(),
            archived: true,
        })
        .await
        .expect("repeat archive");
    assert_eq!(
        repeated["revision"], archived["revision"],
        "repeated archive must not append another lifecycle revision"
    );
    let history = fixture
        .client
        .call(WorkerOperation::SessionCatalog {
            archived: false,
            limit: 100,
        })
        .await
        .expect("ordinary history");
    assert!(
        !history
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == session_id)
    );
    let archive = fixture
        .client
        .call(WorkerOperation::SessionCatalog {
            archived: true,
            limit: 100,
        })
        .await
        .expect("archive catalog");
    assert!(
        archive
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == session_id)
    );
    let retained = fixture
        .client
        .call(WorkerOperation::SessionGet {
            session_id: session_id.clone(),
        })
        .await
        .expect("retained session");
    assert_eq!(retained["title"], "Retained local conversation");
    let restored = fixture
        .client
        .call(WorkerOperation::SessionSetArchived {
            session_id: session_id.clone(),
            archived: false,
        })
        .await
        .expect("restore session");
    assert_eq!(restored["archived"], false);
    assert!(restored["revision"].as_u64().unwrap() > archived["revision"].as_u64().unwrap());
    let read = fixture
        .client
        .call(WorkerOperation::SessionLifecycle {
            session_id: session_id.clone(),
        })
        .await
        .expect("read restored lifecycle");
    assert_eq!(read, restored);
    let history = fixture
        .client
        .call(WorkerOperation::SessionCatalog {
            archived: false,
            limit: 100,
        })
        .await
        .expect("restored history");
    assert!(
        history
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == session_id)
    );
    assert!(
        fixture
            .client
            .call(WorkerOperation::SessionSetArchived {
                session_id: "missing-session".into(),
                archived: true
            })
            .await
            .is_err()
    );
    assert!(
        fixture
            .client
            .call(WorkerOperation::SessionLifecycle {
                session_id: "missing-session".into()
            })
            .await
            .expect("missing lifecycle")
            .is_null()
    );
    fixture
        .client
        .ping()
        .await
        .expect("worker remains responsive");
    fixture.stop().await;
}
