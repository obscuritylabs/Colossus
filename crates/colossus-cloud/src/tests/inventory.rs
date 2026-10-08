use super::*;
use colossus_cloud_protocol::{DeploymentKind, RuntimeInventory, WorkspaceSharing};

fn inventory() -> RuntimeInventory {
    RuntimeInventory {
        host_id: "host-a".into(),
        host_label: "Test host".into(),
        platform: "macos".into(),
        deployment_kind: DeploymentKind::Desktop,
        workspace_id: "workspace-a".into(),
        workspace_label: "Test workspace".into(),
        sharing: WorkspaceSharing::CloudOwned,
        policy: None,
    }
}

#[tokio::test]
async fn host_workspace_queries_keep_project_and_host_boundaries_and_page_exactly() {
    use crate::storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation, EntityValue};
    let (repo, reader, first) = fixture().await;
    repo.register_inventory(&first, inventory(), 100)
        .await
        .unwrap();
    for (id, host, workspace) in [
        ("second", "host-a", "workspace-b"),
        ("foreign", "host-b", "workspace-c"),
    ] {
        let mut node = first.clone();
        node.node_id = id.into();
        repo.storage()
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: EntityKey {
                        kind: EntityKind::Node,
                        project_id: node.project_id.clone(),
                        parent_id: None,
                        id: id.into(),
                    },
                    expected_revision: 0,
                    value: EntityValue::Node(node.clone()),
                    actor: "fixture".into(),
                    operation: "cloud.node.fixture".into(),
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        let mut observed = inventory();
        observed.host_id = host.into();
        observed.workspace_id = workspace.into();
        repo.register_inventory(&node, observed, 100).await.unwrap();
    }
    let all = repo
        .list_host_nodes(&reader, "host-a", None, 10)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .all(|node| node.host_id.as_deref() == Some("host-a"))
    );
    let page = repo
        .list_host_nodes(&reader, "host-a", None, 1)
        .await
        .unwrap();
    assert_eq!(page, all[..1]);
    assert_eq!(
        repo.list_host_nodes(&reader, "host-a", Some(&page[0].node_id), 1)
            .await
            .unwrap(),
        all[1..]
    );
    assert_eq!(
        repo.get_host(&reader, "host-a").await.unwrap().host_id,
        "host-a"
    );
    assert!(matches!(
        repo.list_host_nodes(
            &caller("project-b", &[CloudPermission::Read]),
            "host-a",
            None,
            10
        )
        .await,
        Err(CloudError::NotFound)
    ));
    assert!(matches!(
        repo.get_host(&caller(reader.project_id(), &[]), "host-a")
            .await,
        Err(CloudError::PermissionDenied)
    ));
    assert!(matches!(
        repo.list_host_nodes(&reader, "host-a", None, 101).await,
        Err(CloudError::InvalidArgument)
    ));
}

fn policy() -> RuntimePolicyPosture {
    RuntimePolicyPosture {
        schema_version: 1,
        provenance: PolicyProvenance::RuntimeReported,
        fingerprint: "a".repeat(64),
        configuration_revision: None,
        access_profile: "minimal".into(),
        sandbox_backend: PolicySandboxBackend::Native,
        sandbox_profile: "offline-default".into(),
        boundary_acknowledged: false,
        approval_mode: PolicyApprovalMode::Ask,
        allowed_roles: vec!["primary".into()],
        allowed_tools: vec!["echo".into()],
        capabilities: vec![],
        models: vec![],
        findings: vec![],
        telemetry: PolicyTelemetry {
            provenance: PolicyTelemetryProvenance::Unavailable,
            denied_requests: None,
            approval_requests: None,
            outcome_unknown_runs: None,
        },
    }
}

#[tokio::test]
async fn repeated_inventory_preserves_static_revisions_and_applies_real_changes() {
    let (repo, caller, original) = fixture().await;
    let mut inventory = inventory();
    let node = repo
        .register_inventory(&original, inventory.clone(), 100)
        .await
        .unwrap();
    let host = repo.list_hosts(&caller, None, 10).await.unwrap().remove(0);
    let workspace = repo
        .list_workspaces(&caller, None, 10)
        .await
        .unwrap()
        .remove(0);
    let mut notifications = repo.storage().subscribe();
    for now in 101..165 {
        assert_eq!(
            repo.register_inventory(&node, inventory.clone(), now)
                .await
                .unwrap(),
            node
        );
    }
    assert_eq!(
        repo.list_hosts(&caller, None, 10).await.unwrap(),
        vec![host.clone()]
    );
    assert_eq!(
        repo.list_workspaces(&caller, None, 10).await.unwrap(),
        vec![workspace.clone()]
    );
    assert!(matches!(
        notifications.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    repo.heartbeat_node(&node, true, 200).await.unwrap();
    let heartbeat_host = repo.list_hosts(&caller, None, 10).await.unwrap().remove(0);
    assert_eq!(heartbeat_host.last_seen_at, 200);
    assert_eq!(heartbeat_host.revision, host.revision + 1);
    repo.register_inventory(&node, inventory.clone(), 201)
        .await
        .unwrap();
    assert_eq!(
        repo.list_hosts(&caller, None, 10).await.unwrap(),
        vec![heartbeat_host.clone()]
    );

    inventory.sharing = WorkspaceSharing::SharedVisibleSessions;
    assert_eq!(
        repo.register_inventory(&node, inventory.clone(), 202)
            .await
            .unwrap(),
        node
    );
    let shared = repo
        .list_workspaces(&caller, None, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(shared.revision, workspace.revision + 1);
    assert_eq!(shared.sharing, "shared_visible_sessions");
    assert_eq!(
        repo.list_hosts(&caller, None, 10).await.unwrap(),
        vec![heartbeat_host.clone()]
    );

    inventory.host_label = "Renamed host".into();
    inventory.workspace_label = "Renamed workspace".into();
    let renamed = repo
        .register_inventory(&node, inventory.clone(), 203)
        .await
        .unwrap();
    assert_eq!(
        renamed.workspace_label.as_deref(),
        Some("Renamed workspace")
    );
    assert_eq!(renamed.revision, node.revision + 1);
    let renamed_host = repo.list_hosts(&caller, None, 10).await.unwrap().remove(0);
    assert_eq!(renamed_host.label, "Renamed host");
    assert_eq!(renamed_host.revision, heartbeat_host.revision + 1);
    assert_eq!(renamed_host.last_seen_at, 200);
    let renamed_workspace = repo
        .list_workspaces(&caller, None, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(renamed_workspace.label, "Renamed workspace");
    assert_eq!(renamed_workspace.revision, shared.revision + 1);
    assert_eq!(
        repo.register_inventory(&node, inventory, 204)
            .await
            .unwrap(),
        renamed
    );
}

#[tokio::test]
async fn policy_samples_remain_fresh_without_rewriting_host_or_workspace() {
    let (repo, caller, original) = fixture().await;
    let mut inventory = inventory();
    inventory.policy = Some(policy());
    let first = repo
        .register_inventory(&original, inventory.clone(), 100)
        .await
        .unwrap();
    let hosts = repo.list_hosts(&caller, None, 10).await.unwrap();
    let workspaces = repo.list_workspaces(&caller, None, 10).await.unwrap();
    let observed = repo
        .register_inventory(&first, inventory.clone(), 115)
        .await
        .unwrap();
    assert_eq!(observed.policy, first.policy);
    assert_eq!(observed.policy_observed_at, Some(115));
    assert_eq!(observed.revision, first.revision + 1);
    assert_eq!(
        repo.register_inventory(&first, inventory.clone(), 115)
            .await
            .unwrap(),
        observed
    );
    inventory.policy.as_mut().unwrap().approval_mode = PolicyApprovalMode::Deny;
    inventory.policy.as_mut().unwrap().fingerprint = "b".repeat(64);
    let changed = repo
        .register_inventory(&first, inventory, 130)
        .await
        .unwrap();
    assert_eq!(
        changed.policy.as_ref().unwrap().approval_mode,
        PolicyApprovalMode::Deny
    );
    assert_eq!(changed.policy_observed_at, Some(130));
    assert_eq!(changed.revision, observed.revision + 1);
    assert_eq!(repo.list_hosts(&caller, None, 10).await.unwrap(), hosts);
    assert_eq!(
        repo.list_workspaces(&caller, None, 10).await.unwrap(),
        workspaces
    );
}

#[tokio::test]
async fn unchanged_inventory_still_checks_placement_enrollment_and_connection_fence() {
    let (repo, caller, original) = fixture().await;
    let inventory = inventory();
    let node = repo
        .register_inventory(&original, inventory.clone(), 100)
        .await
        .unwrap();
    let mut rebound = inventory.clone();
    rebound.host_id = "another-host".into();
    assert_eq!(
        repo.register_inventory(&node, rebound, 101).await,
        Err(CloudError::Conflict)
    );
    let mut forged = node.clone();
    forged.certificate_sha256 = "b".repeat(64);
    assert_eq!(
        repo.register_inventory(&forged, inventory.clone(), 101)
            .await,
        Err(CloudError::PermissionDenied)
    );
    forged = node.clone();
    forged.project_id = "another-project".into();
    assert_eq!(
        repo.register_inventory(&forged, inventory.clone(), 101)
            .await,
        Err(CloudError::NotFound)
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let store = repo.storage();
    let lease = store
        .claim_lease(&node.project_id, &node.node_id, "replica-a", now, 30)
        .await
        .unwrap();
    let fenced = repo.with_lease(lease.clone());
    assert_eq!(
        fenced
            .register_inventory(&node, inventory.clone(), 102)
            .await
            .unwrap(),
        node
    );
    store.release_lease(&lease).await.unwrap();
    store
        .claim_lease(&node.project_id, &node.node_id, "replica-b", now, 30)
        .await
        .unwrap();
    assert_eq!(
        fenced
            .register_inventory(&node, inventory.clone(), 103)
            .await,
        Err(CloudError::Storage)
    );
    repo.revoke_node(&caller, &node.node_id, node.revision)
        .await
        .unwrap();
    assert_eq!(
        repo.register_inventory(&node, inventory, 104).await,
        Err(CloudError::PermissionDenied)
    );
}
