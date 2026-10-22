use super::*;

#[tokio::test]
async fn hosted_placement_has_no_enrollment_and_is_filtered_before_fleet_pagination() {
    let (repo, caller, fleet) = fixture().await;
    let hosted = repo
        .register_hosted_target(&caller, "alice", BTreeSet::from(["primary".into()]))
        .await
        .unwrap();
    assert_eq!(hosted.execution_kind, ExecutionKind::Hosted);
    assert!(hosted.certificate_sha256.is_empty());
    assert_eq!(
        repo.authenticate_node("project-a", &hosted.node_id, "", &hosted.instance_id)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let nodes = repo.list_connected_nodes(&caller, None, 1).await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].node_id, fleet.node_id);
    let bob = CloudCaller::new(
        "bob".into(),
        "project-a".into(),
        [CloudPermission::Read].into_iter().collect(),
    )
    .unwrap();
    assert_eq!(
        repo.get_node(&bob, &hosted.node_id).await.unwrap_err(),
        CloudError::NotFound
    );
    let task = repo
        .create_personal_thread(&caller, &hosted.node_id, None, request())
        .await
        .unwrap()
        .1;
    let pending = crate::storage::EntityQuery {
        kind: crate::storage::EntityKind::Node,
        project_id: "project-a".into(),
        status: Some("hosted_pending".into()),
        limit: 1,
        ..Default::default()
    };
    assert_eq!(
        repo.storage().list(&pending).await.unwrap()[0].key.id,
        hosted.node_id
    );
    // Even trusted code must supply a current fenced owner before dispatch.
    assert_eq!(
        repo.commands(&hosted, None, 10).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
    assert_eq!(
        repo.statistics(&caller, None, now, 7)
            .await
            .unwrap()
            .counts
            .agents,
        1,
        "Hosted targets do not inflate enrolled Fleet counts"
    );
    let lease = repo
        .storage()
        .claim_lease("project-a", &hosted.node_id, "worker-a", now, 30)
        .await
        .unwrap();
    let owner = repo.with_lease(lease.clone());
    assert_eq!(
        owner.commands(&hosted, None, 10).await.unwrap()[0].task_id,
        task.task_id
    );
    owner
        .record_receipt(
            &hosted,
            &task.task_id,
            &task.task_id,
            CloudReply::Failed {
                error: ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "request",
                    "fixture rejection",
                ),
            },
        )
        .await
        .unwrap();
    assert!(repo.storage().list(&pending).await.unwrap().is_empty());
    repo.storage().release_lease(&lease).await.unwrap();
    assert!(owner.commands(&hosted, None, 10).await.is_err());
}

#[tokio::test]
async fn legacy_personal_chat_conversion_preserves_threads_and_retires_certificate() {
    let (repo, caller, mut legacy) = fixture().await;
    let fleet = legacy.clone();
    let identity = crate::validation::fingerprint(
        &serde_json::to_vec(&("cloud-chat-v1", "project-a", "alice")).unwrap(),
    );
    legacy.node_id = format!("chat-{identity}");
    legacy.instance_id = format!("hosted-chat-{identity}");
    legacy.owner_user_id = Some("alice".into());
    legacy.revision = 0;
    let legacy = repo.register_node(&caller, legacy).await.unwrap();
    // Historical EnrollmentStore::redeem also published native grouping.
    let inventory = colossus_cloud_protocol::RuntimeInventory {
        host_id: "legacy-host".into(),
        host_label: "Historical installation".into(),
        platform: "linux".into(),
        deployment_kind: colossus_cloud_protocol::DeploymentKind::Cli,
        workspace_id: format!(
            "workspace-{}",
            crate::validation::fingerprint(format!("workspace:{}", legacy.instance_id).as_bytes())
        ),
        workspace_label: "CLI workspace".into(),
        sharing: colossus_cloud_protocol::WorkspaceSharing::CloudOwned,
        policy: None,
    };
    let legacy = repo
        .register_inventory(&legacy, inventory.clone(), 100)
        .await
        .unwrap();
    let thread = repo
        .create_personal_thread(&caller, &legacy.node_id, None, request())
        .await
        .unwrap()
        .0;
    let hosted = repo
        .register_hosted_target(&caller, "alice", legacy.roles.clone())
        .await
        .unwrap();
    assert_eq!(hosted.node_id, legacy.node_id);
    assert_eq!(hosted.host_id, legacy.host_id);
    assert!(repo.list_hosts(&caller, None, 1).await.unwrap().is_empty());
    assert!(
        repo.list_workspaces(&caller, None, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.get_host(&caller, "legacy-host").await.unwrap_err(),
        CloudError::NotFound
    );
    let mut native_inventory = inventory;
    native_inventory.workspace_id = "zz-fleet-workspace".into();
    repo.register_inventory(&fleet, native_inventory, 101)
        .await
        .unwrap();
    assert_eq!(repo.list_hosts(&caller, None, 1).await.unwrap().len(), 1);
    assert_eq!(
        repo.list_host_nodes(&caller, "legacy-host", None, 1)
            .await
            .unwrap()[0]
            .node_id,
        fleet.node_id
    );
    assert_eq!(
        repo.list_workspaces(&caller, None, 1).await.unwrap()[0].node_id,
        fleet.node_id
    );
    assert_eq!(
        repo.get_thread(&caller, &thread.thread_id)
            .await
            .unwrap()
            .thread
            .node_id,
        hosted.node_id
    );
    assert_eq!(
        repo.authenticate_node(
            "project-a",
            &legacy.node_id,
            &legacy.certificate_sha256,
            &legacy.instance_id
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.register_hosted_target(&caller, "alice", hosted.roles.clone())
            .await
            .unwrap(),
        hosted
    );
}
