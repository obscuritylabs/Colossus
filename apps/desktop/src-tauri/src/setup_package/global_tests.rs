use super::{archive, catalog, commands, configuration, globals, tests::saved};
use crate::{
    desktop_settings::DesktopSettings,
    managed_configuration::{CatalogEntrySetting, CatalogRevisionSetting},
};
use serde_json::{Value, json};

fn payload() -> Value {
    json!({
        "defaults": {"executionBoundary":"workspace_isolated", "terminalEnabled":false,
            "fieldOverrides":[{"fieldId":"agent.maxTurns","value":25},{"fieldId":"sandbox.timeoutMs","value":60000}]},
        "mcpServers":[{"id":"docs", "label":"Company documentation", "configuration": {
            "name":"docs", "transport":"streamable_http", "url":"https://docs.example.com/mcp", "headers":{},
            "credentialHeaders":{"Authorization":{"scheme":"Bearer","credentialId":"env:DOCS_TOKEN"}},
            "allowedTools":["search"], "timeoutMs":30000, "maxOutputBytes":65536
        }}],
        "searchProviders":[{"id":"web", "label":"Web search", "configuration": {
            "profile":"web", "kind":"serp_api", "endpoint":"https://serpapi.com/search",
            "credentialId":"env:SEARCH_TOKEN", "timeoutMs":30000
        }}],
        "telemetryProfiles":[{"id":"operations", "label":"Operations telemetry", "configuration": {
            "name":"colossus", "endpoint":"https://telemetry.example.com", "protocol":"grpc", "timeoutMs":10000,
            "tracesEnabled":true, "traceSampleRatioMillionths":100_000, "metricsEnabled":true, "metricExportIntervalMs":60000,
            "logsOtlp":false,"logsStdoutJson":false,"journalPayloads":"disabled",
            "acknowledgeSensitiveContent":false,"acknowledgeInsecureTransport":false,"resourceAttributes":{}
        }}]
    })
}

fn package() -> super::types::SavedSetupPackage {
    let mut package = saved();
    package.manifest.schema_version = 2;
    let mut config: Value = serde_saphyr::from_str(&package.config_yaml).unwrap();
    config["desktop"] = payload();
    package.config_yaml = serde_saphyr::to_string(&config).unwrap();
    package
}

#[test]
fn global_package_roundtrip_preserves_catalogs_and_defaults_without_secrets_or_activation() {
    let mut package = package();
    let mut settings = DesktopSettings::default();
    catalog::import_catalog(&mut settings, &mut package, None).unwrap();
    assert!(settings.spaces.is_empty());
    assert!(
        settings
            .global_configuration
            .defaults
            .current()
            .unwrap()
            .field_overrides
            .is_empty()
    );
    assert_eq!(settings.global_configuration.mcp_servers.len(), 1);
    assert_eq!(settings.global_configuration.search_providers.len(), 1);
    assert_eq!(settings.global_configuration.telemetry_profiles.len(), 1);
    assert_eq!(settings.global_configuration.credentials.len(), 2);
    catalog::apply_defaults(&mut settings, &package).unwrap();
    let exported = commands::export_current(&settings).unwrap();
    let bytes = archive::write(&exported).unwrap();
    let source = archive::read(&bytes).unwrap();
    let globals = globals::from_yaml(&source.config_yaml).unwrap();
    assert_eq!(globals.defaults.as_ref().unwrap().field_overrides.len(), 2);
    assert_eq!(
        globals.mcp_servers[0].configuration.allowed_tools,
        ["search"]
    );
    assert_eq!(
        globals.search_providers[0].configuration.endpoint,
        "https://serpapi.com/search"
    );
    assert!(globals.telemetry_profiles[0].configuration.metrics_enabled);
    for credential in &settings.global_configuration.credentials {
        assert!(!source.config_yaml.contains(&credential.id));
    }
    let inspected: Value =
        serde_json::from_str(&configuration::inspection_yaml(&source).unwrap()).unwrap();
    assert_eq!(inspected["agent"]["maxTurns"], 25);
    assert!(inspected.get("desktop").is_none());
    let imported = configuration::inspected(source, &inspected, &bytes).unwrap();
    let mut recipient = DesktopSettings::default();
    let mut imported = imported;
    catalog::import_catalog(&mut recipient, &mut imported, None).unwrap();
    assert_ne!(
        recipient.global_configuration.credentials[0].id,
        settings.global_configuration.credentials[0].id
    );
    configuration::validate_saved(&[imported]).unwrap();
}

#[test]
fn globals_only_export_needs_no_workspace_or_provider() {
    let package = commands::export_current(&DesktopSettings::default()).unwrap();
    assert!(package.providers.is_empty());
    let source = archive::read(&archive::write(&package).unwrap()).unwrap();
    assert!(configuration::inspection_yaml(&source).is_ok());
}

#[test]
fn imported_defaults_keep_existing_spaces_pinned_and_apply_to_new_spaces() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    // macOS temporary directories may be reached through the /var symlink.
    let first_path = first.path().canonicalize().unwrap();
    let second_path = second.path().canonicalize().unwrap();
    let mut settings = DesktopSettings::default();
    settings
        .add_space(crate::desktop_settings::validate_workspace(&first_path).unwrap())
        .unwrap();
    let mut package = package();
    catalog::import_catalog(&mut settings, &mut package, None).unwrap();
    let accepted = settings.spaces[0].configuration.accepted_global_revision;
    catalog::apply_defaults(&mut settings, &package).unwrap();
    assert_eq!(
        settings.spaces[0].configuration.accepted_global_revision,
        accepted
    );
    let old = crate::managed_configuration::resolve_space_configuration(
        &settings.global_configuration,
        &settings.spaces[0],
    )
    .unwrap();
    assert!(old.field_overrides.is_empty());
    settings
        .add_space(crate::desktop_settings::validate_workspace(&second_path).unwrap())
        .unwrap();
    assert_eq!(
        settings.execution_boundary,
        crate::desktop_settings::ExecutionBoundarySetting::WorkspaceIsolated,
        "the onboarding status must show the inherited boundary"
    );
    assert!(!settings.terminal_enabled);
    let new = crate::managed_configuration::resolve_space_configuration(
        &settings.global_configuration,
        &settings.spaces[1],
    )
    .unwrap();
    assert_eq!(
        new.execution_boundary,
        crate::desktop_settings::ExecutionBoundarySetting::WorkspaceIsolated
    );
    assert!(
        new.field_overrides
            .iter()
            .any(|field| field.field_id == "agent.maxTurns" && field.value == 25)
    );
}

#[test]
fn identical_reimport_reuses_missing_slots_but_new_endpoints_do_not_inherit_secrets() {
    let mut original = package();
    let mut settings = DesktopSettings::default();
    catalog::import_catalog(&mut settings, &mut original, None).unwrap();
    let mut next = original.clone();
    let revision = settings.global_configuration.revision;
    catalog::import_catalog(&mut settings, &mut next, Some(&original)).unwrap();
    assert_eq!(settings.global_configuration.revision, revision);
    assert_eq!(next.credential_bindings, original.credential_bindings);
    next.config_yaml = next.config_yaml.replace(
        "https://docs.example.com/mcp",
        "https://other.example.com/mcp",
    );
    catalog::import_catalog(&mut settings, &mut next, Some(&original)).unwrap();
    assert_ne!(
        next.credential_bindings["env:DOCS_TOKEN"],
        original.credential_bindings["env:DOCS_TOKEN"]
    );
}

#[test]
fn global_import_does_not_replace_manually_edited_entries() {
    let mut original = package();
    let mut settings = DesktopSettings::default();
    catalog::import_catalog(&mut settings, &mut original, None).unwrap();
    settings.global_configuration.mcp_servers[0].revisions[0]
        .value
        .allowed_tools = vec!["lookup".into()];
    let mut next = original.clone();
    catalog::import_catalog(&mut settings, &mut next, Some(&original)).unwrap();
    assert_eq!(settings.global_configuration.mcp_servers.len(), 2);
    assert_eq!(
        settings.global_configuration.mcp_servers[0].revisions[0]
            .value
            .allowed_tools,
        ["lookup"]
    );
}

#[test]
fn export_replaces_literal_headers_and_host_ids_with_slots() {
    let mut globals: globals::SetupGlobals = serde_json::from_value(payload()).unwrap();
    let server = &mut globals.mcp_servers[0].configuration;
    server
        .headers
        .insert("X-Api-Key".into(), "NEVER_EXPORT_THIS_SECRET".into());
    server
        .credential_headers
        .get_mut("Authorization")
        .unwrap()
        .credential_id = "LOCAL_HOST_ID".into();
    let mut settings = DesktopSettings::default();
    settings
        .global_configuration
        .mcp_servers
        .push(CatalogEntrySetting {
            id: uuid::Uuid::now_v7().to_string(),
            label: "Docs".into(),
            archived: false,
            current_revision: 1,
            revisions: vec![CatalogRevisionSetting {
                revision: 1,
                value: server.clone(),
            }],
        });
    let exported = commands::export_current(&settings).unwrap();
    assert!(!exported.config_yaml.contains("NEVER_EXPORT_THIS_SECRET"));
    assert!(!exported.config_yaml.contains("LOCAL_HOST_ID"));
    let globals = globals::from_yaml(&exported.config_yaml).unwrap();
    assert!(globals.mcp_servers[0].configuration.headers.is_empty());
    assert_eq!(globals.credential_slots().len(), 2);
}

#[test]
fn invalid_global_resources_and_credential_handles_are_rejected_before_import() {
    for (pointer, value) in [
        (
            "/mcpServers/0/configuration/credentialHeaders/Authorization/credentialId",
            json!("host:other-machine"),
        ),
        (
            "/mcpServers/0/configuration/headers",
            json!({"Authorization":"secret"}),
        ),
        ("/mcpServers/0/configuration/transport", json!("shell")),
        (
            "/searchProviders/0/configuration/endpoint",
            json!("https://user:password@example.com"),
        ),
        (
            "/telemetryProfiles/0/configuration/traceSampleRatioMillionths",
            json!(2_000_000),
        ),
        ("/defaults/fieldOverrides/0/fieldId", json!("storage.path")),
    ] {
        let mut payload = payload();
        *payload.pointer_mut(pointer).unwrap() = value;
        let parsed = serde_json::from_value::<globals::SetupGlobals>(payload);
        assert!(
            parsed.is_err() || parsed.unwrap().validate().is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn advanced_defaults_reject_unbound_credentials_on_import_and_export() {
    for reference in [
        "host:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "env:TOKEN",
        "literal-secret",
    ] {
        for (field_id, value) in [
            (
                "audit.exporter",
                json!({"kind":"worm_http", "credentialReference":reference}),
            ),
            (
                "memory.semantic",
                json!({"kind":"chroma", "credentialReference":reference}),
            ),
            (
                "memory.semantic",
                json!({"kind":"chroma", "embedding":{"credentialReference":reference}}),
            ),
            (
                "plugins.registries",
                json!({"company":{"auth":{"kind":"bearer", "credentialReference":reference}}}),
            ),
            (
                "plugins.mcpServers",
                json!({"company/docs":{"environment":{"TOKEN":reference}}}),
            ),
            (
                "plugins.mcpServers",
                json!({"company/docs":{"credentialHeaders":{"Authorization":{"reference":reference}}}}),
            ),
            (
                "plugins.mcpServers",
                json!({"company/docs":{"oauth":{"clientSecretReference":reference}}}),
            ),
        ] {
            let mut package = package();
            let mut yaml: Value = serde_saphyr::from_str(&package.config_yaml).unwrap();
            yaml["desktop"]["defaults"]["fieldOverrides"] =
                json!([{"fieldId":field_id,"value":value}]);
            package.config_yaml = serde_saphyr::to_string(&yaml).unwrap();
            let source = archive::read(&archive::write(&package).unwrap()).unwrap();
            assert!(
                configuration::inspection_yaml(&source).is_err(),
                "{field_id}"
            );
            let mut settings = DesktopSettings::default();
            settings.global_configuration.defaults.revisions[0].value =
                globals::from_yaml(&package.config_yaml)
                    .unwrap()
                    .defaults
                    .unwrap();
            assert!(commands::export_current(&settings).is_err(), "{field_id}");
        }
    }
    let mut globals: globals::SetupGlobals = serde_json::from_value(payload()).unwrap();
    globals.defaults.as_mut().unwrap().field_overrides = serde_json::from_value(json!([
        {"fieldId":"plugins.registries","value":{"company":{"auth":{"kind":"docker"}}}}
    ]))
    .unwrap();
    assert!(globals.validate().is_err());
}

#[test]
fn credential_free_advanced_defaults_remain_portable_without_rewriting_user_strings() {
    let mut globals: globals::SetupGlobals = serde_json::from_value(payload()).unwrap();
    globals.defaults.as_mut().unwrap().field_overrides = serde_json::from_value(json!([
        {"fieldId":"audit.exporter","value":{"kind":"disabled"}},
        {"fieldId":"memory.semantic","value":{"kind":"disabled"}},
        {"fieldId":"plugins.registries","value":{"company":{"auth":{"kind":"anonymous"}}}},
        {"fieldId":"plugins.mcpServers","value":{"company/docs":{
            "enabled":true,"environment":{},"credentialHeaders":{},
            "oauth":{"clientId":"public-client","clientSecretReference":null,"callbackPort":8123},
            "researchTools":[{"tool":"search","arguments":{"text":"host:literal-user-query"}}]
        }}}
    ]))
    .unwrap();
    globals.validate().unwrap();
    assert!(
        serde_json::to_string(&globals)
            .unwrap()
            .contains("host:literal-user-query")
    );
}

#[test]
fn setup_export_import_preserves_tool_patterns_and_access_exclusions() {
    let mut package = package();
    let mut config: Value = serde_saphyr::from_str(&package.config_yaml).unwrap();
    config["desktop"]["mcpServers"][0]["configuration"]["allowedTools"] =
        json!(["get_*", "*_search", "echo"]);
    config["desktop"]["defaults"]["fieldOverrides"] = json!([
        {"fieldId":"access.tools.include", "value":["filesystem.*"]},
        {"fieldId":"access.tools.exclude", "value":["*.write"]}
    ]);
    package.config_yaml = serde_saphyr::to_string(&config).unwrap();
    let mut settings = DesktopSettings::default();
    catalog::import_catalog(&mut settings, &mut package, None).unwrap();
    catalog::apply_defaults(&mut settings, &package).unwrap();
    let exported = commands::export_current(&settings).unwrap();
    let bytes = archive::write(&exported).unwrap();
    let imported = archive::read(&bytes).unwrap();
    let globals = globals::from_yaml(&imported.config_yaml).unwrap();
    let inspected: Value =
        serde_json::from_str(&configuration::inspection_yaml(&imported).unwrap()).unwrap();
    let saved = configuration::inspected(imported, &inspected, &bytes).unwrap();
    configuration::validate_saved(&[saved]).unwrap();
    assert_eq!(
        globals.mcp_servers[0].configuration.allowed_tools,
        ["get_*", "*_search", "echo"]
    );
    let fields = globals.defaults.unwrap().field_overrides;
    assert!(
        fields
            .iter()
            .any(|field| field.field_id == "access.tools.include"
                && field.value == json!(["filesystem.*"]))
    );
    assert!(
        fields
            .iter()
            .any(|field| field.field_id == "access.tools.exclude"
                && field.value == json!(["*.write"]))
    );
}
