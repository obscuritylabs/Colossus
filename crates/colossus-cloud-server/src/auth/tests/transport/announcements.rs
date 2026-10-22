//! Actual HTTP authentication, CSRF, publication and account-owned receipts.
use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

async fn request(
    router: Router,
    method: Method,
    path: &str,
    headers: HeaderMap,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    *builder.headers_mut().unwrap() = headers;
    let body = if let Some(body) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&body).unwrap())
    } else {
        Body::empty()
    };
    let response = router.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn announcements_http_enforces_admin_csrf_and_account_bound_durable_receipts() {
    use super::super::local::{PASSWORD, fixture_local, local_cookie};
    let (auth, issuer_server) = fixture_local().await;
    let admin = local_cookie(&auth, "administrator", PASSWORD).await;
    let reader = local_cookie(&auth, "operator", PASSWORD).await;
    let root = tempfile::tempdir().unwrap();
    let mut config = auth.config.clone();
    tls_files(root.path(), &mut config);
    let server = crate::server::CloudServer::with_store(config.clone(), auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(server.state.clone());
    let id = uuid::Uuid::now_v7().to_string();
    let input = json!({
        "id":id,"title":"Shared features","summary":"New notifications",
        "body":"## A feature\n\n**Safe Markdown** and [docs](https://example.test).",
        "tone":"info","banner":true,"priority":50,"expires_at":null,"action":"draft"
    });
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/announcements",
            HeaderMap::new(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            router.clone(),
            Method::GET,
            "/api/admin/announcements",
            reader.clone(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/admin/announcements",
            reader.clone(),
            Some(input.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut missing_csrf = admin.clone();
    missing_csrf.remove("x-colossus-csrf");
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            "/api/admin/announcements",
            missing_csrf,
            Some(input.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, saved) = request(
        router.clone(),
        Method::POST,
        "/api/admin/announcements",
        admin.clone(),
        Some(input.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["announcement"]["publication_revision"], 0);
    assert!(saved["announcement"].get("creation_sha256").is_none());
    let (_, feed) = request(
        router.clone(),
        Method::GET,
        "/api/announcements",
        reader.clone(),
        None,
    )
    .await;
    assert_eq!(feed["announcements"], json!([]));
    let mut publication = input;
    publication.as_object_mut().unwrap().remove("id");
    publication["revision"] = json!(1);
    publication["action"] = json!("publish");
    let detail_path = format!("/api/admin/announcements/{id}");
    let (status, published) = request(
        router.clone(),
        Method::PATCH,
        &detail_path,
        admin.clone(),
        Some(publication.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(published["announcement"]["publication_revision"], 1);
    // Exact retry reconciles an unknown response. A different stale edit conflicts.
    assert_eq!(
        request(
            router.clone(),
            Method::PATCH,
            &detail_path,
            admin.clone(),
            Some(publication.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    publication["title"] = json!("Stale conflicting edit");
    assert_eq!(
        request(
            router.clone(),
            Method::PATCH,
            &detail_path,
            admin.clone(),
            Some(publication)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, before) = request(
        router.clone(),
        Method::GET,
        "/api/announcements",
        reader.clone(),
        None,
    )
    .await;
    assert_eq!(before["announcements"][0]["read"], false);
    assert_eq!(before["active_banner"]["announcement"]["id"], id);
    let receipt_path = format!("/api/announcements/{id}/receipt");
    let acknowledgement = json!({"publication_revision":1,"dismissed":true});
    let mut cross_origin = reader.clone();
    cross_origin.insert(
        "origin",
        HeaderValue::from_static("https://other.example.test"),
    );
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            &receipt_path,
            cross_origin,
            Some(acknowledgement.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut forged_owner = acknowledgement.clone();
    forged_owner["user_id"] = json!("local-admin");
    assert_eq!(
        request(
            router.clone(),
            Method::POST,
            &receipt_path,
            reader.clone(),
            Some(forged_owner)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, receipt) = request(
        router.clone(),
        Method::POST,
        &receipt_path,
        reader.clone(),
        Some(acknowledgement),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        receipt,
        json!({"publication_revision":1,"read":true,"dismissed":true})
    );
    let restarted = crate::server::CloudServer::with_store(config, auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(restarted.state.clone());
    let (_, reader_feed) = request(
        router.clone(),
        Method::GET,
        "/api/announcements",
        reader.clone(),
        None,
    )
    .await;
    assert_eq!(reader_feed["announcements"][0]["dismissed"], true);
    assert!(reader_feed["active_banner"].is_null());
    let (_, admin_feed) = request(
        router.clone(),
        Method::GET,
        "/api/announcements",
        admin.clone(),
        None,
    )
    .await;
    assert_eq!(admin_feed["announcements"][0]["dismissed"], false);
    assert_eq!(admin_feed["active_banner"]["announcement"]["id"], id);
    let settings = json!({"revision":0,"classification":{
        "enabled":true,"text":"INTERNAL","tone":"neutral","position":"top_and_bottom",
        "colors":{"background":"#008000","text":"#ffffff"}
    }});
    assert_eq!(
        request(
            router.clone(),
            Method::PATCH,
            "/api/admin/settings",
            reader,
            Some(settings.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, saved) = request(
        router.clone(),
        Method::PATCH,
        "/api/admin/settings",
        admin.clone(),
        Some(settings.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["revision"], 1);
    let (_, public) = request(
        router.clone(),
        Method::GET,
        "/api/settings",
        HeaderMap::new(),
        None,
    )
    .await;
    assert_eq!(
        public["classification"]["colors"],
        settings["classification"]["colors"]
    );
    let mut unreadable = settings;
    unreadable["revision"] = json!(1);
    unreadable["classification"]["colors"] = json!({"background":"#ffffff","text":"#eeeeee"});
    assert_eq!(
        request(
            router.clone(),
            Method::PATCH,
            "/api/admin/settings",
            admin,
            Some(unreadable),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (_, unchanged) = request(
        router.clone(),
        Method::GET,
        "/api/settings",
        HeaderMap::new(),
        None,
    )
    .await;
    assert_eq!(unchanged["revision"], 1);
    assert_eq!(
        unchanged["classification"]["colors"]["background"],
        "#008000"
    );
    assert_eq!(
        request(
            router,
            Method::GET,
            "/api/announcements?limit=101",
            HeaderMap::new(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    issuer_server.abort();
}
