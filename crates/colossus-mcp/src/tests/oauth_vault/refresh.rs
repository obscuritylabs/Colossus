use super::*;
use std::time::Duration;
use tokio::{io::AsyncReadExt as _, net::TcpListener};

const NEW_TOKEN_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 256 * 1024;

async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, String, Vec<u8>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "incomplete OAuth fixture request");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= MAX_REQUEST_BYTES);
        if let Some(index) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let first = headers.lines().next().unwrap().to_owned();
    let header = |name: &str| {
        headers.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    };
    let authorization = header("authorization").unwrap_or("").to_owned();
    let length: usize = header("content-length").unwrap_or("0").parse().unwrap();
    assert!(length <= MAX_REQUEST_BYTES - header_end);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "incomplete OAuth fixture body");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= MAX_REQUEST_BYTES);
    }
    (
        first,
        authorization,
        bytes[header_end..header_end + length].to_vec(),
    )
}

async fn serve_refresh(listener: TcpListener, origin: String) -> (usize, usize) {
    let endpoint = format!("{origin}/mcp");
    let expected_bearer = format!("Bearer {}", "B".repeat(NEW_TOKEN_BYTES));
    let mut refreshes = 0;
    let mut authenticated_requests = 0;
    for _ in 0..16 {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (first, authorization, body) = read_request(&mut stream).await;
        let mut status = "200 OK";
        let mut extra = "Content-Type: application/json\r\n".to_owned();
        let mut done = false;
        let response = match first.as_str() {
            "GET /mcp HTTP/1.1" if authorization.is_empty() => {
                status = "401 Unauthorized";
                extra.push_str(&format!(
                    "WWW-Authenticate: Bearer resource_metadata=\"{origin}/.well-known/oauth-protected-resource/mcp\"\r\n"
                ));
                Value::Null
            }
            "GET /.well-known/oauth-protected-resource/mcp HTTP/1.1" => json!({
                "resource": endpoint,
                "authorization_servers": [origin],
                "scopes_supported": ["openid"]
            }),
            "GET /.well-known/oauth-authorization-server HTTP/1.1" => json!({
                "issuer": origin,
                "authorization_endpoint": format!("{origin}/authorize"),
                "token_endpoint": format!("{origin}/token"),
                "response_types_supported": ["code"],
                "grant_types_supported": ["authorization_code", "refresh_token"],
                "code_challenge_methods_supported": ["S256"]
            }),
            "POST /token HTTP/1.1" => {
                refreshes += 1;
                assert_eq!(refreshes, 1, "fresh credentials must not refresh again");
                let form = url::form_urlencoded::parse(&body).collect::<BTreeMap<_, _>>();
                assert_eq!(
                    form.get("grant_type").map(|v| v.as_ref()),
                    Some("refresh_token")
                );
                assert!(
                    form.get("refresh_token")
                        .is_some_and(|v| v == &"R".repeat(8192))
                );
                assert_eq!(
                    form.get("client_id").map(|v| v.as_ref()),
                    Some("native-client")
                );
                assert_eq!(
                    form.get("resource").map(|v| v.as_ref()),
                    Some(endpoint.as_str())
                );
                json!({
                    "access_token": "B".repeat(NEW_TOKEN_BYTES),
                    "refresh_token": "T".repeat(NEW_TOKEN_BYTES),
                    "token_type": "Bearer", "expires_in": 3600, "scope": "openid"
                })
            }
            _ => {
                assert!(
                    authorization == expected_bearer,
                    "MCP must use the refreshed token exactly"
                );
                authenticated_requests += 1;
                if first == "DELETE /mcp HTTP/1.1" {
                    status = "202 Accepted";
                    done = true;
                    Value::Null
                } else if first == "GET /mcp HTTP/1.1" {
                    status = "405 Method Not Allowed";
                    Value::Null
                } else {
                    assert_eq!(first, "POST /mcp HTTP/1.1");
                    let message: Value = serde_json::from_slice(&body).unwrap();
                    match message["method"].as_str().unwrap() {
                        "initialize" => {
                            extra.push_str("Mcp-Session-Id: refreshed-session\r\n");
                            json!({"jsonrpc": "2.0", "id": message["id"], "result": {
                                "protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
                                "serverInfo": {"name": "oauth-fixture", "version": "1"}
                            }})
                        }
                        "tools/list" => {
                            json!({"jsonrpc": "2.0", "id": message["id"], "result": {"tools": []}})
                        }
                        "notifications/initialized" => {
                            status = "202 Accepted";
                            Value::Null
                        }
                        _ => panic!("unexpected MCP operation"),
                    }
                }
            }
        };
        let body = if response.is_null() {
            String::new()
        } else {
            response.to_string()
        };
        write_http_response(&mut stream, status, &extra, &body).await;
        if done {
            return (refreshes, authenticated_requests);
        }
    }
    panic!("OAuth fixture request bound exceeded");
}

#[tokio::test]
async fn oauth_refresh_persists_replacement_tokens_and_public_logout_clears_only_its_record() {
    tokio::time::timeout(Duration::from_secs(15), refresh_and_logout())
        .await
        .unwrap();
}

async fn refresh_and_logout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let endpoint = format!("{origin}/mcp");
    let server = tokio::spawn(serve_refresh(listener, origin.clone()));
    let vault = Arc::new(MemoryVault::default());
    let factory = OAuthStoreFactory::platform(vault.clone(), "repository".into());
    let store = factory.store("fixture", &endpoint);
    let mut expired = serde_json::to_value(credentials(8192)).unwrap();
    expired["token_response"]["expires_in"] = 1.into();
    store
        .save(serde_json::from_value(expired).unwrap())
        .await
        .unwrap();
    let other = factory.store("other", &endpoint);
    other.save(credentials(8192)).await.unwrap();

    let mut configured = remote_server(&endpoint);
    configured.oauth = Some(McpOAuthConfig {
        client_id: "native-client".into(),
        client_secret_reference: None,
        callback_port: 8765,
        scopes: vec!["openid".into()],
    });
    let executor = McpExecutor::new(
        &McpConfig {
            servers: BTreeMap::from([
                ("fixture".into(), configured.clone()),
                ("other".into(), configured),
            ]),
            ..Default::default()
        },
        Path::new("."),
        "native",
        Arc::new(McpEffectShapeExecutor {
            reference: "unused",
        }),
    )
    .unwrap()
    .with_oauth_vault(vault.clone(), "repository");
    assert!(
        executor
            .oauth_status("fixture")
            .await
            .unwrap()
            .authenticated
    );
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("mcp.tools", DecisionOutcome::Allow)
                .with_post_effect(false)
                .with_sandbox("native", "oauth-refresh-regression", false)
                .with_network_destination(&origin)
                .with_limits(3000, 1024 * 1024, 1, 64 * 1024 * 1024, 1),
        ),
        Arc::new(AllowApproval {
            approved_by: "test".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]),
        [9; 32],
    );
    let request = executor
        .request(
            Actor {
                actor_type: ActorType::System,
                id: "oauth-refresh-test".into(),
            },
            ExecutionContext::default(),
            McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
        )
        .unwrap();
    let response = gateway.execute(request, &executor).await.unwrap();
    assert!(
        serde_json::from_slice::<McpToolsPage>(&response.bytes)
            .unwrap()
            .tools
            .is_empty()
    );
    let (refreshes, authenticated_requests) = server.await.unwrap();
    assert_eq!(refreshes, 1);
    assert!(authenticated_requests >= 4);
    let restored = serde_json::to_value(
        factory
            .store("fixture", &endpoint)
            .load()
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(restored["token_response"]["access_token"] == "B".repeat(NEW_TOKEN_BYTES));
    assert!(restored["token_response"]["refresh_token"] == "T".repeat(NEW_TOKEN_BYTES));
    assert!(restored["token_received_at"].as_u64().unwrap() > 1);
    assert_eq!(restored["granted_scopes"], json!(["openid"]));

    // The fixture is already stopped: status and logout must use local storage only.
    let status = executor.oauth_logout("fixture").await.unwrap();
    assert!(status.configured && !status.authenticated);
    assert!(store.load().await.unwrap().is_none());
    assert!(executor.oauth_status("other").await.unwrap().authenticated);
    assert!(other.load().await.unwrap().is_some());
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    assert!(
        !executor
            .oauth_logout("fixture")
            .await
            .unwrap()
            .authenticated
    );
}
