use crate::{ControlPlaneClient, Operation, StreamEvent, WatchScope, stream_frames::Frames};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Default)]
struct Fixture {
    cursors: Mutex<Vec<String>>,
}
async fn handle(State(state): State<Arc<Fixture>>, request: Request) -> Response {
    match request.uri().path() {
        "/api/auth/config" => Json(json!({"local_enabled":true})).into_response(),
        "/auth/local" => {
            let mut response = Json(json!({})).into_response();
            response.headers_mut().insert(
                "set-cookie",
                HeaderValue::from_static("colossus_session=native-only; HttpOnly"),
            );
            response
        }
        "/api/me" => {
            assert_eq!(
                request.headers().get("cookie").unwrap(),
                "colossus_session=native-only"
            );
            Json(json!({"user":{"id":"human"}})).into_response()
        }
        path if path.ends_with("/events") => {
            assert_eq!(
                request.headers().get("cookie").unwrap(),
                "colossus_session=native-only"
            );
            assert_eq!(
                request.headers().get("accept").unwrap(),
                "text/event-stream"
            );
            let cursor = request
                .headers()
                .get("last-event-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            state.cursors.lock().unwrap().push(cursor.clone());
            if path == "/api/projects/project/tasks/missing/events" {
                return StatusCode::NOT_FOUND.into_response();
            }
            if cursor == "0" {
                ([("content-type", "text/event-stream")], frame(1)).into_response()
            } else {
                let body = axum::body::Body::from_stream(futures::stream::pending::<
                    Result<String, std::io::Error>,
                >());
                ([("content-type", "text/event-stream")], body).into_response()
            }
        }
        _ => Json(json!({})).into_response(),
    }
}
fn frame(id: u64) -> String {
    format!(
        "id: {id}\r\nevent: run_update\r\ndata: {{\"run_id\":\"run\",\"sequence\":{id},\"created_at\":\"now\",\"update\":{{\"output_delta\":\"hello\"}}}}\r\n\r\n"
    )
}
async fn fixture() -> (
    ControlPlaneClient,
    Arc<Fixture>,
    tokio::task::JoinHandle<()>,
    String,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Fixture::default());
    let router = Router::new()
        .fallback(any(handle))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = ControlPlaneClient::default();
    client.connect(&origin).await.unwrap();
    client
        .sign_in("human".into(), "password".into())
        .await
        .unwrap();
    (client, state, server, origin)
}
fn scope() -> WatchScope {
    WatchScope {
        project: "project".into(),
        task: Some("task".into()),
        thread: None,
        after: None,
    }
}
#[test]
fn frames_handle_fragmented_crlf_replay_and_reject_unbounded_or_invalid_updates() {
    let mut frames = Frames::default();
    let mut cursor = 0;
    let wire = frame(1);
    let mut events = Vec::new();
    for byte in wire.as_bytes() {
        events.extend(frames.push(&[*byte], &mut cursor).unwrap());
    }
    assert_eq!(events.len(), 1);
    assert_eq!(cursor, 1);
    assert!(
        frames
            .push(wire.as_bytes(), &mut cursor)
            .unwrap()
            .is_empty()
    );
    assert!(
        frames
            .push(b"id: 2\nevent: run_update\ndata: {}\n\n", &mut cursor)
            .is_err()
    );
    assert!(
        Frames::default()
            .push(&vec![b'x'; 2 * 1024 * 1024 + 1], &mut cursor)
            .is_err()
    );
}
#[tokio::test]
async fn native_watch_replays_exclusively_and_retains_unary_headroom() {
    let (client, state, server, _) = fixture().await;
    let (sender, mut events) = mpsc::unbounded_channel();
    let id = client
        .watch(scope(), move |event| sender.send(event).is_ok())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !matches!(events.recv().await, Some(StreamEvent::RunUpdate { cursor, .. }) if cursor == "1") {}
        loop {
            if state.cursors.lock().unwrap().len() >= 2 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(*state.cursors.lock().unwrap(), ["0", "1"]);
    tokio::time::timeout(Duration::from_secs(1), client.request(Operation::Me))
        .await
        .unwrap()
        .unwrap();
    client.unwatch(id).await;
    let before = state.cursors.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state.cursors.lock().unwrap().len(), before);
    server.abort();
}
#[tokio::test]
async fn missing_stream_stops_once_without_expiring_session_or_holding_watch_slot() {
    let (client, state, server, _) = fixture().await;
    let (sender, mut events) = mpsc::unbounded_channel();
    let mut missing = scope();
    missing.task = Some("missing".into());
    client
        .watch(missing, move |event| sender.send(event).is_ok())
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        StreamEvent::Error {
            code: "not_found",
            ..
        }
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.inner.selected.lock().await.watches.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        client.request(Operation::Me).await.unwrap()["user"]["id"],
        "human"
    );
    assert_eq!(*state.cursors.lock().unwrap(), ["0"]);
    server.abort();
}

#[tokio::test]
async fn logout_and_server_replacement_fence_watch_delivery_and_release_admission() {
    let (client, _, server, origin) = fixture().await;
    let (sender, mut events) = mpsc::unbounded_channel();
    for _ in 0..4 {
        let sender = sender.clone();
        client
            .watch(scope(), move |event| sender.send(event).is_ok())
            .await
            .unwrap();
    }
    assert_eq!(
        client.watch(scope(), |_| true).await.unwrap_err().code,
        "busy"
    );
    tokio::time::timeout(Duration::from_secs(1), client.request(Operation::Me))
        .await
        .unwrap()
        .unwrap();
    client.sign_out().await.unwrap();
    while events.try_recv().is_ok() {}
    assert!(
        tokio::time::timeout(Duration::from_millis(350), events.recv())
            .await
            .is_err()
    );
    client.connect(&origin).await.unwrap();
    client
        .sign_in("human".into(), "password".into())
        .await
        .unwrap();
    let id = client.watch(scope(), |_| true).await.unwrap();
    client.connect(&origin).await.unwrap();
    assert!(client.inner.selected.lock().await.watches.is_empty());
    client.unwatch(id).await;
    server.abort();
}
#[test]
fn watch_routes_accept_one_exact_scope_and_numeric_cursors_only() {
    let origin = url::Url::parse("https://example.test").unwrap();
    let mut scope = scope();
    assert_eq!(
        scope.route(&origin).unwrap().0.path(),
        "/api/projects/project/tasks/task/events"
    );
    scope.thread = Some("thread".into());
    assert!(scope.route(&origin).is_err());
    scope.task = None;
    scope.after = Some("1&other=2".into());
    assert!(scope.route(&origin).is_err());
    scope.after = Some("42".into());
    assert_eq!(scope.route(&origin).unwrap().1, 42);
}
