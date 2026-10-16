use crate::{ControlPlaneClient, Operation};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Fixture {
    expired: AtomicBool,
    effects: AtomicUsize,
    followed: AtomicUsize,
}
async fn handle(State(state): State<Arc<Fixture>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/api/auth/config" {
        return Json(json!({"local_enabled":true,"oidc":null})).into_response();
    }
    if path == "/auth/local" {
        assert_eq!(request.headers().get("x-colossus-csrf").unwrap(), "1");
        assert!(
            request
                .headers()
                .get("origin")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        let mut response = Json(json!({"ok":true})).into_response();
        response.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_static("colossus_session=fixture-session; Path=/; HttpOnly"),
        );
        return response;
    }
    if path == "/followed" {
        state.followed.fetch_add(1, Ordering::SeqCst);
        return Json(json!({})).into_response();
    }
    assert_eq!(
        request.headers().get("cookie").unwrap(),
        "colossus_session=fixture-session"
    );
    if path == "/api/me" {
        return if state.expired.load(Ordering::SeqCst) {
            StatusCode::FORBIDDEN.into_response()
        } else {
            Json(json!({"user":{"id":"human"},"memberships":[],"projects":[]})).into_response()
        };
    }
    if path.contains("/expire/") {
        state.expired.store(true, Ordering::SeqCst);
        return StatusCode::FORBIDDEN.into_response();
    }
    if path.contains("/denied/") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if path.contains("/redirect/") {
        return (StatusCode::FOUND, [("location", "/followed")]).into_response();
    }
    if path.contains("/large/") {
        return "x".repeat(2 * 1024 * 1024 + 1).into_response();
    }
    if request.method() == "POST" {
        assert_eq!(request.headers().get("x-colossus-csrf").unwrap(), "1");
        state.effects.fetch_add(1, Ordering::SeqCst);
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    Json(json!({"nodes":[]})).into_response()
}
async fn fixture() -> (
    ControlPlaneClient,
    Arc<Fixture>,
    tokio::task::JoinHandle<()>,
) {
    let state = Arc::new(Fixture::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .fallback(any(handle))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = ControlPlaneClient::default();
    client.connect(&origin).await.unwrap();
    client
        .sign_in("human".into(), "fixture-password".into())
        .await
        .unwrap();
    (client, state, server)
}

#[tokio::test]
async fn project_denial_preserves_identity_but_expiry_drops_local_custody() {
    let (client, _, server) = fixture().await;
    let denied = client
        .request(Operation::Fleet {
            project: "denied".into(),
            after: None,
        })
        .await
        .unwrap_err();
    assert_eq!(denied.code, "forbidden");
    assert!(client.request(Operation::Me).await.is_ok());
    let expired = client
        .request(Operation::Fleet {
            project: "expire".into(),
            after: None,
        })
        .await
        .unwrap_err();
    assert_eq!(expired.code, "session_expired");
    assert_eq!(
        client.request(Operation::Me).await.unwrap_err().code,
        "session_expired"
    );
    server.abort();
}

#[tokio::test]
async fn uncertain_effects_are_not_retried_and_redirects_are_not_followed() {
    let (client, state, server) = fixture().await;
    let error = client
        .request(Operation::CreateThread {
            project: "engineering".into(),
            node: "node".into(),
            request: json!({"idempotency_key":"original"}),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "outcome_unknown");
    assert_eq!(state.effects.load(Ordering::SeqCst), 1);
    assert!(
        client
            .request(Operation::Fleet {
                project: "redirect".into(),
                after: None
            })
            .await
            .is_err()
    );
    assert_eq!(state.followed.load(Ordering::SeqCst), 0);
    assert!(
        client
            .request(Operation::Fleet {
                project: "large".into(),
                after: None
            })
            .await
            .is_err()
    );
    server.abort();
}

#[tokio::test]
async fn changing_servers_drops_the_old_session_and_browser_links_carry_no_secret() {
    let (client, _, server) = fixture().await;
    let url = client
        .browser_url(Some("engineering"), Some("task-1"))
        .await
        .unwrap();
    assert!(url.ends_with("/projects/engineering/tasks/task-1"));
    assert!(!url.contains("fixture-session"));
    assert!(client.browser_url(Some(".."), None).await.is_err());
    let origin = client.browser_url(None, None).await.unwrap();
    client.connect(&origin).await.unwrap();
    assert_eq!(
        client.request(Operation::Me).await.unwrap_err().code,
        "session_expired"
    );
    server.abort();
}

#[tokio::test]
async fn logout_discards_the_session_when_remote_revocation_is_unavailable() {
    let (client, _, server) = fixture().await;
    server.abort();
    let _ = server.await;
    client.sign_out().await.unwrap();
    assert_eq!(
        client.request(Operation::Me).await.unwrap_err().code,
        "disconnected"
    );
    assert!(client.browser_url(None, None).await.is_err());
}
