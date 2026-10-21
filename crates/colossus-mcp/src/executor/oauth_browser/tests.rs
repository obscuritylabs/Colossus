use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rmcp::transport::auth::{
    AuthorizationMetadata, OAuthHttpClient, OAuthHttpClientFuture, OAuthHttpRequest,
};
use std::sync::Mutex;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const CALLBACK: &str = "https://colossus.example/auth/mcp/callback";
const ISSUER: &str = "https://identity.example";

#[derive(Default)]
struct TokenHttp(Mutex<Vec<BTreeMap<String, String>>>);

impl OAuthHttpClient for TokenHttp {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            assert_eq!(request.request.uri(), "https://identity.example/token");
            assert_eq!(request.request.method(), "POST");
            let form = url::form_urlencoded::parse(request.request.body())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            self.0.lock().unwrap().push(form);
            Ok(http::Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(
                    br#"{"access_token":"private-access-token","refresh_token":"private-refresh-token","token_type":"Bearer","expires_in":3600,"scope":"read"}"#
                        .to_vec(),
                )
                .unwrap())
        })
    }
}

struct NoProcess;

#[async_trait]
impl EffectExecutor for NoProcess {
    async fn execute(
        &self,
        _: &EffectRequest,
        _: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        panic!("browser OAuth must not invoke a process")
    }
}

fn config() -> McpConfig {
    let server = serde_json::from_value(json!({
        "transport": "streamable_http", "url": "https://mcp.example/mcp",
        "allowedTools": ["read"],
        "oauth": {"clientId": "cloud-client", "callbackPort": 8765, "scopes": ["read"]}
    }))
    .unwrap();
    McpConfig {
        servers: BTreeMap::from([("fixture".into(), server)]),
        ..Default::default()
    }
}

fn executor() -> McpExecutor {
    McpExecutor::new(&config(), Path::new("."), "native", Arc::new(NoProcess))
        .unwrap()
        .with_ephemeral_oauth_storage("project-user")
        .unwrap()
        .with_oauth_policy(
            vec!["https://mcp.example".into(), ISSUER.into()],
            Vec::new(),
            30_000,
            1024 * 1024,
        )
}

async fn begin(
    executor: &McpExecutor,
    http: Arc<TokenHttp>,
    flow_id: &str,
    binding: &str,
) -> McpOAuthBrowserLogin {
    begin_with_callback(
        executor,
        http,
        flow_id,
        binding,
        CALLBACK,
        McpOAuthBrowserCallbackPolicy::HttpsOnly,
    )
    .await
}

async fn begin_with_callback(
    executor: &McpExecutor,
    http: Arc<TokenHttp>,
    flow_id: &str,
    binding: &str,
    callback_url: &str,
    policy: McpOAuthBrowserCallbackPolicy,
) -> McpOAuthBrowserLogin {
    executor
        .reserve_browser_flow("fixture", flow_id, binding, callback_url, policy)
        .await
        .unwrap();
    let mut manager =
        AuthorizationManager::new_with_oauth_http_client("https://mcp.example/mcp", http)
            .await
            .unwrap();
    let metadata: AuthorizationMetadata = serde_json::from_value(json!({
        "issuer": ISSUER,
        "authorization_endpoint": format!("{ISSUER}/authorize"),
        "token_endpoint": format!("{ISSUER}/token"),
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true
    }))
    .unwrap();
    manager.set_metadata(metadata);
    manager.set_credential_store(
        executor
            .oauth_credential_store(executor.servers.get("fixture").unwrap())
            .unwrap(),
    );
    manager
        .configure_client(OAuthClientConfig::new("cloud-client", callback_url))
        .unwrap();
    executor
        .activate_browser_flow("fixture", flow_id, binding, callback_url, manager)
        .await
        .unwrap()
}

fn callback(login: &McpOAuthBrowserLogin) -> String {
    let authorization = url::Url::parse(&login.authorization_url).unwrap();
    let mut callback = url::Url::parse(&login.callback_url).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "private-code")
        .append_pair("state", &query_value(&authorization, "state", 256).unwrap())
        .append_pair("iss", ISSUER);
    callback.into()
}

#[tokio::test]
async fn browser_oauth_binds_caller_callback_state_and_single_use_while_preserving_pkce() {
    let executor = executor();
    let http = Arc::new(TokenHttp::default());
    let first = begin(
        &executor,
        http.clone(),
        "flow-one",
        "project-user-session-one",
    )
    .await;
    let second = begin(
        &executor,
        http.clone(),
        "flow-two",
        "project-user-session-two",
    )
    .await;
    assert_ne!(first.authorization_url, second.authorization_url);
    let valid = callback(&first);
    let authorization = url::Url::parse(&first.authorization_url).unwrap();
    assert_eq!(
        query_value(&authorization, "redirect_uri", 2048).unwrap(),
        CALLBACK
    );
    assert_eq!(
        query_value(&authorization, "code_challenge_method", 32).unwrap(),
        "S256"
    );

    let invalid = [
        ("unknown", "project-user-session-one", valid.clone()),
        ("flow-one", "other-user", valid.clone()),
        (
            "flow-one",
            "project-user-session-one",
            valid.replace("colossus.example", "evil.example"),
        ),
        (
            "flow-one",
            "project-user-session-one",
            valid.replace("/auth/mcp/callback", "/other"),
        ),
        (
            "flow-one",
            "project-user-session-one",
            format!("{valid}&state=duplicate"),
        ),
        ("flow-one", "project-user-session-one", callback(&second)),
    ];
    for (id, binding, callback) in invalid {
        let error = executor
            .oauth_browser_login_complete("fixture", id, binding, &callback)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-code"));
        assert!(!error.contains("state="));
        assert!(http.0.lock().unwrap().is_empty());
    }
    let equivalent = executor
        .snapshot_configuration(&config(), Path::new("."), "native")
        .unwrap();
    let status = equivalent
        .oauth_browser_login_complete("fixture", "flow-one", "project-user-session-one", &valid)
        .await
        .unwrap();
    assert!(status.authenticated);
    {
        let requests = http.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let form = &requests[0];
        assert_eq!(form["redirect_uri"], CALLBACK);
        assert_eq!(form["resource"], "https://mcp.example/mcp");
        assert_eq!(form["client_id"], "cloud-client");
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
            query_value(&authorization, "code_challenge", 256).unwrap()
        );
    }
    let released = serde_json::to_string(&status).unwrap();
    assert!(!released.contains("private-access-token"));
    assert!(!released.contains("private-refresh-token"));
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "flow-one", "project-user-session-one", &valid)
            .await
            .is_err()
    );
    assert!(
        executor
            .reserve_browser_flow(
                "fixture",
                "flow-one",
                "project-user-session-one",
                CALLBACK,
                McpOAuthBrowserCallbackPolicy::HttpsOnly
            )
            .await
            .is_err()
    );
    executor
        .oauth_browser_login_complete(
            "fixture",
            "flow-two",
            "project-user-session-two",
            &callback(&second),
        )
        .await
        .unwrap();
    assert_eq!(http.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn browser_oauth_rejects_expiry_cancellation_logout_and_configuration_generation() {
    let executor = executor();
    let http = Arc::new(TokenHttp::default());
    let expired = begin(&executor, http.clone(), "expired", "caller").await;
    executor
        .browser_oauth_sessions
        .flows
        .lock()
        .await
        .get_mut("expired")
        .unwrap()
        .deadline = Instant::now() - Duration::from_secs(1);
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "expired", "caller", &callback(&expired))
            .await
            .is_err()
    );
    let cancelled = begin(&executor, http.clone(), "cancelled", "caller").await;
    assert!(
        executor
            .oauth_browser_login_cancel("fixture", "cancelled", "other")
            .await
            .is_err()
    );
    executor
        .oauth_browser_login_cancel("fixture", "cancelled", "caller")
        .await
        .unwrap();
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "cancelled", "caller", &callback(&cancelled))
            .await
            .is_err()
    );
    let logged_out = begin(&executor, http.clone(), "logged-out", "caller").await;
    executor.oauth_logout("fixture").await.unwrap();
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "logged-out", "caller", &callback(&logged_out))
            .await
            .is_err()
    );
    let changed = begin(&executor, http.clone(), "changed", "caller").await;
    let mut updated = config();
    updated.servers.get_mut("fixture").unwrap().url = Some("https://other.example/mcp".into());
    let snapshot = executor
        .snapshot_configuration(&updated, Path::new("."), "native")
        .unwrap();
    assert!(
        snapshot
            .oauth_browser_login_complete("fixture", "changed", "caller", &callback(&changed))
            .await
            .is_err()
    );
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "changed", "caller", &callback(&changed))
            .await
            .is_err()
    );
    let restored = snapshot
        .snapshot_configuration(&config(), Path::new("."), "native")
        .unwrap();
    assert!(
        restored
            .oauth_browser_login_complete("fixture", "changed", "caller", &callback(&changed))
            .await
            .is_err()
    );
    assert!(http.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn browser_oauth_checks_issuer_before_any_token_exchange() {
    let executor = executor();
    let http = Arc::new(TokenHttp::default());
    for (id, issuer) in [
        ("wrong-issuer", "https://evil.example"),
        ("missing-issuer", ""),
    ] {
        let login = begin(&executor, http.clone(), id, "caller").await;
        let value = callback(&login);
        let invalid = if issuer.is_empty() {
            value.split("&iss=").next().unwrap().to_owned()
        } else {
            value.replace("identity.example", "evil.example")
        };
        assert!(
            executor
                .oauth_browser_login_complete("fixture", id, "caller", &invalid)
                .await
                .is_err()
        );
        assert!(
            executor
                .oauth_browser_login_complete("fixture", id, "caller", &value)
                .await
                .is_err()
        );
    }
    assert!(http.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn browser_oauth_rejects_untrusted_inputs_and_bounds_concurrent_reservations() {
    let executor = executor();
    for callback in [
        "http://colossus.example/callback",
        "http://127.0.0.1:8765/callback",
        "https://user@colossus.example/callback",
        "https://colossus.example/callback?query=1",
        "https://colossus.example/callback#fragment",
    ] {
        assert!(
            executor
                .oauth_browser_login_begin("fixture", "flow", "caller", callback)
                .await
                .is_err()
        );
    }
    for (id, binding) in [
        ("", "caller"),
        ("flow/path", "caller"),
        ("flow", "user:project"),
        ("flow", ""),
    ] {
        assert!(
            executor
                .oauth_browser_login_begin("fixture", id, binding, CALLBACK)
                .await
                .is_err()
        );
    }
    for index in 0..MAX_PENDING_FLOWS {
        executor
            .reserve_browser_flow(
                "fixture",
                &format!("flow-{index}"),
                "caller",
                CALLBACK,
                McpOAuthBrowserCallbackPolicy::HttpsOnly,
            )
            .await
            .unwrap();
    }
    assert!(
        executor
            .reserve_browser_flow(
                "fixture",
                "overflow",
                "caller",
                CALLBACK,
                McpOAuthBrowserCallbackPolicy::HttpsOnly
            )
            .await
            .is_err()
    );
    executor
        .oauth_browser_login_cancel("fixture", "flow-0", "caller")
        .await
        .unwrap();
    assert!(
        executor
            .reserve_browser_flow(
                "fixture",
                "flow-0",
                "caller",
                CALLBACK,
                McpOAuthBrowserCallbackPolicy::HttpsOnly
            )
            .await
            .is_err()
    );
    assert_eq!(executor.oauth_callback_port("fixture").unwrap(), 8765);
    assert!(
        validate_callback_url("http://127.0.0.1:8765/callback?code=code&state=state", 8765).is_ok()
    );
    assert!(validate_callback_url(&format!("{CALLBACK}?code=code&state=state"), 8765).is_err());
}

#[tokio::test]
async fn browser_oauth_public_begin_uses_fixed_https_redirect_and_native_keeps_loopback() {
    tokio::time::timeout(Duration::from_secs(10), public_begin_and_native())
        .await
        .unwrap();
}

async fn public_begin_and_native() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let endpoint = format!("{origin}/mcp");
    let server_origin = origin.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|value| value == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() <= 8192);
            }
            let first = std::str::from_utf8(&request)
                .unwrap()
                .lines()
                .next()
                .unwrap();
            let (status, headers, body) = if first.starts_with("GET /mcp ") {
                (
                    "401 Unauthorized",
                    format!(
                        "WWW-Authenticate: Bearer resource_metadata=\"{server_origin}/.well-known/oauth-protected-resource/mcp\"\r\n"
                    ),
                    json!({}),
                )
            } else if first.contains("oauth-protected-resource") {
                (
                    "200 OK",
                    String::new(),
                    json!({"resource": format!("{server_origin}/mcp"), "authorization_servers": [server_origin], "scopes_supported": ["read"]}),
                )
            } else {
                assert!(first.contains(".well-known/"));
                (
                    "200 OK",
                    String::new(),
                    json!({"issuer": server_origin, "authorization_endpoint": format!("{server_origin}/authorize"), "token_endpoint": format!("{server_origin}/token"), "response_types_supported": ["code"], "code_challenge_methods_supported": ["S256"]}),
                )
            };
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let mut settings = config();
    settings.servers.get_mut("fixture").unwrap().url = Some(endpoint);
    let executor = McpExecutor::new(&settings, Path::new("."), "native", Arc::new(NoProcess))
        .unwrap()
        .with_ephemeral_oauth_storage("public-api")
        .unwrap()
        .with_oauth_policy(vec![origin], Vec::new(), 5000, 1024 * 1024);
    let browser = executor
        .oauth_browser_login_begin("fixture", "flow", "caller", CALLBACK)
        .await
        .unwrap();
    let browser_url = url::Url::parse(&browser.authorization_url).unwrap();
    assert_eq!(
        query_value(&browser_url, "redirect_uri", 2048).unwrap(),
        CALLBACK
    );
    assert_eq!(
        query_value(&browser_url, "code_challenge_method", 32).unwrap(),
        "S256"
    );
    let native = executor.oauth_login_begin("fixture").await.unwrap();
    let native_url = url::Url::parse(&native.authorization_url).unwrap();
    assert_eq!(native.callback_url, "http://127.0.0.1:8765/callback");
    assert_eq!(
        query_value(&native_url, "redirect_uri", 2048).unwrap(),
        native.callback_url
    );
    let development_callback = "http://127.0.0.1:18090/auth/mcp/callback";
    let development = executor
        .oauth_browser_login_begin_with_policy(
            "fixture",
            "development",
            "caller",
            development_callback,
            McpOAuthBrowserCallbackPolicy::LocalDevelopmentLoopback,
        )
        .await
        .unwrap();
    assert_eq!(
        query_value(
            &url::Url::parse(&development.authorization_url).unwrap(),
            "redirect_uri",
            2048
        )
        .unwrap(),
        development_callback
    );
    assert!(
        executor
            .oauth_login_complete("fixture", &callback(&browser))
            .await
            .is_err()
    );
    executor
        .oauth_browser_login_cancel("fixture", "flow", "caller")
        .await
        .unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn browser_oauth_local_development_callback_requires_explicit_policy_and_exact_loopback() {
    use McpOAuthBrowserCallbackPolicy::{HttpsOnly, LocalDevelopmentLoopback};
    for callback in [
        "http://127.0.0.1:18090/auth/mcp/callback",
        "http://[::1]:18090/auth/mcp/callback",
        "http://localhost:18090/auth/mcp/callback",
    ] {
        assert!(fixed_callback(callback, HttpsOnly).is_err());
        assert!(fixed_callback(callback, LocalDevelopmentLoopback).is_ok());
    }
    for callback in [
        "http://cloud.example/callback",
        "http://127.0.0.1.evil.example/callback",
        "http://localhost.example/callback",
        "http://192.168.1.1/callback",
        "http://user@127.0.0.1/callback",
        "http://127.0.0.1/callback?query=1",
        "http://127.0.0.1/callback#fragment",
    ] {
        assert!(fixed_callback(callback, LocalDevelopmentLoopback).is_err());
    }
    let executor = executor();
    let http = Arc::new(TokenHttp::default());
    let callback_url = "http://127.0.0.1:18090/auth/mcp/callback";
    let login = begin_with_callback(
        &executor,
        http.clone(),
        "local",
        "project-user-session",
        callback_url,
        LocalDevelopmentLoopback,
    )
    .await;
    let redirected = callback(&login);
    assert!(
        executor
            .oauth_browser_login_complete(
                "fixture",
                "local",
                "project-user-session",
                &redirected.replace("18090", "18091")
            )
            .await
            .is_err()
    );
    assert!(http.0.lock().unwrap().is_empty());
    assert!(
        executor
            .oauth_browser_login_complete("fixture", "local", "project-user-session", &redirected)
            .await
            .unwrap()
            .authenticated
    );
    assert_eq!(http.0.lock().unwrap()[0]["redirect_uri"], callback_url);
}
