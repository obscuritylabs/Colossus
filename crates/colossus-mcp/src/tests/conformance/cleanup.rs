use super::*;

#[tokio::test]
async fn legacy_cleanup_does_not_delay_confirmed_results_or_repeat_operations() {
    for is_call in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_cleanup, cleanup_released) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut operations = 0;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (headers, message) = request(&mut stream).await;
                match headers[":method"].as_str() {
                    "GET" => {
                        write_http_response(&mut stream, "405 Method Not Allowed", "", "").await;
                        continue;
                    }
                    "DELETE" => {
                        assert_eq!(headers["mcp-session-id"], "cleanup-session");
                        assert_eq!(operations, 1, "cleanup must not reissue an operation");
                        // Cleanup cannot finish until the caller receives its result.
                        cleanup_released.await.unwrap();
                        write_http_response(&mut stream, "200 OK", "", "").await;
                        break;
                    }
                    _ => {}
                }
                let (extra, result) = match message["method"].as_str().unwrap() {
                    "initialize" => (
                        "Mcp-Session-Id: cleanup-session\r\n",
                        json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}}, "serverInfo":{"name":"fixture", "version":"1"}}),
                    ),
                    "notifications/initialized" => {
                        write_http_response(&mut stream, "202 Accepted", "", "").await;
                        continue;
                    }
                    "tools/list" => {
                        assert!(!is_call);
                        operations += 1;
                        (
                            "",
                            json!({"tools":[{"name":"probe", "inputSchema":input_schema()}]}),
                        )
                    }
                    "tools/call" => {
                        assert!(is_call);
                        operations += 1;
                        ("", json!({"content":[], "structuredContent":{"count":3}}))
                    }
                    method => panic!("unexpected legacy request {method}"),
                };
                let body = json!({"jsonrpc":"2.0", "id":message["id"], "result":result});
                write_http_response(
                    &mut stream,
                    "200 OK",
                    &format!("Content-Type: application/json\r\n{extra}"),
                    &body.to_string(),
                )
                .await;
            }
        });
        let mut server = remote_server(&format!("http://{address}/mcp"));
        server.timeout_ms = Some(500);
        let executor = McpExecutor::new(
            &McpConfig {
                servers: BTreeMap::from([("fixture".into(), server)]),
                ..McpConfig::default()
            },
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
                    .with_action("mcp.call", DecisionOutcome::Allow)
                    .with_post_effect(false)
                    .with_sandbox("native", "mcp-cleanup-regression", false)
                    .with_limits(5000, 1024 * 1024, 4, 64 * 1024 * 1024, 1)
                    .with_network_destination(format!("http://{address}")),
            ),
            Arc::new(AllowApproval {
                approved_by: "test".into(),
            }),
            SafetyKernel::new(["mcp.invoke".into()]),
            [84_u8; 32],
        );
        let operation = if is_call {
            call_operation(Some(output_schema()))
        } else {
            McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            }
        };
        let effect = executor
            .request(
                Actor {
                    actor_type: ActorType::System,
                    id: "mcp-cleanup-regression".into(),
                },
                ExecutionContext::default(),
                operation,
            )
            .unwrap();
        let result = gateway.execute(effect, &executor).await;
        release_cleanup.send(()).unwrap();
        let result = result.expect("confirmed result must survive an unfinished session DELETE");
        let result: Value = serde_json::from_slice(&result.bytes).unwrap();
        if is_call {
            assert_eq!(result["result"]["structuredContent"]["count"], 3);
        } else {
            assert_eq!(result["tools"][0]["name"], "probe");
        }
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("best-effort session cleanup must still be attempted")
            .unwrap();
    }
}
