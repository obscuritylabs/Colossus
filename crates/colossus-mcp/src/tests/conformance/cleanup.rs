use super::*;
use std::time::Duration;

#[tokio::test]
async fn abandoned_legacy_requests_only_close_the_session() {
    for (method, operation) in [
        ("tools/call", call_operation(Some(output_schema()))),
        (
            "tools/list",
            McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (seen, dispatched) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut seen = Some(seen);
            let mut pending = None;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (headers, message) = request(&mut stream).await;
                match headers[":method"].as_str() {
                    "GET" => {
                        write_http_response(&mut stream, "405 Method Not Allowed", "", "").await
                    }
                    "DELETE" => {
                        assert_eq!(headers["mcp-session-id"], "deadline-session");
                        assert!(pending.is_some(), "session closed before the operation");
                        write_http_response(&mut stream, "200 OK", "", "").await;
                        break;
                    }
                    "POST" => match message["method"].as_str().unwrap() {
                        "initialize" => {
                            let body = json!({"jsonrpc":"2.0", "id":message["id"], "result":{
                                "protocolVersion":"2025-11-25", "capabilities":{"tools":{}},
                                "serverInfo":{"name":"fixture", "version":"1"}
                            }});
                            write_http_response(
                                &mut stream,
                                "200 OK",
                                "Content-Type: application/json\r\nMcp-Session-Id: deadline-session\r\n",
                                &body.to_string(),
                            )
                            .await;
                        }
                        "notifications/initialized" => {
                            write_http_response(&mut stream, "202 Accepted", "", "").await;
                        }
                        request_method if request_method == method => {
                            pending = Some(stream);
                            seen.take()
                                .expect("single operation POST")
                                .send(())
                                .unwrap();
                        }
                        request_method => panic!("unexpected POST {request_method}"),
                    },
                    verb => panic!("unexpected HTTP method {verb}"),
                }
            }
        });
        let http = HardenedStreamableHttpClient::for_test(
            endpoint.clone(),
            reqwest::Client::builder().no_proxy().build().unwrap(),
            1024 * 1024,
        );
        let server = configured_http_server(endpoint);
        let call_dispatched = AtomicBool::new(false);
        {
            let response = execute_remote_operation(
                http,
                &server,
                &operation,
                HashMap::new(),
                &call_dispatched,
            );
            tokio::pin!(response);
            tokio::select! {
                _ = &mut response => panic!("operation completed before the response was sent"),
                seen = tokio::time::timeout(Duration::from_secs(5), dispatched) => {
                    seen.expect("operation dispatched before deadline").unwrap();
                }
            }
            // The operation's future is dropped with the POST still pending.
        }
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("legacy session cleanup completes")
            .unwrap();
    }
}

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
