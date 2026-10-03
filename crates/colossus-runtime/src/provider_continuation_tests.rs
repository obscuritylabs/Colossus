use super::*;
use crate::test_support::private_tempdir;
use colossus_contracts::ModelFeatureMode;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn responses_server(
    streamed: bool,
    reject_first: bool,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (offset, length) = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..offset]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        break (offset + 4, length);
                    }
                }
            };
            requests.push(serde_json::from_slice(&bytes[offset..offset + length]).unwrap());
            let mut output = vec![
                json!({"type":"message", "id":format!("message-{index}"), "role":"assistant", "status":"completed", "content":[{"type":"output_text", "text":"done"}]}),
            ];
            if index == 0 {
                output.insert(0, json!({"type":"compaction", "id":"cmp-1", "encrypted_content":"opaque-sentinel-do-not-release"}));
            }
            let response =
                json!({"id":format!("response-{index}"),"status":"completed","output":output});
            let (content_type, body) = if streamed {
                (
                    "text/event-stream",
                    format!(
                        "data: {}\n\ndata: {}\n\n",
                        json!({"type":"response.output_text.delta", "delta":"done"}),
                        json!({"type":"response.completed", "response": response})
                    ),
                )
            } else {
                ("application/json", response.to_string())
            };
            let (status, content_type, body) = if reject_first && index == 0 {
                ("400 Bad Request", "application/json", json!({"error":{"code":"unsupported_parameter","param":"context_management","message":"untrusted rejection body"}}).to_string())
            } else {
                ("200 OK", content_type, body)
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (url, handle)
}

async fn compaction_round_trip(streamed: bool, reject_first: bool) {
    let (base_url, server) = responses_server(streamed, reject_first).await;
    let temporary = private_tempdir();
    let mut config = RuntimeConfig::offline_template(temporary.path().join("state.redb"));
    config.sandbox.backend = "danger_full_access".into();
    config.sandbox.acknowledge_danger_full_access = true;
    config
        .sandbox
        .network_destinations
        .push(base_url.trim_end_matches("/v1").into());
    config.providers.profiles.insert(
        "responses".into(),
        ProviderProfileConfig {
            kind: ProviderKind::OpenAiResponses,
            base_url: Some(base_url),
            credential_reference: None,
            timeout_ms: Some(10_000),
            generation_timeout_ms: None,
            chat_completions_output_token_parameter: None,
        },
    );
    config
        .models
        .profiles
        .get_mut("echo")
        .unwrap()
        .provider_profile = "responses".into();
    let model = config.models.profiles.get_mut("echo").unwrap();
    model.capabilities = crate::ModelFeatureSettings::default();
    model.capabilities.tool_calls = ModelFeatureMode::Off;
    model.capabilities.streaming = streamed.into();
    config.memory.index_enabled = false;
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(temporary.path()).unwrap(),
    )
    .unwrap();
    let session = runtime.create_session(Some("server compaction")).unwrap();
    if reject_first {
        let error = runtime
            .run_model_in_session(
                "primary",
                "test instructions",
                "rejected",
                Some(1),
                &session.id,
            )
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("untrusted rejection body"));
        for prompt in ["explicit next request", "still unsupported"] {
            runtime
                .run_model_in_session("primary", "test instructions", prompt, Some(1), &session.id)
                .await
                .unwrap();
        }
        let requests = server.await.unwrap();
        assert!(requests[0].get("context_management").is_some());
        assert!(requests[1].get("context_management").is_none());
        assert!(requests[2].get("context_management").is_none());
        assert_eq!(
            config.models.profiles["echo"]
                .capabilities
                .server_compaction,
            ModelFeatureMode::Auto
        );
        return;
    }
    for prompt in ["first unique prompt", "second unique prompt"] {
        runtime
            .run_model_in_session("primary", "test instructions", prompt, Some(1), &session.id)
            .await
            .unwrap();
    }
    // A change in instructions invalidates state before the next request.
    runtime
        .run_model_in_session(
            "primary",
            "changed instructions",
            "third unique prompt",
            Some(1),
            &session.id,
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["store"], false);
    assert_eq!(
        requests[0]["context_management"][0]["compact_threshold"],
        runtime
            .providers
            .resolve("primary")
            .unwrap()
            .route()
            .limits
            .input_budget_tokens
            * 75
            / 100
    );
    assert_eq!(
        requests[1]["input"][0]["encrypted_content"],
        "opaque-sentinel-do-not-release"
    );
    assert!(
        !requests[1]["input"]
            .to_string()
            .contains("first unique prompt")
    );
    assert!(
        requests[1]["input"]
            .to_string()
            .contains("second unique prompt")
    );
    assert!(!requests[2]["input"].to_string().contains("opaque-sentinel"));
    assert!(
        requests[2]["input"]
            .to_string()
            .contains("first unique prompt")
    );
    // Keyless journal never stores opaque bytes, including effect/run events.
    let head = runtime.journal.head().unwrap().0;
    let events = runtime
        .journal
        .read_global(1, usize::try_from(head).unwrap())
        .unwrap();
    for event in events {
        assert!(
            !runtime
                .journal
                .decrypt_payload(&event)
                .unwrap()
                .to_string()
                .contains("opaque-sentinel")
        );
    }
}

#[tokio::test]
async fn responses_compaction_chains_nonstream_and_invalidates_changed_instructions() {
    compaction_round_trip(false, false).await;
}

#[tokio::test]
async fn responses_compaction_chains_stream_and_keeps_opaque_state_private() {
    compaction_round_trip(true, false).await;
}

#[tokio::test]
async fn responses_compaction_auto_remembers_only_structured_rejection_without_replaying_generation()
 {
    compaction_round_trip(false, true).await;
    compaction_round_trip(true, true).await;
}
