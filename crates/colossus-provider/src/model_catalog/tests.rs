use super::*;
use serde_json::json;

fn normalize(value: Value) -> Vec<ProviderModelInfo> {
    normalize_models(&serde_json::to_vec(&value).expect("catalog JSON")).expect("valid catalog")
}

#[test]
fn openrouter_cards_preserve_declared_limits_and_capabilities() {
    let models = normalize(json!({"data": [{
        "id": "author/model", "name": "Useful model", "description": "A catalog description.",
        "context_length": 128_000,
        "top_provider": {"max_completion_tokens": 16_384},
        "supported_parameters": ["temperature", "tools", "reasoning"],
        "architecture": {"input_modalities": ["text", "image"]}
    }]}));
    let model = &models[0];
    assert_eq!(model.display_name.as_deref(), Some("Useful model"));
    assert_eq!(model.description.as_deref(), Some("A catalog description."));
    assert_eq!(model.context_window_tokens, Some(128_000));
    assert_eq!(model.max_output_tokens, Some(16_384));
    assert_eq!(model.tool_calls, Some(true));
    assert_eq!(model.image_inputs, Some(true));
    assert_eq!(model.streaming, None);
    assert!(model.supported_reasoning_efforts.is_empty());
}

#[test]
fn codex_cards_preserve_names_context_and_supported_reasoning() {
    let models = normalize(json!({"models": [{
        "slug": "codex-model", "display_name": "Codex model", "description": "For coding.",
        "context_window": 272_000, "input_modalities": ["text", "image"],
        "supported_reasoning_levels": [
            {"effort": "low", "description": "Quick"},
            {"effort": "high", "description": "Thorough"},
            {"effort": "high"}, {"effort": "future-value"}, {"effort": false}
        ]
    }]}));
    let model = &models[0];
    assert_eq!(model.display_name.as_deref(), Some("Codex model"));
    assert_eq!(model.context_window_tokens, Some(272_000));
    assert_eq!(model.image_inputs, Some(true));
    assert_eq!(model.tool_calls, None);
    assert_eq!(model.max_output_tokens, None);
    assert_eq!(
        model.supported_reasoning_efforts,
        vec![ReasoningEffort::Low, ReasoningEffort::High]
    );
}

#[test]
fn codex_catalog_preserves_new_model_families_without_a_local_allowlist() {
    let ids = ["gpt-5.6", "gpt-6-astra", "gpt-6-luna", "gpt-6-sol"];
    let models = normalize(json!({"models": ids.map(|id| json!({
        "slug": id, "display_name": id,
        "supported_reasoning_levels": [{"effort": "high"}],
        "future_catalog_field": {"enabled": true}
    }))}));
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    assert!(
        models
            .iter()
            .all(|model| { model.supported_reasoning_efforts == vec![ReasoningEffort::High] })
    );
}

#[test]
fn sparse_and_malformed_metadata_stay_unknown() {
    let models = normalize(json!({"data": [
        {"id": "gpt-looking-name"},
        {"id": "malformed", "context_length": -1, "max_output_tokens": "4096",
         "supports_streaming": "true", "supported_parameters": [false],
         "architecture": {"input_modalities": [null]}}
    ]}));
    for model in models {
        assert_eq!(model.context_window_tokens, None);
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.tool_calls, None);
        assert_eq!(model.image_inputs, None);
        assert_eq!(model.streaming, None);
    }
}

#[test]
fn explicit_unsupported_capabilities_and_custom_metadata_are_preserved() {
    let models = normalize(json!({"data": [{
        "id": "custom", "context_window": 32_768, "max_output_tokens": 4_096,
        "supports_streaming": false, "supported_parameters": [],
        "architecture": {"input_modalities": ["text"]},
        "supported_reasoning_efforts": ["none", "minimal", "medium", "xhigh", "max", "ultra"]
    }]}));
    let model = &models[0];
    assert_eq!(model.tool_calls, Some(false));
    assert_eq!(model.image_inputs, Some(false));
    assert_eq!(model.streaming, Some(false));
    assert_eq!(model.supported_reasoning_efforts.len(), 6);
}

#[test]
fn custom_gateway_cards_normalize_vllm_context_and_tooling_support() {
    let models = normalize(json!({"data": [{
        "id": "custom-chat", "max_model_len": 131_072,
        "tooling_support": true, "max_tokens_field": "max_completion_tokens"
    }]}));
    let model = &models[0];
    assert_eq!(model.context_window_tokens, Some(131_072));
    assert_eq!(model.tool_calls, Some(true));
    // A wire-parameter name does not advertise an output ceiling or other capabilities.
    assert_eq!(model.max_output_tokens, None);
    assert_eq!(model.image_inputs, None);
    assert_eq!(model.streaming, None);
}

#[test]
fn nested_capabilities_normalize_known_fields_and_preserve_false() {
    for capabilities in [
        json!({"tool_calls": true, "image_inputs": true, "streaming": false}),
        json!({"supports_tool_calls": true, "supports_image_inputs": true, "supports_streaming": false}),
        json!({"tooling_support": true, "image_inputs": true, "streaming": false}),
    ] {
        let models = normalize(json!({"data": [{
            "id": "custom-responses", "context_window_tokens": 200_000,
            "max_output_tokens": 32_000, "capabilities": capabilities
        }]}));
        let model = &models[0];
        assert_eq!(model.context_window_tokens, Some(200_000));
        assert_eq!(model.max_output_tokens, Some(32_000));
        assert_eq!(model.tool_calls, Some(true));
        assert_eq!(model.image_inputs, Some(true));
        assert_eq!(model.streaming, Some(false));
    }
}

#[test]
fn explicit_capability_values_precede_nested_values_and_parameter_lists() {
    let models = normalize(json!({"data": [{
        "id": "conflicting", "tool_calls": false, "tooling_support": true,
        "image_inputs": false, "streaming": false,
        "capabilities": {"tool_calls": true, "image_inputs": true, "streaming": true},
        "supported_parameters": ["tools"], "input_modalities": ["image"],
        "context_window_tokens": 64_000, "context_window": 128_000, "max_model_len": 256_000
    }]}));
    let model = &models[0];
    assert_eq!(model.context_window_tokens, Some(64_000));
    assert_eq!(model.tool_calls, Some(false));
    assert_eq!(model.image_inputs, Some(false));
    assert_eq!(model.streaming, Some(false));

    let nested = normalize(json!({"data": [{
        "id": "nested", "capabilities": {"tooling_support": false, "image_inputs": false},
        "supported_parameters": ["tools"], "input_modalities": ["image"]
    }]}));
    assert_eq!(nested[0].tool_calls, Some(false));
    assert_eq!(nested[0].image_inputs, Some(false));
}

#[test]
fn malformed_optional_capabilities_do_not_hide_valid_aliases() {
    let models = normalize(json!({"data": [{
        "id": "nullable", "tool_calls": null, "tooling_support": true,
        "context_window_tokens": null, "context_window": "131072", "max_model_len": 131_072,
        "max_output_tokens": 0, "max_completion_tokens": 16_384,
        "image_inputs": "true", "streaming": 1,
        "capabilities": {"image_inputs": false, "streaming": false}
    }]}));
    assert_eq!(models[0].tool_calls, Some(true));
    assert_eq!(models[0].image_inputs, Some(false));
    assert_eq!(models[0].streaming, Some(false));
    assert_eq!(models[0].context_window_tokens, Some(131_072));
    assert_eq!(models[0].max_output_tokens, Some(16_384));
}

#[test]
fn unrelated_azure_capabilities_and_malformed_extensions_stay_unknown() {
    for capabilities in [
        json!({"chat_completion": true, "completion": true, "inference": true, "embeddings": false}),
        json!({"tooling_support": "true", "image_inputs": 1, "streaming": {"enabled": true}}),
        json!(["tool_calls", "image_inputs", "streaming"]),
    ] {
        let models = normalize(json!({"data": [{
            "id": "unknown", "max_model_len": -1, "max_tokens_field": "max_completion_tokens",
            "capabilities": capabilities
        }]}));
        let model = &models[0];
        assert_eq!(model.context_window_tokens, None);
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.tool_calls, None);
        assert_eq!(model.image_inputs, None);
        assert_eq!(model.streaming, None);
    }
}

#[test]
fn cards_bound_text_and_numbers_and_deduplicate_exact_ids() {
    let models = normalize(json!({"data": [
        {"id": "z"},
        {"id": "a", "name": format!("\u{1b}{}", "é".repeat(MAX_LABEL_BYTES)),
         "description": "界".repeat(MAX_DESCRIPTION_BYTES),
         "context_length": u64::MAX, "max_output_tokens": 0},
        {"id": "a", "name": "Conflicting duplicate"},
        {"id": ""}, {"id": "invalid\nidentifier"}, {"id": " invalid"},
        {"id": "x".repeat(MAX_MODEL_ID_BYTES + 1)}, null
    ]}));
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "z"]
    );
    let model = &models[0];
    assert_eq!(
        model.display_name.as_ref().map(String::len),
        Some(MAX_LABEL_BYTES)
    );
    assert!(
        !model
            .display_name
            .as_ref()
            .expect("name")
            .chars()
            .any(char::is_control)
    );
    assert!(model.description.as_ref().expect("description").len() <= MAX_DESCRIPTION_BYTES);
    assert_eq!(model.context_window_tokens, None);
    assert_eq!(model.max_output_tokens, None);
}

#[test]
fn invalid_or_excessive_catalogs_fail_with_bounded_errors() {
    for value in [
        json!({}),
        json!({"data": []}),
        json!({"data": [{"id": ""}]}),
        json!({"data": vec![json!({"id": "model"}); MAX_CATALOG_MODELS + 1]}),
    ] {
        assert!(normalize_models(&serde_json::to_vec(&value).expect("JSON")).is_err());
    }
}
