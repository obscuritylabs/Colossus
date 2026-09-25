use super::*;
use colossus_provider::HostCredentialResolver;
use tokio::{io::AsyncReadExt as _, net::TcpListener};

async fn authenticated_request(
    stream: &mut tokio::net::TcpStream,
    expected: &str,
) -> (String, Value) {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.expect("request read");
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 128 * 1024);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
    let authorization = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("authorization")
            .then(|| value.trim())
    });
    assert!(
        authorization == Some(expected),
        "credential changed in transport"
    );
    let first = headers.lines().next().unwrap().to_owned();
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    (
        first,
        serde_json::from_slice(&bytes[end..end + length]).unwrap_or(Value::Null),
    )
}

async fn serve(listener: TcpListener, secret: String) -> usize {
    let expected = format!("Bearer {secret}");
    let mut deletes = 0;
    let mut requests = 0;
    while deletes < 2 {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (first, message) = authenticated_request(&mut stream, &expected).await;
        requests += 1;
        let method = message["method"].as_str().unwrap_or("");
        let (status, extra, result) = match method {
            "initialize" => (
                "200 OK",
                "Mcp-Session-Id: large-token-session\r\n",
                Some(json!({
                    "protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
                    "serverInfo": {"name": "fixture", "version": "1"}
                })),
            ),
            "tools/list" => (
                "200 OK",
                "",
                Some(json!({"tools": [{
                    "name": "probe", "inputSchema": {"type": "object"}
                }]})),
            ),
            "tools/call" => (
                "200 OK",
                "",
                Some(json!({"content": [{
                    "type": "text", "text": format!("echo {secret}")
                }]})),
            ),
            _ if first.starts_with("GET ") => ("405 Method Not Allowed", "", None),
            _ => ("202 Accepted", "", None),
        };
        let body = result
            .map(|result| {
                json!({"jsonrpc": "2.0", "id": message["id"], "result": result}).to_string()
            })
            .unwrap_or_default();
        write_http_response(
            &mut stream,
            status,
            &format!("Content-Type: application/json\r\n{extra}"),
            &body,
        )
        .await;
        if first.starts_with("DELETE ") {
            deletes += 1;
        }
    }
    requests
}

#[tokio::test]
async fn large_host_tokens_reach_real_http_adapter_and_echoes_are_redacted() {
    for size in [8 * 1024, 64 * 1024] {
        let token = "S".repeat(size);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server_task = tokio::spawn(serve(listener, token.clone()));
        let mut configured = remote_server(&format!("{origin}/mcp"));
        configured.credential_headers.insert(
            "Authorization".into(),
            McpCredentialHeaderConfig {
                scheme: Some("Bearer".into()),
                reference: "host:large-token".into(),
            },
        );
        let executor = McpExecutor::new(
            &McpConfig {
                servers: BTreeMap::from([("fixture".into(), configured)]),
                ..Default::default()
            },
            Path::new("."),
            "native",
            Arc::new(McpEffectShapeExecutor {
                reference: "host:large-token",
            }),
        )
        .unwrap()
        .with_credentials(Arc::new(
            HostCredentialResolver::new([("large-token".into(), token.clone())]).unwrap(),
        ));
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(
                BuiltInPolicy::offline_default()
                    .with_action("mcp.tools", DecisionOutcome::Allow)
                    .with_action("mcp.call", DecisionOutcome::Allow)
                    .with_post_effect(false)
                    .with_sandbox("native", "large-token-regression", false)
                    .with_network_destination(&origin)
                    .with_limits(10_000, 1024 * 1024, 1, 64 * 1024 * 1024, 1),
            ),
            Arc::new(AllowApproval {
                approved_by: "test".into(),
            }),
            SafetyKernel::new(["mcp.invoke".into()]),
            [9; 32],
        );
        let actor = || Actor {
            actor_type: ActorType::System,
            id: "large-token-test".into(),
        };
        let request = executor
            .request(
                actor(),
                ExecutionContext::default(),
                McpOperation::ListTools {
                    server: "fixture".into(),
                    cursor: None,
                },
            )
            .unwrap();
        assert!(!serde_json::to_string(&request).unwrap().contains(&token));
        let page = gateway.execute(request, &executor).await.unwrap();
        let page: McpToolsPage = serde_json::from_slice(&page.bytes).unwrap();
        let tool = page.tools.into_iter().next().unwrap();
        let request = executor
            .request(
                actor(),
                ExecutionContext::default(),
                McpOperation::CallTool {
                    server: "fixture".into(),
                    tool: tool.name,
                    description: tool.description,
                    annotations: tool.annotations,
                    arguments: json!({}),
                    input_schema: Box::new(tool.input_schema),
                    schema_sha256: tool.schema_sha256,
                },
            )
            .unwrap();
        let result = gateway.execute(request, &executor).await.unwrap();
        let output = String::from_utf8(result.bytes).unwrap();
        assert!(!output.contains(&token));
        assert!(output.contains("echo <redacted>"));
        let count = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            count >= 8,
            "both MCP sessions completed with exact credentials"
        );
    }
}
