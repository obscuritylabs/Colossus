use super::*;
use crate::test_support::private_tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn round_trip(streamed: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::<Value>::new();
        for step in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (start, len) = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(at) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..at]);
                    assert!(header.starts_with("POST /v1/messages HTTP/1.1"));
                    let len = header
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= at + 4 + len {
                        break (at + 4, len);
                    }
                }
            };
            requests.push(serde_json::from_slice(&bytes[start..start + len]).unwrap());
            let content = if step < 2 {
                let mut blocks = vec![
                    json!({"type":"thinking","thinking":format!("PRIVATE_THOUGHT_{step}"),"signature":format!("EXACT_SIGNATURE_{step}")}),
                ];
                blocks.push(json!({"type":"tool_use","id":format!("call-{step}"),"name":"filesystem_list","input":{"path":"."}}));
                if step == 0 {
                    blocks.push(json!({"type":"tool_use","id":"parallel-call","name":"filesystem_list","input":{"path":"."}}));
                }
                blocks
            } else {
                vec![json!({"type":"text","text":"finished"})]
            };
            let stop = if step < 2 { "tool_use" } else { "end_turn" };
            let response = json!({"id":format!("msg-{step}"),"type":"message","role":"assistant","model":"native-test","content":content,"stop_reason":stop,"usage":{"input_tokens":20,"output_tokens":10}});
            let (mime, body) = if streamed {
                let mut events = vec![
                    json!({"type":"message_start","message":{"id":format!("msg-{step}"),"type":"message","role":"assistant","usage":{"input_tokens":20,"output_tokens":1}}}),
                ];
                for (index, block) in content.iter().enumerate() {
                    events.push(
                        json!({"type":"content_block_start","index":index,"content_block":block}),
                    );
                    events.push(json!({"type":"content_block_stop","index":index}));
                }
                events.push(json!({"type":"message_delta","delta":{"stop_reason":stop},"usage":{"output_tokens":10}}));
                events.push(json!({"type":"message_stop"}));
                (
                    "text/event-stream",
                    events
                        .into_iter()
                        .map(|e| format!("data: {e}\n\n"))
                        .collect::<String>(),
                )
            } else {
                ("application/json", response.to_string())
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let temp = private_tempdir();
    let mut config = RuntimeConfig::offline_template(temp.path().join("state.redb"));
    config.sandbox.backend = "danger_full_access".into();
    config.sandbox.acknowledge_danger_full_access = true;
    config.providers.profiles.insert(
        "native".into(),
        ProviderProfileConfig {
            kind: colossus_provider::ProviderKind::AnthropicMessages,
            base_url: Some(base),
            credential_reference: None,
            timeout_ms: Some(10_000),
            generation_timeout_ms: None,
            chat_completions_output_token_parameter: None,
        },
    );
    let model = config.models.profiles.get_mut("echo").unwrap();
    model.provider_profile = "native".into();
    model.model = "native-test".into();
    model.capabilities = crate::ModelFeatureSettings::default();
    model.capabilities.streaming = streamed.into();
    config.memory.index_enabled = false;
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(temp.path()).unwrap(),
    )
    .unwrap();
    let session = runtime
        .create_session(Some("native thinking round trip"))
        .unwrap();
    let result = runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            "List the workspace",
            Some(5),
            &session.id,
        )
        .await;
    if result.is_err() {
        server.abort();
    }
    result.unwrap();
    runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            "New human turn",
            Some(1),
            &session.id,
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert!(requests.iter().all(|r| r["stream"] == streamed));
    assert!(!requests[0].to_string().contains("EXACT_SIGNATURE"));
    for request in requests.iter().take(3).skip(1) {
        let wire = request.to_string();
        assert!(wire.contains("EXACT_SIGNATURE_0"));
        assert!(wire.contains("PRIVATE_THOUGHT_0"));
        let last = request["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["type"], "tool_result");
    }
    assert!(requests[2].to_string().contains("EXACT_SIGNATURE_1"));
    assert!(!requests[3].to_string().contains("EXACT_SIGNATURE"));
    let canonical = runtime.sessions.list_messages(&session.id).unwrap();
    let visible =
        serde_json::to_string(&canonical.into_iter().map(|m| m.message).collect::<Vec<_>>())
            .unwrap();
    assert!(!visible.contains("PRIVATE_THOUGHT"));
    assert!(!visible.contains("EXACT_SIGNATURE"));
}

#[tokio::test]
async fn anthropic_json_thinking_parallel_tools_and_second_exchange() {
    round_trip(false).await;
}

#[tokio::test]
async fn anthropic_sse_thinking_parallel_tools_and_second_exchange() {
    round_trip(true).await;
}
