//! Catalog discovery does not turn Cloud project membership into Hosted execution authority.
use super::*;

async fn status(
    router: &Router,
    headers: &HeaderMap,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> StatusCode {
    let mut request = Request::builder().method(method).uri(path);
    *request.headers_mut().unwrap() = headers.clone();
    let request = if let Some(body) = body {
        request
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
    } else {
        request.body(Body::empty())
    };
    router
        .clone()
        .oneshot(request.unwrap())
        .await
        .unwrap()
        .status()
}

pub(super) async fn exercise(
    router: &Router,
    admin: &HeaderMap,
    user: &HeaderMap,
    state: &Arc<crate::server::State>,
) {
    let settings = read(
        router.clone(),
        admin.clone(),
        "/api/projects/without-hosted/chat/settings",
    )
    .await;
    assert_eq!(settings["execution"]["enabled"], false);
    assert_eq!(settings["execution"]["settings_editable"], false);
    assert!(
        settings["execution"]["reason"]
            .as_str()
            .unwrap()
            .contains("deployment operator")
    );
    assert_eq!(settings["variables"]["REGION"], "region-a");
    assert_eq!(settings["inheritance"]["global_revision"], 1);
    assert_eq!(
        settings["inheritance"]["provenance"]["providers"]["inherited"],
        "global"
    );
    assert_eq!(settings["inheritance"]["overrides"]["providers"], json!([]));
    assert!(settings.get("sandbox").is_none());
    assert!(settings.get("storage").is_none());
    assert_eq!(
        settings["credential_status"],
        json!({"providers":{},"mcp":{},"search":{}})
    );

    let presets = read(
        router.clone(),
        admin.clone(),
        "/api/projects/without-hosted/chat/settings/provider-presets",
    )
    .await;
    assert!(
        presets["availability"]
            .as_object()
            .unwrap()
            .values()
            .all(|entry| entry["status"] == "deployment_required")
    );
    for scope in ["project", "personal"] {
        let snapshot = read(
            router.clone(),
            admin.clone(),
            &format!("/api/projects/without-hosted/chat/credential-store/{scope}"),
        )
        .await;
        assert_eq!(snapshot["revision"], 0);
        assert_eq!(snapshot["credentials"], json!([]));
        assert_eq!(snapshot["connections"], json!([]));
        assert_eq!(snapshot["execution"]["settings_editable"], false);
    }
    let variables = read(
        router.clone(),
        admin.clone(),
        "/api/projects/without-hosted/chat/variables/personal",
    )
    .await;
    assert_eq!(variables["revision"], 0);
    assert_eq!(variables["variables"], json!({}));
    assert_eq!(variables["execution"]["settings_editable"], false);
    let accounts = read(
        router.clone(),
        admin.clone(),
        "/api/projects/without-hosted/chat/accounts",
    )
    .await;
    assert_eq!(accounts["accounts"], json!([]));
    assert_eq!(accounts["execution"]["enabled"], false);

    assert_eq!(
        status(
            router,
            admin,
            Method::GET,
            "/api/projects/missing/chat/settings",
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(
            router,
            user,
            Method::GET,
            "/api/projects/without-hosted/chat/settings",
            None
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            router,
            user,
            Method::GET,
            "/api/projects/without-hosted/chat/credential-store/personal",
            None
        )
        .await,
        StatusCode::FORBIDDEN
    );

    // Give the user a genuine execution role; it still cannot provision a runtime.
    let administrator = state
        .auth
        .caller(admin, "without-hosted", false)
        .await
        .unwrap();
    state
        .repo
        .save_membership(&administrator, "local-user", ProjectRole::Operator, 0)
        .await
        .unwrap();
    let owner = state
        .auth
        .caller(user, "without-hosted", true)
        .await
        .unwrap();
    let host = state.hosted.as_ref().unwrap();
    let node = host.node_id(&owner).unwrap().unwrap();
    assert!(matches!(
        host.require_live_submission(&owner, &node).await,
        Err(colossus_cloud::CloudError::Conflict)
    ));
    assert!(matches!(
        host.ensure(state, &owner).await,
        Err(colossus_cloud::CloudError::NotFound)
    ));
    assert!(
        host.project_config("without-hosted")
            .await
            .unwrap()
            .is_none()
    );
    assert!(!host.deployment.projects.contains_key("without-hosted"));
    assert!(!host.projects.read().await.contains_key("without-hosted"));
    assert!(host.live.lock().await.is_empty());

    let mut attempted_settings = settings;
    attempted_settings
        .as_object_mut()
        .unwrap()
        .remove("credential_status");
    assert_eq!(
        status(
            router,
            admin,
            Method::PUT,
            "/api/projects/without-hosted/chat/settings",
            Some(attempted_settings)
        )
        .await,
        StatusCode::NOT_FOUND
    );
    for (headers, scope) in [(admin, "project"), (user, "personal")] {
        assert_eq!(status(router, headers, Method::POST, &format!("/api/projects/without-hosted/chat/credential-store/{scope}"), Some(json!({"revision":0,"label":"Unprovisioned","kind":"api_key","secret":"must-not-be-stored"}))).await, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        status(
            router,
            user,
            Method::PUT,
            "/api/projects/without-hosted/chat/variables/personal",
            Some(json!({"revision":0,"variables":{"REGION":"must-not-be-stored"}}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert!(host.live.lock().await.is_empty());
}
