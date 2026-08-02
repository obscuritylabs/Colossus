use super::*;
use colossus_ports::{KeyProvider, StoreError};
use std::collections::HashMap;
use std::sync::Mutex;

#[test]
fn request_uses_official_protocol_models_and_no_secret_values() {
    let operation = McpOperation::CallTool {
        server: "local".into(),
        tool: "echo".into(),
        arguments: json!({"text": "hello"}),
        input_schema: json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": false
        }),
    };
    let bytes = protocol_input(&operation).expect("protocol");
    let lines = std::str::from_utf8(&bytes)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .collect::<Vec<_>>();
    assert_eq!(lines[0]["method"], "initialize");
    assert_eq!(lines[0]["params"]["protocolVersion"], "2025-11-25");
    assert_eq!(lines[1]["method"], "notifications/initialized");
    assert_eq!(lines[2]["method"], "tools/call");
    assert_eq!(lines[2]["params"]["name"], "echo");
}

#[test]
fn remote_call_timeout_certainty_follows_dispatch_stage() {
    let call = McpOperation::CallTool {
        server: "fixture".into(),
        tool: "echo".into(),
        arguments: json!({}),
        input_schema: json!({"type": "object"}),
    };
    assert!(matches!(
        remote_timeout_error(&call, false),
        ExecutionError::Failed(_)
    ));
    assert!(matches!(
        remote_timeout_error(&call, true),
        ExecutionError::OutcomeUnknown(_)
    ));
    assert!(matches!(
        remote_timeout_error(
            &McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
            false,
        ),
        ExecutionError::Failed(_)
    ));
}

#[test]
fn discovered_schema_is_enforced_before_call() {
    let tool = McpToolSummary {
        server: "local".into(),
        name: "echo".into(),
        title: None,
        description: None,
        input_schema: json!({
            "type": "object",
            "properties": {"count": {"type": "integer"}},
            "required": ["count"],
            "additionalProperties": false
        }),
        schema_sha256: "unused".into(),
    };
    assert!(validate_tool_arguments(&tool, &json!({"count": 2})).is_ok());
    assert!(validate_tool_arguments(&tool, &json!({"count": "two"})).is_err());
}

#[test]
fn secret_values_are_redacted_from_nested_results() {
    let mut value = json!({"text": "token=secret-value", "nested": ["secret-value"]});
    redact_value(&mut value, &["secret-value".into()]);
    assert_eq!(value["text"], "token=<redacted>");
    assert_eq!(value["nested"][0], "<redacted>");
}

#[test]
fn discovery_pages_containing_configured_credentials_fail_before_release() {
    let page = McpToolsPage {
        server: "splunk".into(),
        tools: vec![McpToolSummary {
            server: "splunk".into(),
            name: "search".into(),
            title: None,
            description: Some("accidentally echoed hard-secret".into()),
            input_schema: json!({"type": "object"}),
            schema_sha256: "hash".into(),
        }],
        next_cursor: None,
    };
    assert!(tools_page_contains_secret(&page, &["hard-secret".into()]));
}

#[test]
fn streamable_http_content_types_require_an_exact_media_type() {
    assert!(content_type_matches("application/json", "application/json"));
    assert!(content_type_matches(
        "Application/JSON; charset=utf-8",
        "application/json"
    ));
    assert!(content_type_matches(
        "text/event-stream; charset=utf-8",
        "text/event-stream"
    ));
    assert!(!content_type_matches(
        "application/json-seq",
        "application/json"
    ));
    assert!(!content_type_matches(
        "text/event-streaming",
        "text/event-stream"
    ));
}

#[test]
fn tool_wildcard_is_exclusive_and_empty_or_duplicate_lists_fail_closed() {
    let wildcard = ToolAllowlist::from_config("remote", &["*".into()]).expect("wildcard");
    assert!(matches!(&wildcard, ToolAllowlist::All));
    assert_eq!(wildcard.summary(), vec!["*"]);
    assert!(ToolAllowlist::from_config("remote", &["*".into(), "search".into()]).is_err());
    assert!(ToolAllowlist::from_config("remote", &[]).is_err());
    assert!(ToolAllowlist::from_config("remote", &["search".into(), "search".into()]).is_err());
}

fn remote_server(endpoint: &str) -> McpServerConfig {
    McpServerConfig {
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        working_directory: None,
        environment: BTreeMap::new(),
        url: Some(endpoint.into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: None,
        allowed_tools: vec!["*".into()],
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    }
}

#[test]
fn streamable_http_config_accepts_env_credentials_and_rejects_unsafe_http_identity() {
    let mut server = remote_server("https://splunk.example.com/services/mcp");
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    let mut config = McpConfig {
        oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
        servers: BTreeMap::from([("splunk".into(), server.clone())]),
    };
    let validate = |config: &McpConfig| {
        validate_config(
            config,
            Path::new("."),
            &[],
            &[],
            &["SPLUNK_MCP_TOKEN".into()],
            30_000,
            1024 * 1024,
        )
    };
    validate(&config).expect("valid remote config");
    config.servers.get_mut("splunk").expect("server").url = Some("http://[::1]:8787/mcp".into());
    validate(&config).expect("IPv6 loopback development endpoint");

    config.servers.get_mut("splunk").expect("server").url =
        Some("http://splunk.example.com/services/mcp".into());
    assert!(validate(&config).is_err());
    config.servers.get_mut("splunk").expect("server").url =
        Some("https://user:secret@splunk.example.com/services/mcp".into());
    assert!(validate(&config).is_err());
    config.servers.get_mut("splunk").expect("server").url =
        Some("https://splunk.example.com/services/mcp?token=secret".into());
    assert!(validate(&config).is_err());

    let server = config.servers.get_mut("splunk").expect("server");
    server.url = Some("https://splunk.example.com/services/mcp".into());
    server.credential_headers.clear();
    server
        .headers
        .insert("Authorization".into(), "secret".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.headers.clear();
    server.headers.insert("X-API-Key".into(), "secret".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.headers.clear();
    server.credential_headers.insert(
        "Accept".into(),
        McpCredentialHeaderConfig {
            scheme: None,
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    assert!(validate(&config).is_err());

    let server = config.servers.get_mut("splunk").expect("server");
    server.credential_headers.clear();
    server.oauth = Some(McpOAuthConfig {
        client_id: "colossus".into(),
        client_secret_reference: Some("env:SPLUNK_MCP_TOKEN".into()),
        callback_port: 8787,
        scopes: vec!["openid".into(), "offline_access".into()],
    });
    validate(&config).expect("valid OAuth alternative");
    config
        .servers
        .get_mut("splunk")
        .expect("server")
        .oauth
        .as_mut()
        .expect("OAuth")
        .scopes
        .push("openid".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.oauth.as_mut().expect("OAuth").scopes.pop();
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    assert!(validate(&config).is_err());
}

#[test]
fn wildcard_releases_new_valid_tools_but_rejects_invalid_discovery_names() {
    let server = ConfiguredServer {
        name: "remote".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some("https://splunk.example.com/services/mcp".into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let first: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "splunk_run_search",
            "description": "Run a search",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    let later: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "splunk_new_tool",
            "description": "Published later",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert_eq!(
        parse_tools_result(first, &server).expect("first").tools[0].name,
        "splunk_run_search"
    );
    assert_eq!(
        parse_tools_result(later, &server).expect("later").tools[0].name,
        "splunk_new_tool"
    );

    let invalid: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "invalid tool",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert!(parse_tools_result(invalid, &server).is_err());

    let oversized_description: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "valid_tool",
            "description": "x".repeat(32 * 1024 + 1),
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert!(parse_tools_result(oversized_description, &server).is_err());
}

struct RotatingTestKeys {
    active: Mutex<String>,
    keys: BTreeMap<String, [u8; 32]>,
}

impl KeyProvider for RotatingTestKeys {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        let id = self.active.lock().expect("active key").clone();
        Ok((id.clone(), self.keys[&id]))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        self.keys
            .get(key_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(key_id.into()))
    }

    fn store_anchor(&self, _anchor: &colossus_contracts::SecureAnchor) -> Result<(), StoreError> {
        Ok(())
    }

    fn load_anchor(&self) -> Result<Option<colossus_contracts::SecureAnchor>, StoreError> {
        Ok(None)
    }
}

#[tokio::test]
async fn encrypted_oauth_store_is_ciphertext_at_rest_and_reencrypts_after_rotation() {
    use redb::ReadableDatabase as _;
    use rmcp::transport::auth::{CredentialStore as _, StoredCredentials};

    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("oauth.redb");
    let keys = Arc::new(RotatingTestKeys {
        active: Mutex::new("key-1".into()),
        keys: BTreeMap::from([("key-1".into(), [7_u8; 32]), ("key-2".into(), [9_u8; 32])]),
    });
    let factory = OAuthStoreFactory::encrypted_state(
        &path,
        keys.clone() as Arc<dyn KeyProvider>,
        "repository-1".into(),
    )
    .expect("store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    let credentials: StoredCredentials = serde_json::from_value(json!({
        "client_id": "colossus",
        "token_response": null,
        "granted_scopes": ["openid"],
        "token_received_at": 1
    }))
    .expect("credentials");
    store.save(credentials.clone()).await.expect("save");
    assert!(
        factory
            .store("other", "https://splunk.example.com/services/mcp")
            .load()
            .await
            .expect("identity load")
            .is_none()
    );
    assert!(
        factory
            .store("splunk", "https://splunk.example.com/other")
            .load()
            .await
            .expect("endpoint load")
            .is_none()
    );
    let at_rest = fs::read(&path).expect("database bytes");
    assert!(
        !at_rest
            .windows(b"colossus".len())
            .any(|value| value == b"colossus")
    );

    *keys.active.lock().expect("active key") = "key-2".into();
    let loaded = store.load().await.expect("load").expect("credentials");
    assert_eq!(
        serde_json::to_value(loaded).expect("loaded JSON"),
        serde_json::to_value(credentials).expect("expected JSON")
    );
    let record = {
        let OAuthCredentialStore::EncryptedState {
            database, identity, ..
        } = &store
        else {
            panic!("encrypted store");
        };
        let read = database.begin_read().expect("read");
        let table = read.open_table(OAUTH_RECORDS).expect("table");
        let value = table
            .get(identity.as_str())
            .expect("record read")
            .expect("record");
        serde_json::from_slice::<Value>(value.value()).expect("record JSON")
    };
    assert_eq!(record["key_id"], "key-2");
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> (String, Option<Value>) {
    use tokio::io::AsyncReadExt as _;

    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.expect("read request");
        assert!(count > 0, "client disconnected before sending a request");
        request.extend_from_slice(&chunk[..count]);
        if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).expect("request headers");
    let first = headers.lines().next().expect("request line").to_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::trim)
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.expect("read request body");
        assert!(count > 0, "client disconnected before sending its body");
        request.extend_from_slice(&chunk[..count]);
    }
    let body = &request[header_end..header_end + content_length];
    (first, serde_json::from_slice(body).ok())
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    extra_headers: &str,
    body: &str,
) {
    use tokio::io::AsyncWriteExt as _;

    let response = format!(
        "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

fn configured_http_server(endpoint: String) -> ConfiguredServer {
    ConfiguredServer {
        name: "fixture".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(5_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    }
}

#[tokio::test]
async fn streamable_http_call_initialization_failure_has_a_known_outcome() {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (_, message) = read_http_request(&mut stream).await;
        assert_eq!(
            message
                .and_then(|value| value["method"].as_str().map(str::to_owned))
                .as_deref(),
            Some("initialize")
        );
        write_http_response(&mut stream, "500 Internal Server Error", "", "").await;
    });
    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let dispatched = std::sync::atomic::AtomicBool::new(false);
    let result = execute_remote_operation(
        http,
        &configured_http_server(endpoint),
        &McpOperation::CallTool {
            server: "fixture".into(),
            tool: "echo".into(),
            arguments: json!({}),
            input_schema: json!({"type": "object"}),
        },
        HashMap::new(),
        &dispatched,
    )
    .await;
    let Err(error) = result else {
        panic!("initialization must fail");
    };
    assert!(matches!(error, ExecutionError::Failed(_)));
    assert!(!dispatched.load(std::sync::atomic::Ordering::Acquire));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn streamable_http_call_failure_after_dispatch_has_an_unknown_outcome() {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let (first, message) = read_http_request(&mut stream).await;
            let method = message
                .as_ref()
                .and_then(|value| value.get("method"))
                .and_then(Value::as_str);
            match method {
                Some("initialize") => {
                    let body = json!({
                        "jsonrpc": "2.0",
                        "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                        "result": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fixture", "version": "1.0.0"}
                        }
                    })
                    .to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\nMcp-Session-Id: test-session\r\n",
                        &body,
                    )
                    .await;
                }
                Some("notifications/initialized") => {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                }
                Some("tools/call") => break,
                None if first.starts_with("GET ") => {
                    write_http_response(&mut stream, "405 Method Not Allowed", "", "").await;
                }
                _ => panic!("unexpected MCP request: {first} {message:?}"),
            }
        }
    });
    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let dispatched = std::sync::atomic::AtomicBool::new(false);
    let result = execute_remote_operation(
        http,
        &configured_http_server(endpoint),
        &McpOperation::CallTool {
            server: "fixture".into(),
            tool: "echo".into(),
            arguments: json!({}),
            input_schema: json!({"type": "object"}),
        },
        HashMap::new(),
        &dispatched,
    )
    .await;
    let Err(error) = result else {
        panic!("dispatched call must fail without a response");
    };
    assert!(matches!(error, ExecutionError::OutcomeUnknown(_)));
    assert!(dispatched.load(std::sync::atomic::Ordering::Acquire));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn streamable_http_json_session_discovery_uses_fresh_stateful_transport() {
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let saw_delete = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_saw_delete = Arc::clone(&saw_delete);
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 2048];
                let count = stream.read(&mut chunk).await.expect("read");
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let (content_length, first) = {
                let headers = std::str::from_utf8(&request[..header_end]).expect("headers");
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let first = headers.lines().next().expect("request line").to_owned();
                (content_length, first)
            };
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 2048];
                let count = stream.read(&mut chunk).await.expect("body");
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
            }
            let body = &request[header_end..header_end + content_length];
            let message = serde_json::from_slice::<Value>(body).ok();
            let (status, extra, response_body) = if first.starts_with("GET ") {
                ("405 Method Not Allowed", "", String::new())
            } else if first.starts_with("DELETE ") {
                ("200 OK", "", String::new())
            } else {
                match message
                    .as_ref()
                    .and_then(|value| value.get("method"))
                    .and_then(Value::as_str)
                {
                    Some("initialize") => (
                        "200 OK",
                        "Content-Type: application/json\r\nMcp-Session-Id: test-session\r\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "fixture", "version": "1.0.0"}
                            }
                        })
                        .to_string(),
                    ),
                    Some("tools/list") => (
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                            "result": {
                                "tools": [{
                                    "name": "splunk_run_search",
                                    "inputSchema": {"type": "object"}
                                }]
                            }
                        })
                        .to_string(),
                    ),
                    _ => ("202 Accepted", "", String::new()),
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
            if first.starts_with("DELETE ") {
                server_saw_delete.store(true, std::sync::atomic::Ordering::Release);
                break;
            }
        }
    });

    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let configured = ConfiguredServer {
        name: "splunk".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(5_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let result = execute_remote_operation(
        http,
        &configured,
        &McpOperation::ListTools {
            server: "splunk".into(),
            cursor: None,
        },
        HashMap::new(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await
    .expect("remote discovery");
    let RemoteOperationResult::Tools(result) = result else {
        panic!("tools result");
    };
    assert_eq!(result.tools[0].name, "splunk_run_search");
    tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("session delete")
        .expect("server");
    assert!(saw_delete.load(std::sync::atomic::Ordering::Acquire));
}

#[cfg(feature = "live-splunk")]
#[tokio::test]
#[ignore = "requires COLOSSUS_LIVE_SPLUNK_MCP_URL and SPLUNK_MCP_TOKEN"]
async fn live_splunk_streamable_http_discovery() {
    let endpoint = env::var("COLOSSUS_LIVE_SPLUNK_MCP_URL").expect("COLOSSUS_LIVE_SPLUNK_MCP_URL");
    let token = env::var("SPLUNK_MCP_TOKEN").expect("SPLUNK_MCP_TOKEN");
    let url = url::Url::parse(&endpoint).expect("endpoint URL");
    let client = colossus_network::pinned_reqwest_client(
        &url,
        &AdditionalRootCertificates::default(),
        30_000,
        true,
    )
    .await
    .expect("hardened HTTP client");
    let http = HardenedStreamableHttpClient::for_test(endpoint.clone(), client, 1024 * 1024);
    let server = ConfiguredServer {
        name: "splunk".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let authorization = format!("Bearer {token}")
        .parse::<http::HeaderValue>()
        .expect("bearer header");
    let result = execute_remote_operation(
        http,
        &server,
        &McpOperation::ListTools {
            server: "splunk".into(),
            cursor: None,
        },
        HashMap::from([(
            http::HeaderName::from_static("authorization"),
            authorization,
        )]),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await
    .expect("Splunk discovery");
    assert!(matches!(result, RemoteOperationResult::Tools(_)));
}
