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
        connection_network_policy: Default::default(),
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
fn project_admins_can_add_loopback_http_connections_but_external_http_is_rejected() {
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
    input.mcp.servers.get_mut("docs").unwrap().url = Some("http://127.0.0.1:19000/changed".into());
    let changed = reviewed_configuration(&current, input, &current).unwrap();
    assert!(
        crate::global_settings::network::execution(&changed)
            .unwrap()
            .runtime
            .sandbox
            .network_destinations
            .contains(&"http://127.0.0.1:19000".into())
    );
    let mut input = public_configuration(current.clone()).configuration;
    input.mcp.servers.get_mut("docs").unwrap().url = Some("http://remote.example/mcp".into());
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
fn browser_settings_admit_connections_without_host_credential_or_process_authority() {
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
    let updated = reviewed_configuration(&current, input, &current).unwrap();
    assert_eq!(
        updated.runtime.sandbox.network_destinations,
        current.runtime.sandbox.network_destinations
    );
    assert!(
        crate::global_settings::network::execution(&updated)
            .unwrap()
            .runtime
            .sandbox
            .network_destinations
            .contains(&"https://unreviewed.example".into())
    );
    let mut input = public_configuration(current.clone()).configuration;
    input.mcp.servers.get_mut("docs").unwrap().transport = McpTransportKind::Stdio;
    assert!(matches!(
        reviewed_configuration(&current, input, &current),
        Err(CloudError::InvalidArgument)
    ));
}

#[test]
fn mcp_tool_selectors_preserve_credentials_and_operator_authority() {
    let mut current = configuration();
    let server = current.runtime.mcp.servers.get_mut("docs").unwrap();
    server.oauth = None;
    server.credential_headers = serde_json::from_value(json!({
        "Authorization":{"scheme":"Bearer","reference":"host:private-mcp-binding"}
    }))
    .unwrap();
    current
        .mcp_authentication
        .insert("docs".into(), ChatAuthentication::Shared);
    current.mcp_generations.insert("docs".into(), 7);
    let selections: Vec<Vec<String>> = vec![
        vec!["*".into()],
        vec!["get_*".into(), "*_search".into(), "echo".into()],
        vec!["a".repeat(128)],
    ];
    for selectors in selections {
        let mut input = public_configuration(current.clone()).configuration;
        input.mcp.servers.get_mut("docs").unwrap().allowed_tools = selectors.clone();
        let next = reviewed_configuration(&current, input, &current).unwrap();
        assert_eq!(next.runtime.mcp.servers["docs"].allowed_tools, selectors);
        assert_eq!(next.mcp_authentication["docs"], ChatAuthentication::Shared);
        assert_eq!(next.generation(ChatConnectionKind::Mcp, "docs"), 7);
        validate_retained(&next, &current).unwrap();
        let mut runtime = next.runtime.clone();
        runtime.mcp.servers.get_mut("docs").unwrap().allowed_tools =
            current.runtime.mcp.servers["docs"].allowed_tools.clone();
        assert_eq!(
            serde_json::to_value(runtime).unwrap(),
            serde_json::to_value(&current.runtime).unwrap(),
            "selectors cannot change credential custody, sandbox, storage, or policy"
        );
        let projection = serde_json::to_string(&public_configuration(next)).unwrap();
        assert!(!projection.contains("private-mcp-binding"));
    }
}

#[test]
fn mcp_tool_selectors_retain_core_syntax_and_list_bounds() {
    let current = configuration();
    let mut invalid: Vec<Vec<String>> = [
        vec![],
        vec!["*", "read"],
        vec!["get_*", "get_*"],
        vec!["get_**"],
        vec!["^get_.*$"],
        vec!["get_?"],
        vec!["get_[ab]"],
        vec!["get_\\*"],
        vec!["gét_*"],
        vec!["with space"],
        vec!["read\n"],
    ]
    .into_iter()
    .map(|tools| tools.into_iter().map(str::to_owned).collect())
    .collect();
    invalid.push(vec!["a".repeat(129)]);
    invalid.push((0..1025).map(|index| format!("read_{index}")).collect());
    for selectors in invalid {
        let mut input = public_configuration(current.clone()).configuration;
        input.mcp.servers.get_mut("docs").unwrap().allowed_tools = selectors.clone();
        assert!(
            matches!(
                reviewed_configuration(&current, input, &current),
                Err(CloudError::InvalidArgument)
            ),
            "unsupported selectors were accepted: {selectors:?}"
        );
    }
}

#[test]
fn mcp_tool_selectors_cannot_bypass_browser_connection_boundaries() {
    let current = configuration();
    for forbidden in [
        json!({"transport":"stdio"}),
        json!({"command":"/usr/bin/mcp-server"}),
        json!({"environment":{"TOKEN":"env:PRIVATE_MCP_KEY"}}),
        json!({"credentialHeaders":{"Authorization":{"reference":"env:PRIVATE_MCP_KEY"}}}),
    ] {
        let mut input =
            serde_json::to_value(public_configuration(current.clone()).configuration).unwrap();
        let server = input["mcp"]["servers"]["docs"].as_object_mut().unwrap();
        server.insert("allowedTools".into(), json!(["*"]));
        server.extend(forbidden.as_object().unwrap().clone());
        assert!(matches!(
            reviewed_configuration(&current, serde_json::from_value(input).unwrap(), &current),
            Err(CloudError::InvalidArgument)
        ));
    }
}

#[test]
fn project_network_policy_uses_revision_cas_and_never_edits_seed_runtime_authority() {
    let current = configuration();
    let mut input = public_configuration(current.clone()).configuration;
    input.connection_network_policy = Some(
        serde_json::from_value(
            json!({"mode":"allow_list","allowed_origins":["https://models.example"]}),
        )
        .unwrap(),
    );
    let next = reviewed_configuration(&current, input.clone(), &current).unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(
        next.runtime.sandbox.network_destinations,
        current.runtime.sandbox.network_destinations
    );
    assert!(!crate::global_settings::network::assess(&next).mcp["docs"].enabled);
    validate_retained(&next, &current).unwrap();
    assert!(matches!(
        reviewed_configuration(&next, input, &current),
        Err(CloudError::Conflict)
    ));
    let mut omitted = public_configuration(next.clone()).configuration;
    omitted.connection_network_policy = None;
    let preserved = reviewed_configuration(&next, omitted, &current).unwrap();
    assert!(preserved.connection_network_policy == next.connection_network_policy);
    let mut forged = serde_json::to_value(public_configuration(next.clone())).unwrap();
    forged.as_object_mut().unwrap().remove("credential_status");
    forged["connection_availability"]["mcp"]["docs"] = json!({"enabled":true});
    let reviewed =
        reviewed_configuration(&next, serde_json::from_value(forged).unwrap(), &current).unwrap();
    assert!(!crate::global_settings::network::assess(&reviewed).mcp["docs"].enabled);
}
