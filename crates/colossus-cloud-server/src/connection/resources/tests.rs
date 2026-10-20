use super::*;
use colossus_cloud::storage::MemoryCloudStore;
use colossus_cloud::{CloudCaller, CloudNode, CloudPermission, CloudRepository};
use std::{collections::BTreeSet, sync::Arc};

#[tokio::test]
async fn legacy_connector_never_receives_additive_model_catalog_operation() {
    let store = Arc::new(MemoryCloudStore::default());
    let repo = CloudRepository::new(store.clone()).unwrap();
    let caller = CloudCaller::new(
        "alice".into(),
        "project".into(),
        BTreeSet::from([
            CloudPermission::Read,
            CloudPermission::Execute,
            CloudPermission::Administer,
        ]),
    )
    .unwrap();
    let node = repo
        .register_node(
            &caller,
            CloudNode {
                node_id: "runtime".into(),
                project_id: "project".into(),
                owner_user_id: None,
                instance_id: "instance".into(),
                label: "Runtime".into(),
                certificate_sha256: "a".repeat(64),
                roles: BTreeSet::from(["primary".into()]),
                revoked: false,
                host_id: None,
                workspace_id: None,
                workspace_label: None,
                runtime_ready: false,
                policy: None,
                policy_observed_at: None,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let now = crate::http::now();
    let lease = store
        .claim_lease(&node.project_id, &node.node_id, "replica", now, 30)
        .await
        .unwrap();
    store.resource_connect(&lease, true).await.unwrap();
    let (sender, mut receiver) = mpsc::channel(4);
    for (operation, supported, forwarded) in [
        (ResourceOperation::ModelCatalog {}, false, false),
        (ResourceOperation::Context, false, true),
        (ResourceOperation::ModelCatalog {}, true, true),
    ] {
        let id = store
            .resource_submit(&caller, &lease, operation.clone(), now)
            .await
            .unwrap();
        let request = store
            .resource_take(&lease, now)
            .await
            .unwrap()
            .pop()
            .unwrap();
        dispatch(store.as_ref(), &lease, request, supported, &sender)
            .await
            .unwrap();
        if forwarded {
            let frame = receiver.try_recv().unwrap().unwrap();
            let Some(control_frame::Body::ResourceRequest(frame)) = frame.body else {
                panic!("resource frame expected");
            };
            assert_eq!(frame.request_id, id);
            assert_eq!(
                colossus_cloud_protocol::decode::<ResourceOperation>(&frame.operation_json)
                    .unwrap(),
                operation
            );
        } else {
            assert!(receiver.try_recv().is_err());
            let Some(ResourceReply::Failed { error }) =
                store.resource_read(&caller, &id).await.unwrap()
            else {
                panic!("safe unsupported reply expected");
            };
            assert_eq!(error.code, colossus_sdk::ApiErrorCode::FailedPrecondition);
            assert!(!error.retryable);
        }
    }
}
