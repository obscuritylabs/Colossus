use super::*;

fn gateway(profile: &ProviderProfile, post: bool) -> EffectGateway {
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action(profile.kind.generation_action(), DecisionOutcome::Allow)
                .with_action("provider.models", DecisionOutcome::Allow)
                .with_network_destination(profile.network_origin().unwrap().unwrap())
                .with_post_effect(post),
        ),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [77; 32],
    )
}

fn builtin_tool_request(profile: &ProviderProfile) -> EffectRequest {
    let mut model = model_request();
    model.tools = colossus_tools::builtin_specs()
        .into_iter()
        .map(|spec| ModelToolDefinition {
            name: spec.name,
            description: spec.description,
            input_schema: spec.input_schema,
        })
        .collect();
    for (name, keyword) in [
        ("shell.run", "oneOf"),
        ("user.ask", "allOf"),
        ("workflow.schedule.create", "allOf"),
    ] {
        let schema = &model
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap()
            .input_schema;
        assert!(
            schema.get(keyword).is_some(),
            "{name} lost its canonical {keyword}"
        );
    }
    let mut request = provider_request(profile);
    request.content["request"] = serde_json::to_value(model).unwrap();
    request
}

fn assert_builtin_tool_wire(wire: &str) {
    let (_, body) = wire.split_once("\r\n\r\n").unwrap();
    let payload: Value = serde_json::from_str(body).unwrap();
    let tools = payload["tools"].as_array().unwrap();
    assert_eq!(tools.len(), colossus_tools::builtin_specs().len());
    for tool in tools {
        assert_eq!(tool["input_schema"]["type"], "object");
        for keyword in ["oneOf", "anyOf", "allOf"] {
            assert!(
                tool["input_schema"].get(keyword).is_none(),
                "{} retained unsupported root {keyword}",
                tool["name"]
            );
        }
    }
}

#[tokio::test]
async fn anthropic_json_uses_native_headers_and_policy_release() {
    let (url, server) = one_response_server(json!({"id":"msg-test","type":"message","role":"assistant",
        "content":[{"type":"thinking","thinking":"PRIVATE_THOUGHT","signature":"unit-secret"},{"type":"text","text":"hello"}],
        "stop_reason":"end_turn","usage":{"input_tokens":4,"output_tokens":2,
            "cache_read_input_tokens":null,"cache_creation_input_tokens":null}})).await;
    let profile = ProviderProfile::new(
        "native",
        ProviderKind::AnthropicMessages,
        Some(url),
        Some("env:UNIT_PROVIDER_KEY".into()),
        5000,
    )
    .unwrap();
    let credentials = Arc::new(CountingCredentialResolver::new());
    let executor = ProviderExecutor::with_credentials(profile.clone(), credentials.clone());
    let released = gateway(&profile, true)
        .execute(builtin_tool_request(&profile), &executor)
        .await
        .unwrap();
    let turn: ProviderTurn = serde_json::from_slice(&released.bytes).unwrap();
    assert_eq!(turn.provider, "anthropic_messages");
    assert!(!String::from_utf8_lossy(&released.bytes).contains("PRIVATE_THOUGHT"));
    let wire = server.await.unwrap();
    assert_builtin_tool_wire(&wire);
    assert!(wire.contains("POST /v1/messages HTTP/1.1"));
    let headers = wire.split("\r\n\r\n").next().unwrap().to_ascii_lowercase();
    assert!(headers.contains("x-api-key: unit-secret"));
    assert!(headers.contains("anthropic-version: 2023-06-01"));
    assert!(!headers.contains("authorization:"));
    assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn anthropic_sse_crosses_gateway_with_exact_text_and_cumulative_usage() {
    let frames = [
        json!({"type":"message_start","message":{"id":"msg-stream","type":"message","role":"assistant","usage":{"input_tokens":5,"output_tokens":1,"cache_read_input_tokens":null,"cache_creation_input_tokens":null}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello 🌍"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3,"cache_read_input_tokens":null,"cache_creation_input_tokens":null}}),
        json!({"type":"message_stop"}),
    ];
    let body = frames
        .iter()
        .map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap()))
        .collect::<String>();
    let (url, server) = one_sse_server_with_chunk_size(body, 1).await;
    let profile = ProviderProfile::new(
        "native",
        ProviderKind::AnthropicMessages,
        Some(url),
        None,
        5000,
    )
    .unwrap();
    let executor = ProviderExecutor::new(profile.clone());
    let mut items = ReleasedItems::default();
    gateway(&profile, true)
        .execute_stream(builtin_tool_request(&profile), &executor, &mut items)
        .await
        .unwrap();
    let encoded = serde_json::to_string(&items.0).unwrap();
    assert!(encoded.contains("hello 🌍"));
    assert!(encoded.contains("\"total_tokens\":8"));
    let wire = server.await.unwrap();
    assert!(wire.contains("POST /v1/messages"));
    assert_builtin_tool_wire(&wire);
}

#[tokio::test]
async fn anthropic_invalid_arguments_are_recoverable_only_after_a_confirmed_stream() {
    for (stop, completed) in [
        ("tool_use", true),
        ("max_tokens", true),
        ("tool_use", false),
    ] {
        let mut frames = vec![
            json!({"type":"message_start","message":{"id":"msg-invalid","type":"message","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"must-not-start","name":"lookup","input":{}}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-1","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"PRIVATE_ARGUMENT\":"}}),
            json!({"type":"content_block_stop","index":1}),
        ];
        if completed {
            frames.extend([
                json!({"type":"message_delta","delta":{"stop_reason":stop},"usage":{"output_tokens":2}}),
                json!({"type":"message_stop"}),
            ]);
        }
        let body = frames
            .into_iter()
            .map(|frame| format!("data: {frame}\n\n"))
            .collect();
        let (url, server) = one_sse_server_with_chunk_size(body, 1).await;
        let profile = ProviderProfile::new(
            "native",
            ProviderKind::AnthropicMessages,
            Some(url),
            None,
            5000,
        )
        .unwrap();
        let executor = ProviderExecutor::new(profile.clone());
        let mut released = ReleasedItems::default();
        let error = gateway(&profile, true)
            .execute_stream(provider_request(&profile), &executor, &mut released)
            .await
            .unwrap_err();
        if !completed {
            assert!(matches!(&error, GatewayError::OutcomeUnknown(_)));
        } else if stop == "max_tokens" {
            assert!(matches!(&error, GatewayError::ProviderRejected(failure)
                if failure.reason == colossus_contracts::ProviderFailureReason::OutputLimitExceeded));
        } else {
            assert!(
                matches!(&error, GatewayError::RecoverableExecution { code, .. }
                if code == "provider.invalid_tool_arguments")
            );
        }
        assert!(!error.to_string().contains("PRIVATE_ARGUMENT"));
        assert!(released.0.iter().all(|item| !matches!(
            item,
            ProviderStreamItem::Event {
                event: ProviderEvent::ToolCallRequested { .. }
            } | ProviderStreamItem::Completed { .. }
        )));
        server.await.unwrap();
    }
}

#[test]
fn anthropic_images_and_private_replay_are_omitted_from_diagnostics() {
    let payload = json!({"messages":[{"content":[
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":"PRIVATE_IMAGE"}},
        {"type":"thinking","thinking":"PRIVATE_THOUGHT","signature":"PRIVATE_SIGNATURE"},
        {"type":"redacted_thinking","data":"PRIVATE_ENCRYPTED"}
    ]}]});
    let redacted = crate::executor::redacted_image_payload(&payload).to_string();
    for sentinel in [
        "PRIVATE_IMAGE",
        "PRIVATE_THOUGHT",
        "PRIVATE_SIGNATURE",
        "PRIVATE_ENCRYPTED",
    ] {
        assert!(!redacted.contains(sentinel));
    }
    crate::executor::validate_serialized_provider_request(&payload, 2 * 1024 * 1024).unwrap();
}

#[tokio::test]
async fn anthropic_does_not_release_tool_calls_when_replay_cannot_be_staged() {
    for transport in ["json", "sse", "json_observer"] {
        let response = json!({"id":"msg-private","type":"message","role":"assistant",
            "content":[{"type":"thinking","thinking":"private","signature":"signed"},
                {"type":"tool_use","id":"call-1","name":"lookup","input":{}}],
            "stop_reason":"tool_use","usage":{"input_tokens":4,"output_tokens":2}});
        let (url, server) = if transport == "sse" {
            let mut frames = vec![json!({"type":"message_start","message":{
                "id":"msg-private","type":"message","role":"assistant",
                "usage":{"input_tokens":4,"output_tokens":1}}})];
            for (index, block) in response["content"].as_array().unwrap().iter().enumerate() {
                frames.push(
                    json!({"type":"content_block_start","index":index,"content_block":block}),
                );
                frames.push(json!({"type":"content_block_stop","index":index}));
            }
            frames.push(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":2}}));
            frames.push(json!({"type":"message_stop"}));
            let body = frames
                .into_iter()
                .map(|frame| format!("data: {frame}\n\n"))
                .collect();
            one_sse_server_with_chunk_size(body, 1).await
        } else {
            one_response_server(response).await
        };
        let profile = ProviderProfile::new(
            "native",
            ProviderKind::AnthropicMessages,
            Some(url),
            None,
            5000,
        )
        .unwrap();
        let executor = ProviderExecutor::new(profile.clone());
        let mut request = provider_request(&profile);
        request.content["request"] =
            serde_json::to_value(model_request_with_tools(&["lookup"])).unwrap();
        if transport == "json_observer" {
            request.content["stream_response"] = json!(false);
        }
        let mut released = ReleasedItems::default();
        let error = if transport != "json" {
            let error = gateway(&profile, true)
                .execute_stream(request, &executor, &mut released)
                .await
                .unwrap_err();
            assert!(released.0.iter().all(|item| !matches!(
                item,
                ProviderStreamItem::Event {
                    event: ProviderEvent::ToolCallRequested { .. }
                }
            )));
            error
        } else {
            gateway(&profile, true)
                .execute(request, &executor)
                .await
                .unwrap_err()
        };
        assert!(
            error.to_string().contains("private replay storage"),
            "{transport}: {error}"
        );
        server.await.unwrap();
    }
}

#[test]
fn anthropic_projects_verified_images_without_losing_content_order() {
    let (request, image, images) = multipart_model_request();
    let names = ProviderToolNames::from_request(&request).unwrap();
    let wire = crate::anthropic::payload(
        &request,
        "test-model",
        1024,
        None,
        false,
        ProviderProjection::new(&names, &images),
    )
    .unwrap();
    let content = &wire["messages"][0]["content"];
    assert_eq!(content[0]["text"], "before");
    assert_eq!(content[1]["source"]["type"], "base64");
    assert_eq!(content[1]["source"]["media_type"], image.media_type);
    assert_eq!(content[1]["source"]["data"], "cG5n");
    assert_eq!(content[2]["text"], "after");
    let mut image_only = request.clone();
    image_only.messages[0].content = ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: String::new(),
        },
        ModelContentPart::Image {
            image: image.clone(),
        },
    ]);
    let wire = crate::anthropic::payload(
        &image_only,
        "test-model",
        1024,
        None,
        false,
        ProviderProjection::new(&names, &images),
    )
    .unwrap();
    let content = wire["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "image");
    assert!(
        crate::anthropic::payload(
            &request,
            "test-model",
            1024,
            None,
            false,
            ProviderProjection::new(&names, &ProviderResolvedImages::default())
        )
        .is_err()
    );
}

#[tokio::test]
async fn anthropic_catalog_follows_native_cursors_and_preserves_first_metadata() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for (page,body) in [
            json!({"data":[{"id":"model-one","display_name":"First"}],"has_more":true,"last_id":"model-one"}),
            json!({"data":[{"id":"model-one","display_name":"Conflicting duplicate"},{"id":"model-two"}],"has_more":false}),
        ].into_iter().enumerate() {
            let (mut socket,_)=listener.accept().await.unwrap();
            let mut bytes=Vec::new();
            loop {
                let mut buffer=[0;2048];
                let count=socket.read(&mut buffer).await.unwrap();
                assert!(count>0);
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.windows(4).any(|w|w==b"\r\n\r\n") {break;}
            }
            let request=String::from_utf8(bytes).unwrap();
            if page==0 { assert!(request.starts_with("GET /v1/models HTTP/1.1")); }
            else { assert!(request.starts_with("GET /v1/models?after_id=model-one HTTP/1.1")); }
            let body=body.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let profile = ProviderProfile::new(
        "native",
        ProviderKind::AnthropicMessages,
        Some(url),
        None,
        5000,
    )
    .unwrap();
    let executor = ProviderExecutor::new(profile.clone());
    let mut request = provider_request(&profile);
    request.action = "provider.models".into();
    request.resource = profile.models_endpoint().unwrap().unwrap();
    for key in [
        "model_profile",
        "model",
        "max_output_tokens",
        "reasoning_effort",
        "request",
    ] {
        request.content[key] = Value::Null;
    }
    let result = gateway(&profile, true)
        .execute(request, &executor)
        .await
        .unwrap();
    let models: Vec<ProviderModelInfo> = serde_json::from_slice(&result.bytes).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].display_name.as_deref(), Some("First"));
    server.await.unwrap();
}
