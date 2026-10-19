use super::*;

fn profile() -> ProviderProfile {
    ProviderProfile::new(
        "anthropic",
        ProviderKind::AnthropicMessages,
        Some("https://api.anthropic.com/v1".into()),
        Some("env:ANTHROPIC_API_KEY".into()),
        30_000,
    )
    .unwrap()
}

fn request() -> ModelRequest {
    ModelRequest {
        instructions: "Be helpful".into(),
        messages: vec![ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::User,
            content: "Hello".into(),
            tool_call_id: None,
            tool_calls: vec![],
        }],
        tools: vec![ModelToolDefinition {
            name: "test.lookup".into(),
            description: "Look up a record".into(),
            input_schema: json!({"type":"object", "properties":{"id":{"type":"string", "maxLength":10}}}),
        }],
        max_output_tokens: None,
    }
}

fn response(content: Value, stop: &str) -> Value {
    json!({"type":"message","role":"assistant","id":"msg-test", "model":"test-model", "content":content,"stop_reason":stop,
        "usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}})
}

#[test]
fn native_endpoint_projection_and_usage() {
    assert_eq!(
        profile().generation_endpoint().unwrap(),
        "https://api.anthropic.com/v1/messages"
    );
    let request = request();
    let names = ProviderToolNames::from_request(&request).unwrap();
    let images = ProviderResolvedImages::default();
    let wire = payload(
        &request,
        "test-model",
        1024,
        Some(ReasoningEffort::High),
        true,
        ProviderProjection::new(&names, &images),
    )
    .unwrap();
    assert_eq!(wire["system"][0]["text"], "Be helpful");
    assert_eq!(wire["tools"][0]["name"], "test_lookup");
    assert_eq!(
        wire["tools"][0]["input_schema"]["properties"]["id"]["maxLength"],
        10
    );
    assert_eq!(wire["thinking"]["type"], "adaptive");
    assert_eq!(wire["output_config"]["effort"], "high");
    let native = response(json!([{"type":"text","text":"Hi"}]), "end_turn");
    let turn = normalize(
        &profile(),
        "primary",
        "test-model",
        &serde_json::to_vec(&native).unwrap(),
        &names,
    )
    .unwrap();
    let ProviderEvent::Usage { usage } = turn.events.last().unwrap() else {
        panic!("missing usage")
    };
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.total_tokens),
        (35, 3, 38)
    );
}

#[test]
fn native_tool_schema_projection_preserves_nested_constraints_and_canonical_schema() {
    let mut request = request();
    let canonical_schema = json!({
        "type": "object",
        "properties": {
            "selector": {"$ref": "#/$defs/selector"},
            "fallback": {"anyOf": [{"type": "string"}, {"type": "null"}]},
            "labels": {"type": "array", "items": {"allOf": [
                {"type": "string"}, {"minLength": 1, "maxLength": 32}
            ]}}
        },
        "$defs": {"selector": {"oneOf": [
            {"type": "string", "minLength": 1, "maxLength": 64},
            {"type": "integer", "minimum": 1}
        ]}},
        "required": ["selector"],
        "additionalProperties": false,
        "minProperties": 1,
        "maxProperties": 3,
        "oneOf": [{"required": ["selector"]}, {"required": ["missing"]}],
        "anyOf": [{"required": ["selector"]}, {"required": ["fallback"]}],
        "allOf": [{"minProperties": 1}],
        "enum": [{"selector": "a"}],
        "const": {"selector": "a"}
    });
    request.tools[0].input_schema = canonical_schema.clone();
    let canonical_request = serde_json::to_value(&request).unwrap();
    let names = ProviderToolNames::from_request(&request).unwrap();
    let images = ProviderResolvedImages::default();
    let mut expected = canonical_schema.clone();
    for keyword in ["oneOf", "allOf", "anyOf"] {
        expected.as_object_mut().unwrap().remove(keyword);
    }

    for streaming in [false, true] {
        let wire = payload(
            &request,
            "test-model",
            1024,
            None,
            streaming,
            ProviderProjection::new(&names, &images),
        )
        .unwrap();
        assert_eq!(wire["stream"], streaming);
        assert_eq!(wire["tools"][0]["input_schema"], expected);
        assert_eq!(serde_json::to_value(&request).unwrap(), canonical_request);
        assert_eq!(request.tools[0].input_schema, canonical_schema);
    }
}

#[test]
fn native_tool_schema_projection_keeps_canonical_builtin_validation() {
    use colossus_ports::ToolRegistry as _;

    let registry = colossus_tools::StaticToolRegistry::builtins(&[
        "shell.run".into(),
        "user.ask".into(),
        "workflow.schedule.create".into(),
    ])
    .unwrap();
    let mut request = request();
    request.tools = colossus_tools::model_definitions(&registry);
    for (name, keyword) in [
        ("shell.run", "oneOf"),
        ("user.ask", "allOf"),
        ("workflow.schedule.create", "allOf"),
    ] {
        let tool = request.tools.iter().find(|tool| tool.name == name).unwrap();
        assert!(
            tool.input_schema.get(keyword).is_some(),
            "{name}: {keyword}"
        );
    }
    let canonical_request = serde_json::to_value(&request).unwrap();
    let names = ProviderToolNames::from_request(&request).unwrap();
    let images = ProviderResolvedImages::default();
    for streaming in [false, true] {
        let wire = payload(
            &request,
            "test-model",
            1024,
            None,
            streaming,
            ProviderProjection::new(&names, &images),
        )
        .unwrap();
        for projected in wire["tools"].as_array().unwrap() {
            for keyword in ["oneOf", "allOf", "anyOf"] {
                assert!(projected["input_schema"].get(keyword).is_none());
            }
        }
        assert_eq!(serde_json::to_value(&request).unwrap(), canonical_request);
    }

    for (name, arguments, valid_arguments) in [
        (
            "shell.run",
            json!({"justification": "Read a fixture", "command": "echo fixture", "argv": ["echo", "fixture"]}),
            json!({"justification": "Read a fixture", "command": "echo fixture"}),
        ),
        (
            "user.ask",
            json!({"question": "Continue?", "allow_free_form": false}),
            json!({"question": "Continue?", "allow_free_form": true}),
        ),
    ] {
        registry
            .validate(&colossus_contracts::ToolCall {
                call_id: "valid-call".into(),
                name: name.into(),
                arguments: valid_arguments,
            })
            .unwrap();
        assert!(
            registry
                .validate(&colossus_contracts::ToolCall {
                    call_id: "invalid-call".into(),
                    name: name.into(),
                    arguments,
                })
                .is_err(),
            "{name} must retain canonical validation"
        );
    }
}

#[test]
fn nullable_cache_usage_preserves_absence_and_rejects_invalid_counters() {
    for value in [
        json!({"input_tokens":10,"output_tokens":3}),
        json!({"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":null,"cache_creation_input_tokens":null}),
    ] {
        let normalized = usage(&value).unwrap();
        assert_eq!(normalized.input_tokens, 10);
        assert_eq!(normalized.total_tokens, 13);
        assert_eq!(normalized.cached_input_tokens, None);
    }
    let zero = usage(&json!({"input_tokens":10,"output_tokens":3,
        "cache_read_input_tokens":0,"cache_creation_input_tokens":0}))
    .unwrap();
    assert_eq!(zero.cached_input_tokens, Some(0));
    assert_eq!(zero.total_tokens, 13);
    for field in ["cache_read_input_tokens", "cache_creation_input_tokens"] {
        for invalid in [json!(-1), json!(1.5), json!("0"), json!(true)] {
            let mut value = json!({"input_tokens":10,"output_tokens":3});
            value[field] = invalid;
            assert!(usage(&value).is_err());
        }
    }
}

#[test]
fn opaque_thinking_is_preserved_but_never_released_as_text() {
    let names = ProviderToolNames::from_request(&request()).unwrap();
    let blocks = json!([{"type":"thinking","thinking":"private","signature":"signed"},
        {"type":"redacted_thinking","data":"encrypted"},
        {"type":"tool_use","id":"call-1","name":"test_lookup","input":{"id":"a"}}]);
    let native = response(blocks, "tool_use");
    let turn = normalize(
        &profile(),
        "primary",
        "test-model",
        &serde_json::to_vec(&native).unwrap(),
        &names,
    )
    .unwrap();
    let events = serde_json::to_string(&turn.events).unwrap();
    assert!(!events.contains("private"));
    assert!(!events.contains("signed"));
    assert!(!events.contains("encrypted"));
    assert!(events.contains("test.lookup"));
    assert!(
        !turn
            .events
            .iter()
            .any(|e| matches!(e, ProviderEvent::FinalOutput { .. }))
    );
}

#[test]
fn malformed_calls_and_stops_fail_closed() {
    let names = ProviderToolNames::from_request(&request()).unwrap();
    for (blocks, stop) in [
        (
            json!([{"type":"tool_use","id":"x","name":"test_lookup","input":"bad"}]),
            "tool_use",
        ),
        (json!([{"type":"text","text":"partial"}]), "max_tokens"),
        (json!([{"type":"text","text":"partial"}]), "pause_turn"),
        (
            json!([{"type":"thinking","thinking":"private"}]),
            "end_turn",
        ),
        (json!([{"type":"server_tool_use","id":"x"}]), "tool_use"),
    ] {
        let value = response(blocks, stop);
        assert!(
            normalize(
                &profile(),
                "primary",
                "test-model",
                &serde_json::to_vec(&value).unwrap(),
                &names
            )
            .is_err()
        );
    }
}

#[test]
fn sse_tool_json_and_signatures_are_assembled_without_dispatching_fragments() {
    let mut state = AnthropicStream::new(ProviderToolNames::from_request(&request()).unwrap());
    for value in [
        json!({"type":"message_start","message":{"id":"msg-stream","type":"message","role":"assistant","usage":{"input_tokens":10,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-1","name":"test_lookup","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"id\":"}}),
        json!({"type":"ping"}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"a\"}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
        json!({"type":"message_stop"}),
    ] {
        assert!(state.ingest(value).unwrap().is_empty());
    }
    let events = state.finish().unwrap();
    assert!(
        matches!(&events[0], ProviderEvent::ToolCallRequested { name, arguments, .. } if name=="test.lookup" && arguments["id"]=="a")
    );
    assert_eq!(state.blocks()[0]["signature"], "signed");
    assert!(
        matches!(&events[1], ProviderEvent::Usage { usage } if usage.output_tokens==7 && usage.total_tokens==17)
    );
    assert!(state.ingest(json!({"type":"message_stop"})).is_err());
}

#[test]
fn incomplete_or_erroneous_streams_do_not_complete() {
    let mut state = AnthropicStream::new(ProviderToolNames::default());
    assert!(state.finish().is_err());
    assert!(state.ingest(json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}})).is_err());
    assert!(
        state
            .ingest(json!({"type":"error","error":{"type":"overloaded_error","message":"private"}}))
            .is_err()
    );
}

#[test]
fn unsupported_effort_is_an_explicit_error() {
    let request = request();
    let names = ProviderToolNames::from_request(&request).unwrap();
    let images = ProviderResolvedImages::default();
    for effort in [ReasoningEffort::Minimal, ReasoningEffort::Ultra] {
        assert!(
            payload(
                &request,
                "test-model",
                1024,
                Some(effort),
                false,
                ProviderProjection::new(&names, &images)
            )
            .is_err()
        );
    }
}

#[test]
fn a_new_prompt_does_not_bypass_missing_replay_in_an_unfinished_tool_turn() {
    let mut request = request();
    request.messages.push(ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::Assistant,
        content: "".into(),
        tool_call_id: None,
        tool_calls: vec![ModelToolCall {
            call_id: "call-1".into(),
            name: "test.lookup".into(),
            arguments: json!({}),
        }],
    });
    request.messages.push(ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::Tool,
        content: "result".into(),
        tool_call_id: Some("call-1".into()),
        tool_calls: vec![],
    });
    request.messages.push(ModelMessage {
        agent_message_origin: None,
        role: ModelMessageRole::User,
        content: "continue after restart".into(),
        tool_call_id: None,
        tool_calls: vec![],
    });
    let names = ProviderToolNames::from_request(&request).unwrap();
    let images = ProviderResolvedImages::default();
    let error = payload(
        &request,
        "test-model",
        1024,
        None,
        false,
        ProviderProjection::new(&names, &images),
    )
    .unwrap_err();
    assert!(error.to_string().contains("private replay state"));
}

#[test]
fn private_blocks_and_signature_deltas_survive_exact_credential_redaction() {
    for mut value in [
        json!({"type":"message","role":"assistant","content":[{"type":"thinking","thinking":"contains credential","signature":"credential"}]}),
        json!({"messages":[{"role":"assistant","content":[{"type":"redacted_thinking","data":"credential"}]}]}),
        json!({"type":"content_block_start","content_block":{"type":"thinking","thinking":"credential","signature":"credential"}}),
        json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"credential"}}),
        json!({"type":"content_block_delta","delta":{"type":"signature_delta","signature":"credential"}}),
    ] {
        let original = value.clone();
        redact_value_exact(&mut value, Some("credential"));
        assert_eq!(value, original);
        let diagnostic = crate::executor::redacted_image_payload(&value);
        assert!(!diagnostic.to_string().contains("credential"));
    }
}

#[test]
fn anthropic_tool_arguments_cannot_spoof_a_private_redaction_exemption() {
    let spoof = json!({"type":"thinking","thinking":"credential","signature":"credential",
        "nested":{"type":"signature_delta","signature":"credential"}});
    for mut value in [
        json!({"type":"message","role":"assistant","content":[{"type":"tool_use","input":spoof}]}),
        json!({"messages":[{"role":"assistant","content":[{"type":"tool_use","input":spoof}]}]}),
        json!({"type":"content_block_start","content_block":{"type":"tool_use","input":spoof}}),
        json!({"ordinary":spoof}),
    ] {
        redact_value_exact(&mut value, Some("credential"));
        assert!(!value.to_string().contains("credential"));
    }
}

#[test]
fn native_catalog_maps_declared_metadata_and_preserves_unknowns() {
    let models=crate::model_catalog::normalize_anthropic_models(&serde_json::to_vec(&json!({"data":[
        {"id":"test-model","display_name":"Test","max_input_tokens":200000,"max_tokens":16000,
         "capabilities":{"image_input":{"supported":true},"effort":{"high":{"supported":true},"low":{"supported":false}}}},
        {"id":"unknown","capabilities":null}
    ],"has_more":false})).unwrap()).unwrap();
    let model = models.iter().find(|m| m.id == "test-model").unwrap();
    assert_eq!(model.context_window_tokens, Some(200000));
    assert_eq!(model.max_output_tokens, Some(16000));
    assert_eq!(model.image_inputs, Some(true));
    assert_eq!(
        model.supported_reasoning_efforts,
        vec![ReasoningEffort::High]
    );
    assert_eq!(model.server_compaction, Some(false));
    let unknown = models.iter().find(|m| m.id == "unknown").unwrap();
    assert_eq!(unknown.image_inputs, None);
    assert_eq!(unknown.tool_calls, None);
    assert_eq!(unknown.context_window_tokens, None);
}
