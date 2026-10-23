use super::*;
use serde_json::json;

fn configuration() -> ChatProjectConfig {
    let mut runtime =
        colossus_runtime::RuntimeConfig::controlled_chat_template("/unused/policy.redb");
    runtime.providers.profiles.insert(
        "remote".into(),
        serde_json::from_value(
            json!({"kind":"open_ai_compatible","baseUrl":"https://models.example/v1"}),
        )
        .unwrap(),
    );
    runtime.models.profiles.insert("remote".into(), serde_json::from_value(json!({"providerProfile":"remote","model":"remote.model/1","contextWindowTokens":32768,"maxOutputTokens":4096})).unwrap());
    runtime.mcp.servers.insert("tools".into(), serde_json::from_value(json!({"transport":"streamable_http","url":"https://tools.example/mcp","allowedTools":["get_*"]})).unwrap());
    runtime.search = serde_json::from_value(json!({"profiles":{"web":{"kind":"searxng","endpoint":"https://search.example/search"}},"roles":{"agent":"web"}})).unwrap();
    ChatProjectConfig::new(runtime)
}

#[test]
fn default_admits_exact_connection_origins_without_wildcards_or_other_authority_changes() {
    let full = configuration();
    let mut expected = full.runtime.clone();
    let executable = execution(&full).unwrap();
    assert!(
        executable
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://models.example".into())
    );
    assert!(
        executable
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://tools.example".into())
    );
    assert!(
        executable
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://search.example".into())
    );
    assert!(
        !executable
            .runtime
            .sandbox
            .network_destinations
            .contains(&"*".into())
    );
    expected.sandbox.network_destinations = executable.runtime.sandbox.network_destinations.clone();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(executable.runtime).unwrap()
    );
    assert!(full.runtime.sandbox.network_destinations.is_empty());
}

#[test]
fn restrictions_keep_full_definitions_but_remove_blocked_runtime_and_credential_targets() {
    let mut full = configuration();
    full.connection_network_policy = serde_json::from_value(
        json!({"mode":"allow_list","allowed_origins":["https://tools.example"]}),
    )
    .unwrap();
    let availability = assess(&full);
    assert!(!availability.providers["remote"].enabled);
    assert!(!availability.models["remote"].enabled);
    assert!(!availability.search["web"].enabled);
    assert!(!availability.roles["search:agent"].enabled);
    assert!(availability.mcp["tools"].enabled);
    assert!(!connection_enabled(
        &full,
        ChatConnectionKind::Provider,
        "remote"
    ));
    let executable = execution(&full).unwrap();
    assert!(!executable.runtime.providers.profiles.contains_key("remote"));
    assert!(!executable.runtime.models.profiles.contains_key("remote"));
    assert!(executable.runtime.mcp.servers.contains_key("tools"));
    assert!(executable.runtime.search.profiles.is_empty());
    assert!(full.runtime.providers.profiles.contains_key("remote"));
    assert!(full.runtime.models.profiles.contains_key("remote"));
    assert_eq!(
        executable.runtime.sandbox.network_destinations,
        ["https://tools.example"]
    );
}

#[test]
fn primary_dependency_failure_is_explicit_and_never_substitutes_echo() {
    let mut full = configuration();
    full.runtime
        .models
        .roles
        .insert("primary".into(), "remote".into());
    full.connection_network_policy =
        serde_json::from_value(json!({"mode":"allow_list","allowed_origins":[]})).unwrap();
    assert!(!assess(&full).roles["primary"].enabled);
    assert!(matches!(
        execution(&full),
        Err(CloudError::PermissionDenied)
    ));
    assert_eq!(full.runtime.models.roles["primary"], "remote");
}

#[test]
fn codex_fixed_api_and_auth_origins_are_both_subject_to_policy() {
    let mut full = configuration();
    full.runtime.providers.profiles.insert(
        "codex".into(),
        serde_json::from_value(
            json!({"kind":"open_ai_codex","credentialReference":"codex:default"}),
        )
        .unwrap(),
    );
    full.connection_network_policy = serde_json::from_value(
        json!({"mode":"allow_list","allowed_origins":["https://chatgpt.com"]}),
    )
    .unwrap();
    assert!(!assess(&full).providers["codex"].enabled);
    full.connection_network_policy
        .allowed_origins
        .insert(colossus_codex_auth::CODEX_AUTH_ORIGIN.into());
    assert!(assess(&full).providers["codex"].enabled);
    let config = execution(&full).unwrap();
    assert!(
        config
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://chatgpt.com".into())
    );
    assert!(
        config
            .runtime
            .sandbox
            .network_destinations
            .contains(&colossus_codex_auth::CODEX_AUTH_ORIGIN.into())
    );
}

#[test]
fn exact_origin_policy_rejects_ambient_or_noncanonical_authority() {
    for origin in [
        "*",
        "https://*.example",
        "https://example/path",
        "https://example/",
        "https://user:pass@example",
        "https://example?token=x",
        "https://example#part",
        "file:///tmp",
        "HTTPS://EXAMPLE",
        "https://example:443",
    ] {
        let policy: ConnectionNetworkPolicy =
            serde_json::from_value(json!({"mode":"allow_list","allowed_origins":[origin]}))
                .unwrap();
        assert!(policy.validate().is_err(), "accepted {origin}");
    }
    for origin in [
        "https://example",
        "https://example:8443",
        "http://127.0.0.1:8000",
        "http://[::1]:8000",
    ] {
        let policy: ConnectionNetworkPolicy =
            serde_json::from_value(json!({"mode":"allow_list","allowed_origins":[origin]}))
                .unwrap();
        policy.validate().unwrap();
    }
    let policy = ConnectionNetworkPolicy {
        mode: ConnectionNetworkMode::AllowList,
        allowed_origins: (0..257)
            .map(|id| format!("https://host{id}.example"))
            .collect(),
    };
    assert!(matches!(
        policy.validate(),
        Err(CloudError::ResourceExhausted)
    ));
}

#[test]
fn restricted_auth_origins_replace_seed_egress_and_allow_all_ignores_inactive_list() {
    let mut full = configuration();
    full.runtime
        .sandbox
        .network_destinations
        .push("https://old-auth.example".into());
    full.connection_network_policy = serde_json::from_value(json!({"mode":"allow_list","allowed_origins":["https://tools.example","https://auth.example"]})).unwrap();
    let projected = execution(&full).unwrap();
    assert_eq!(
        projected.runtime.sandbox.network_destinations,
        ["https://auth.example", "https://tools.example"]
    );
    assert!(
        !projected
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://old-auth.example".into())
    );
    full.connection_network_policy.mode = ConnectionNetworkMode::AllowAll;
    let projected = execution(&full).unwrap();
    assert!(
        projected
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://old-auth.example".into())
    );
    assert!(
        !projected
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://auth.example".into())
    );
}
