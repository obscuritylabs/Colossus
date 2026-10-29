use super::*;
use colossus_api::PredictNextPromptRequest;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
mod concurrent;
mod timeout;

async fn provider(listener: TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for text in [
        "Implemented cookie persistence. Next steps:\n- Review the cookie restore path.",
        "Review the cookie restore path",
        "The second conversation turn completed.",
    ] {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let (start, length) = loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() < 1_048_576);
            if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break (end + 4, length);
                }
            }
        };
        requests.push(serde_json::from_slice(&bytes[start..start + length]).unwrap());
        let response = json!({"id":"prediction-fixture", "object":"chat.completion", "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}], "usage":{"prompt_tokens":18,"completion_tokens":7,"total_tokens":25}}).to_string();
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
    }
    requests
}

fn fixture(origin: String, provider_timeout_ms: u64) -> RuntimeFixture {
    let directory = runtime_tempdir();
    let root = fs::canonicalize(directory.path()).unwrap();
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("workflows");
    config.workflows.user = root.join("workflows");
    fs::create_dir_all(&config.workflows.repository).unwrap();
    config.access = serde_json::from_value(json!({"profile":"pinned","tools":{"include":[]},"actions":{"allow":["provider.openai.chat"]}})).unwrap();
    config.providers = serde_json::from_value(json!({"profiles":{"fixture":{"kind":"open_ai_compatible","baseUrl":format!("{origin}/v1"),"timeoutMs":provider_timeout_ms}}})).unwrap();
    config.models = serde_json::from_value(json!({"profiles":{"fixture":{"providerProfile":"fixture","model":"prediction-fixture","contextWindowTokens":32768,"maxOutputTokens":4096,"capabilities":{"toolCalls":false,"streaming":false}}},"roles":{"primary":"fixture"}})).unwrap();
    config.sandbox.network_destinations = vec![origin];
    config.sandbox.timeout_ms = 30_000;
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(&root).unwrap(),
        )
        .unwrap(),
    );
    RuntimeFixture {
        runtime,
        _directory: directory,
    }
}

#[tokio::test]
async fn prediction_is_owned_deduped_accounted_and_never_a_conversation_turn() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(20), provider(listener))
            .await
            .unwrap()
    });
    let fixture = fixture(origin, 5000);
    let runtime = fixture.runtime.clone();
    // Exercise several immediate replay/authority probes without consuming the
    // production listing burst intended for human-paced UI traffic.
    let admission = RunAdmissionConfig::default()
        .with_list_limits(64, 8, 100, 100, 100, 100)
        .unwrap();
    let api = service(runtime.clone(), admission.clone());
    let owner = caller("app:prediction-owner", "prediction-owner");
    let created = api
        .create_run(
            &owner,
            request("prediction-first", "Implement persistent browser cookies."),
        )
        .await
        .unwrap();
    let source = wait_terminal(&api, &owner, &created.run.id).await;
    assert_eq!(source.status, RunStatus::Completed);
    runtime
        .append_application_message(
            &source.session_id,
            &source.id,
            colossus_contracts::ModelMessage {
                agent_message_origin: Some(colossus_contracts::AgentMessageOrigin {
                    message_id: "private-peer-message".into(),
                    sender: colossus_contracts::AgentMessageSender::Application {
                        application_id: "app:private-peer".into(),
                    },
                    recipient_id: "private-attempt".into(),
                }),
                role: ModelMessageRole::User,
                content: "private coordination must never be used for composer prediction".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            owner.actor(),
        )
        .unwrap();
    let predict = PredictNextPromptRequest {
        source_run_id: source.id.clone(),
    };
    let before = runtime.get_session(&source.session_id).unwrap().unwrap();
    let public_before = api
        .get_run(
            &owner,
            GetRunRequest {
                run_id: source.id.clone(),
            },
        )
        .await
        .unwrap();
    let repository = EventSourcedRunRepository::new(runtime.journal());
    let updates_before = repository
        .updates_after(&owner, &source.id, 0, 128)
        .unwrap();
    assert!(
        api.predict_next_prompt(
            &caller("app:prediction-stranger", "stranger"),
            predict.clone()
        )
        .await
        .is_err()
    );
    let read_only =
        caller_with_exact_scopes("app:prediction-owner", "read-only", &[scopes::RUNS_READ]);
    assert!(
        api.predict_next_prompt(&read_only, predict.clone())
            .await
            .is_err()
    );
    let prediction = api
        .predict_next_prompt(&owner, predict.clone())
        .await
        .unwrap();
    assert_eq!(
        prediction.text.as_deref(),
        Some("Review the cookie restore path")
    );
    assert_eq!(prediction.usage.as_ref().unwrap().total_tokens, 25);
    assert_eq!(
        runtime
            .get_session(&source.session_id)
            .unwrap()
            .unwrap()
            .message_count,
        before.message_count
    );
    let public_after = api
        .get_run(
            &owner,
            GetRunRequest {
                run_id: source.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(public_after.last_sequence, public_before.last_sequence);
    assert_eq!(public_after.etag, public_before.etag);
    assert_eq!(
        public_after, public_before,
        "prediction must not mutate source run state"
    );
    assert_eq!(
        repository
            .updates_after(&owner, &source.id, 0, 128)
            .unwrap(),
        updates_before,
        "source usage updates must exclude auxiliary prediction tokens"
    );
    assert_eq!(
        api.predict_next_prompt(&owner, predict.clone())
            .await
            .unwrap(),
        prediction
    );
    let recreated = service(runtime.clone(), admission);
    assert_eq!(
        recreated
            .predict_next_prompt(&owner, predict.clone())
            .await
            .unwrap(),
        prediction
    );
    let mut next = request("prediction-second", "Continue with the next task.");
    next.session_id = Some(source.session_id.clone());
    let next = api.create_run(&owner, next).await.unwrap();
    let next_source = wait_terminal(&api, &owner, &next.run.id).await;
    assert_eq!(next_source.status, RunStatus::Completed);
    assert!(
        api.predict_next_prompt(&owner, predict)
            .await
            .unwrap()
            .text
            .is_none()
    );
    let requests = server.await.unwrap();
    // The fixture listener has closed: provider failure stays optional and its
    // durable attempt cannot be repeated by a new request or API facade.
    let failed = PredictNextPromptRequest {
        source_run_id: next_source.id.clone(),
    };
    let count_before_failure = runtime
        .get_session(&next_source.session_id)
        .unwrap()
        .unwrap()
        .message_count;
    let unavailable = api
        .predict_next_prompt(&owner, failed.clone())
        .await
        .unwrap();
    assert!(unavailable.text.is_none());
    assert_eq!(
        recreated.predict_next_prompt(&owner, failed).await.unwrap(),
        unavailable
    );
    assert_eq!(
        runtime
            .get_session(&next_source.session_id)
            .unwrap()
            .unwrap()
            .message_count,
        count_before_failure
    );
    assert_eq!(requests.len(), 3);
    let prediction_request = &requests[1];
    assert!(
        prediction_request
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );
    assert_eq!(
        prediction_request
            .get("max_tokens")
            .or_else(|| prediction_request.get("max_completion_tokens"))
            .and_then(Value::as_u64),
        Some(96)
    );
    let context = prediction_request["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(context.contains("Implement persistent browser cookies"));
    assert!(!context.contains("deterministic conformance test"));
    assert!(!context.contains("private coordination"));
}
