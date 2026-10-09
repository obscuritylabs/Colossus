use super::*;
use colossus_cloud_protocol::{ResourceOperation, ResourceReply};

#[tokio::test]
async fn resources_are_user_project_and_generation_bound_without_replay() {
    let (repo, caller, node) = fixture().await;
    let store = repo.storage();
    let lease = store
        .claim_lease(&node.project_id, &node.node_id, "replica-a", 100, 30)
        .await
        .unwrap();
    // An older connector cannot accept even a context request.
    assert_eq!(
        store
            .resource_submit(&caller, &lease, ResourceOperation::Context, 101)
            .await,
        Err(CloudError::Conflict)
    );
    store.resource_connect(&lease, true).await.unwrap();
    let id = store
        .resource_submit(&caller, &lease, ResourceOperation::Context, 101)
        .await
        .unwrap();
    let dispatched = store.resource_take(&lease, 102).await.unwrap();
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].request_id, id);
    assert!(store.resource_take(&lease, 103).await.unwrap().is_empty());
    let other = CloudCaller::new(
        "bob".into(),
        caller.project_id().into(),
        BTreeSet::from([CloudPermission::Read]),
    )
    .unwrap();
    assert!(matches!(
        store.resource_read(&other, &id).await,
        Err(CloudError::NotFound)
    ));
    let outsider = super::caller("other-project", &[CloudPermission::Read]);
    assert!(matches!(
        store.resource_read(&outsider, &id).await,
        Err(CloudError::NotFound)
    ));
    store
        .resource_complete(
            &lease,
            &id,
            ResourceReply::Result {
                value: serde_json::json!({"capabilities":[]}),
            },
            104,
        )
        .await
        .unwrap();
    assert!(matches!(
        store.resource_read(&caller, &id).await.unwrap(),
        Some(ResourceReply::Result { .. })
    ));
    let lost = store
        .resource_submit(
            &caller,
            &lease,
            ResourceOperation::ListSchedules { after: None },
            105,
        )
        .await
        .unwrap();
    assert_eq!(store.resource_take(&lease, 106).await.unwrap().len(), 1);
    store.release_lease(&lease).await.unwrap();
    let replacement = store
        .claim_lease(&node.project_id, &node.node_id, "replica-b", 107, 30)
        .await
        .unwrap();
    store.resource_connect(&replacement, true).await.unwrap();
    assert!(
        store
            .resource_take(&replacement, 108)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .resource_complete(
                &replacement,
                &lost,
                ResourceReply::Result {
                    value: serde_json::json!({})
                },
                108
            )
            .await
            .is_ok()
    );
    assert!(store.resource_read(&caller, &lost).await.unwrap().is_none());
}
#[tokio::test]
async fn resources_enforce_separate_permissions_budgets_and_revocation() {
    let (repo, admin, node) = fixture().await;
    let store = repo.storage();
    let lease = store
        .claim_lease(&node.project_id, &node.node_id, "replica", 100, 30)
        .await
        .unwrap();
    store.resource_connect(&lease, true).await.unwrap();
    let viewer = caller("project-a", &[CloudPermission::Read]);
    let write = ResourceOperation::SetScheduleEnabled {
        request: colossus_sdk::SetWorkflowScheduleEnabledRequest {
            schedule_id: "schedule".into(),
            enabled: false,
            etag: "revision".into(),
        },
    };
    assert_eq!(
        store
            .resource_submit(&viewer, &lease, write.clone(), 101)
            .await,
        Err(CloudError::PermissionDenied)
    );
    let operator = caller(
        "project-a",
        &[CloudPermission::Read, CloudPermission::Execute],
    );
    assert_eq!(
        store.resource_submit(&operator, &lease, write, 101).await,
        Err(CloudError::PermissionDenied)
    );
    for _ in 0..colossus_cloud_protocol::MAX_RESOURCE_REQUESTS {
        store
            .resource_submit(&viewer, &lease, ResourceOperation::Context, 101)
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .resource_submit(&viewer, &lease, ResourceOperation::Context, 101)
            .await,
        Err(CloudError::ResourceExhausted)
    );
    assert_eq!(
        store
            .resource_submit(
                &viewer,
                &lease,
                ResourceOperation::ValidateWorkflow {
                    yaml: "a".repeat(193 * 1024)
                },
                101
            )
            .await,
        Err(CloudError::InvalidArgument)
    );
    // Deadline expiry frees admission but does not dispatch stale requests.
    assert!(store.resource_take(&lease, 122).await.unwrap().is_empty());
    store
        .resource_submit(&viewer, &lease, ResourceOperation::Context, 122)
        .await
        .unwrap();
    repo.revoke_node(&admin, &node.node_id, node.revision)
        .await
        .unwrap();
    assert!(store.resource_take(&lease, 123).await.is_err());
}
