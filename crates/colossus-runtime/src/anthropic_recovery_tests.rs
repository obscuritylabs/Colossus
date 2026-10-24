use super::*;
use colossus_journal_redb::StaticKeyProvider;
use colossus_ports::{ProviderContinuationRepository as _, RuntimeExecutionFence};
use colossus_testkit::InMemoryProjectionStore;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
};

struct LiveFence;

impl RuntimeExecutionFence for LiveFence {
    fn verify(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "provider request ended before its body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 2 * 1024 * 1024);
        if let Some(at) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..at]);
            assert!(header.starts_with("POST /v1/messages HTTP/1.1"));
            let length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= at + 4 + length {
                return serde_json::from_slice(&bytes[at + 4..at + 4 + length]).unwrap();
            }
        }
    }
}

fn signed_title(step: usize) -> Vec<Value> {
    vec![
        json!({"type":"thinking","thinking":format!("PRIVATE_RECOVERY_THOUGHT_{step}"),
            "signature":format!("EXACT_RECOVERY_SIGNATURE_{step}")}),
        json!({"type":"tool_use","id":format!("title-{step}"),"name":"session_set_title",
            "input":{"title":format!("Recovery title {step}")}}),
    ]
}

fn response_body(step: usize, content: &[Value], stop: &str, streamed: bool) -> String {
    if !streamed {
        return json!({"id":format!("msg-recovery-{step}"),"type":"message",
            "role":"assistant","model":"native-recovery","content":content,
            "stop_reason":stop,"usage":{"input_tokens":10,"output_tokens":3}})
        .to_string();
    }
    let mut events = vec![json!({"type":"message_start","message":{
        "id":format!("msg-recovery-{step}"),"type":"message","role":"assistant",
        "usage":{"input_tokens":10,"output_tokens":1}}})];
    for (index, block) in content.iter().enumerate() {
        if block["type"] == "tool_use" && block["input"] == "PRIVATE_INVALID_ARGUMENT" {
            let mut initial = block.clone();
            initial["input"] = json!({});
            events
                .push(json!({"type":"content_block_start","index":index,"content_block":initial}));
            events.push(json!({"type":"content_block_delta","index":index,
                "delta":{"type":"input_json_delta","partial_json":"{\"PRIVATE_INVALID_ARGUMENT\":"}}));
            events.push(json!({"type":"content_block_stop","index":index}));
            continue;
        }
        events.push(json!({"type":"content_block_start","index":index,"content_block":block}));
        events.push(json!({"type":"content_block_stop","index":index}));
    }
    events.push(json!({"type":"message_delta","delta":{"stop_reason":stop},
        "usage":{"output_tokens":3}}));
    events.push(json!({"type":"message_stop"}));
    events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

async fn scripted_server(
    streamed: bool,
    malformed: bool,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for step in 0..if malformed { 5 } else { 7 } {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(20), listener.accept())
                .await
                .expect("expected the next bounded provider request")
                .unwrap();
            requests.push(read_request(&mut socket).await);
            let (status, mime, body) = if !malformed && matches!(step, 2 | 3) {
                (
                    404,
                    "application/json",
                    json!({"error":"fixture unavailable"}).to_string(),
                )
            } else {
                let invalid = malformed && matches!(step, 0 | 2);
                let tools = if malformed {
                    step < 4
                } else {
                    matches!(step, 0 | 1 | 4)
                };
                let content = if invalid {
                    vec![json!({"type":"tool_use","id":format!("invalid-{step}"),
                        "name":"session_set_title","input":"PRIVATE_INVALID_ARGUMENT"})]
                } else if tools {
                    signed_title(step)
                } else {
                    vec![
                        json!({"type":"text","text":if step == 5 || malformed {"recovered"} else {"normal turn"}}),
                    ]
                };
                (
                    200,
                    if streamed {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                    response_body(
                        step,
                        &content,
                        if tools { "tool_use" } else { "end_turn" },
                        streamed,
                    ),
                )
            };
            socket.write_all(format!("HTTP/1.1 {status} Response\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (base, server)
}

fn recovery_config(
    workspace: &Path,
    base: String,
    streamed: bool,
    protected: bool,
) -> RuntimeConfig {
    let mut config = RuntimeConfig::controlled_chat_template(workspace.join("state.redb"));
    if protected {
        // The injected host storage owns keys. These references select protected
        // continuation persistence without reading or mutating process environment.
        config.storage.keys = KeyConfig::Environment {
            journal_variable: "COLOSSUS_UNUSED_RECOVERY_JOURNAL".into(),
            journal_key_id: "recovery-test".into(),
            signing_variable: "COLOSSUS_UNUSED_RECOVERY_SIGNER".into(),
            anchor_path: workspace.join("unused-anchor.json"),
        };
    } else {
        config.use_ephemeral_storage();
    }
    let origin = canonical_network_origin(&base).unwrap();
    config.sandbox.network_destinations = vec![origin];
    config.providers.profiles.insert(
        "native".into(),
        ProviderProfileConfig {
            kind: ProviderKind::AnthropicMessages,
            base_url: Some(base),
            credential_reference: None,
            timeout_ms: Some(10_000),
            generation_timeout_ms: None,
            chat_completions_output_token_parameter: None,
        },
    );
    let model = config.models.profiles.get_mut("echo").unwrap();
    model.provider_profile = "native".into();
    model.model = "native-recovery".into();
    model.capabilities = ModelFeatureSettings::default();
    model.capabilities.streaming = streamed.into();
    config.memory.index_enabled = false;
    config
}

fn open_runtime(
    config: &RuntimeConfig,
    workspace: &Path,
    keys: Option<&Arc<StaticKeyProvider>>,
) -> Runtime {
    let mut options = RuntimeOpenOptions::for_controlled_chat(workspace).unwrap();
    if let Some(keys) = keys {
        let journal = Arc::new(
            RedbEventJournal::open(
                &config.storage.path,
                keys.clone(),
                Arc::new(Ed25519CheckpointSigner::new("recovery-signer", [9; 32])),
            )
            .unwrap(),
        );
        let storage = RuntimeSharedStorage::new(
            "anthropic-recovery".into(),
            journal,
            Arc::new(InMemoryProjectionStore::default()),
            keys.clone(),
            Arc::new(LiveFence),
        )
        .unwrap();
        options = options.with_shared_storage(storage).unwrap();
    }
    Runtime::open_with_options(config, Arc::new(DenyApproval), None, options).unwrap()
}

fn flattened_blocks(request: &Value) -> Vec<(Value, Value)> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| {
            message["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|block| (message["role"].clone(), block.clone()))
        })
        .collect()
}

fn assert_private_blocks_absent(runtime: &Runtime, session: &str, tool_count: usize) {
    let messages = runtime.session_messages(session).unwrap();
    let visible = serde_json::to_string(&messages).unwrap();
    assert!(!visible.contains("PRIVATE_RECOVERY_THOUGHT"));
    assert!(!visible.contains("EXACT_RECOVERY_SIGNATURE"));
    assert_eq!(
        messages
            .iter()
            .map(|record| record.message.tool_calls.len())
            .sum::<usize>(),
        tool_count
    );
    assert_eq!(
        messages
            .iter()
            .filter(|record| record.message.role == ModelMessageRole::Tool)
            .count(),
        tool_count
    );
    let titles = runtime
        .journal
        .read_stream(&format!("session:{session}"))
        .unwrap();
    assert_eq!(
        titles
            .iter()
            .filter(|event| event.event_type == "session.title.set.v1")
            .count(),
        tool_count
    );
}

async fn repeated_failure_round_trip(streamed: bool, protected_restart: bool) {
    let (base, server) = scripted_server(streamed, false).await;
    let workspace = crate::test_support::private_tempdir();
    let config = recovery_config(workspace.path(), base, streamed, protected_restart);
    let keys =
        protected_restart.then(|| Arc::new(StaticKeyProvider::new("recovery-test", [7; 32])));
    let mut runtime = open_runtime(&config, workspace.path(), keys.as_ref());
    let result = runtime
        .run_model_with_max_turns(
            "primary",
            "test instructions",
            "Set up this recovery session",
            5,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("404"));
    let session = runtime.latest_session().unwrap().id;
    assert_private_blocks_absent(&runtime, &session, 2);
    let message_count = runtime.session_messages(&session).unwrap().len();
    let oversized_guidance = "g".repeat(896 * 1024);
    let guidance_error = runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            &oversized_guidance,
            Some(5),
            &session,
        )
        .await
        .unwrap_err();
    assert!(
        guidance_error.to_string().contains("input budget"),
        "{guidance_error}"
    );
    assert_eq!(
        runtime.session_messages(&session).unwrap().len(),
        message_count,
        "rejected guidance must not poison the canonical recovery prefix"
    );

    if protected_restart {
        drop(runtime);
        runtime = open_runtime(&config, workspace.path(), keys.as_ref());
    }
    let result = runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            "Continue",
            Some(5),
            &session,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("404"));
    assert_private_blocks_absent(&runtime, &session, 2);
    if protected_restart {
        drop(runtime);
        runtime = open_runtime(&config, workspace.path(), keys.as_ref());
    }
    let result = runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            "Continue",
            Some(5),
            &session,
        )
        .await
        .unwrap();
    assert_eq!(result.output, "recovered");
    assert_private_blocks_absent(&runtime, &session, 3);
    assert_eq!(
        runtime
            .get_session(&session)
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Recovery title 4")
    );
    if protected_restart {
        let continuations =
            colossus_context::EventSourcedProviderContinuations::new(runtime.journal.clone(), true);
        assert!(
            continuations.load(&session).unwrap().is_none(),
            "final output must retire durable private replay"
        );
    }
    let result = runtime
        .run_model_in_session(
            "primary",
            "test instructions",
            "Normal new user turn",
            Some(1),
            &session,
        )
        .await
        .unwrap();
    assert_eq!(result.output, "normal turn");
    assert_private_blocks_absent(&runtime, &session, 3);
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 7);
    assert!(requests.iter().all(|request| request["stream"] == streamed));
    assert!(
        requests[0]["system"]
            .to_string()
            .contains("[Colossus session title]")
    );
    for request in requests.iter().take(6).skip(1) {
        assert_eq!(
            request["system"], requests[0]["system"],
            "recovery changed signed system instructions"
        );
        assert_eq!(
            request["tools"], requests[0]["tools"],
            "recovery changed tool definitions"
        );
        assert!(request.to_string().contains("EXACT_RECOVERY_SIGNATURE_0"));
    }
    let failed_prefix = flattened_blocks(&requests[2]);
    for request in &requests[3..6] {
        assert!(
            flattened_blocks(request).starts_with(&failed_prefix),
            "recovery changed the accepted tool/result prefix"
        );
        assert!(request.to_string().contains("EXACT_RECOVERY_SIGNATURE_1"));
    }
    assert!(
        requests[5]
            .to_string()
            .contains("EXACT_RECOVERY_SIGNATURE_4")
    );
    assert!(!requests[6].to_string().contains("EXACT_RECOVERY_SIGNATURE"));
    assert!(
        !requests[6]["system"]
            .to_string()
            .contains("[Colossus session title]")
    );
}

#[tokio::test]
async fn anthropic_recovery_repeated_http_failures_keep_keyless_signed_replay() {
    for streamed in [false, true] {
        repeated_failure_round_trip(streamed, false).await;
    }
}

#[tokio::test]
async fn anthropic_recovery_repeated_http_failures_keep_protected_signed_replay_after_restart() {
    for streamed in [false, true] {
        repeated_failure_round_trip(streamed, true).await;
    }
}

#[tokio::test]
async fn anthropic_recovery_invalid_arguments_before_and_after_a_signed_tool_checkpoint() {
    for streamed in [false, true] {
        let (base, server) = scripted_server(streamed, true).await;
        let workspace = crate::test_support::private_tempdir();
        let config = recovery_config(workspace.path(), base, streamed, false);
        let runtime = open_runtime(&config, workspace.path(), None);
        let result = runtime
            .run_model_with_max_turns(
                "primary",
                "test instructions",
                "Set up this recovery session",
                6,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "recovered");
        let session = result.session_id.as_deref().unwrap();
        assert_private_blocks_absent(&runtime, session, 2);
        let canonical = serde_json::to_string(&runtime.session_messages(session).unwrap()).unwrap();
        assert!(!canonical.contains("invalid-"));
        assert!(!canonical.contains("PRIVATE_INVALID_ARGUMENT"));
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 5);
        for request in &requests {
            assert_eq!(request["system"], requests[0]["system"]);
            assert_eq!(request["tools"], requests[0]["tools"]);
        }
        assert!(
            requests[1]["messages"]
                .to_string()
                .contains("No tool was executed")
        );
        let prefix = flattened_blocks(&requests[2]);
        for request in &requests[3..] {
            assert!(flattened_blocks(request).starts_with(&prefix));
            assert!(request.to_string().contains("EXACT_RECOVERY_SIGNATURE_1"));
        }
        assert!(
            requests[4]
                .to_string()
                .contains("EXACT_RECOVERY_SIGNATURE_3")
        );
    }
}

async fn plan_recovery_server(streamed: bool) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for step in 0..4 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(20), listener.accept())
                .await
                .expect("expected Plan recovery provider request")
                .unwrap();
            requests.push(read_request(&mut socket).await);
            let (status, mime, body) = if step == 2 {
                (
                    404,
                    "application/json",
                    json!({"error":"fixture unavailable"}).to_string(),
                )
            } else {
                let content = if step == 1 {
                    vec![
                        json!({"type":"thinking","thinking":"PRIVATE_PLAN_THOUGHT",
                            "signature":"EXACT_PLAN_SIGNATURE"}),
                        json!({"type":"tool_use","id":"plan-create-once","name":"plan_create",
                            "input":{"prompt":"Plan the recovery fix","content":"# Recovery plan",
                                "steps":[{"title":"Verify","detail":"Run the focused gate",
                                    "requires_mutation":false}]}}),
                    ]
                } else {
                    vec![
                        json!({"type":"text","text":if step == 0 {"Premature final"} else {"Plan is ready"}}),
                    ]
                };
                (
                    200,
                    if streamed {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                    response_body(
                        step,
                        &content,
                        if step == 1 { "tool_use" } else { "end_turn" },
                        streamed,
                    ),
                )
            };
            socket.write_all(format!("HTTP/1.1 {status} Response\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (base, server)
}

#[tokio::test]
async fn anthropic_recovery_plan_keeps_a_narrowed_create_catalog_after_a_committed_write() {
    for streamed in [false, true] {
        let (base, server) = plan_recovery_server(streamed).await;
        let workspace = crate::test_support::private_tempdir();
        let config = recovery_config(workspace.path(), base, streamed, false);
        let runtime = open_runtime(&config, workspace.path(), None);
        let error = runtime
            .run_plan_with_skills(
                "primary",
                "test instructions",
                "Plan the recovery fix",
                Some(5),
                None,
                &[],
                &[],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("404"), "{error}");
        let session = runtime.latest_session().unwrap().id;
        let plans = runtime.work.list_plans(Some(&session), None, 10).unwrap();
        assert_eq!(plans.len(), 1);
        let original = plans[0].clone();
        assert_eq!(original.revision, 1);
        let result = runtime
            .run_plan_target_with_skills(
                "primary",
                "test instructions",
                "Continue",
                Some(5),
                Some(&session),
                &[],
                &[],
                PlanDraftTarget::Update {
                    plan_id: original.id.clone(),
                    revision: original.revision,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.output, "Plan is ready");
        assert_eq!(result.plan.as_ref(), Some(&original));
        assert_eq!(
            runtime.work.list_plans(Some(&session), None, 10).unwrap(),
            vec![original]
        );
        let canonical = runtime.session_messages(&session).unwrap();
        assert_eq!(
            canonical
                .iter()
                .flat_map(|record| &record.message.tool_calls)
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            ["plan.create"]
        );
        assert_eq!(
            canonical
                .iter()
                .filter(|record| record.message.role == ModelMessageRole::Tool)
                .count(),
            1
        );
        let visible = serde_json::to_string(&canonical).unwrap();
        assert!(!visible.contains("PRIVATE_PLAN_THOUGHT"));
        assert!(!visible.contains("EXACT_PLAN_SIGNATURE"));
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0]["tools"].as_array().unwrap().len() > 1);
        assert_eq!(
            requests[1]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["plan_create"]
        );
        for request in &requests[2..] {
            assert_eq!(request["system"], requests[1]["system"]);
            assert_eq!(request["tools"], requests[1]["tools"]);
            assert!(request.to_string().contains("EXACT_PLAN_SIGNATURE"));
        }
        assert!(flattened_blocks(&requests[3]).starts_with(&flattened_blocks(&requests[2])));
    }
}
