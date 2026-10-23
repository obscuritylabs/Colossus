//! Account inventories preserve personal ownership before every page boundary.
use super::*;
use crate::storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation};

async fn account(repo: &CloudRepository, id: &str, is_admin: bool) -> CloudUser {
    let user = CloudUser {
        id: id.into(),
        display_name: id.into(),
        email: None,
        active: true,
        is_admin,
        revision: 1,
        created_at: "2026-10-20T00:00:00Z".into(),
        updated_at: "2026-10-20T00:00:00Z".into(),
        identities: Vec::new(),
    };
    repo.create_account(
        "fixture",
        UserAccount {
            user,
            security_epoch: 1,
        },
        Some(OidcIdentity {
            user_id: id.into(),
            issuer: "https://issuer.example.test".into(),
            subject: id.into(),
        }),
        None,
    )
    .await
    .unwrap()
}

async fn project(repo: &CloudRepository, admin: &CloudUser, id: &str) {
    repo.save_project(
        admin,
        CloudProject {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            parent_project_id: None,
            archived: false,
            revision: 0,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn user_inventory_pages_only_explicit_memberships_and_owned_metadata() {
    let (repo, alice, shared_node) = fixture().await;
    let admin = account(&repo, "alice", true).await;
    let bob = account(&repo, "bob", false).await;
    for id in ["project-a", "project-b", "project-c"] {
        project(&repo, &admin, id).await;
        let caller = CloudCaller::new(
            "alice".into(),
            id.into(),
            [CloudPermission::Read, CloudPermission::Administer].into(),
        )
        .unwrap();
        let member = repo
            .save_membership(&caller, "alice", ProjectRole::ProjectAdmin, 0)
            .await
            .unwrap();
        if id == "project-b" {
            repo.remove_membership(&caller, "alice", member.revision)
                .await
                .unwrap();
        }
    }
    repo.save_membership(&alice, "bob", ProjectRole::Operator, 0)
        .await
        .unwrap();
    let (first, cursor) = repo
        .admin_user_projects(&admin, "alice", None, 1)
        .await
        .unwrap();
    assert_eq!(first[0].project.id, "project-a");
    assert_eq!(first[0].membership.user_id, "alice");
    let (second, cursor) = repo
        .admin_user_projects(&admin, "alice", cursor.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(second[0].project.id, "project-c");
    assert!(
        repo.admin_user_projects(&admin, "alice", cursor.as_deref(), 1)
            .await
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        repo.admin_user_projects(&bob, "alice", None, 25)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.admin_user_projects(&admin, "missing", None, 25)
            .await
            .unwrap_err(),
        CloudError::NotFound
    );
    assert_eq!(
        repo.admin_user_projects(&admin, "alice", None, 101)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );

    let mut personal = shared_node.clone();
    personal.node_id = "alice-node".into();
    personal.instance_id = "alice-instance".into();
    personal.owner_user_id = Some("alice".into());
    personal.revision = 0;
    let personal = repo.register_node(&alice, personal).await.unwrap();
    let mut foreign = shared_node.clone();
    foreign.node_id = "bob-node".into();
    foreign.instance_id = "bob-instance".into();
    foreign.owner_user_id = Some("bob".into());
    foreign.revision = 0;
    let foreign = repo.register_node(&alice, foreign).await.unwrap();
    let bob_caller = CloudCaller::new(
        "bob".into(),
        "project-a".into(),
        ProjectRole::Operator.permissions(),
    )
    .unwrap();
    for (id, node, title, owner) in [
        ("own-one", &personal, "Alice first", &alice),
        ("own-two", &personal, "Alice second", &alice),
        ("bob-private", &foreign, "Bob private title", &bob_caller),
    ] {
        let mut request = request();
        request.idempotency_key = IdempotencyKey::new(id).unwrap();
        repo.create_personal_thread(owner, &node.node_id, Some(title.into()), request)
            .await
            .unwrap();
    }
    let mut shared_request = request();
    shared_request.idempotency_key = IdempotencyKey::new("shared-chat").unwrap();
    repo.create_thread(
        &alice,
        &shared_node.node_id,
        Some("Shared title".into()),
        shared_request,
    )
    .await
    .unwrap();
    let first = repo
        .admin_user_chats(&admin, &alice, "alice", None, 1)
        .await
        .unwrap();
    assert_eq!(first.visibility, UserResourceVisibility::Visible);
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].owner_user_id.as_deref(), Some("alice"));
    let second = repo
        .admin_user_chats(&admin, &alice, "alice", first.next_cursor.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_ne!(first.items[0].thread_id, second.items[0].thread_id);
    assert_eq!(
        repo.query_threads(&alice, None, None, None, first.next_cursor, 1)
            .await
            .unwrap_err(),
        CloudError::InvalidArgument
    );
    let restricted = repo
        .admin_user_chats(&admin, &alice, "bob", None, 1)
        .await
        .unwrap();
    assert_eq!(
        restricted.visibility,
        UserResourceVisibility::PersonalResourcesPrivate
    );
    assert!(restricted.items.is_empty());
    assert!(restricted.next_cursor.is_none());
    assert_eq!(
        repo.admin_user_chats(&admin, &bob_caller, "alice", None, 1)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );

    let mut writes = Vec::new();
    for (id, node) in [
        ("a-shared", &shared_node),
        ("b-foreign", &foreign),
        ("c-owned", &personal),
        ("d-owned", &personal),
    ] {
        let workspace = CloudWorkspace {
            workspace_id: id.into(),
            project_id: "project-a".into(),
            host_id: "host".into(),
            node_id: node.node_id.clone(),
            label: id.into(),
            sharing: "read_only".into(),
            revision: 1,
        };
        writes.push(EntityMutation {
            key: EntityKey {
                kind: EntityKind::Workspace,
                project_id: "project-a".into(),
                parent_id: None,
                id: id.into(),
            },
            expected_revision: 0,
            value: workspace.into(),
            actor: "fixture".into(),
            operation: "fixture.workspace".into(),
        });
    }
    repo.storage()
        .commit(CloudTransaction {
            entities: writes,
            ..Default::default()
        })
        .await
        .unwrap();
    let first = repo
        .admin_user_workspaces(&admin, &alice, "alice", None, 1)
        .await
        .unwrap();
    assert_eq!(first.items[0].workspace_id, "c-owned");
    let second = repo
        .admin_user_workspaces(&admin, &alice, "alice", first.next_cursor.as_deref(), 1)
        .await
        .unwrap();
    assert_eq!(second.items[0].workspace_id, "d-owned");
    let restricted = repo
        .admin_user_workspaces(&admin, &alice, "bob", None, 1)
        .await
        .unwrap();
    assert!(restricted.items.is_empty());
    assert_eq!(
        restricted.visibility,
        UserResourceVisibility::PersonalResourcesPrivate
    );
    assert_eq!(
        repo.admin_user_workspaces(
            &admin,
            &caller("project-b", &[CloudPermission::Read]),
            "alice",
            None,
            1
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );
}
