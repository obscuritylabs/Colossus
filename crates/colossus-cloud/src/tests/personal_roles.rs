//! Hosted role metadata reconciliation never widens another enrollment's authority.
use super::*;

#[tokio::test]
async fn hosted_role_reconciliation_requires_administration_exact_owner_and_revision() {
    let (repo, admin, shared) = fixture().await;
    let provisioner = CloudCaller::new(
        "chat-provisioner".into(),
        admin.project_id().into(),
        [CloudPermission::Administer].into(),
    )
    .unwrap();
    let operator = caller(admin.project_id(), &[CloudPermission::Execute]);
    let mut personal = shared.clone();
    personal.node_id = "chat-alice".into();
    personal.owner_user_id = Some("alice".into());
    personal.revision = 0;
    let personal = repo.register_node(&admin, personal).await.unwrap();
    let roles = BTreeSet::from(["primary".into(), "research".into()]);

    for (caller, node, owner, revision, expected) in [
        (
            &operator,
            &personal.node_id,
            "alice",
            1,
            CloudError::PermissionDenied,
        ),
        (
            &provisioner,
            &shared.node_id,
            "alice",
            1,
            CloudError::NotFound,
        ),
        (
            &provisioner,
            &personal.node_id,
            "bob",
            1,
            CloudError::NotFound,
        ),
        (
            &provisioner,
            &personal.node_id,
            "alice",
            0,
            CloudError::Conflict,
        ),
    ] {
        assert_eq!(
            repo.update_personal_node_roles(caller, node, owner, revision, roles.clone())
                .await
                .unwrap_err(),
            expected
        );
    }
    for rejected in [BTreeSet::new(), BTreeSet::from(["invalid role".into()])] {
        assert_eq!(
            repo.update_personal_node_roles(&provisioner, &personal.node_id, "alice", 1, rejected)
                .await
                .unwrap_err(),
            CloudError::InvalidArgument
        );
    }
    let foreign_project = CloudCaller::new(
        "chat-provisioner".into(),
        "other-project".into(),
        [CloudPermission::Administer].into(),
    )
    .unwrap();
    assert_eq!(
        repo.update_personal_node_roles(
            &foreign_project,
            &personal.node_id,
            "alice",
            1,
            roles.clone()
        )
        .await
        .unwrap_err(),
        CloudError::NotFound
    );

    let updated = repo
        .update_personal_node_roles(&provisioner, &personal.node_id, "alice", 1, roles.clone())
        .await
        .unwrap();
    let mut expected = personal.clone();
    expected.roles = roles.clone();
    expected.revision = 2;
    assert_eq!(updated, expected);
    assert_eq!(
        repo.update_personal_node_roles(&provisioner, &personal.node_id, "alice", 2, roles.clone())
            .await
            .unwrap(),
        updated
    );
    assert_eq!(
        repo.update_personal_node_roles(&provisioner, &personal.node_id, "alice", 1, roles.clone())
            .await
            .unwrap_err(),
        CloudError::Conflict
    );
    let revoked = repo
        .revoke_node(&admin, &personal.node_id, 2)
        .await
        .unwrap();
    assert_eq!(
        repo.update_personal_node_roles(
            &provisioner,
            &personal.node_id,
            "alice",
            revoked.revision,
            roles
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        repo.get_node(&admin, &shared.node_id).await.unwrap(),
        shared
    );
}
