//! Account inventory HTTP routes never substitute the selected identity for the viewer.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use colossus_cloud::ProjectRole;
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
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn account_inventory_http_requires_admin_and_preserves_personal_visibility() {
    use super::super::local::{PASSWORD, fixture_local, local_cookie};
    let (auth, issuer) = fixture_local().await;
    let admin = local_cookie(&auth, "administrator", PASSWORD).await;
    let operator = local_cookie(&auth, "operator", PASSWORD).await;
    let repo = auth.repository().unwrap();
    let actor = auth.caller(&admin, "project-a", false).await.unwrap();
    for user in ["local-admin", "local-user"] {
        repo.save_membership(&actor, user, ProjectRole::ProjectAdmin, 0)
            .await
            .unwrap();
    }
    // Refresh the caller after granting membership; the previous authority
    // snapshot deliberately did not contain Execute or Control permission.
    let actor = auth.caller(&admin, "project-a", false).await.unwrap();
    let node = repo
        .register_hosted_target(&actor, "local-admin", BTreeSet::from(["primary".into()]))
        .await
        .unwrap();
    let request = serde_json::from_value(json!({
        "plugin_skill_ids":[],"input":[{"text":"Private retained content must not be returned"}],
        "session_id":null,"end_user_id":null,"role":"primary","mode":"execute",
        "research_depth":null,"research_sources":[],"plan_action":null,
        "branch":null,"max_turns":4,"idempotency_key":"inventory-self"
    }))
    .unwrap();
    let (thread, _) = repo
        .create_personal_thread(
            &actor,
            &node.node_id,
            Some("Own inventory chat".into()),
            request,
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
    let project_path = "/api/admin/users/local-user/projects";
    let (status, body) = read(router.clone(), project_path, admin.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["projects"][0]["project"]["id"], "project-a");
    assert_eq!(body["projects"][0]["membership"]["user_id"], "local-user");
    for kind in ["workspaces", "chats"] {
        let path = format!("/api/admin/users/local-user/projects/project-a/{kind}");
        let (status, body) = read(router.clone(), &path, admin.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[kind], json!([]));
        assert_eq!(body["visibility"], "personal_resources_private");
        assert_eq!(body["next_cursor"], Value::Null);
        for headers in [operator.clone(), HeaderMap::new()] {
            assert_eq!(
                read(router.clone(), &path, headers).await.0,
                StatusCode::FORBIDDEN
            );
        }
    }
    let path = "/api/admin/users/local-admin/projects/project-a/chats?limit=1";
    let (status, body) = read(router.clone(), path, admin.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["visibility"], "visible");
    assert_eq!(body["chats"][0]["thread_id"], thread.thread_id);
    assert!(body.get("messages").is_none());
    assert!(body.get("tasks").is_none());
    assert!(!body.to_string().contains("Private retained content"));
    for path in [
        "/api/admin/users/local-admin/projects?limit=0",
        "/api/admin/users/local-admin/projects/project-a/chats?limit=101",
    ] {
        assert_eq!(
            read(router.clone(), path, admin.clone()).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        read(router.clone(), project_path, operator).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        read(router, "/api/admin/users/missing/projects", admin)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    issuer.abort();
}
