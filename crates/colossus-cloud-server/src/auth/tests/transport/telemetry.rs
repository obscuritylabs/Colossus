//! Instance collector probes require real administration and the applied revision.
use super::*;
use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Method, Request, StatusCode},
};
use tower::ServiceExt;

const CHILD: &str = "COLOSSUS_COLLECTOR_TEST_CHILD";
const WRAPPING: &str = "COLOSSUS_COLLECTOR_TEST_WRAPPING";
const TEST: &str = "auth::tests::transport::telemetry::collector_http_requires_admin_csrf_current_revision_and_applied_exporters";
const PATH: &str = "/api/admin/hosted-settings/telemetry/test";

async fn request(
    router: Router,
    path: &str,
    headers: HeaderMap,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(Method::POST).uri(path);
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn save(router: Router, headers: HeaderMap, value: Value) -> Value {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri("/api/admin/hosted-settings");
    *request.headers_mut().unwrap() = headers;
    let response = router
        .oneshot(
            request
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn collector_http_requires_admin_csrf_current_revision_and_applied_exporters() {
    if std::env::var_os(CHILD).is_none() {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--test-threads=1"])
            .env(CHILD, "1")
            .env(WRAPPING, "d".repeat(64))
            .kill_on_drop(true);
        // The isolated process owns exporter environment before its first thread.
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("OTEL_") {
                command.env_remove(name);
            }
        }
        let output = tokio::time::timeout(Duration::from_secs(90), command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated collector probe failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    use super::super::local::{PASSWORD, fixture_local, local_cookie};
    let (auth, issuer) = fixture_local().await;
    let admin = local_cookie(&auth, "administrator", PASSWORD).await;
    let reader = local_cookie(&auth, "operator", PASSWORD).await;
    let root = tempfile::tempdir().unwrap();
    let mut config = auth.config.clone();
    tls_files(root.path(), &mut config);
    config.chat = Some(crate::config::ChatConfig {
        home: root.path().join("hosted"),
        wrapping_key_variable: WRAPPING.into(),
        shared: None,
        max_active_runtimes: 1,
        idle_seconds: 300,
        projects: Default::default(),
    });
    let server = crate::server::CloudServer::with_store(config.clone(), auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(server.state.clone());
    for headers in [HeaderMap::new(), reader, {
        let mut missing = admin.clone();
        missing.remove("x-colossus-csrf");
        missing
    }] {
        assert_eq!(
            request(router.clone(), PATH, headers, json!({"revision":0}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        request(router.clone(), PATH, admin.clone(), json!({"revision":1}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(router.clone(), PATH, admin.clone(), json!({"revision":0}))
            .await
            .0,
        StatusCode::CONFLICT
    );

    let exports = Arc::new(AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(Vec::new()));
    type CollectorState = (Arc<AtomicUsize>, Arc<Mutex<Vec<u8>>>);
    let state = (exports.clone(), captured.clone());
    let collector = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", collector.local_addr().unwrap());
    let collector = tokio::spawn(async move {
        axum::serve(
            collector,
            Router::new()
                .route(
                    "/v1/logs",
                    axum::routing::post(
                        |axum::extract::State((exports, captured)): axum::extract::State<
                            CollectorState,
                        >,
                         bytes: Bytes| async move {
                            exports.fetch_add(1, Ordering::SeqCst);
                            *captured.lock().unwrap() = bytes.to_vec();
                            StatusCode::OK
                        },
                    ),
                )
                .with_state(state),
        )
        .await
        .unwrap();
    });
    let mut catalog = crate::global_settings::types::HostedCatalog::default();
    catalog.observability.enabled = true;
    catalog.observability.logs.otlp = true;
    catalog.observability.otlp.endpoint = Some(endpoint);
    catalog.observability.otlp.protocol = colossus_observability::OtlpProtocol::HttpProtobuf;
    catalog.observability.otlp.timeout_ms = 1_000;
    let saved = save(
        router.clone(),
        admin.clone(),
        serde_json::to_value(&catalog).unwrap(),
    )
    .await;
    assert_eq!(saved["revision"], 1);
    assert_eq!(saved["telemetry"]["state"], "restart_required");
    assert_eq!(
        request(router.clone(), PATH, admin.clone(), json!({"revision":1}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(exports.load(Ordering::SeqCst), 0);
    drop(router);
    drop(server);

    let server = crate::server::CloudServer::with_store(config, auth.store.clone())
        .await
        .unwrap();
    let router = crate::http::router(server.state.clone());
    let (diagnostic, permit) = server.state.telemetry.diagnostic().unwrap();
    assert_eq!(
        request(router.clone(), PATH, admin.clone(), json!({"revision":1}))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    drop((diagnostic, permit));
    let (status, result) =
        request(router.clone(), PATH, admin.clone(), json!({"revision":1})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["status"], "passed");
    assert_eq!(result["revision"], 1);
    assert!(exports.load(Ordering::SeqCst) > 0);
    assert!(
        captured
            .lock()
            .unwrap()
            .windows(b"colossus.observability.diagnostic".len())
            .any(|window| window == b"colossus.observability.diagnostic")
    );
    assert_eq!(result.as_object().unwrap().len(), 3);
    assert!(!result.to_string().contains("127.0.0.1"));
    assert_eq!(
        request(router, PATH, admin, json!({"revision":0})).await.0,
        StatusCode::CONFLICT
    );
    let telemetry = server.state.telemetry.clone();
    tokio::task::spawn_blocking(move || telemetry.shutdown())
        .await
        .unwrap();
    collector.abort();
    issuer.abort();
}
