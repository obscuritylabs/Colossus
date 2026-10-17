use super::*;

#[test]
fn image_tool_results_keep_pairing_and_exact_images_on_responses_codex_and_chat() {
    let (mut request, image, images) = multipart_model_request();
    request.messages = vec![
        ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::User,
            content: "Inspect a workspace image".into(),
            tool_call_id: None,
            tool_calls: vec![],
        },
        ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::Assistant,
            content: "".into(),
            tool_call_id: None,
            tool_calls: vec![
                ModelToolCall {
                    call_id: "view".into(),
                    name: "image.view".into(),
                    arguments: json!({"paths": ["diagram.png"]}),
                },
                ModelToolCall {
                    call_id: "other".into(),
                    name: "echo".into(),
                    arguments: json!({"text": "ok"}),
                },
            ],
        },
        ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::Tool,
            content: ModelContent::Parts(vec![
                ModelContentPart::Text {
                    text: "Image retained".into(),
                },
                ModelContentPart::Image { image },
            ]),
            tool_call_id: Some("view".into()),
            tool_calls: vec![],
        },
        ModelMessage {
            agent_message_origin: None,
            role: ModelMessageRole::Tool,
            content: "ok".into(),
            tool_call_id: Some("other".into()),
            tool_calls: vec![],
        },
    ];
    let names = ProviderToolNames::from_request(&request).unwrap();
    for provider in [ProviderKind::OpenAiResponses, ProviderKind::OpenAiCodex] {
        let wire = responses_payload_with_images(
            &request,
            provider,
            "image-test",
            1024,
            None,
            false,
            ProviderProjection::new(&names, &images),
        )
        .unwrap();
        assert_eq!(wire["input"][3]["type"], "function_call_output");
        assert_eq!(wire["input"][3]["call_id"], "view");
        assert_eq!(
            wire["input"][3]["output"][1]["image_url"],
            "data:image/png;base64,cG5n"
        );
    }
    let wire = chat_payload_with_images(
        &request,
        "image-test",
        1024,
        ChatCompletionsOutputTokenParameter::MaxTokens,
        None,
        false,
        ProviderProjection::new(&names, &images),
    )
    .unwrap();
    let messages = wire["messages"].as_array().unwrap();
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "view");
    assert_eq!(messages[4]["role"], "tool");
    assert_eq!(messages[4]["tool_call_id"], "other");
    assert_eq!(messages[5]["role"], "user");
    assert_eq!(
        messages[5]["content"][1]["image_url"]["url"],
        "data:image/png;base64,cG5n"
    );

    use colossus_contracts::{
        ProviderContinuation, ProviderContinuationPlan, ProviderContinuationView,
    };
    let assistant = request.messages[1].clone();
    let replay = ProviderContinuation {
        view: ProviderContinuationView {
            replay: true,
            id: "image-replay".into(),
            covered_count: 2,
            context_binding_hash: "context".into(),
            reserved_tokens: 100,
            bytes: 100,
        },
        plan: ProviderContinuationPlan {
            session_id: "image-session".into(),
            binding: "binding".into(),
            context_binding_hash: "context".into(),
            source_count: 1,
            source_hash: "source".into(),
            snapshot_epoch: 0,
            selected: None,
        },
        settled_hash: "settled".into(),
        assistant: assistant.clone(),
        hidden_reasoning: vec![json!({"message": assistant, "content": [
            {"type": "tool_use", "id": "view", "name": "image_view", "input": {"paths": ["diagram.png"]}},
            {"type": "tool_use", "id": "other", "name": "echo", "input": {"text": "ok"}}
        ]})],
    };
    let wire = crate::anthropic::payload(
        &request,
        "image-test",
        1024,
        None,
        false,
        ProviderProjection::new(&names, &images).with_continuation(Some(&replay)),
    )
    .unwrap();
    assert_eq!(wire["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(wire["messages"][2]["content"][0]["tool_use_id"], "view");
    assert_eq!(
        wire["messages"][2]["content"][0]["content"][1]["source"]["data"],
        "cG5n"
    );
    let diagnostics = crate::executor::redacted_image_payload(&wire).to_string();
    assert!(!diagnostics.contains("cG5n"));
}
