use super::*;
use crate::test_support::private_tempdir;
use colossus_contracts::ModelFeatureMode;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[path = "provider_replay_budget_tests.rs"]
mod replay_budget;

mod replay_admission {
    use super::*;
    use crate::provider_gateway::GatewayModelProvider;
    use colossus_contracts::{
        ModelCapabilities, ModelToolCall, PendingSessionToolTurn, ProviderContinuation,
        ProviderContinuationView, ReasoningEffort, WorkflowAgentOptions,
    };
    use colossus_ports::ProviderContinuationRepository;
    use colossus_testkit::InMemoryEventJournal;

    pub(super) struct Fixture {
        pub(super) provider: GatewayModelProvider,
        pub(super) context: ExecutionContext,
        pub(super) request: ModelRequest,
    }

    fn fixture() -> Fixture {
        fixture_with_content("", "original instructions")
    }

    pub(super) fn fixture_with_content(assistant_text: &str, instructions: &str) -> Fixture {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(journal.clone()));
        let snapshots: Arc<dyn ContextRepository> =
            Arc::new(EventSourcedContextRepository::new(journal.clone()));
        let continuations: Arc<dyn ProviderContinuationRepository> = Arc::new(
            colossus_context::EventSourcedProviderContinuations::new(journal.clone(), true),
        );
        let provider_profile = |name, kind, url: &str| {
            ProviderExecutor::new(
                ProviderProfile::new(name, kind, Some(url.into()), None, 5_000).unwrap(),
            )
        };
        let model = |name, provider, model| {
            ModelProfile::new(
                name,
                provider,
                model,
                100_000,
                4096,
                ModelCapabilities {
                    tool_calls: true,
                    streaming: false,
                    image_inputs: false,
                },
                Some(ReasoningEffort::Low),
            )
            .unwrap()
        };
        let providers = Arc::new(
            ProviderRegistry::new(
                vec![
                    provider_profile(
                        "native",
                        ProviderKind::AnthropicMessages,
                        "https://native.invalid/v1",
                    ),
                    provider_profile(
                        "other-native",
                        ProviderKind::AnthropicMessages,
                        "https://other.invalid/v1",
                    ),
                    provider_profile(
                        "chat",
                        ProviderKind::OpenAiCompatible,
                        "https://chat.invalid/v1",
                    ),
                ],
                vec![
                    model("native", "native", "native-model"),
                    model("other-model", "native", "different-model"),
                    model("other-route", "other-native", "native-model"),
                    model("chat", "chat", "native-model"),
                    model("image-route", "native", "native-model").with_feature_settings(
                        crate::ModelFeatureSettings {
                            image_inputs: ModelFeatureMode::On,
                            ..Default::default()
                        },
                    ),
                ],
                BTreeMap::from([("primary".into(), "native".into())]),
            )
            .unwrap()
            .with_continuations(continuations.clone())
            .unwrap(),
        );
        let gateway = Arc::new(EffectGateway::new(
            journal,
            Arc::new(BuiltInPolicy::offline_default()),
            Arc::new(DenyApproval),
            SafetyKernel::new(["provider.call".into()]),
            [41; 32],
        ));
        let provider = GatewayModelProvider {
            gateway,
            providers,
            sessions,
            snapshots,
            continuations,
            pending: StdMutex::default(),
        };
        let actor = system_actor("replay-test");
        provider
            .sessions
            .create_session("replay-session", None, actor.clone())
            .unwrap();
        let user = ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::User,
            content: "original request".into(),
            tool_call_id: None,
            tool_calls: vec![],
        };
        provider
            .sessions
            .append_message(
                "replay-session",
                "original-run",
                user.clone(),
                actor.clone(),
            )
            .unwrap();
        let context = ExecutionContext {
            session_id: Some("replay-session".into()),
            run_id: Some("original-run".into()),
            ..Default::default()
        };
        let mut request = ModelRequest {
            instructions: instructions.into(),
            messages: vec![user],
            tools: vec![ModelToolDefinition {
                name: "test.lookup".into(),
                description: "lookup".into(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            }],
            max_output_tokens: None,
        };
        let plan = provider
            .continuation_plan("primary", &request, &context)
            .unwrap()
            .unwrap();
        let assistant = ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::Assistant,
            content: assistant_text.into(),
            tool_call_id: None,
            tool_calls: vec![ModelToolCall {
                call_id: "call-1".into(),
                name: "test.lookup".into(),
                arguments: json!({}),
            }],
        };
        let mut content = vec![
            json!({"type":"thinking","thinking":"private-thought","signature":"private-signature"}),
        ];
        if !assistant_text.is_empty() {
            content.push(json!({"type":"text","text":assistant_text}));
        }
        content.push(json!({"type":"tool_use","id":"call-1","name":"test_lookup","input":{}}));
        let hidden_reasoning = vec![
            json!({"type":"anthropic_replay"}),
            json!({"message": assistant, "content":content}),
        ];
        let bytes = serde_json::to_vec(&hidden_reasoning).unwrap().len();
        let state = ProviderContinuation {
            view: ProviderContinuationView {
                replay: true,
                id: "released-candidate".into(),
                covered_count: 2,
                context_binding_hash: plan.context_binding_hash.clone(),
                reserved_tokens: (bytes as u64).div_ceil(3) + 64,
                bytes,
            },
            plan,
            settled_hash: String::new(),
            assistant: assistant.clone(),
            replay_request: None,
            route_binding: String::new(),
            hidden_reasoning,
        };
        provider.continuations.stage(state).unwrap();
        let resolved = provider.providers.resolve("primary").unwrap();
        provider
            .remember_released_candidate(
                Some("replay-session"),
                Some("original-run"),
                Some("released-candidate".into()),
                &request,
                &resolved,
                Some(ReasoningEffort::Low),
            )
            .unwrap();
        let pending = PendingSessionToolTurn {
            run_id: "original-run".into(),
            turn: 1,
            call_ids: vec!["call-1".into()],
        };
        let admission = provider.tool_turn_admission_events(&context).unwrap();
        provider
            .sessions
            .begin_tool_turn_with_events(
                "replay-session",
                pending.clone(),
                actor.clone(),
                admission,
            )
            .unwrap();
        let result = ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::Tool,
            content: "already completed".into(),
            tool_call_id: Some("call-1".into()),
            tool_calls: vec![],
        };
        request.messages.extend([assistant.clone(), result.clone()]);
        let settlement = provider
            .tool_turn_settlement_events(&context, &request.messages)
            .unwrap();
        provider
            .sessions
            .complete_tool_turn_with_events(
                "replay-session",
                &pending,
                vec![
                    SessionMessageAppend {
                        message: assistant,
                        actor: actor.clone(),
                    },
                    SessionMessageAppend {
                        message: result,
                        actor,
                    },
                ],
                system_actor("replay-test"),
                settlement,
            )
            .unwrap();
        provider.tool_turn_committed(&context).unwrap();
        request.messages.push(ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::User,
            content: "continue with this new guidance".into(),
            tool_call_id: None,
            tool_calls: vec![],
        });
        Fixture {
            provider,
            context,
            request,
        }
    }

    #[test]
    fn finished_run_cleanup_keeps_other_owners_and_durable_reconciliation_evidence() {
        let fixture = fixture();
        let settled = fixture
            .provider
            .continuations
            .load("replay-session")
            .unwrap()
            .unwrap();
        let mut own = settled.clone();
        own.view.id = "pending-own".into();
        fixture.provider.continuations.stage(own.clone()).unwrap();
        fixture
            .provider
            .remember_candidate(
                Some("replay-session"),
                Some("original-run"),
                Some("pending-own".into()),
            )
            .unwrap();
        let events = fixture
            .provider
            .tool_turn_admission_events(&fixture.context)
            .unwrap();
        let pending = PendingSessionToolTurn {
            run_id: "original-run".into(),
            turn: 2,
            call_ids: vec!["call-1".into()],
        };
        fixture
            .provider
            .sessions
            .begin_tool_turn_with_events(
                "replay-session",
                pending.clone(),
                system_actor("replay-test"),
                events,
            )
            .unwrap();
        let mut other = own;
        other.view.id = "pending-other".into();
        other.plan.session_id = "other-session".into();
        fixture.provider.continuations.stage(other).unwrap();
        fixture
            .provider
            .remember_candidate(
                Some("other-session"),
                Some("other-run"),
                Some("pending-other".into()),
            )
            .unwrap();
        fixture.provider.finish_run_candidates("original-run");
        assert!(
            fixture
                .provider
                .continuations
                .take_staged("pending-own")
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .provider
                .continuations
                .candidate("replay-session", "pending-own")
                .unwrap()
                .is_some(),
            "durable admitted replay remains reconciliation evidence"
        );
        assert_eq!(
            fixture
                .provider
                .sessions
                .pending_tool_turn("replay-session")
                .unwrap(),
            Some(pending)
        );
        assert_eq!(
            fixture
                .provider
                .continuations
                .load("replay-session")
                .unwrap()
                .unwrap()
                .view
                .id,
            settled.view.id
        );
        let other_context = ExecutionContext {
            session_id: Some("other-session".into()),
            run_id: Some("other-run".into()),
            ..Default::default()
        };
        assert_eq!(
            fixture
                .provider
                .tool_turn_admission_events(&other_context)
                .unwrap()
                .len(),
            1,
            "cleanup cannot remove another run's staged output"
        );
    }

    #[test]
    fn released_candidates_cannot_be_admitted_settled_or_discarded_by_another_run() {
        let fixture = fixture();
        let mut candidate = fixture
            .provider
            .continuations
            .load("replay-session")
            .unwrap()
            .unwrap();
        candidate.view.id = "pending-next".into();
        fixture.provider.continuations.stage(candidate).unwrap();
        fixture
            .provider
            .remember_candidate(
                Some("replay-session"),
                Some("original-run"),
                Some("pending-next".into()),
            )
            .unwrap();
        for run_id in [Some("another-run".to_owned()), None] {
            let foreign = ExecutionContext {
                run_id,
                ..fixture.context.clone()
            };
            assert!(
                fixture
                    .provider
                    .tool_turn_admission_events(&foreign)
                    .is_err()
            );
            assert!(
                fixture
                    .provider
                    .tool_turn_settlement_events(&foreign, &fixture.request.messages[..3])
                    .is_err()
            );
            assert!(fixture.provider.tool_turn_committed(&foreign).is_err());
            assert!(fixture.provider.discard_continuation(&foreign).is_err());
        }
        assert_eq!(
            fixture
                .provider
                .tool_turn_admission_events(&fixture.context)
                .unwrap()
                .len(),
            1
        );
        assert!(
            fixture
                .provider
                .continuations
                .candidate("replay-session", "pending-next")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            fixture
                .provider
                .sessions
                .list_messages("replay-session")
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn replay_binding_rejections_preserve_checkpoint_and_accept_original_preferences() {
        let fixture = fixture();
        let original = fixture
            .provider
            .continuations
            .load("replay-session")
            .unwrap()
            .unwrap();
        let attempts = [
            WorkflowAgentOptions {
                model_profile: Some("image-route".into()),
                ..Default::default()
            },
            WorkflowAgentOptions {
                model_profile: Some("other-model".into()),
                ..Default::default()
            },
            WorkflowAgentOptions {
                model_profile: Some("other-route".into()),
                ..Default::default()
            },
            WorkflowAgentOptions {
                model_profile: Some("chat".into()),
                ..Default::default()
            },
            WorkflowAgentOptions {
                reasoning_effort: Some(ReasoningEffort::High),
                ..Default::default()
            },
        ];
        for options in attempts {
            assert!(matches!(
                fixture.provider.replay_request_with_options(
                    "primary",
                    &fixture.request,
                    &fixture.context,
                    &options
                ),
                Err(ModelProviderError::Configuration(_))
            ));
            assert_eq!(
                fixture
                    .provider
                    .sessions
                    .list_messages("replay-session")
                    .unwrap()
                    .len(),
                3,
                "preflight never appends rejected guidance"
            );
            assert_eq!(
                fixture
                    .provider
                    .continuations
                    .load("replay-session")
                    .unwrap()
                    .unwrap()
                    .view
                    .id,
                original.view.id
            );
        }
        for change_schema in [false, true] {
            let mut narrowed = fixture.request.clone();
            if change_schema {
                narrowed.tools[0].input_schema =
                    json!({"type":"object","properties":{"changed":{"type":"string"}}});
            } else {
                narrowed.tools.clear();
            }
            assert!(
                fixture
                    .provider
                    .replay_request_with_options(
                        "primary",
                        &narrowed,
                        &fixture.context,
                        &Default::default()
                    )
                    .is_err()
            );
        }
        let (replayed, _) = fixture
            .provider
            .replay_request_with_options(
                "primary",
                &fixture.request,
                &fixture.context,
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(replayed.messages, fixture.request.messages);
        assert_eq!(replayed.instructions, "original instructions");
        assert!(replayed.messages[2].content.contains("already completed"));
        let mut changed_instructions = fixture.request.clone();
        changed_instructions.instructions = "new run no longer has the initial title cue".into();
        let frozen = fixture
            .provider
            .replay_request_with_options(
                "primary",
                &changed_instructions,
                &fixture.context,
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(frozen.0.instructions, "original instructions");
        let mut broader_catalog = fixture.request.clone();
        broader_catalog.tools.push(ModelToolDefinition {
            name: "another.authorized_tool".into(),
            description: "unused by this frozen exchange".into(),
            input_schema: json!({"type":"object"}),
        });
        let frozen_subset = fixture
            .provider
            .replay_request_with_options(
                "primary",
                &broader_catalog,
                &fixture.context,
                &Default::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            frozen_subset.0.tools, fixture.request.tools,
            "only the originally offered subset may be replayed"
        );
    }

    #[test]
    fn snapshot_change_rejects_replay_without_mutating_private_state() {
        let fixture = fixture();
        fixture
            .provider
            .snapshots
            .create(
                ContextSnapshot {
                    id: "changed-snapshot".into(),
                    session_id: "replay-session".into(),
                    source_start_sequence: 1,
                    source_end_sequence: 1,
                    summary: "changed context".into(),
                    pinned_facts: vec![],
                    open_tasks: vec![],
                    files_touched: vec![],
                    notable_tool_results: vec![],
                    strategy: "deterministic".into(),
                    created_at: String::new(),
                },
                system_actor("replay-test"),
            )
            .unwrap();
        assert!(
            fixture
                .provider
                .replay_request_with_options(
                    "primary",
                    &fixture.request,
                    &fixture.context,
                    &Default::default()
                )
                .is_err()
        );
        assert_eq!(
            fixture
                .provider
                .sessions
                .list_messages("replay-session")
                .unwrap()
                .len(),
            3
        );
        assert!(
            fixture
                .provider
                .continuations
                .load("replay-session")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn legacy_retired_replay_revives_only_when_its_exact_assistant_and_bindings_match() {
        for matching in [true, false] {
            let fixture = fixture();
            let mut state = fixture
                .provider
                .continuations
                .load("replay-session")
                .unwrap()
                .unwrap();
            state.replay_request = None;
            state.route_binding.clear();
            if !matching {
                state.assistant.tool_calls[0].call_id = "different-call".into();
            }
            fixture
                .provider
                .continuations
                .save(state, &fixture.context)
                .unwrap();
            fixture
                .provider
                .continuations
                .clear("replay-session", &fixture.context)
                .unwrap();
            assert!(
                fixture
                    .provider
                    .continuations
                    .load("replay-session")
                    .unwrap()
                    .is_none()
            );
            let recovered = fixture.provider.replay_request_with_options(
                "primary",
                &fixture.request,
                &fixture.context,
                &Default::default(),
            );
            if matching {
                assert_eq!(
                    recovered.unwrap().unwrap().0.messages,
                    fixture.request.messages
                );
                assert!(
                    fixture
                        .provider
                        .continuations
                        .load("replay-session")
                        .unwrap()
                        .is_some()
                );
            } else {
                assert!(recovered.is_err());
                assert!(
                    fixture
                        .provider
                        .continuations
                        .load("replay-session")
                        .unwrap()
                        .is_none(),
                    "invalid historical replay remains retired"
                );
            }
            assert_eq!(
                fixture
                    .provider
                    .sessions
                    .list_messages("replay-session")
                    .unwrap()
                    .len(),
                3
            );
        }
    }
}

#[test]
fn changed_projection_or_unmappable_prefix_retires_opaque_state_without_slicing_panics() {
    let message = ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::User,
        content: "source".into(),
        tool_call_id: None,
        tool_calls: vec![],
    };
    let canonical = vec![message.clone(), message];
    let changed = super::provider_continuation::projection_prefix_changed;
    assert!(!changed(1, &canonical, &canonical, true));
    assert!(changed(usize::MAX, &canonical, &canonical, true));
    // Defensive handling also covers image-budget projections from older
    // runtimes that removed observation slots rather than retaining markers.
    assert!(changed(2, &canonical, &canonical[..1], true));
    let mut projected = canonical.clone();
    projected[0].content = "archived source".into();
    assert!(changed(1, &canonical, &projected, true));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstResponse {
    Completed,
    FeatureRejected,
    GatewayUnavailable,
}

async fn responses_server(
    streamed: bool,
    first_response: FirstResponse,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        let retry_first = first_response == FirstResponse::GatewayUnavailable;
        for index in 0..(3 + usize::from(retry_first)) {
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
            if index == usize::from(retry_first) {
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
            let (status, content_type, body) = if first_response == FirstResponse::FeatureRejected
                && index == 0
            {
                ("400 Bad Request", "application/json", json!({"error":{"code":"unsupported_parameter","param":"context_management","message":"untrusted rejection body"}}).to_string())
            } else if retry_first && index == 0 {
                ("503 Service Unavailable", "application/json", "{}".into())
            } else {
                ("200 OK", content_type, body)
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (url, handle)
}

async fn compaction_round_trip(
    streamed: bool,
    first_response: FirstResponse,
    archive_binding: bool,
) {
    let (base_url, server) = responses_server(streamed, first_response).await;
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
    let decision = if archive_binding {
        Some(
            runtime
                .create_decision(
                    &session.id,
                    "Temporary binding",
                    "Retired decision sentinel",
                    DecisionPriority::High,
                    "Test continuation invalidation",
                    "While active",
                    "",
                    "",
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    if first_response == FirstResponse::FeatureRejected {
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
        assert!(requests.iter().all(|request| request["stream"] == streamed));
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
    // Changes to dynamic bindings or instructions invalidate the opaque prefix.
    if let Some(decision) = &decision {
        runtime.archive_decision(&decision.id).await.unwrap();
    }
    runtime
        .run_model_in_session(
            "primary",
            if archive_binding {
                "test instructions"
            } else {
                "changed instructions"
            },
            "third unique prompt",
            Some(1),
            &session.id,
        )
        .await
        .unwrap();
    let mut requests = server.await.unwrap();
    if first_response == FirstResponse::GatewayUnavailable {
        assert_eq!(
            requests[0], requests[1],
            "retry must preserve the exact payload"
        );
        requests.remove(0);
    }
    assert!(requests.iter().all(|request| request["stream"] == streamed));
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
    if archive_binding {
        assert!(
            requests[0]["input"]
                .to_string()
                .contains("Retired decision sentinel")
        );
        assert!(
            !requests[2]["input"]
                .to_string()
                .contains("Retired decision sentinel")
        );
    }
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
    compaction_round_trip(false, FirstResponse::Completed, false).await;
}

#[tokio::test]
async fn responses_compaction_chains_stream_and_keeps_opaque_state_private() {
    compaction_round_trip(true, FirstResponse::Completed, false).await;
}

#[tokio::test]
async fn responses_compaction_auto_remembers_only_structured_rejection_without_replaying_generation()
 {
    compaction_round_trip(false, FirstResponse::FeatureRejected, false).await;
    compaction_round_trip(true, FirstResponse::FeatureRejected, false).await;
}

#[tokio::test]
async fn archived_decisions_invalidate_streamed_and_nonstreamed_compaction() {
    compaction_round_trip(false, FirstResponse::Completed, true).await;
    compaction_round_trip(true, FirstResponse::Completed, true).await;
}

#[tokio::test]
async fn gateway_recovery_preserves_streamed_and_nonstreamed_compaction() {
    compaction_round_trip(false, FirstResponse::GatewayUnavailable, false).await;
    compaction_round_trip(true, FirstResponse::GatewayUnavailable, false).await;
}
