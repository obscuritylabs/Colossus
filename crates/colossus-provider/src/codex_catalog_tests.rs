//! Subscription catalog negotiation through the same gateway as generation.
use super::*;

#[tokio::test]
async fn codex_catalog_uses_audited_version_headers_and_preserves_gpt_6_1_sol() {
    let (base_url, server) = one_response_server(json!({"models": [{
        "slug": "gpt-6.1-sol",
        "display_name": "GPT-6.1 Sol",
        "input_modalities": ["text", "image"],
        "supported_reasoning_levels": [
            {"effort": "low"}, {"effort": "medium"}, {"effort": "high"},
            {"effort": "xhigh"}, {"effort": "max"}
        ],
        "future_catalog_field": {"enabled": true}
    }]}))
    .await;
    let mut profile = ProviderProfile::new(
        "codex-catalog",
        ProviderKind::OpenAiCodex,
        None,
        Some(CODEX_CREDENTIAL_REFERENCE.into()),
        5_000,
    )
    .expect("Codex profile");
    // Synthetic loopback transport never reads the user's Codex sign-in.
    profile.base_url = Some(base_url);
    let directory = codex_auth_tempdir();
    let auth_path = directory.path().join("private").join("auth.json");
    write_test_codex_auth(&auth_path, 4_102_444_800);
    let executor = ProviderExecutor::new(profile.clone())
        .with_codex_auth_store(CodexAuthStore::at_path(auth_path));
    let policy = BuiltInPolicy::offline_default()
        .with_action("provider.models", DecisionOutcome::Allow)
        .with_network_destination(profile.network_origin().unwrap().unwrap())
        .with_post_effect(true);
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["provider.call".into()]),
        [54; 32],
    );
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
    let released = gateway
        .execute(request, &executor)
        .await
        .expect("catalog effect succeeds");
    let models: Vec<ProviderModelInfo> =
        serde_json::from_slice(&released.bytes).expect("released catalog");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-6.1-sol");
    assert_eq!(models[0].display_name.as_deref(), Some("GPT-6.1 Sol"));
    assert_eq!(models[0].image_inputs, Some(true));
    assert_eq!(
        models[0].supported_reasoning_efforts,
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
            ReasoningEffort::XHigh,
            ReasoningEffort::Max,
        ]
    );
    let request = server.await.expect("synthetic server");
    assert!(request.starts_with(&format!(
        "GET /v1/models?client_version={CODEX_PROTOCOL_VERSION} HTTP/1.1\r\n"
    )));
    let headers = request.to_ascii_lowercase();
    assert!(headers.contains(&format!("\r\nversion: {CODEX_PROTOCOL_VERSION}\r\n")));
    assert!(headers.contains("\r\nchatgpt-account-id: account-secret\r\n"));
    assert!(headers.contains("\r\noriginator: codex colossus\r\n"));
    assert!(headers.contains(concat!(
        "\r\nuser-agent: colossus/",
        env!("CARGO_PKG_VERSION"),
        "\r\n"
    )));
    assert!(!request.contains("0.156.1"));
}
