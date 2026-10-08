use super::*;
use crate::executor::{parse_tools, validate_call_output};
use rmcp::model::{DiscoverResult, ServerCapabilities};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::{
    io::AsyncReadExt as _,
    net::{TcpListener, TcpStream},
};

mod cleanup;

async fn request(stream: &mut TcpStream) -> (BTreeMap<String, String>, Value) {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 2048];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 1024 * 1024);
        if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let mut headers: BTreeMap<_, _> = std::str::from_utf8(&bytes[..end])
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    headers.insert(
        ":method".into(),
        std::str::from_utf8(&bytes[..end])
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .into(),
    );
    let length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < end + length {
        let mut chunk = [0; 2048];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    (
        headers,
        if length == 0 {
            Value::Null
        } else {
            serde_json::from_slice(&bytes[end..end + length]).unwrap()
        },
    )
}

fn input_schema() -> Value {
    json!({"type":"object", "properties":{"tenant":{"type":"string", "x-mcp-header":"Tenant"}}, "required":["tenant"]})
}

fn output_schema() -> Value {
    json!({"type":"object", "properties":{"count":{"type":"integer"}}, "required":["count"]})
}

async fn modern_server(listener: TcpListener, calls: bool, sse: bool, continuation: Option<Value>) {
    let mut lists = 0;
    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (headers, message) = request(&mut stream).await;
        assert_eq!(headers[":method"], "POST", "2026 uses POST only");
        let method = message["method"].as_str().unwrap();
        assert_eq!(headers["mcp-protocol-version"], "2026-07-28");
        assert_eq!(headers["mcp-method"], method);
        assert!(!headers.contains_key("mcp-session-id"));
        assert_eq!(
            message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            message["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
            "colossus"
        );
        let result = match method {
            "server/discover" => serde_json::to_value(DiscoverResult::new(
                vec![ProtocolVersion::V_2026_07_28],
                serde_json::from_value::<ServerCapabilities>(json!({"tools":{}})).unwrap(),
            ))
            .unwrap(),
            "tools/list" => {
                assert!(
                    !calls,
                    "calls retain the permit-bound schema without re-listing"
                );
                lists += 1;
                if lists == 1 {
                    assert!(message["params"].get("cursor").is_none());
                    json!({"tools":[
                        {"name":"task_only", "execution":{"taskSupport":"required"}, "inputSchema":{"type":"object"}},
                        {"name":"task_meta", "_meta":{"io.modelcontextprotocol/tasks/taskSupport":"required"}, "inputSchema":{"type":"object"}}
                    ], "nextCursor":"same-connection"})
                } else {
                    assert_eq!(lists, 2);
                    assert_eq!(message["params"]["cursor"], "same-connection");
                    json!({"tools":[{"name":"probe", "inputSchema":input_schema(), "outputSchema":output_schema()}]})
                }
            }
            "tools/call" => {
                assert!(calls);
                assert_eq!(lists, 0, "invocation must not mint a new discovery context");
                assert_eq!(headers["mcp-name"], "probe");
                assert_eq!(headers["mcp-param-tenant"], "engineering");
                continuation
                    .clone()
                    .unwrap_or_else(|| json!({"content":[], "structuredContent":{"count":3}}))
            }
            _ => panic!("unexpected method {method}"),
        };
        let body = json!({"jsonrpc":"2.0", "id":message["id"], "result":result}).to_string();
        if sse {
            write_http_response(
                &mut stream,
                "200 OK",
                "Content-Type: text/event-stream\r\n",
                &format!("event: message\ndata: {body}\n\n"),
            )
            .await;
        } else {
            write_http_response(
                &mut stream,
                "200 OK",
                "Content-Type: application/json\r\n",
                &body,
            )
            .await;
        }
        if method == "tools/call" || (method == "tools/list" && lists == 2 && !calls) {
            break;
        }
    }
}

#[tokio::test]
async fn modern_http_discovers_all_pages_and_filters_task_only_tools_in_json_and_sse() {
    for sse in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(modern_server(listener, false, sse, None));
        let http = HardenedStreamableHttpClient::for_test(
            endpoint.clone(),
            reqwest::Client::builder().no_proxy().build().unwrap(),
            1024 * 1024,
        );
        let mut server = configured_http_server(endpoint);
        server.protocol_version = McpProtocolVersion::Auto;
        let result = execute_remote_operation(
            http,
            &server,
            &McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
            HashMap::new(),
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        let RemoteOperationResult::Tools(result) = result else {
            panic!("expected tools");
        };
        let page = parse_tools_result(result, &server).unwrap();
        assert_eq!(page.tools.len(), 1);
        assert_eq!(page.tools[0].name, "probe");
        assert_eq!(page.tools[0].output_schema, Some(output_schema()));
        assert!(page.next_cursor.is_none());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn modern_http_calls_once_with_standard_parameter_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(modern_server(listener, true, false, None));
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        1024 * 1024,
    );
    let mut server = configured_http_server(endpoint);
    server.protocol_version = McpProtocolVersion::V2026;
    let operation = call_operation(Some(output_schema()));
    let http = http.with_call_headers(&operation);
    let dispatched = AtomicBool::new(false);
    let result = execute_remote_operation(http, &server, &operation, HashMap::new(), &dispatched)
        .await
        .unwrap();
    assert!(dispatched.load(Ordering::Acquire));
    let RemoteOperationResult::Call(result) = result else {
        panic!("expected call");
    };
    validate_call_output(&result, &operation).unwrap();
    task.await.unwrap();
}

fn call_operation(output: Option<Value>) -> McpOperation {
    McpOperation::CallTool {
        server: "fixture".into(),
        tool: "probe".into(),
        description: None,
        annotations: None,
        arguments: json!({"tenant":"engineering"}),
        input_schema: Box::new(input_schema()),
        output_schema: output.map(Box::new),
        schema_sha256: test_schema_sha256(&input_schema()),
    }
}

#[test]
fn permit_bound_parameter_headers_encode_values_and_reject_ambiguous_annotations() {
    let mut operation = call_operation(None);
    let McpOperation::CallTool { arguments, .. } = &mut operation else {
        unreachable!();
    };
    arguments["tenant"] = json!(" 東京\r\n");
    let headers = crate::param_headers::call_headers(&operation).unwrap();
    assert_eq!(
        headers[&http::HeaderName::from_static("mcp-param-tenant")],
        "=?base64?IOadseS6rA0K?="
    );
    let McpOperation::CallTool { input_schema, .. } = &mut operation else {
        unreachable!();
    };
    input_schema["properties"]["duplicate"] = json!({"type":"string", "x-mcp-header":"tenant"});
    assert!(crate::param_headers::call_headers(&operation).is_err());
    let McpOperation::CallTool { input_schema, .. } = &mut operation else {
        unreachable!();
    };
    input_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("duplicate");
    input_schema["properties"]["tenant"]["x-mcp-header"] = json!("bad header");
    assert!(crate::param_headers::call_headers(&operation).is_err());
}

#[tokio::test]
async fn legacy_calls_do_not_apply_modern_parameter_header_rules() {
    for mode in [McpProtocolVersion::Auto, McpProtocolVersion::V2025] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut initialized = false;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (headers, message) = request(&mut stream).await;
                assert!(!headers.keys().any(|name| name.starts_with("mcp-param-")));
                let method = message["method"].as_str().unwrap();
                let response = match method {
                    "server/discover" => {
                        json!({"jsonrpc":"2.0", "id":message["id"], "error":{"code":-32601, "message":"Method not found"}})
                    }
                    "initialize" => {
                        json!({"jsonrpc":"2.0", "id":message["id"], "result":{"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}, "serverInfo":{"name":"legacy", "version":"1"}}})
                    }
                    "notifications/initialized" => {
                        initialized = true;
                        write_http_response(&mut stream, "202 Accepted", "", "").await;
                        continue;
                    }
                    "tools/call" => {
                        assert!(initialized);
                        json!({"jsonrpc":"2.0", "id":message["id"], "result":{"content":[], "structuredContent":{"count":3}}})
                    }
                    _ => panic!("unexpected legacy request {method}"),
                };
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &response.to_string(),
                )
                .await;
                if method == "tools/call" {
                    break;
                }
            }
        });
        let mut operation = call_operation(Some(output_schema()));
        let McpOperation::CallTool { input_schema, .. } = &mut operation else {
            unreachable!();
        };
        input_schema["properties"]["tenant"]["x-mcp-header"] = json!("not a header token");
        let http = HardenedStreamableHttpClient::for_test(
            endpoint.clone(),
            reqwest::Client::builder().no_proxy().build().unwrap(),
            1024 * 1024,
        )
        .with_call_headers(&operation);
        let mut server = configured_http_server(endpoint);
        server.protocol_version = mode;
        server.allow_stateless = true;
        let result = execute_remote_operation(
            http,
            &server,
            &operation,
            HashMap::new(),
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        let RemoteOperationResult::Call(result) = result else {
            panic!("expected call");
        };
        validate_call_output(&result, &operation).unwrap();
        task.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_modern_parameter_headers_fail_before_tool_dispatch() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(modern_server(listener, true, false, None));
    let mut operation = call_operation(Some(output_schema()));
    let McpOperation::CallTool { input_schema, .. } = &mut operation else {
        unreachable!();
    };
    input_schema["properties"]["tenant"]["x-mcp-header"] = json!("not a header token");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        1024 * 1024,
    )
    .with_call_headers(&operation);
    let mut server = configured_http_server(endpoint);
    server.protocol_version = McpProtocolVersion::V2026;
    let dispatched = AtomicBool::new(false);
    let result =
        execute_remote_operation(http, &server, &operation, HashMap::new(), &dispatched).await;
    assert!(result.is_err());
    assert!(!dispatched.load(Ordering::Acquire));
    task.abort();
}

#[tokio::test]
async fn configured_remote_limits_tighten_a_broader_execution_permit() {
    for slow in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (_, message) = request(&mut stream).await;
                seen.fetch_add(1, Ordering::Relaxed);
                let result = if message["method"] == "server/discover" {
                    if slow {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    let mut result = DiscoverResult::new(
                        vec![ProtocolVersion::V_2026_07_28],
                        serde_json::from_value::<ServerCapabilities>(json!({"tools":{}})).unwrap(),
                    );
                    if !slow {
                        result.instructions = Some("x".repeat(2048));
                    }
                    serde_json::to_value(result).unwrap()
                } else {
                    assert_eq!(message["method"], "tools/list");
                    json!({"tools":[]})
                };
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    &json!({"jsonrpc":"2.0", "id":message["id"], "result":result}).to_string(),
                )
                .await;
                if message["method"] == "tools/list" {
                    break;
                }
            }
        });
        let mut server = remote_server(&format!("http://{address}/mcp"));
        server.protocol_version = McpProtocolVersion::V2026;
        server.timeout_ms = Some(if slow { 50 } else { 5000 });
        server.max_output_bytes = Some(if slow { 1024 * 1024 } else { 1024 });
        let config = McpConfig {
            servers: BTreeMap::from([("remote".into(), server)]),
            ..McpConfig::default()
        };
        let executor = McpExecutor::new(
            &config,
            Path::new("."),
            "native",
            Arc::new(McpEffectShapeExecutor {
                reference: "unused",
            }),
        )
        .unwrap();
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.tools", DecisionOutcome::Allow)
                    .with_post_effect(false)
                    .with_sandbox("native", "mcp-limit-regression", false)
                    .with_limits(5000, 1024 * 1024, 4, 64 * 1024 * 1024, 1)
                    .with_network_destination(format!("http://{address}")),
            ),
            Arc::new(AllowApproval {
                approved_by: "test".into(),
            }),
            SafetyKernel::new(["mcp.invoke".into()]),
            [83_u8; 32],
        );
        let request = executor
            .request(
                Actor {
                    actor_type: ActorType::System,
                    id: "mcp-limit-regression".into(),
                },
                ExecutionContext::default(),
                McpOperation::ListTools {
                    server: "remote".into(),
                    cursor: None,
                },
            )
            .unwrap();
        let started = std::time::Instant::now();
        let result = gateway.execute(request, &executor).await;
        assert!(
            result.is_err(),
            "configured server limit must tighten the permit"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(
            requests.load(Ordering::Relaxed),
            1,
            "bounded negotiation must not dispatch tools/list"
        );
        task.abort();
    }
}

#[tokio::test]
async fn legacy_http_pagination_keeps_one_initialized_session() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut initialized = false;
        let mut pages = 0;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (headers, message) = request(&mut stream).await;
            match headers[":method"].as_str() {
                "GET" => {
                    write_http_response(&mut stream, "405 Method Not Allowed", "", "").await;
                    continue;
                }
                "DELETE" => {
                    assert_eq!(pages, 2);
                    assert_eq!(headers["mcp-session-id"], "one-session");
                    write_http_response(&mut stream, "200 OK", "", "").await;
                    break;
                }
                _ => {}
            }
            let method = message["method"].as_str().unwrap();
            let (extra, result) = match method {
                "initialize" => {
                    assert!(!initialized, "pagination must not reinitialize");
                    initialized = true;
                    (
                        "Mcp-Session-Id: one-session\r\n",
                        json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture", "version":"1"}}),
                    )
                }
                "notifications/initialized" => {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                    continue;
                }
                "tools/list" => {
                    assert_eq!(headers["mcp-session-id"], "one-session");
                    assert_eq!(headers["mcp-protocol-version"], "2025-11-25");
                    pages += 1;
                    let result = if pages == 1 {
                        json!({"tools":[{"name":"first", "inputSchema":{"type":"object"}}], "nextCursor":"one-session-cursor"})
                    } else {
                        assert_eq!(pages, 2);
                        assert_eq!(message["params"]["cursor"], "one-session-cursor");
                        json!({"tools":[{"name":"second", "inputSchema":{"type":"object"}}]})
                    };
                    ("", result)
                }
                _ => panic!("unexpected legacy method {method}"),
            };
            let body = json!({"jsonrpc":"2.0", "id":message["id"], "result":result}).to_string();
            write_http_response(
                &mut stream,
                "200 OK",
                &format!("Content-Type: application/json\r\n{extra}"),
                &body,
            )
            .await;
        }
    });
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        1024 * 1024,
    );
    let result = execute_remote_operation(
        http,
        &configured_http_server(endpoint),
        &McpOperation::ListTools {
            server: "fixture".into(),
            cursor: None,
        },
        HashMap::new(),
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    let RemoteOperationResult::Tools(result) = result else {
        panic!("expected tools");
    };
    assert_eq!(result.tools.len(), 2);
    assert!(result.next_cursor.is_none());
    task.await.unwrap();
}

#[tokio::test]
async fn automatic_http_falls_back_only_for_legacy_discovery_rejection() {
    for (mode, modern_error, expected_success) in [
        (McpProtocolVersion::Auto, false, true),
        (McpProtocolVersion::V2026, false, false),
        (McpProtocolVersion::Auto, true, false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, message) = request(&mut stream).await;
            assert_eq!(message["method"], "server/discover");
            let error = if modern_error {
                json!({"code":-32022, "message":"Unsupported protocol version", "data":{"supportedVersions":["2025-11-25"]}})
            } else {
                json!({"code":-32601, "message":"Method not found"})
            };
            // Middleware need not echo the request ID. The HTTP adapter
            // correlates only these bounded sessionless discovery rejections.
            let body = json!({"jsonrpc":"2.0", "id":"server-error", "error":error}).to_string();
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "Content-Type: application/json\r\n",
                &body,
            )
            .await;
            if !expected_success {
                return;
            }
            for expected in ["initialize", "notifications/initialized", "tools/list"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (_, message) = request(&mut stream).await;
                assert_eq!(message["method"], expected);
                if expected == "notifications/initialized" {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                } else {
                    let result = if expected == "initialize" {
                        assert_eq!(message["params"]["protocolVersion"], "2025-11-25");
                        json!({"protocolVersion":"2025-06-18", "capabilities":{"tools":{}}, "serverInfo":{"name":"legacy", "version":"1"}})
                    } else {
                        json!({"tools":[]})
                    };
                    let body =
                        json!({"jsonrpc":"2.0", "id":message["id"], "result":result}).to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        &body,
                    )
                    .await;
                }
            }
        });
        let http = HardenedStreamableHttpClient::for_test(
            endpoint.clone(),
            reqwest::Client::builder().no_proxy().build().unwrap(),
            1024 * 1024,
        );
        let mut server = configured_http_server(endpoint);
        server.protocol_version = mode;
        server.allow_stateless = true;
        let result = execute_remote_operation(
            http,
            &server,
            &McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
            HashMap::new(),
            &AtomicBool::new(false),
        )
        .await;
        assert_eq!(result.is_ok(), expected_success);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn client_input_continuations_do_not_reissue_the_authorized_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(modern_server(
        listener,
        true,
        false,
        Some(json!({
            "resultType":"input_required", "requestState":"continuation-token",
        })),
    ));
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
        1024 * 1024,
    );
    let mut server = configured_http_server(endpoint);
    server.protocol_version = McpProtocolVersion::V2026;
    let operation = call_operation(Some(output_schema()));
    let http = http.with_call_headers(&operation);
    let result = execute_remote_operation(
        http,
        &server,
        &operation,
        HashMap::new(),
        &AtomicBool::new(false),
    )
    .await;
    assert!(matches!(result, Err(ExecutionError::OutcomeUnknown(_))));
    task.await.unwrap();
}

#[test]
fn mcp_schemas_allow_embedded_definitions_without_external_file_or_network_access() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema.json");
    std::fs::write(&path, br#"{"type":"object"}"#).unwrap();
    for reference in [
        url::Url::from_file_path(path).unwrap().to_string(),
        "http://127.0.0.1:9/schema.json".into(),
    ] {
        assert!(crate::schema::validator(&json!({"$ref":reference})).is_err());
    }
    let schema = json!({
        "type":"object", "$defs":{"count":{"type":"integer"}},
        "properties":{"count":{"$ref":"#/$defs/count"}}, "required":["count"],
    });
    let validator = crate::schema::validator(&schema).unwrap();
    assert!(validator.is_valid(&json!({"count":3})));
    assert!(!validator.is_valid(&json!({"count":"wrong"})));
}

#[test]
fn structured_results_must_match_the_authorized_output_schema() {
    let operation = call_operation(Some(output_schema()));
    for structured in [None, Some(json!({"count":"wrong"})), Some(json!({}))] {
        let result: CallToolResult =
            serde_json::from_value(json!({"content":[], "structuredContent":structured})).unwrap();
        assert!(validate_call_output(&result, &operation).is_err());
    }
    let result: CallToolResult =
        serde_json::from_value(json!({"content":[], "structuredContent":{"count":2}})).unwrap();
    validate_call_output(&result, &operation).unwrap();
    validate_call_output(
        &CallToolResult::error(vec![rmcp::model::ContentBlock::text("failure")]),
        &operation,
    )
    .unwrap();
}

#[test]
fn stdio_discovery_collects_session_cursors_and_retains_output_schema() {
    let server = configured_http_server("http://127.0.0.1/mcp".into());
    let messages = [
        json!({"jsonrpc":"2.0", "id":1, "result":{"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture", "version":"1"}}}),
        json!({"jsonrpc":"2.0", "id":2, "result":{"tools":[{"name":"task_only", "execution":{"taskSupport":"required"}, "inputSchema":{"type":"object"}}], "nextCursor":"cursor"}}),
        json!({"jsonrpc":"2.0", "id":3, "result":{"tools":[{"name":"probe", "inputSchema":input_schema(), "outputSchema":output_schema()}]}}),
    ];
    let bytes = messages
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>();
    let page = parse_tools(bytes.as_bytes(), &server).unwrap();
    assert_eq!(page.tools.len(), 1);
    assert_eq!(page.tools[0].output_schema, Some(output_schema()));
    assert!(page.next_cursor.is_none());
    let incomplete = messages[..2]
        .iter()
        .map(|v| format!("{v}\n"))
        .collect::<String>();
    assert!(parse_tools(incomplete.as_bytes(), &server).is_err());
}
