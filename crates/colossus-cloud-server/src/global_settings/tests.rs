//! Effective inheritance and explicit global credential grants retain project authority.
use super::*;
use colossus_cloud::CloudRepository;
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, ChatCredentialResolver,
    CredentialStore, StoredCredentialKind, StoredCredentialMetadata,
};
use colossus_contracts::{HostSecret, VaultRecord};
use colossus_ports::{CredentialKey, CredentialResolutionError, CredentialResolver};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn configuration() -> crate::config::ChatProjectConfig {
    let mut runtime = colossus_runtime::RuntimeConfig::controlled_chat_template(
        "/private/hosted/global-test.redb",
    );
    runtime.storage.keys = serde_json::from_value(json!({"kind":"environment","journal_variable":"TEST_JOURNAL","journal_key_id":"test","signing_variable":"TEST_SIGNING","anchor_path":"/private/hosted/test.anchor"})).unwrap();
    runtime.providers = serde_json::from_value(json!({"profiles":{"main":{"kind":"open_ai_compatible","baseUrl":"https://models.example/v1"}}})).unwrap();
    runtime.models = serde_json::from_value(json!({"profiles":{"main":{"providerProfile":"main","model":"project-model","contextWindowTokens":32768,"maxOutputTokens":4096}},"roles":{"primary":"main"}})).unwrap();
    runtime.sandbox.network_destinations = vec![
        "https://models.example".into(),
        "https://search.example".into(),
    ];
    serde_json::from_value(json!({"runtime":runtime,"revision":1})).unwrap()
}

fn catalog(seed: &crate::config::ChatProjectConfig) -> HostedCatalog {
    let mut global = HostedCatalog {
        revision: 7,
        ..Default::default()
    };
    global.providers.profiles.insert(
        "shared".into(),
        seed.runtime.providers.profiles["main"].clone(),
    );
    let mut model = seed.runtime.models.profiles["main"].clone();
    model.provider_profile = "shared".into();
    model.model = "global-model".into();
    global.models.profiles.insert("shared".into(), model);
    global.provider_generations.insert("shared".into(), 2);
    global.variables.insert("REGION".into(), "global".into());
    global
}

fn settings(config: &crate::config::ChatProjectConfig, revision: u64) -> Value {
    json!({
        "revision":config.revision,"labels":config.labels,
        "providers":config.runtime.providers,"models":config.runtime.models,"mcp":config.runtime.mcp,
        "search":search::public_search(&config.runtime.search),
        "provider_authentication":config.provider_authentication,"mcp_authentication":config.mcp_authentication,
        "search_authentication":config.search_authentication,"variables":config.variables,
        "observability":config.runtime.observability,"global_revision":revision
    })
}

#[test]
fn catalog_only_projection_inherits_definitions_without_operator_authority() {
    let global = catalog(&configuration());
    let projection = inheritance::catalog_only(&global);
    assert_eq!(
        projection.runtime.models.profiles["shared"].model,
        "global-model"
    );
    assert_eq!(projection.variables["REGION"], "global");
    assert_eq!(projection.inheritance.global_revision, global.revision);
    assert_eq!(
        projection.inheritance.provenance.providers["shared"],
        "global"
    );
    assert!(projection.inheritance.overrides.providers.is_empty());
    assert!(projection.runtime.sandbox.network_destinations.is_empty());
    assert!(projection.codex_auth_path.is_none());
    assert!(!projection.codex_auth_shared);
}

#[test]
fn definitions_inherit_while_project_overrides_and_resets_remain_explicit() {
    let seed = configuration();
    let mut global = catalog(&seed);
    let mut raw = seed.clone();
    raw.variables.insert("REGION".into(), "project".into());
    let current = inheritance::resolve(&raw, &global, &seed).unwrap();
    assert_eq!(current.inheritance.provenance.providers["shared"], "global");
    assert_eq!(current.variables["REGION"], "project");
    let mut input = settings(&current, global.revision);
    input["models"]["profiles"]["shared"]["model"] = json!("project-override");
    let updated = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    let local = inheritance::local_configuration(&updated);
    assert!(local.runtime.models.profiles.contains_key("shared"));
    assert!(!local.runtime.providers.profiles.contains_key("shared"));
    global.models.profiles.get_mut("shared").unwrap().model = "updated-global".into();
    global.revision += 1;
    let current = inheritance::resolve(&local, &global, &seed).unwrap();
    assert_eq!(
        current.runtime.models.profiles["shared"].model,
        "project-override"
    );
    let mut input = settings(&current, global.revision);
    input["inheritance"] = json!({"global_revision":global.revision,"reset":{"models":["shared"],"variables":["REGION"]}});
    let reset = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(
        reset.runtime.models.profiles["shared"].model,
        "updated-global"
    );
    assert_eq!(reset.variables["REGION"], "global");
    assert!(
        !inheritance::local_configuration(&reset)
            .runtime
            .models
            .profiles
            .contains_key("shared")
    );
}

#[test]
fn stale_global_revision_is_rejected_and_reset_recomputes_project_availability() {
    let seed = configuration();
    let mut global = catalog(&seed);
    let current = inheritance::resolve(&seed, &global, &seed).unwrap();
    let input = serde_json::from_value(settings(&current, global.revision - 1)).unwrap();
    assert!(matches!(
        crate::chat_settings::reviewed_project_update(&current, input, &global, &seed),
        Err(CloudError::Conflict)
    ));
    // A project restriction affects use of the reset global definition, not its storage.
    global.providers.profiles.insert(
        "main".into(),
        seed.runtime.providers.profiles["main"].clone(),
    );
    global.providers.profiles.get_mut("main").unwrap().base_url =
        Some("https://unreviewed.example/v1".into());
    let current = inheritance::resolve(&seed, &global, &seed).unwrap();
    let mut input = settings(&current, global.revision);
    input["inheritance"] =
        json!({"global_revision":global.revision,"reset":{"providers":["main"]}});
    input["connection_network_policy"] =
        json!({"mode":"allow_list","allowed_origins":["https://models.example"]});
    let reset = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(
        reset.runtime.providers.profiles["main"].base_url.as_deref(),
        Some("https://unreviewed.example/v1")
    );
    assert!(!network::assess(&reset).providers["main"].enabled);
    assert!(matches!(
        network::execution(&reset),
        Err(CloudError::PermissionDenied)
    ));
}

#[test]
fn mcp_tool_selectors_flow_from_global_definitions_through_explicit_project_overrides() {
    let seed = configuration();
    let mut global = catalog(&seed);
    global.mcp.servers = serde_json::from_value(json!({
        "docs":{"transport":"streamable_http","url":"https://models.example/mcp",
            "allowedTools":["get_*","*_search","echo"]}
    }))
    .unwrap();
    intrinsic::review(&global).unwrap();
    let inherited = inheritance::resolve(&seed, &global, &seed).unwrap();
    assert_eq!(inherited.inheritance.provenance.mcp["docs"], "global");
    let workspace = tempfile::tempdir().unwrap();
    let mut runtime = network::execution(&inherited).unwrap().runtime;
    runtime.use_ephemeral_storage();
    colossus_runtime::Runtime::open_with_options(
        &runtime,
        std::sync::Arc::new(colossus_policy::DenyApproval),
        None,
        colossus_runtime::RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
    )
    .expect("inherited selector patterns must pass actual controlled runtime admission");
    assert_eq!(
        inherited.runtime.mcp.servers["docs"].allowed_tools,
        ["get_*", "*_search", "echo"]
    );
    assert_eq!(
        serde_json::to_value(&inherited.runtime.sandbox).unwrap(),
        serde_json::to_value(&seed.runtime.sandbox).unwrap()
    );
    assert!(
        inherited.runtime.mcp.servers["docs"]
            .credential_headers
            .is_empty()
    );
    assert_eq!(
        inherited.mcp_authentication["docs"],
        ChatAuthentication::Personal
    );

    let mut input = settings(&inherited, global.revision);
    input["mcp"]["servers"]["docs"]["allowedTools"] = json!(["*"]);
    let overridden = crate::chat_settings::reviewed_project_update(
        &inherited,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(overridden.inheritance.provenance.mcp["docs"], "project");
    assert_eq!(overridden.runtime.mcp.servers["docs"].allowed_tools, ["*"]);
    let mut input = settings(&overridden, global.revision);
    input["inheritance"] = json!({"global_revision":global.revision,"reset":{"mcp":["docs"]}});
    let reset = crate::chat_settings::reviewed_project_update(
        &overridden,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(reset.inheritance.provenance.mcp["docs"], "global");
    assert_eq!(
        reset.runtime.mcp.servers["docs"].allowed_tools,
        ["get_*", "*_search", "echo"]
    );
    crate::chat_settings::validate_retained(&reset, &seed).unwrap();

    global.mcp.servers.get_mut("docs").unwrap().allowed_tools = vec!["*".into()];
    intrinsic::review(&global).unwrap();
    let dormant = inheritance::catalog_only(&global);
    assert_eq!(dormant.runtime.mcp.servers["docs"].allowed_tools, ["*"]);
    assert!(dormant.runtime.sandbox.network_destinations.is_empty());
    global.mcp.servers.get_mut("docs").unwrap().url = Some("https://unreviewed.example/mcp".into());
    let mut restricted = seed.clone();
    restricted.connection_network_policy = serde_json::from_value(
        json!({"mode":"allow_list","allowed_origins":["https://models.example"]}),
    )
    .unwrap();
    let full = inheritance::resolve(&restricted, &global, &seed).unwrap();
    assert!(!network::assess(&full).mcp["docs"].enabled);
    assert!(
        !network::execution(&full)
            .unwrap()
            .runtime
            .mcp
            .servers
            .contains_key("docs")
    );
}

#[test]
fn mcp_tool_selectors_are_core_validated_before_a_global_definition_is_available() {
    let mut global = HostedCatalog::default();
    global.mcp.servers = serde_json::from_value(json!({
        "docs":{"transport":"streamable_http","url":"https://models.example/mcp",
            "allowedTools":["read"]}
    }))
    .unwrap();
    for selectors in [
        vec![],
        vec!["*".into(), "get_*".into()],
        vec!["get_*".into(), "get_*".into()],
        vec!["^get_.*$".into()],
        vec!["get_**".into()],
        vec!["a".repeat(129)],
    ] {
        global.mcp.servers.get_mut("docs").unwrap().allowed_tools = selectors;
        assert!(matches!(
            intrinsic::review(&global),
            Err(CloudError::InvalidArgument)
        ));
    }
}

#[test]
fn variables_are_bounded_visible_configuration_and_collector_is_global_only() {
    let seed = configuration();
    let global = catalog(&seed);
    let current = inheritance::resolve(&seed, &global, &seed).unwrap();
    let mut values = BTreeMap::from([("REGION".into(), "region-a".into())]);
    assert!(inheritance::validate_variables(&values).is_ok());
    values.insert("INVALID-KEY".into(), "value".into());
    assert_eq!(
        inheritance::validate_variables(&values),
        Err(CloudError::InvalidArgument)
    );
    values.remove("INVALID-KEY");
    values.insert("REGION".into(), "界".repeat(1366));
    assert_eq!(
        inheritance::validate_variables(&values),
        Err(CloudError::InvalidArgument)
    );
    let mut input = settings(&current, global.revision);
    input["observability"]["serviceName"] = json!("project-collector-override");
    assert!(matches!(
        crate::chat_settings::reviewed_project_update(
            &current,
            serde_json::from_value(input).unwrap(),
            &global,
            &seed
        ),
        Err(CloudError::InvalidArgument)
    ));
}

#[test]
fn unchanged_authenticated_search_does_not_become_a_project_override() {
    let seed = configuration();
    let mut global = catalog(&seed);
    global.search = serde_json::from_value(json!({"profiles":{"search":{"kind":"searxng","endpoint":"https://search.example/search"}}})).unwrap();
    global
        .search_authentication
        .insert("search".into(), ChatAuthentication::Personal);
    global.search_generations.insert("search".into(), 1);
    inheritance::normalize_search(&mut global).unwrap();
    let current = inheritance::resolve(&seed, &global, &seed).unwrap();
    let updated = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(settings(&current, global.revision)).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(updated.inheritance.provenance.search["search"], "global");
    assert!(
        !inheritance::local_configuration(&updated)
            .runtime
            .search
            .profiles
            .contains_key("search")
    );
}

#[test]
fn personal_search_override_of_anonymous_inheritance_survives_save_and_resets_explicitly() {
    let seed = configuration();
    let mut global = catalog(&seed);
    global.search = serde_json::from_value(json!({
        "profiles":{"web":{"kind":"searxng","endpoint":"https://search.example/search","credentialReference":null,"authHeader":"X-Search-Key","userAgent":"colossus","timeoutMs":30000}},
        "roles":{"agent":"web"}
    })).unwrap();
    global.search_generations.insert("web".into(), 1);
    let current = inheritance::resolve(&seed, &global, &seed).unwrap();
    assert_eq!(
        search::authentication(&current, "web"),
        ChatAuthentication::None
    );
    let mut input = settings(&current, global.revision);
    input["search_authentication"] = json!({"web":"personal"});
    let updated = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(
        search::authentication(&updated, "web"),
        ChatAuthentication::Personal
    );
    let local = inheritance::local_configuration(&updated);
    assert!(local.runtime.search.profiles.contains_key("web"));
    assert_eq!(
        local.search_authentication["web"],
        ChatAuthentication::Personal
    );
    let current = inheritance::resolve(&local, &global, &seed).unwrap();
    assert_eq!(
        search::authentication(&current, "web"),
        ChatAuthentication::Personal
    );
    let mut input = settings(&current, global.revision);
    input["inheritance"] = json!({"global_revision":global.revision,"reset":{"search":["web"]}});
    let reset = crate::chat_settings::reviewed_project_update(
        &current,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(
        search::authentication(&reset, "web"),
        ChatAuthentication::None
    );
    assert_eq!(reset.inheritance.provenance.search["web"], "global");
    assert!(
        !inheritance::local_configuration(&reset)
            .runtime
            .search
            .profiles
            .contains_key("web")
    );
}

const INSTANCE_SECRET_CHILD: &str = "COLOSSUS_INSTANCE_SECRET_TEST_CHILD";
const WRAPPING_KEY: &str = "COLOSSUS_INSTANCE_SECRET_TEST_WRAPPING";

#[tokio::test]
async fn instance_credentials_require_exact_grants_and_preserve_disconnect_and_rotation() {
    if std::env::var_os(INSTANCE_SECRET_CHILD).is_none() {
        let output = tokio::time::timeout(std::time::Duration::from_secs(60), tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "global_settings::tests::instance_credentials_require_exact_grants_and_preserve_disconnect_and_rotation", "--test-threads=1"])
            .env(INSTANCE_SECRET_CHILD,"1").env(WRAPPING_KEY,"a".repeat(64)).kill_on_drop(true).output()).await.unwrap().unwrap();
        assert!(
            output.status.success(),
            "isolated instance custody regression failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut restricted = configuration();
    restricted.connection_network_policy = serde_json::from_value(json!({"mode":"allow_list","allowed_origins":["https://models.example","https://search.example"]})).unwrap();
    let deployment = crate::config::ChatConfig {
        shared: None,
        home: directory.path().join("hosted"),
        wrapping_key_variable: WRAPPING_KEY.into(),
        max_active_runtimes: 1,
        idle_seconds: 300,
        projects: BTreeMap::from([("project-a".into(), restricted.clone())]),
    };
    let database = serde_json::from_value(json!({"connectionVariable":"UNUSED_INSTANCE_TEST_DATABASE","schema":"instance_test","tls":{"kind":"disabled"},"maxConnections":1,"connectionTimeoutMs":5000,"statementTimeoutMs":5000})).unwrap();
    let host = HostedExecution::open(
        deployment,
        CloudRepository::new(Arc::new(
            colossus_cloud::storage::MemoryCloudStore::default(),
        ))
        .unwrap(),
        database,
    )
    .unwrap();
    // An instance administrator can publish intrinsically valid definitions
    // even when every consuming project's policy blocks their destinations.
    let mut outside = HostedCatalog {
        providers: serde_json::from_value(json!({"profiles":{"outside":{"kind":"open_ai_compatible","baseUrl":"https://outside.example/v1"}}})).unwrap(),
        models: serde_json::from_value(json!({"profiles":{"outside":{"providerProfile":"outside","model":"outside","contextWindowTokens":32768,"maxOutputTokens":4096}},"roles":{}})).unwrap(),
        mcp: colossus_mcp::McpConfig {
            servers: serde_json::from_value(json!({"outside":{"transport":"streamable_http","url":"https://outside.example/mcp","allowedTools":["*"]}})).unwrap(),
            ..Default::default()
        },
        search: serde_json::from_value(json!({"profiles":{"outside":{"kind":"searxng","endpoint":"https://outside.example/search"}}})).unwrap(),
        ..Default::default()
    };
    inheritance::review_catalog(&host, &HostedCatalog::default(), &mut outside).unwrap();
    let merged = inheritance::resolve(&restricted, &outside, &restricted).unwrap();
    assert!(!network::assess(&merged).providers["outside"].enabled);
    assert!(!network::assess(&merged).mcp["outside"].enabled);
    assert!(!network::assess(&merged).search["outside"].enabled);
    let executable = network::execution(&merged).unwrap();
    assert!(executable.runtime.providers.profiles.contains_key("main"));
    assert!(
        !executable
            .runtime
            .providers
            .profiles
            .contains_key("outside")
    );
    let binding = ChatConnectionBinding::new(
        "project-a",
        "alice",
        ChatConnectionKind::Search,
        "search",
        1,
        "https://search.example/search",
        ChatAuthentication::Shared,
    )
    .unwrap()
    .unwrap()
    .with_catalog_generation(1)
    .unwrap();
    let vault = host.instance_vault().unwrap();
    let mut store = CredentialStore::load(vault.as_ref()).unwrap();
    let metadata = StoredCredentialMetadata {
        id: "instance-key".into(),
        label: "Search service key".into(),
        kind: StoredCredentialKind::ApiKey,
    };
    store
        .create(metadata.clone(), &HostSecret::new("first-secret").unwrap())
        .unwrap();
    store.commit(vault.as_ref()).unwrap();
    assert!(
        credentials::assigned_vault(&host, &binding)
            .unwrap()
            .is_none(),
        "saving an instance secret grants no connection access"
    );
    store.attach(&binding, Some("instance-key")).unwrap();
    store.commit(vault.as_ref()).unwrap();
    let resolver = ChatCredentialResolver::new(
        "project-a",
        "alice",
        [(
            binding.clone(),
            credentials::assigned_vault(&host, &binding)
                .unwrap()
                .unwrap(),
        )],
    )
    .unwrap();
    let reference = ChatCredentialResolver::reference(&binding);
    assert_eq!(resolver.resolve(&reference).unwrap(), "first-secret");
    store
        .update(metadata, Some(&HostSecret::new("rotated-secret").unwrap()))
        .unwrap();
    store.commit(vault.as_ref()).unwrap();
    assert_eq!(resolver.resolve(&reference).unwrap(), "rotated-secret");
    store.attach(&binding, None).unwrap();
    store.commit(vault.as_ref()).unwrap();
    assert!(
        credentials::assigned_vault(&host, &binding)
            .unwrap()
            .is_some(),
        "a global disconnect remains authoritative rather than falling back"
    );
    assert_eq!(
        resolver.resolve(&reference),
        Err(CredentialResolutionError::Unavailable)
    );
    let other = ChatConnectionBinding::new(
        "project-b",
        "alice",
        ChatConnectionKind::Search,
        "search",
        1,
        "https://search.example/search",
        ChatAuthentication::Shared,
    )
    .unwrap()
    .unwrap()
    .with_catalog_generation(1)
    .unwrap();
    assert!(
        credentials::assigned_vault(&host, &other)
            .unwrap()
            .is_none()
    );
    let next = binding.with_catalog_generation(2).unwrap();
    assert!(credentials::assigned_vault(&host, &next).unwrap().is_none());
    vault
        .write(
            &CredentialKey::new("cloud-chat-credentials", "catalog-v1").unwrap(),
            &VaultRecord::new(b"invalid custody".to_vec()).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        credentials::assigned_vault(&host, &next),
        Err(CloudError::Storage)
    ));
}

#[test]
fn removed_global_dependencies_remain_visible_and_survive_unrelated_project_edits() {
    let seed = configuration();
    let mut global = catalog(&seed);
    let mut raw = seed.clone();
    let mut model = seed.runtime.models.profiles["main"].clone();
    model.provider_profile = "shared".into();
    raw.runtime.models.profiles.insert("local".into(), model);
    raw.runtime
        .models
        .roles
        .insert("subagent_default".into(), "shared".into());
    let before = inheritance::resolve(&raw, &global, &seed).unwrap();
    assert!(network::assess(&before).models["local"].enabled);
    global.providers.profiles.clear();
    global.models.profiles.clear();
    global.revision += 1;
    let full = inheritance::resolve(&raw, &global, &seed).unwrap();
    assert!(!network::assess(&full).models["local"].enabled);
    assert!(!network::assess(&full).roles["subagent_default"].enabled);
    let mut input = settings(&full, global.revision);
    input["variables"]["REGION"] = json!("updated");
    let updated = crate::chat_settings::reviewed_project_update(
        &full,
        serde_json::from_value(input).unwrap(),
        &global,
        &seed,
    )
    .unwrap();
    assert_eq!(
        updated.runtime.models.profiles["local"].provider_profile,
        "shared"
    );
    assert_eq!(updated.runtime.models.roles["subagent_default"], "shared");
    assert_eq!(updated.variables["REGION"], "updated");
    let persisted = inheritance::local_configuration(&updated);
    crate::chat_settings::validate_retained(&persisted, &seed).unwrap();
    let restarted = inheritance::resolve(&persisted, &global, &seed).unwrap();
    assert!(!network::assess(&restarted).models["local"].enabled);
    assert_eq!(
        network::execution(&restarted).unwrap().runtime.models.roles["primary"],
        "main"
    );
    let mut invalid = settings(&restarted, global.revision);
    invalid["models"]["profiles"]["new"] = json!({"providerProfile":"missing","model":"new","contextWindowTokens":32768,"maxOutputTokens":4096});
    assert!(matches!(
        crate::chat_settings::reviewed_project_update(
            &restarted,
            serde_json::from_value(invalid).unwrap(),
            &global,
            &seed
        ),
        Err(CloudError::InvalidArgument)
    ));
}
