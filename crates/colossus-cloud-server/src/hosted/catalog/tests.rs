use super::*;
use serde_json::json;

fn configuration() -> ChatProjectConfig {
    ChatProjectConfig {
        runtime: colossus_runtime::RuntimeConfig::controlled_chat_template("/private/catalog.redb"),
        labels: Default::default(),
        mcp_authentication: BTreeMap::new(),
        provider_authentication: BTreeMap::new(),
        provider_generations: BTreeMap::new(),
        mcp_generations: BTreeMap::new(),
        revision: 1,
        codex_auth_path: None,
        codex_auth_shared: false,
    }
}

#[test]
fn complete_provider_inventory_reports_availability_without_releasing_host_configuration() {
    let mut current = configuration();
    current.runtime.sandbox.network_destinations = vec![
        "https://api.openai.com".into(),
        "https://private-not-released.example".into(),
    ];
    let value = preset_catalog(&current, &current);
    assert_eq!(
        value["presets"],
        json!(colossus_contracts::provider_presets())
    );
    assert_eq!(value["availability"].as_object().unwrap().len(), 12);
    assert_eq!(value["availability"]["openai"]["status"], "available");
    assert_eq!(
        value["availability"]["anthropic"]["status"],
        "deployment_required"
    );
    assert_eq!(
        value["availability"]["codex"]["status"],
        "deployment_required"
    );
    assert_eq!(
        value["availability"]["ollama"]["status"],
        "deployment_required"
    );
    assert_eq!(
        value["availability"]["lmstudio"]["status"],
        "deployment_required"
    );
    assert_eq!(
        value["availability"]["custom-chat"]["status"],
        "configure_endpoint"
    );
    assert_eq!(
        value["availability"]["custom-responses"]["status"],
        "configure_endpoint"
    );
    let serialized = value.to_string();
    assert!(!serialized.contains("private-not-released"));
    assert!(!serialized.contains("/private/catalog"));
    assert!(!serialized.contains("network_destinations"));
}

#[test]
fn configured_codex_is_editable_but_never_advertises_new_account_enrollment() {
    let mut current = configuration();
    current.codex_auth_shared = true;
    current.runtime.providers.profiles.insert(
        "subscription".into(),
        serde_json::from_value(
            json!({"kind":"open_ai_codex","credentialReference":"codex:default"}),
        )
        .unwrap(),
    );
    let value = preset_catalog(&current, &current);
    assert_eq!(value["availability"]["codex"]["status"], "existing_only");
    current.runtime.providers.profiles.remove("subscription");
    assert_eq!(
        preset_catalog(&current, &current)["availability"]["codex"]["status"],
        "deployment_required"
    );
}

#[test]
fn local_presets_require_retained_operator_connection_and_exact_network_policy() {
    let mut current = configuration();
    current.runtime.sandbox.network_destinations = vec!["http://localhost:11434".into()];
    current.runtime.providers.profiles.insert(
        "local".into(),
        serde_json::from_value(
            json!({"kind":"open_ai_compatible","baseUrl":"http://localhost:11434/v1"}),
        )
        .unwrap(),
    );
    let mut ceiling = current.clone();
    assert_eq!(
        preset_catalog(&current, &ceiling)["availability"]["ollama"]["status"],
        "existing_only"
    );
    ceiling.runtime.providers.profiles.remove("local");
    assert_eq!(
        preset_catalog(&current, &ceiling)["availability"]["ollama"]["status"],
        "deployment_required"
    );
    ceiling.runtime.providers = current.runtime.providers.clone();
    ceiling.runtime.sandbox.network_destinations.clear();
    assert_eq!(
        preset_catalog(&current, &ceiling)["availability"]["ollama"]["status"],
        "deployment_required"
    );
}
