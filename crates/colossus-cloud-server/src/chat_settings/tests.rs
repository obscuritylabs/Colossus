use super::*;
use serde_json::json;

fn configuration() -> ChatProjectConfig {
    let mut runtime =
        colossus_runtime::RuntimeConfig::controlled_chat_template("/private/hosted/state.redb");
    runtime.storage.keys = serde_json::from_value(json!({"kind":"environment","journal_variable":"CHAT_JOURNAL","journal_key_id":"chat","signing_variable":"CHAT_SIGNING","anchor_path":"/private/hosted/anchor"})).unwrap();
    runtime.providers = serde_json::from_value(json!({"profiles":{"main":{"kind":"open_ai_compatible","baseUrl":"https://models.example/v1","credentialReference":"env:PRIVATE_PROVIDER_KEY"}}})).unwrap();
    runtime.models = serde_json::from_value(json!({"profiles":{"main":{"providerProfile":"main","model":"example-model","contextWindowTokens":32768,"maxOutputTokens":4096}},"roles":{"primary":"main"}})).unwrap();
    runtime.sandbox.network_destinations = vec![
        "https://models.example".into(),
        "https://mcp.example".into(),
    ];
    runtime.mcp = serde_json::from_value(json!({"oauthCredentialStore":"platform","servers":{"docs":{"transport":"streamable_http","url":"https://mcp.example/api","allowedTools":["search"],"oauth":{"clientId":"client","callbackPort":8787,"scopes":["read"]}}}})).unwrap();
    ChatProjectConfig {
        labels: Default::default(),
        codex_auth_shared: false,
        runtime,
        mcp_authentication: BTreeMap::new(),
        provider_authentication: BTreeMap::from([("main".into(), ChatAuthentication::Shared)]),
        provider_generations: BTreeMap::new(),
        mcp_generations: BTreeMap::new(),
        search_authentication: BTreeMap::new(),
        search_generations: BTreeMap::new(),
        variables: BTreeMap::new(),
        inheritance: Default::default(),
        catalog_disabled: Default::default(),
        revision: 1,
        codex_auth_path: None,
    }
}

#[test]
fn settings_projection_releases_no_host_paths_keys_or_credential_references() {
    let projection = serde_json::to_string(&public_configuration(configuration())).unwrap();
    for hidden in [
        "PRIVATE_PROVIDER_KEY",
        "PRIVATE_CLIENT_SECRET",
        "journal_variable",
        "anchor_path",
        "/private/hosted",
        "storage",
        "sandbox",
        "codex_auth_path",
    ] {
        assert!(
            !projection.contains(hidden),
            "private host authority appeared: {hidden}"
        );
    }
    assert!(projection.contains("example-model"));
    assert!(projection.contains("https://mcp.example/api"));
}

#[test]
fn model_changes_keep_auth_bindings_and_host_policy_unchanged() {
    let current = configuration();
    colossus_runtime::RuntimeConfig::from_yaml(&current.runtime.to_yaml().unwrap())
        .expect("valid reviewed fixture");
    let mut input = public_configuration(current.clone()).configuration;
    input.models.profiles.get_mut("main").unwrap().model = "other-model".into();
    let next = reviewed_configuration(&current, input, &current).unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(next.generation(ChatConnectionKind::Provider, "main"), 1);
    assert_eq!(next.generation(ChatConnectionKind::Mcp, "docs"), 1);
    assert_eq!(
        next.runtime.providers.profiles["main"]
            .credential_reference
            .as_deref(),
        Some("env:PRIVATE_PROVIDER_KEY")
    );
    assert_eq!(
        next.runtime.mcp.servers["docs"]
            .oauth
            .as_ref()
            .unwrap()
            .client_secret_reference
            .as_deref(),
        None
    );
    assert_eq!(
        next.runtime.sandbox.network_destinations,
        current.runtime.sandbox.network_destinations
    );
}

#[test]
fn display_labels_roundtrip_outside_runtime_and_survive_legacy_writes() {
    let current = configuration();
    let runtime_before = serde_json::to_value(&current.runtime).unwrap();
    let mut input = public_configuration(current.clone()).configuration;
    input.labels = Some(
        serde_json::from_value(json!({
            "providers":{"main":"Production provider"},
            "models":{"main":"Primary reasoning model"}
        }))
        .unwrap(),
    );
    let next = reviewed_configuration(&current, input, &current).unwrap();
    assert_eq!(serde_json::to_value(&next.runtime).unwrap(), runtime_before);
    assert_eq!(next.generation(ChatConnectionKind::Provider, "main"), 1);
    let snapshot = serde_json::to_value(public_configuration(next.clone())).unwrap();
    assert_eq!(
        snapshot["labels"]["providers"]["main"],
        "Production provider"
    );
    assert_eq!(
        snapshot["labels"]["models"]["main"],
        "Primary reasoning model"
    );
    let mut legacy = snapshot;
    legacy.as_object_mut().unwrap().remove("labels");
    legacy.as_object_mut().unwrap().remove("credential_status");
    legacy.as_object_mut().unwrap().remove("inheritance");
    let input: SettingsInput = serde_json::from_value(legacy).unwrap();
    let restored = reviewed_configuration(&next, input, &current).unwrap();
    assert_eq!(restored.labels.providers["main"], "Production provider");
    assert_eq!(restored.labels.models["main"], "Primary reasoning model");
    let mut retained = serde_json::to_value(&restored).unwrap();
    retained.as_object_mut().unwrap().remove("labels");
    let migrated: ChatProjectConfig = serde_json::from_value(retained).unwrap();
    assert!(migrated.labels.providers.is_empty());
    assert!(migrated.labels.models.is_empty());
    validate_retained(&restored, &current).unwrap();
}

#[test]
fn display_labels_reject_unknown_profiles_control_characters_and_oversized_values() {
    let current = configuration();
    for labels in [
        json!({"providers":{"missing":"Unknown"}}),
        json!({"models":{"missing":"Unknown"}}),
        json!({"providers":{"main":"   "}}),
        json!({"models":{"main":"new\nline"}}),
        json!({"providers":{"main":"a".repeat(97)}}),
        json!({"models":{"main":"界".repeat(33)}}),
    ] {
        let mut input = public_configuration(current.clone()).configuration;
        input.labels = Some(serde_json::from_value(labels).unwrap());
        assert!(matches!(
            reviewed_configuration(&current, input, &current),
            Err(CloudError::InvalidArgument)
        ));
    }
}

#[test]
fn model_edits_preserve_reviewed_loopback_mcp_but_cannot_add_http_destinations() {
    let mut current = configuration();
    current
        .runtime
        .sandbox
        .network_destinations
        .push("http://127.0.0.1:18999".into());
    current.runtime.mcp.servers.get_mut("docs").unwrap().url =
        Some("http://127.0.0.1:18999/mcp".into());
    let mut input = public_configuration(current.clone()).configuration;
    input.models.profiles.get_mut("main").unwrap().model = "new-model".into();
    let next = reviewed_configuration(&current, input, &current).unwrap();
    assert_eq!(
        next.runtime.mcp.servers["docs"].url,
        current.runtime.mcp.servers["docs"].url
    );
    let mut input = public_configuration(current.clone()).configuration;
    input.mcp.servers.get_mut("docs").unwrap().url = Some("http://127.0.0.1:18999/changed".into());
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
}

#[test]
fn retained_settings_cannot_replace_operator_authority() {
    let seed = configuration();
    let mut input = public_configuration(seed.clone()).configuration;
    input.models.profiles.get_mut("main").unwrap().model = "updated-model".into();
    let retained = reviewed_configuration(&seed, input, &seed).unwrap();
    validate_retained(&retained, &seed).unwrap();
    let mut changed = retained.clone();
    changed
        .runtime
        .sandbox
        .network_destinations
        .push("https://other.example".into());
    assert!(matches!(
        validate_retained(&changed, &seed),
        Err(CloudError::Storage)
    ));
    let mut changed = retained.clone();
    changed.runtime.storage.path = "/another/user/state.redb".into();
    assert!(matches!(
        validate_retained(&changed, &seed),
        Err(CloudError::Storage)
    ));
    let mut changed = retained;
    changed.codex_auth_path = Some("/another/user/auth.json".into());
    assert!(matches!(
        validate_retained(&changed, &seed),
        Err(CloudError::Storage)
    ));
}

#[test]
fn endpoint_and_scope_changes_retire_old_bindings_without_reusing_keys() {
    let current = configuration();
    let mut input = public_configuration(current.clone()).configuration;
    input.providers.profiles.get_mut("main").unwrap().base_url =
        Some("https://models.example/v2".into());
    input
        .mcp
        .servers
        .get_mut("docs")
        .unwrap()
        .oauth
        .as_mut()
        .unwrap()
        .scopes
        .push("write".into());
    let next = reviewed_configuration(&current, input, &current).unwrap();
    assert_eq!(next.generation(ChatConnectionKind::Provider, "main"), 2);
    assert_eq!(next.generation(ChatConnectionKind::Mcp, "docs"), 2);
    assert!(
        next.runtime.providers.profiles["main"]
            .credential_reference
            .is_none()
    );
    assert!(
        next.runtime.mcp.servers["docs"]
            .oauth
            .as_ref()
            .unwrap()
            .client_secret_reference
            .is_none()
    );
}

#[test]
fn browser_settings_cannot_nominate_host_credentials_processes_or_new_origins() {
    let current = configuration();
    let mut input = public_configuration(current.clone()).configuration;
    input
        .providers
        .profiles
        .get_mut("main")
        .unwrap()
        .credential_reference = Some("env:OIDC_CLIENT_SECRET".into());
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
    let mut input = public_configuration(current.clone()).configuration;
    input.providers.profiles.get_mut("main").unwrap().base_url =
        Some("https://unreviewed.example/v1".into());
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
    let mut input = public_configuration(current.clone()).configuration;
    input.mcp.servers.get_mut("docs").unwrap().transport = McpTransportKind::Stdio;
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
    let mut input = public_configuration(current.clone()).configuration;
    input.mcp.servers.get_mut("docs").unwrap().allowed_tools = vec!["*".into()];
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
}
