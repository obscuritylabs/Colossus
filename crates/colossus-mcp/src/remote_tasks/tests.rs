use super::*;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::TcpListener,
};

const TASK_ID: &str = "fixture-durable-task-57733209bc09411f885a710620caeab9";

#[tokio::test]
async fn direct_requests_route_one_call_and_follow_the_handle_on_new_connections() {
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let fixture = tokio::spawn(serve_fixture(listener, cancel));
        let http = HardenedStreamableHttpClient::for_test(
            endpoint.clone(),
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            16 * 1024,
        );
        let headers = HashMap::from([(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture-credential"),
        )]);
        let call = CallToolRequest::new(CallToolRequestParams::new("probe"));
        let TaskCallResult::Started(started) = call_tool(&http, &endpoint, headers.clone(), call)
            .await
            .unwrap()
        else {
            panic!("expected a task handle");
        };
        assert_eq!(started.status, TaskStatus::Working);
        assert_eq!(started.poll_interval, Some(Duration::from_millis(100)));
        let first = get_task(&http, &endpoint, headers.clone(), &started.handle)
            .await
            .unwrap();
        assert!(matches!(first.state, TaskState::Working));
        assert_eq!(first.poll_interval, Some(Duration::from_millis(200)));
        if cancel {
            cancel_task(&http, &endpoint, headers.clone(), &started.handle)
                .await
                .unwrap();
        }
        let final_state = get_task(&http, &endpoint, headers.clone(), &started.handle)
            .await
            .unwrap();
        if cancel {
            assert!(matches!(final_state.state, TaskState::Cancelled));
            assert!(matches!(
                get_task(&http, &endpoint, headers, &started.handle).await,
                Err(TaskRequestError::Unavailable)
            ));
        } else {
            let TaskState::Completed(result) = final_state.state else {
                panic!("expected a completed tool result");
            };
            assert_eq!(result.is_error, Some(true));
            assert_eq!(result.structured_content, Some(json!({"count": 3})));
        }
        fixture.await.unwrap();
    }
}

async fn serve_fixture(listener: TcpListener, cancel: bool) {
    let final_result = if cancel {
        task("complete", "cancelled", 200)
    } else {
        let mut task = task("complete", "completed", 200);
        task["result"] = json!({"content": [], "structuredContent": {"count": 3}, "isError": true});
        task
    };
    let mut steps = vec![
        ("tools/call", task("task", "working", 100), true),
        ("tasks/get", task("complete", "working", 200), true),
    ];
    if cancel {
        steps.push(("tasks/cancel", json!({"resultType": "complete"}), true));
    }
    steps.push(("tasks/get", final_result, true));
    if cancel {
        steps.push((
            "tasks/get",
            json!({"code": -32602, "message": format!("Expired task {TASK_ID}")}),
            false,
        ));
    }
    let mut prior_id = Value::Null;
    for (index, (method, result, success)) in steps.into_iter().enumerate() {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut line = String::new();
        socket.read_line(&mut line).await.unwrap();
        assert_eq!(line.trim_end(), "POST /mcp HTTP/1.1");
        let mut headers = HashMap::new();
        loop {
            line.clear();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
        let mut bytes = vec![0; headers["content-length"].parse().unwrap()];
        socket.read_exact(&mut bytes).await.unwrap();
        let request: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request["method"], method);
        assert_eq!(headers["mcp-method"], method);
        assert_eq!(headers["mcp-protocol-version"], "2026-07-28");
        assert_eq!(headers["authorization"], "Bearer fixture-credential");
        assert!(!headers.contains_key("mcp-session-id"));
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"]
                ["io.modelcontextprotocol/tasks"],
            json!({})
        );
        if method == "tools/call" {
            assert_eq!(request["params"]["name"], "probe");
            assert_eq!(headers["mcp-name"], "probe");
            assert!(request["params"].get("task").is_none());
        } else {
            assert_eq!(request["params"]["taskId"], TASK_ID);
            assert_eq!(headers["mcp-name"], TASK_ID);
        }
        assert_ne!(request["id"], prior_id);
        prior_id = request["id"].clone();
        let result_key = if success { "result" } else { "error" };
        let body = json!({"jsonrpc": "2.0", "id": prior_id, (result_key): result}).to_string();
        let (status, content_type, body) = if !success {
            ("400 Bad Request", "application/json", body)
        } else if index % 2 == 0 {
            ("200 OK", "application/json", body)
        } else {
            (
                "200 OK",
                "text/event-stream",
                format!("event: message\ndata: {body}\n\n"),
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    }
}

fn task(result_type: &str, status: &str, poll_interval_ms: u64) -> Value {
    json!({
        "resultType": result_type,
        "taskId": TASK_ID,
        "status": status,
        "createdAt": "2026-07-28T12:00:00Z",
        "lastUpdatedAt": "2026-07-28T12:00:00Z",
        "ttlMs": 30_000,
        "pollIntervalMs": poll_interval_ms
    })
}

#[test]
fn observations_reject_wrong_handles_malformed_results_and_handle_echo_keys() {
    let handle = TaskHandle::new(TASK_ID.into()).unwrap();
    let mut wrong_handle = task("complete", "working", 100);
    wrong_handle["taskId"] = json!("different-task");
    let mut missing_result = task("complete", "completed", 100);
    missing_result["result"] = json!({"unrecognized": true});
    let mut echoed_key = task("complete", "completed", 100);
    echoed_key["result"] = json!({"structuredContent": { (TASK_ID): 3 }});
    for result in [wrong_handle, missing_result, echoed_key] {
        assert!(matches!(
            observation(serde_json::from_value(result).unwrap(), &handle),
            Err(TaskRequestError::Protocol)
        ));
    }
    let mut input_required = task("complete", "input_required", 100);
    input_required["inputRequests"] = json!({
        "question": {
            "method": "elicitation/create",
            "params": {"mode": "form", "message": "What name?", "requestedSchema": {
                "type": "object", "properties": {"name": {"type":"string"}}
            }}
        }
    });
    let observed = observation(serde_json::from_value(input_required).unwrap(), &handle).unwrap();
    assert!(matches!(observed.state, TaskState::InputRequired));
}

#[test]
fn handles_redact_debug_and_use_nonce_bound_receipts() {
    let handle = TaskHandle::new(TASK_ID.into()).unwrap();
    assert!(!format!("{handle:?}").contains(TASK_ID));
    assert_ne!(handle.binding(&[1; 32]), handle.binding(&[2; 32]));
    assert_ne!(
        handle.binding(&[1; 32]),
        TaskHandle::new("another-fixture-handle".into())
            .unwrap()
            .binding(&[1; 32])
    );
    assert!(TaskHandle::new("a".repeat(MAX_TASK_ID_BYTES + 1)).is_err());
    let mut secrets = Vec::new();
    let extended_id = TaskHandle::new(" opaque-λ".into()).unwrap();
    extended_id.protect_output(&mut secrets);
    assert_eq!(secrets[0], " opaque-λ");
    assert_eq!(secrets[1], "=?base64?IG9wYXF1ZS3Ouw==?=");
    assert!(routing_name(" opaque-λ").unwrap().is_sensitive());
}
