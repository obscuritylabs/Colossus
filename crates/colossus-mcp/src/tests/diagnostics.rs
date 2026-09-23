use super::*;
use colossus_contracts::{McpDiagnosticCode, McpDiagnosticFailure, McpDiagnosticStage};

#[test]
fn adapter_snapshot_reports_actual_trust_without_endpoint_or_credentials() {
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let roots =
        AdditionalRootCertificates::from_pem_bundle(certificate.cert.pem().as_bytes()).unwrap();
    let config = McpConfig {
        servers: BTreeMap::from([(
            "fixture".into(),
            remote_server("https://private.example/mcp?secret-query"),
        )]),
        ..Default::default()
    };
    let executor = McpExecutor::new(
        &config,
        Path::new("."),
        "native",
        Arc::new(McpEffectShapeExecutor {
            reference: "host:secret-binding",
        }),
    )
    .unwrap()
    .with_tls_roots(roots);
    let snapshot = executor
        .snapshot_configuration(&config, Path::new("."), "native")
        .unwrap();
    let report = snapshot.diagnostic_configuration("fixture").unwrap();
    assert_eq!(
        report,
        executor.diagnostic_configuration("fixture").unwrap()
    );
    assert_eq!(report.additional_ca_certificates, 1);
    assert_eq!(report.additional_ca_sha256.as_ref().unwrap().len(), 64);
    assert!(report.direct_http);
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains("private.example"));
    assert!(!encoded.contains("secret"));
}
use std::time::Duration;

async fn probe(
    endpoint: String,
    client: reqwest::Client,
    limit: usize,
) -> (bool, McpDiagnosticStage, Option<McpDiagnosticFailure>) {
    let capture = McpDiagnosticCapture::default();
    let result = capture
        .scope(async {
            let http = HardenedStreamableHttpClient::for_test(endpoint.clone(), client, limit);
            let mut server = configured_http_server(endpoint);
            server.allow_stateless = true;
            execute_remote_operation(
                http,
                &server,
                &McpOperation::ListTools {
                    server: "fixture".into(),
                    cursor: None,
                },
                HashMap::new(),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .await
        })
        .await;
    let (stage, failure) = capture.snapshot();
    (result.is_ok(), stage, failure)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

async fn response_server(
    status: &'static str,
    headers: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request(&mut stream).await;
        write_http_response(&mut stream, status, headers, body).await;
    });
    (format!("http://{address}/mcp?private-query"), task)
}

#[tokio::test]
async fn concurrent_checks_preserve_http_status_without_headers_bodies_or_urls() {
    let (first, first_task) = response_server(
        "401 Unauthorized",
        "WWW-Authenticate: Bearer secret-challenge\r\n",
        "secret-body",
    )
    .await;
    let (second, second_task) = response_server("403 Forbidden", "", "other-secret").await;
    let (first, second) = tokio::join!(probe(first, client(), 1024), probe(second, client(), 1024));
    first_task.await.unwrap();
    second_task.await.unwrap();
    for (observed, status) in [(first, 401), (second, 403)] {
        assert!(!observed.0);
        assert_eq!(observed.1, McpDiagnosticStage::Initialize);
        assert_eq!(
            observed.2,
            Some(McpDiagnosticFailure {
                code: McpDiagnosticCode::HttpStatus,
                http_status: Some(status)
            })
        );
        let encoded = serde_json::to_string(&observed.2).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("private-query"));
    }
}

#[tokio::test]
async fn expired_session_preserves_404_during_tool_discovery() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, request) = read_http_request(&mut stream).await;
            match request.as_ref().and_then(|value| value["method"].as_str()) {
                Some("initialize") => {
                    let body = json!({"jsonrpc": "2.0", "id": request.unwrap()["id"], "result": {
                        "protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fixture", "version": "1"}
                    }})
                    .to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\nMcp-Session-Id: private-session\r\n",
                        &body,
                    )
                    .await;
                }
                Some("notifications/initialized") => {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                }
                Some("tools/list") => {
                    write_http_response(&mut stream, "404 Not Found", "", "private-body").await;
                }
                None => {
                    write_http_response(&mut stream, "405 Method Not Allowed", "", "").await;
                }
                other => panic!("unexpected method: {other:?}"),
            }
        }
    });
    let observed = probe(endpoint, client(), 4096).await;
    task.abort();
    assert!(!observed.0);
    assert_eq!(observed.1, McpDiagnosticStage::ListTools);
    assert_eq!(
        observed.2,
        Some(McpDiagnosticFailure {
            code: McpDiagnosticCode::HttpStatus,
            http_status: Some(404)
        })
    );
}

#[tokio::test]
async fn refused_connection_and_request_timeout_have_distinct_categories() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let refused = probe(endpoint, client(), 1024).await;
    assert_eq!(refused.2.unwrap().code, McpDiagnosticCode::Connect);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let waiting = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let timed_out = probe(endpoint, client, 1024).await;
    waiting.abort();
    assert_eq!(timed_out.2.unwrap().code, McpDiagnosticCode::Timeout);
}

#[tokio::test]
async fn oversized_and_malformed_responses_remain_bounded() {
    let (endpoint, task) = response_server(
        "200 OK",
        "Content-Type: application/json\r\n",
        "secret-invalid-json",
    )
    .await;
    let oversized = probe(endpoint, client(), 4).await;
    task.await.unwrap();
    assert_eq!(
        oversized.2.unwrap().code,
        McpDiagnosticCode::ResponseTooLarge
    );
    let (endpoint, task) = response_server(
        "200 OK",
        "Content-Type: application/json\r\n",
        "secret-invalid-json",
    )
    .await;
    let malformed = probe(endpoint, client(), 1024).await;
    task.await.unwrap();
    assert!(!malformed.0);
    assert_eq!(malformed.1, McpDiagnosticStage::Initialize);
    assert!(malformed.2.is_none()); // The runtime assigns the protocol fallback.
}

#[tokio::test]
async fn imported_ca_changes_tls_failure_into_successful_mcp_discovery() {
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let roots =
        AdditionalRootCertificates::from_pem_bundle(certificate.cert.pem().as_bytes()).unwrap();
    // Workspace feature unification can enable both Rustls crypto backends.
    // Keep this fixture independent of process-global provider initialization.
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok(mut stream) = acceptor.accept(stream).await else {
                continue;
            };
            let (_, request) = read_http_request(&mut stream).await;
            let request = request.unwrap();
            let result = match request["method"].as_str().unwrap() {
                "initialize" => {
                    json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}})
                }
                "notifications/initialized" => {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                    continue;
                }
                "tools/list" => json!({"tools": []}),
                method => panic!("unexpected method {method}"),
            };
            let body = json!({"jsonrpc": "2.0", "id": request["id"], "result": result}).to_string();
            write_http_response(
                &mut stream,
                "200 OK",
                "Content-Type: application/json\r\n",
                &body,
            )
            .await;
        }
    });
    let endpoint = format!("https://localhost:{}/mcp", address.port());
    let untrusted = probe(endpoint.clone(), client(), 4096).await;
    assert!(!untrusted.0);
    assert_eq!(untrusted.2.unwrap().code, McpDiagnosticCode::Tls);
    let trusted_client = roots
        .configure_reqwest(reqwest::Client::builder())
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let trusted = probe(endpoint, trusted_client, 4096).await;
    server_task.abort();
    assert!(trusted.0, "trusted discovery: {:?}", trusted.2);
    assert_eq!(trusted.1, McpDiagnosticStage::ListTools);
    assert!(trusted.2.is_none());
}
