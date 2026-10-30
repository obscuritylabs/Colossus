//! Agent/context/gateway/HTTP acceptance using disposable provider endpoints.
use super::*;
use crate::provider_gateway::GatewayModelProvider;
use colossus_contracts::{ModelCapabilities, ModelFeatureSettings};
use colossus_ports::ProviderContinuationRepository;
use colossus_testkit::InMemoryEventJournal;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

const HTTP_KINDS: [ProviderKind; 4] = [
    ProviderKind::OpenAiCompatible,
    ProviderKind::OpenAiResponses,
    ProviderKind::OpenAiCodex,
    ProviderKind::AnthropicMessages,
];

fn call_id(step: usize) -> String {
    format!("call-{step:02}-{}", "x".repeat(120))
}

fn windows(step: usize) -> Value {
    json!({"windows":[{
        "window_id": format!("cu_w_{step:032x}"),
        "title": format!("OBSERVATION_PAYLOAD_{step}"),
        "application_name":"Disposable provider fixture",
        "rect":{"x":0,"y":0,"width":640,"height":480},
        "foreground":true,"available":true,"unavailable_reason":null
    }]})
}

#[derive(Default)]
struct ReadTools {
    calls: AtomicUsize,
    large: bool,
    screenshots: bool,
}

#[async_trait]
impl ToolExecutor for ReadTools {
    async fn execute(&self, call: ToolCall, _: ExecutionContext) -> Result<ToolResult, ToolError> {
        let step = self.calls.fetch_add(1, Ordering::SeqCst);
        let image = colossus_contracts::ModelImageReference {
            artifact_id: format!("artifact-{step:064x}"),
            file_name: "fixture.png".into(),
            media_type: "image/png".into(),
            size_bytes: 3,
            sha256: format!("{step:064x}"),
            width_pixels: 1,
            height_pixels: 1,
            detail: colossus_contracts::ModelImageDetail::Auto,
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: if self.screenshots {
                json!({
                    "session_id":"cu_s_11111111111111111111111111111111",
                    "window_id":"cu_w_11111111111111111111111111111111",
                    "observation_id":format!("cu_o_{step:032x}"),
                    "control_generation":1,"artifact":image
                })
                .to_string()
            } else if self.large {
                format!("LARGE_OBSERVATION_{step}{}", "x".repeat(64 * 1024))
            } else {
                windows(step).to_string()
            },
            exit_code: 0,
            images: if self.screenshots {
                vec![image]
            } else {
                vec![]
            },
        })
    }
}

fn response(kind: ProviderKind, step: usize, tool: Option<&str>, compact: bool) -> Value {
    let id = call_id(step);
    let arguments = if tool == Some("computer_screenshot") {
        json!({"session_id":"cu_s_11111111111111111111111111111111","control_generation":1})
    } else {
        json!({})
    };
    match kind {
        ProviderKind::OpenAiCompatible => {
            let message = tool.map_or_else(
                || json!({"role":"assistant","content":"finished"}),
                |name| {
                    json!({"role":"assistant","content":null,"tool_calls":[{
                        "id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}
                    }]})
                },
            );
            json!({"id":format!("response-{step}"),"choices":[{"index":0,"message":message,
                "finish_reason":if tool.is_some(){"tool_calls"}else{"stop"}}]})
        }
        ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => {
            let mut output = Vec::new();
            if compact {
                output.push(json!({"type":"compaction","id":format!("cmp-{step}"),
                    "encrypted_content":format!("OPAQUE_OBSERVATIONS_{step}")}));
            }
            output.push(tool.map_or_else(
                || {
                    json!({"type":"message","id":format!("message-{step}"),"role":"assistant",
                    "status":"completed","content":[{"type":"output_text","text":"finished"}]})
                },
                |name| {
                    json!({"type":"function_call","id":format!("fc-{step}"),"call_id":id,
                    "name":name,"arguments":arguments.to_string(),"status":"completed"})
                },
            ));
            json!({"id":format!("response-{step}"),"status":"completed","output":output})
        }
        ProviderKind::AnthropicMessages => {
            let content = tool.map_or_else(
                || vec![json!({"type":"text","text":"finished"})],
                |name| {
                    vec![
                        json!({"type":"thinking","thinking":format!("PRIVATE_THOUGHT_{step}"),
                        "signature":format!("EXACT_SIGNATURE_{step}")}),
                        json!({"type":"redacted_thinking","data":format!("EXACT_REDACTED_{step}")}),
                        json!({"type":"tool_use","id":id,"name":name,"input":arguments}),
                    ]
                },
            );
            json!({"id":format!("msg-{step}"),"type":"message","role":"assistant","content":content,
                "stop_reason":if tool.is_some(){"tool_use"}else{"end_turn"},
                "usage":{"input_tokens":20,"output_tokens":10}})
        }
        ProviderKind::Echo => unreachable!("Echo has no HTTP transport"),
    }
}

fn sse(kind: ProviderKind, response: &Value) -> String {
    let events = match kind {
        ProviderKind::OpenAiCompatible => {
            let mut delta = response["choices"][0]["message"].clone();
            if let Some(calls) = delta["tool_calls"].as_array_mut() {
                for (index, call) in calls.iter_mut().enumerate() {
                    call["index"] = json!(index);
                }
            }
            vec![json!({"id":response["id"],"choices":[{
                "index":0,"delta":delta,
                "finish_reason":response["choices"][0]["finish_reason"]
            }]})]
        }
        ProviderKind::OpenAiResponses | ProviderKind::OpenAiCodex => {
            let mut events = Vec::new();
            if response["output"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "message")
            {
                events.push(json!({"type":"response.output_text.delta","delta":"finished"}));
            }
            events.push(json!({"type":"response.completed","response":response}));
            events
        }
        ProviderKind::AnthropicMessages => {
            let mut events = vec![json!({"type":"message_start","message":{
                "id":response["id"],"type":"message","role":"assistant",
                "usage":{"input_tokens":20,"output_tokens":1}
            }})];
            for (index, block) in response["content"].as_array().unwrap().iter().enumerate() {
                events.push(
                    json!({"type":"content_block_start","index":index,"content_block":block}),
                );
                events.push(json!({"type":"content_block_stop","index":index}));
            }
            events.push(
                json!({"type":"message_delta","delta":{"stop_reason":response["stop_reason"]},
                "usage":{"output_tokens":10}}),
            );
            events.push(json!({"type":"message_stop"}));
            events
        }
        ProviderKind::Echo => unreachable!("Echo has no HTTP transport"),
    };
    let mut body = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    if kind == ProviderKind::OpenAiCompatible {
        body.push_str("data: [DONE]\n\n");
    }
    body
}

async fn read_request(socket: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 8192];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(offset) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
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
                return serde_json::from_slice(&bytes[offset + 4..offset + 4 + length]).unwrap();
            }
        }
    }
}

fn prefix(request: &Value, count: usize) -> Value {
    json!([
        request.get("system"),
        request.get("tools"),
        &request["messages"].as_array().unwrap()[..count]
    ])
}

struct Loopback {
    base: String,
    stop: oneshot::Sender<()>,
    requests: tokio::task::JoinHandle<Vec<Value>>,
}

async fn loopback(
    kind: ProviderKind,
    tool_turns: usize,
    large: bool,
    reject: bool,
    interrupted: Option<oneshot::Sender<()>>,
) -> Loopback {
    loopback_with_screenshots(kind, tool_turns, large, reject, interrupted, false).await
}

async fn loopback_with_screenshots(
    kind: ProviderKind,
    tool_turns: usize,
    large: bool,
    reject: bool,
    interrupted: Option<oneshot::Sender<()>>,
    screenshots: bool,
) -> Loopback {
    loopback_with_reply_text(
        kind,
        tool_turns,
        large,
        reject,
        interrupted,
        screenshots,
        None,
    )
    .await
}

async fn loopback_with_reply_text(
    kind: ProviderKind,
    tool_turns: usize,
    large: bool,
    reject: bool,
    mut interrupted: Option<oneshot::Sender<()>>,
    screenshots: bool,
    reply_text: Option<String>,
) -> Loopback {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (stop, mut stopped) = oneshot::channel();
    let requests = tokio::spawn(async move {
        let mut requests = Vec::new();
        let mut protected: Vec<(Value, Value)> = Vec::new();
        loop {
            let mut socket = tokio::select! {
                result = listener.accept() => result.unwrap().0,
                _ = &mut stopped => break,
            };
            let request = read_request(&mut socket).await;
            if kind == ProviderKind::AnthropicMessages {
                for (content, expected) in &protected {
                    if let Some(at) =
                        request["messages"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .position(|message| {
                                message["role"] == "assistant" && message["content"] == *content
                            })
                    {
                        assert_eq!(
                            prefix(&request, at),
                            *expected,
                            "signed prefix changed on the wire"
                        );
                    }
                }
            }
            let step = requests.len();
            let mut response = response(
                kind,
                step,
                (step < tool_turns).then_some(if screenshots {
                    "computer_screenshot"
                } else if large {
                    "filesystem_read"
                } else {
                    "computer_windows"
                }),
                kind == ProviderKind::OpenAiResponses && step < tool_turns,
            );
            if kind == ProviderKind::AnthropicMessages && step < tool_turns {
                if let Some(text) = &reply_text {
                    response["content"].as_array_mut().unwrap().insert(
                        2,
                        json!({"type":"text", "text":format!("TOOL_EXPLANATION_{step}{text}")}),
                    );
                }
                protected.push((
                    response["content"].clone(),
                    prefix(&request, request["messages"].as_array().unwrap().len()),
                ));
            }
            let streamed = request["stream"] == true;
            requests.push(request);
            if step == 2
                && let Some(started) = interrupted.take()
            {
                socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await.unwrap();
                let _ = started.send(());
                let mut byte = [0];
                let closed = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
                    .await
                    .expect("cancelled adapter must close pending HTTP body");
                // Cancelling before reading the response headers can close TCP
                // with a reset instead of EOF; both close the pending body.
                match closed {
                    Ok(0) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                    result => panic!("cancelled adapter left pending HTTP body: {result:?}"),
                }
                continue;
            }
            let (status, mime, body) = if reject && step == 2 {
                (
                    "400 Bad Request",
                    "application/json",
                    json!({"error":{"type":"invalid_request_error",
                    "message":"PRIVATE_SIGNED_PREFIX_ERROR"}})
                    .to_string(),
                )
            } else if streamed {
                ("200 OK", "text/event-stream", sse(kind, &response))
            } else {
                ("200 OK", "application/json", response.to_string())
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    Loopback {
        base,
        stop,
        requests,
    }
}

struct Fixture {
    agent: AgentService,
    sessions: Arc<dyn SessionRepository>,
    continuations: Arc<dyn ProviderContinuationRepository>,
    tools: Arc<ReadTools>,
    _auth: tempfile::TempDir,
}

fn fixture(kind: ProviderKind, base: Option<String>, streamed: bool, large: bool) -> Fixture {
    fixture_with_context(kind, base, streamed, large, true)
}

fn fixture_with_context(
    kind: ProviderKind,
    base: Option<String>,
    streamed: bool,
    large: bool,
    with_context: bool,
) -> Fixture {
    fixture_with_screenshots(kind, base, streamed, large, with_context, false)
}

fn fixture_with_screenshots(
    kind: ProviderKind,
    base: Option<String>,
    streamed: bool,
    large: bool,
    with_context: bool,
    screenshots: bool,
) -> Fixture {
    let auth = crate::test_support::private_tempdir();
    let mut profile = ProviderProfile::new(
        "fixture",
        kind,
        if kind == ProviderKind::OpenAiCodex {
            None
        } else {
            base.clone()
        },
        (kind == ProviderKind::OpenAiCodex).then(|| "codex:default".into()),
        5_000,
    )
    .unwrap();
    let mut executor = ProviderExecutor::new(profile.clone());
    if kind == ProviderKind::OpenAiCodex {
        // The production constructor forbids custom Codex origins. Only this
        // synthetic adapter instance receives loopback and a disposable sign-in.
        profile.base_url = base;
        let path = auth.path().join("auth.json");
        let token = |claims: Value| {
            format!(
                "header.{}.signature",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
            )
        };
        let bytes = json!({"auth_mode":"chatgpt","tokens":{
            "id_token":token(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}})),
            "access_token":token(json!({"exp":4_102_444_800_i64})),"refresh_token":"fixture-refresh"
        }}).to_string();
        #[cfg(windows)]
        colossus_windows_native::create_private_file(&path, bytes.as_bytes()).unwrap();
        #[cfg(not(windows))]
        {
            std::fs::write(&path, bytes).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        executor = ProviderExecutor::new(profile.clone())
            .with_codex_auth_store(CodexAuthStore::at_path(path));
    }
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions: Arc<dyn SessionRepository> =
        Arc::new(EventSourcedSessionRepository::new(journal.clone()));
    let snapshots: Arc<dyn ContextRepository> =
        Arc::new(EventSourcedContextRepository::new(journal.clone()));
    let continuations: Arc<dyn ProviderContinuationRepository> = Arc::new(
        colossus_context::EventSourcedProviderContinuations::new(journal.clone(), false),
    );
    let mut policy = BuiltInPolicy::offline_default()
        .with_action(kind.generation_action(), DecisionOutcome::Allow);
    if let Some(origin) = profile.network_origin().unwrap() {
        policy = policy.with_network_destination(origin);
    }
    let gateway = Arc::new(EffectGateway::new(
        journal.clone(),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [19; 32],
    ));
    let settings = ModelFeatureSettings::default();
    let model = ModelProfile::new(
        "fixture",
        "fixture",
        "fixture-model",
        1_000_000,
        4096,
        ModelCapabilities {
            tool_calls: true,
            streaming: streamed,
            image_inputs: false,
        },
        None,
    )
    .unwrap()
    .with_feature_settings(ModelFeatureSettings {
        streaming: streamed.into(),
        image_inputs: false.into(),
        ..settings
    });
    let providers = Arc::new(
        ProviderRegistry::new(
            vec![executor],
            vec![model],
            BTreeMap::from([("primary".into(), "fixture".into())]),
        )
        .unwrap()
        .with_continuations(continuations.clone())
        .unwrap(),
    );
    assert!(
        !providers
            .resolve("primary")
            .unwrap()
            .route()
            .capabilities
            .image_inputs
    );
    let provider: Arc<dyn ModelProvider> = Arc::new(GatewayModelProvider {
        gateway,
        providers,
        sessions: sessions.clone(),
        snapshots: snapshots.clone(),
        continuations: continuations.clone(),
        pending: StdMutex::default(),
    });
    let context = ContextService::new(
        ContextConfig {
            auto_compaction: false,
            model_assisted: false,
            ..Default::default()
        },
        sessions.clone(),
        snapshots,
        provider.clone(),
    )
    .unwrap();
    let tools = Arc::new(ReadTools {
        large,
        screenshots,
        ..Default::default()
    });
    let name = if screenshots {
        "computer.screenshot"
    } else if large {
        "filesystem.read"
    } else {
        "computer.windows"
    };
    let registry = if screenshots {
        StaticToolRegistry::builtins(&[name.into()]).unwrap()
    } else {
        StaticToolRegistry::new([ToolSpec {
            name: name.into(),
            description: "Read disposable fixture state".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            effect_action: None,
            capability: None,
            max_output_bytes: 128 * 1024,
        }])
        .unwrap()
    };
    let agent = AgentService::new(
        journal,
        provider,
        Arc::new(registry),
        tools.clone(),
        sessions.clone(),
    );
    let agent = if with_context {
        agent.with_context_preparer(Arc::new(context))
    } else {
        agent
    };
    Fixture {
        agent,
        sessions,
        continuations,
        tools,
        _auth: auth,
    }
}

async fn completed(kind: ProviderKind, streamed: bool, with_context: bool) {
    let server = loopback(kind, 3, false, false, None).await;
    let fixture = fixture_with_context(
        kind,
        Some(server.base.clone()),
        streamed,
        false,
        with_context,
    );
    let result = fixture
        .agent
        .run(
            "primary",
            "Read the current window choices",
            "Inspect repeated windows",
            8,
        )
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = server.stop.send(());
            let _ = server.requests.await;
            panic!("{kind:?}/{streamed}: {error}");
        }
    };
    assert_eq!(result.output, "finished");
    let session = result.session_id.as_deref().unwrap();
    assert!(
        fixture.continuations.load(session).unwrap().is_none(),
        "completed private exchange must be retired after canonical settlement"
    );
    fixture
        .agent
        .run_in_session(
            "primary",
            "Read the current window choices",
            "New human turn",
            1,
            Some(session),
        )
        .await
        .unwrap();
    let _ = server.stop.send(());
    let requests = server.requests.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 3);
    let last = requests[3].to_string();
    assert!(
        last.contains("OBSERVATION_PAYLOAD_2"),
        "{kind:?}/{streamed}/{with_context}: {last}"
    );
    assert_eq!(
        last.contains("OBSERVATION_PAYLOAD_0"),
        kind == ProviderKind::AnthropicMessages
    );
    assert_eq!(
        last.contains("EXACT_SIGNATURE_0"),
        kind == ProviderKind::AnthropicMessages
    );
    if kind == ProviderKind::OpenAiResponses {
        assert!(requests[2].to_string().contains("OBSERVATION_PAYLOAD_1"));
        assert!(!requests[2].to_string().contains("OPAQUE_OBSERVATIONS_1"));
        assert!(fixture.continuations.load(session).unwrap().is_none());
    }
    assert!(!requests[4].to_string().contains("EXACT_SIGNATURE"));
    let canonical = fixture
        .sessions
        .list_messages(session)
        .unwrap()
        .into_iter()
        .map(|record| record.message)
        .collect::<Vec<_>>();
    let serialized = serde_json::to_string(&canonical).unwrap();
    for step in 0..3 {
        assert!(serialized.contains(&format!("OBSERVATION_PAYLOAD_{step}")));
        assert!(serialized.contains(&call_id(step)));
    }
    assert!(!serialized.contains("PRIVATE_THOUGHT"));
    assert!(!serialized.contains("OPAQUE_OBSERVATIONS"));
    validate_model_transcript(&canonical).unwrap();
}

#[tokio::test]
async fn text_only_routes_keep_native_screenshot_slots_for_http_continuations() {
    for kind in HTTP_KINDS {
        for streamed in [false, true] {
            for with_context in [false, true] {
                let server = loopback_with_screenshots(kind, 3, false, false, None, true).await;
                let fixture = fixture_with_screenshots(
                    kind,
                    Some(server.base.clone()),
                    streamed,
                    false,
                    with_context,
                    true,
                );
                let result = fixture
                    .agent
                    .run(
                        "primary",
                        "Inspect screenshot metadata",
                        "Compare captures",
                        8,
                    )
                    .await;
                let _ = server.stop.send(());
                let requests = server.requests.await.unwrap();
                let result = result.unwrap_or_else(|error| {
                    panic!(
                        "{kind:?}/{streamed}/context={with_context}: {error}; requests={}, tool_calls={}",
                        requests.len(),
                        fixture.tools.calls.load(Ordering::SeqCst)
                    )
                });
                assert_eq!(result.output, "finished");
                assert_eq!(requests.len(), 4);
                assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 3);
                for request in &requests {
                    let wire = request.to_string();
                    assert!(!wire.contains("input_image"));
                    assert!(!wire.contains("\"type\":\"image\""));
                    assert!(!wire.contains("data:image/"));
                }
                let last = requests.last().unwrap().to_string();
                assert!(last.contains("_colossusArchivedToolImage"));
                assert!(last.contains("image_inputs_unavailable"));
                if kind == ProviderKind::AnthropicMessages {
                    for step in 0..3 {
                        assert!(last.contains(&format!("EXACT_SIGNATURE_{step}")));
                        assert!(last.contains(&format!("EXACT_REDACTED_{step}")));
                    }
                } else {
                    assert!(last.contains("_colossusSuperseded"));
                }
                if kind == ProviderKind::OpenAiResponses {
                    assert!(!last.contains("OPAQUE_OBSERVATIONS_2"));
                }
                let canonical = fixture
                    .sessions
                    .list_messages(result.session_id.as_deref().unwrap())
                    .unwrap()
                    .into_iter()
                    .map(|record| record.message)
                    .collect::<Vec<_>>();
                assert_eq!(
                    canonical
                        .iter()
                        .filter(|message| message.role == ModelMessageRole::ToolObservation)
                        .count(),
                    3
                );
                assert_eq!(
                    canonical
                        .iter()
                        .flat_map(|message| message.content.images())
                        .count(),
                    3
                );
                validate_model_transcript(&canonical).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn every_http_provider_runs_repeated_observations_through_agent_context_and_gateway() {
    for kind in HTTP_KINDS {
        for streamed in [false, true] {
            for with_context in [false, true] {
                completed(kind, streamed, with_context).await;
            }
        }
    }
}

#[tokio::test]
async fn every_http_provider_stops_on_400_without_repeating_completed_tools() {
    for kind in HTTP_KINDS {
        for streamed in [false, true] {
            let server = loopback(kind, 3, false, true, None).await;
            let fixture = fixture(kind, Some(server.base.clone()), streamed, false);
            let error = fixture
                .agent
                .run("primary", "Read windows", "Inspect", 8)
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("PRIVATE_SIGNED_PREFIX_ERROR"));
            assert!(error.to_string().contains("400"));
            let _ = server.stop.send(());
            assert_eq!(server.requests.await.unwrap().len(), 3);
            assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 2);
        }
    }
}

#[tokio::test]
async fn anthropic_aggregate_observation_budget_cannot_rewrite_a_signed_prefix() {
    for streamed in [false, true] {
        let server = loopback(ProviderKind::AnthropicMessages, 8, true, false, None).await;
        let fixture = fixture(
            ProviderKind::AnthropicMessages,
            Some(server.base.clone()),
            streamed,
            true,
        );
        let error = fixture
            .agent
            .run("primary", "Read large fixture observations", "Inspect", 12)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("unchanged observation prefix"),
            "{error}"
        );
        let _ = server.stop.send(());
        let requests = server.requests.await.unwrap();
        assert!(requests.len() >= 3 && requests.len() < 8);
        assert_eq!(requests.len(), fixture.tools.calls.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn anthropic_replay_budget_keeps_signed_tool_history_without_counting_metadata_twice() {
    for streamed in [false, true] {
        let server = loopback_with_reply_text(
            ProviderKind::AnthropicMessages,
            3,
            true,
            false,
            None,
            false,
            Some("explanation ".repeat(30 * 1024 / 12)),
        )
        .await;
        let fixture = fixture(
            ProviderKind::AnthropicMessages,
            Some(server.base.clone()),
            streamed,
            true,
        );
        let instructions = "i".repeat(440 * 1024);
        let result = fixture
            .agent
            .run(
                "primary",
                &instructions,
                "Inspect three files and report the result",
                4,
            )
            .await;
        let _ = server.stop.send(());
        let requests = server.requests.await.unwrap();
        let result = result.unwrap_or_else(|error| {
            panic!(
                "streamed={streamed}: {error}; requests={}, tools={}",
                requests.len(),
                fixture.tools.calls.load(Ordering::SeqCst)
            )
        });
        assert_eq!(result.output, "finished");
        assert_eq!(requests.len(), 4);
        assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 3);
        for request in &requests {
            assert!(serde_json::to_vec(request).unwrap().len() <= 1024 * 1024);
        }
        let session = result.session_id.as_deref().unwrap();
        let canonical = fixture
            .sessions
            .list_messages(session)
            .unwrap()
            .into_iter()
            .map(|record| record.message)
            .collect::<Vec<_>>();
        validate_model_transcript(&canonical).unwrap();
        assert!(
            fixture
                .sessions
                .pending_tool_turn(session)
                .unwrap()
                .is_none()
        );
        assert!(fixture.continuations.load(session).unwrap().is_none());

        let final_request = requests.last().unwrap();
        let mut old_storage = vec![json!({"type":"anthropic_replay"})];
        for step in 0..3 {
            let id = call_id(step);
            let assistant = canonical
                .iter()
                .find(|message| message.tool_calls.iter().any(|call| call.call_id == id))
                .unwrap();
            let content = final_request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| {
                    message["role"] == "assistant"
                        && message["content"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|block| block["id"] == id)
                })
                .unwrap()["content"]
                .clone();
            assert!(content.as_array().unwrap().iter().any(|block| {
                block["type"] == "thinking"
                    && block["signature"] == format!("EXACT_SIGNATURE_{step}")
            }));
            assert!(content.as_array().unwrap().iter().any(|block| {
                block["type"] == "redacted_thinking"
                    && block["data"] == format!("EXACT_REDACTED_{step}")
            }));
            let result = canonical
                .iter()
                .find(|message| {
                    message.role == ModelMessageRole::Tool
                        && message.tool_call_id.as_deref() == Some(&id)
                })
                .unwrap();
            let observed = final_request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|message| message["content"].as_array().unwrap())
                .find(|block| block["type"] == "tool_result" && block["tool_use_id"] == id)
                .unwrap();
            assert_eq!(observed["content"], result.content.plain_text().as_ref());
            old_storage
                .push(json!({"message":assistant,"content":content,"prefix_hash":"0".repeat(64)}));
        }
        // Reconstruct the former conservative accounting: canonical messages plus
        // every private storage entry, which duplicated these same assistants.
        let previous_bytes = serde_json::to_vec(&ModelRequest {
            instructions,
            messages: canonical[..canonical.len() - 1].to_vec(),
            tools: vec![],
            max_output_tokens: None,
        })
        .unwrap()
        .len()
            + serde_json::to_vec(&old_storage).unwrap().len();
        assert!(
            previous_bytes > 896 * 1024,
            "fixture must exercise the former rejection"
        );
    }
}

#[tokio::test]
async fn echo_still_uses_the_normal_context_and_gateway_without_http() {
    for with_context in [false, true] {
        let fixture = fixture_with_context(ProviderKind::Echo, None, false, false, with_context);
        let result = fixture
            .agent
            .run(
                "primary",
                "Echo the newest user prompt",
                "echo observation fixture",
                1,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "echo observation fixture");
        assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn every_http_provider_closes_an_interrupted_turn_without_repeating_completed_tools() {
    for kind in HTTP_KINDS {
        let (started, interrupted) = oneshot::channel();
        let server = loopback(kind, 3, false, false, Some(started)).await;
        let fixture = fixture(kind, Some(server.base.clone()), true, false);
        {
            let run = fixture.agent.run("primary", "Read windows", "Inspect", 8);
            tokio::pin!(run);
            tokio::select! {
                result = &mut run => panic!("provider finished before cancellation: {result:?}"),
                result = interrupted => result.unwrap(),
            }
            // Dropping the run future interrupts the actual pending HTTP/SSE
            // request through the normal gateway guard, not a mocked adapter.
        }
        let _ = server.stop.send(());
        assert_eq!(server.requests.await.unwrap().len(), 3);
        assert_eq!(fixture.tools.calls.load(Ordering::SeqCst), 2);
        if kind == ProviderKind::AnthropicMessages {
            let sessions = fixture.sessions.list_sessions(1).unwrap();
            let state = fixture
                .continuations
                .load(&sessions[0].id)
                .unwrap()
                .unwrap();
            assert!(
                state.view.replay,
                "unfinished native replay must survive an interrupted provider request"
            );
            assert_eq!(
                fixture
                    .sessions
                    .list_messages(&sessions[0].id)
                    .unwrap()
                    .last()
                    .unwrap()
                    .message
                    .role,
                ModelMessageRole::Tool
            );
        }
    }
}
