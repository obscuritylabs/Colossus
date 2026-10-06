use super::*;
use colossus_cloud::{
    CloudCaller, CloudPermission, CloudProject, CloudRepository, CloudUser, OidcIdentity,
    ProjectRole, UserAccount,
};
use std::sync::Arc;
fn account(id: &str, admin: bool) -> UserAccount {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    UserAccount {
        user: CloudUser {
            id: id.into(),
            display_name: format!("Fixture {id}"),
            email: None,
            active: true,
            is_admin: admin,
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
            identities: Vec::new(),
        },
        security_epoch: 1,
    }
}
fn project(id: &str, parent: Option<&str>) -> CloudProject {
    CloudProject {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        parent_project_id: parent.map(str::to_owned),
        archived: false,
        revision: 0,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
#[tokio::test]
#[ignore = "requires an explicit isolated local PostgreSQL fixture"]
async fn postgres_identity_membership_hierarchy_and_administrator_invariants() {
    let config = config();
    let store = Arc::new(
        CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
            .await
            .unwrap(),
    );
    let repo = CloudRepository::new(store.clone()).unwrap();
    for (id, is_admin) in [("administrator", true), ("operator", false)] {
        repo.create_account(
            "fixture",
            account(id, is_admin),
            Some(OidcIdentity {
                user_id: id.into(),
                issuer: "https://issuer.example.test".into(),
                subject: id.into(),
            }),
            None,
        )
        .await
        .unwrap();
    }
    let admin = repo.account("administrator").await.unwrap().user;
    let first = repo
        .save_project(&admin, project("parent-project", None))
        .await
        .unwrap();
    let second = repo
        .save_project(&admin, project("child-project", Some("parent-project")))
        .await
        .unwrap();
    let mut cycle = first.clone();
    cycle.parent_project_id = Some(second.id.clone());
    assert_eq!(
        repo.save_project(&admin, cycle).await.unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        repo.project("parent-project")
            .await
            .unwrap()
            .parent_project_id,
        None
    );
    let caller = CloudCaller::new(
        admin.id.clone(),
        first.id.clone(),
        [CloudPermission::Read, CloudPermission::Administer].into(),
    )
    .unwrap();
    let role = repo
        .save_membership(&caller, "operator", ProjectRole::Operator, 0)
        .await
        .unwrap();
    assert_eq!(repo.user_memberships("operator").await.unwrap().len(), 1);
    assert_eq!(
        repo.project_members(&caller, None, 100).await.unwrap()[0].user_id,
        "operator"
    );
    let names = store.user_accounts(&["operator".into()]).await.unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(
        names[0].value.pointer("/user/display_name"),
        Some(&json!("Fixture operator"))
    );
    repo.remove_membership(&caller, "operator", role.revision)
        .await
        .unwrap();
    assert!(
        repo.project_members(&caller, None, 100)
            .await
            .unwrap()
            .is_empty()
    );
    let restored = repo
        .save_membership(&caller, "operator", ProjectRole::ProjectAdmin, 0)
        .await
        .unwrap();
    assert!(restored.permissions.contains(&CloudPermission::Execute));
    assert!(!restored.permissions.contains(&CloudPermission::Approve));
    let mut disabled = admin.clone();
    disabled.active = false;
    assert_eq!(
        repo.update_account(&admin, disabled).await.unwrap_err(),
        CloudError::Conflict
    );
    assert!(repo.account("administrator").await.unwrap().user.active);
    let mut demoted = admin.clone();
    demoted.is_admin = false;
    assert_eq!(
        repo.update_account(&admin, demoted).await.unwrap_err(),
        CloudError::Conflict
    );
    // A concurrent/differently configured bootstrap cannot create a second administrator after the winner.
    assert_eq!(
        repo.create_account(
            "operator-bootstrap",
            account("late-bootstrap", true),
            Some(OidcIdentity {
                user_id: "late-bootstrap".into(),
                issuer: "https://issuer.example.test".into(),
                subject: "late-bootstrap".into()
            }),
            None
        )
        .await
        .unwrap_err(),
        CloudError::Conflict
    );
    assert_eq!(
        repo.account("late-bootstrap").await.unwrap_err(),
        CloudError::NotFound
    );
    let projects = repo.projects_page(&admin, None, 100).await.unwrap();
    assert_eq!(projects.len(), 2);
    let mut child = second;
    child.archived = true;
    repo.save_project(&admin, child).await.unwrap();
    let reopened = CloudPostgresStore::open(config.clone(), &AdditionalRootCertificates::default())
        .await
        .unwrap();
    assert!(
        CloudRepository::new(Arc::new(reopened))
            .unwrap()
            .project("child-project")
            .await
            .unwrap()
            .archived
    );
    store.remove_fixture_schema(&config.schema).await.unwrap();
}
