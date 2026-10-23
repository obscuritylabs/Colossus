//! Metadata reads share history authority without returning retained content.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use colossus_cloud::{CloudProject, ProjectMembership, ProjectRole};
use tower::ServiceExt;

async fn read(router: Router, path: &str, headers: HeaderMap) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(path);
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn thread_metadata_http_requires_read_and_exact_personal_project_visibility() {
    use super::super::local::{PASSWORD, fixture_local, local_cookie};
    let (auth, issuer_server) = fixture_local().await;
    let admin = local_cookie(&auth, "administrator", PASSWORD).await;
    let owner = local_cookie(&auth, "operator", PASSWORD).await;
    let administrator = auth.caller(&admin, "project-a", false).await.unwrap();
    let repo = auth.repository().unwrap();
    let membership = repo
        .save_membership(&administrator, "local-user", ProjectRole::Operator, 0)
        .await
        .unwrap();
    let caller = auth.caller(&owner, "project-a", false).await.unwrap();
    let node = repo
        .register_hosted_target(
            &administrator,
            "local-user",
            BTreeSet::from(["primary".into()]),
        )
        .await
        .unwrap();
    let request = serde_json::from_value(json!({
        "plugin_skill_ids":[],"input":[{"text":"Private opening message"}],
        "session_id":null,"end_user_id":null,"role":"primary","mode":"execute",
        "research_depth":null,"research_sources":[],"plan_action":null,
        "branch":null,"max_turns":4,"idempotency_key":"metadata-fixture"
    }))
    .unwrap();
    let (thread, _) = repo
        .create_personal_thread(&caller, &node.node_id, Some("Pinned chat".into()), request)
        .await
        .unwrap();
    let membership = repo
        .save_membership(
            &administrator,
            "local-user",
            ProjectRole::Viewer,
            membership.revision,
        )
        .await
        .unwrap();
    let read_only = auth.caller(&owner, "project-a", false).await.unwrap();
    assert!(read_only.require(CloudPermission::Read).is_ok());
    assert!(read_only.require(CloudPermission::Execute).is_err());
    let admin_user = auth.admin(&admin, false).await.unwrap();
    repo.save_project(
        &admin_user,
        CloudProject {
            id: "project-b".into(),
            name: "Other project".into(),
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
    let root = tempfile::tempdir().unwrap();
    let mut config = auth.config.clone();
    tls_files(root.path(), &mut config);
    let server = crate::server::CloudServer::with_store(config, auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(server.state.clone());
    let path = format!(
        "/api/projects/project-a/threads/{}/metadata",
        thread.thread_id
    );
    let mut without_csrf = owner.clone();
    without_csrf.remove("x-colossus-csrf");
    without_csrf.remove("origin");
    let (status, body) = read(router.clone(), &path, without_csrf).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"thread":thread}));
    assert!(body.get("tasks").is_none());
    assert!(body.get("messages").is_none());
    for (path, headers, expected) in [
        (path.clone(), HeaderMap::new(), StatusCode::FORBIDDEN),
        (path.clone(), admin.clone(), StatusCode::NOT_FOUND),
        (
            path.replace("project-a", "project-b"),
            admin.clone(),
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/projects/project-a/threads/missing/metadata".into(),
            owner.clone(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(read(router.clone(), &path, headers).await.0, expected);
    }
    // Legacy explicit ceilings can contain Execute without Read. Authentication
    // accepts the nonempty membership; the metadata service must still require Read.
    let mut no_read: ProjectMembership = membership;
    let revision = no_read.revision;
    no_read.permissions = BTreeSet::from([CloudPermission::Execute]);
    auth.store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: colossus_cloud::membership_key("project-a", "local-user"),
                expected_revision: revision,
                value: no_read.into(),
                actor: "fixture-provisioning".into(),
                operation: "fixture.membership-ceiling".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        read(router.clone(), &path, owner).await.0,
        StatusCode::FORBIDDEN
    );
    issuer_server.abort();
}
