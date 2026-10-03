use super::*;
use crate::features::feature_rejection;
use colossus_contracts::{
    ModelFeatureMode, ModelFeatureSettings, ProviderContinuation, ProviderContinuationPlan,
    ProviderContinuationView,
};

#[test]
fn only_exact_unsupported_codes_and_sent_fields_are_capability_evidence() {
    let payload = json!({"context_management":[{"type":"compaction","compact_threshold":100}],"stream":true,"tools":[{}],"input":[{"type":"input_image","image_url":"data:image/png;base64,AA"}]});
    let mut diagnostic = ProviderResponseDiagnostic {
        request_method: "POST".into(),
        request_url: "http://localhost/responses".into(),
        request_body: None,
        status: 400,
        content_type: None,
        body: String::new(),
        body_encoding: "utf8".into(),
        body_truncated: false,
    };
    for (code, param, feature) in [
        (
            "unsupported_parameter",
            "context_management",
            ProviderFeature::ServerCompaction,
        ),
        ("unsupported_value", "stream", ProviderFeature::Streaming),
        ("unsupported_feature", "tools", ProviderFeature::Tools),
        ("unsupported_image", "input", ProviderFeature::Images),
    ] {
        diagnostic.body =
            json!({"error":{"code":code,"param":param,"message":"untrusted text"}}).to_string();
        assert_eq!(
            feature_rejection(&diagnostic, Some(&payload))
                .unwrap()
                .provider_feature_rejected,
            feature
        );
    }
    for status in [401, 403, 408, 429, 500, 503] {
        diagnostic.status = status;
        assert!(feature_rejection(&diagnostic, Some(&payload)).is_none());
    }
    diagnostic.status = 400;
    for body in [
        json!({"error":{"code":"context_length_exceeded","param":"context_management"}}),
        json!({"error":{"message":"unsupported images"}}),
        json!({"error":{"code":"unsupported_parameter","param":"other"}}),
        json!({"error":{"code":"unsupported_value","param":"context_management[0].compact_threshold"}}),
    ] {
        diagnostic.body = body.to_string();
        assert!(feature_rejection(&diagnostic, Some(&payload)).is_none());
    }
    diagnostic.body =
        json!({"error":{"code":"unsupported_parameter","param":"stream"}}).to_string();
    assert!(feature_rejection(&diagnostic, Some(&json!({"stream":false}))).is_none());
    diagnostic.body_truncated = true;
    assert!(feature_rejection(&diagnostic, Some(&payload)).is_none());
}

#[test]
fn auto_observations_are_route_scoped_and_on_overrides_negative_evidence() {
    let provider = ProviderExecutor::new(
        ProviderProfile::new(
            "test",
            ProviderKind::OpenAiResponses,
            Some("http://localhost:9000/v1".into()),
            None,
            1_000,
        )
        .unwrap(),
    );
    let settings = ModelFeatureSettings::default();
    let model = ModelProfile::new(
        "auto",
        "test",
        "model-a",
        32_768,
        4_096,
        settings.capabilities(),
        None,
    )
    .unwrap()
    .with_feature_settings(settings);
    let mut on = model.clone();
    on.name = "on".into();
    on.feature_settings.streaming = ModelFeatureMode::On;
    let mut other = model.clone();
    other.name = "other".into();
    other.model = "model-b".into();
    let registry = ProviderRegistry::new(
        vec![provider],
        vec![model, on, other],
        BTreeMap::from([("primary".into(), "auto".into())]),
    )
    .unwrap();
    let resolved = registry.resolve("primary").unwrap();
    let plan = ProviderContinuationPlan {
        session_id: "session-a".into(),
        binding: "binding".into(),
        context_binding_hash: "0".repeat(64),
        source_count: 0,
        source_hash: "hash".into(),
        snapshot_epoch: 0,
        selected: None,
    };
    assert!(
        resolved
            .provider()
            .resolve_continuation(Some(&plan), Some("session-b"))
            .is_err()
    );
    assert!(
        resolved
            .provider()
            .resolve_continuation(Some(&plan), None)
            .is_err()
    );
    assert!(
        resolved
            .provider()
            .resolve_continuation(Some(&plan), Some("session-a"))
            .unwrap()
            .is_none()
    );
    assert!(resolved.route().capabilities.streaming);
    registry.observe_feature(&resolved, ProviderFeature::Streaming, "unsupported");
    assert!(
        !registry
            .resolve("primary")
            .unwrap()
            .route()
            .capabilities
            .streaming
    );
    assert!(registry.model("on").unwrap().route().capabilities.streaming);
    assert!(
        registry
            .model("other")
            .unwrap()
            .route()
            .capabilities
            .streaming
    );
    assert!(registry.server_compaction_enabled(&resolved));
    registry.observe_feature(&resolved, ProviderFeature::ServerCompaction, "unsupported");
    assert!(!registry.server_compaction_enabled(&registry.resolve("primary").unwrap()));
}

#[test]
fn responses_continuation_preserves_tool_ids_without_duplicate_assistant_calls() {
    let calls = vec![ModelToolCall {
        call_id: "exact-call-1".into(),
        name: "lookup".into(),
        arguments: json!({}),
    }];
    let state = ProviderContinuation {
        view: ProviderContinuationView {
            id: "state".into(),
            covered_count: 2,
            context_binding_hash: "0".repeat(64),
            reserved_tokens: 100,
            bytes: 100,
        },
        plan: ProviderContinuationPlan {
            session_id: "session".into(),
            binding: "binding".into(),
            context_binding_hash: "0".repeat(64),
            source_count: 1,
            source_hash: "hash".into(),
            snapshot_epoch: 0,
            selected: None,
        },
        settled_hash: "settled".into(),
        assistant: ModelMessage {
            role: ModelMessageRole::Assistant,
            content: "".into(),
            tool_call_id: None,
            tool_calls: calls,
        },
        hidden_reasoning: vec![
            json!({"type":"compaction","id":"cmp-exact","encrypted_content":"exact-opaque-value"}),
            json!({"type":"function_call","call_id":"exact-call-1","name":"lookup","arguments":"{}"}),
        ],
    };
    let request = ModelRequest {
        instructions: "test".into(),
        messages: vec![ModelMessage {
            role: ModelMessageRole::Tool,
            content: "result".into(),
            tool_call_id: Some("exact-call-1".into()),
            tool_calls: vec![],
        }],
        tools: vec![],
        max_output_tokens: None,
    };
    let names = ProviderToolNames::default();
    let images = ProviderResolvedImages::default();
    let payload = responses_payload_with_images(
        &request,
        ProviderKind::OpenAiResponses,
        "model",
        100,
        None,
        false,
        ProviderProjection::new(&names, &images).with_continuation(Some(&state)),
    )
    .unwrap();
    assert_eq!(payload["input"][0], state.hidden_reasoning[0]);
    assert_eq!(payload["input"][1], state.hidden_reasoning[1]);
    assert_eq!(payload["input"][2]["call_id"], "exact-call-1");
    assert_eq!(payload["input"].as_array().unwrap().len(), 3);
    assert!(
        !redacted_image_payload(&payload)
            .to_string()
            .contains("exact-opaque-value")
    );
    let mut invalid = request;
    invalid.messages[0].tool_call_id = Some("wrong-call".into());
    assert!(
        responses_payload_with_images(
            &invalid,
            ProviderKind::OpenAiResponses,
            "model",
            100,
            None,
            false,
            ProviderProjection::new(&names, &images).with_continuation(Some(&state))
        )
        .is_err()
    );
}

#[test]
fn feature_evidence_does_not_collide_on_colons_in_profile_and_model_names() {
    let settings = ModelFeatureSettings::default();
    let providers = ["test:a", "test"].map(|name| {
        ProviderExecutor::new(
            ProviderProfile::new(
                name,
                ProviderKind::OpenAiResponses,
                Some("http://localhost:9000/v1".into()),
                None,
                1_000,
            )
            .unwrap(),
        )
    });
    let models =
        [("first", "test:a", "b"), ("second", "test", "a:b")].map(|(name, provider, model)| {
            ModelProfile::new(
                name,
                provider,
                model,
                32_768,
                4_096,
                settings.capabilities(),
                None,
            )
            .unwrap()
            .with_feature_settings(settings)
        });
    let registry = ProviderRegistry::new(
        providers.into(),
        models.into(),
        BTreeMap::from([("primary".into(), "first".into())]),
    )
    .unwrap();
    registry.observe_feature(
        &registry.resolve("primary").unwrap(),
        ProviderFeature::Streaming,
        "unsupported",
    );
    assert!(
        !registry
            .model("first")
            .unwrap()
            .route()
            .capabilities
            .streaming
    );
    assert!(
        registry
            .model("second")
            .unwrap()
            .route()
            .capabilities
            .streaming
    );
}
