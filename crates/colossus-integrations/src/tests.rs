use super::{
    EventSourcedExtensionRepository, IntegrationExecutor, IntegrationRequest, compile_native,
    compile_openapi, normalize_native_response, operation_url, prepare_native_request,
    redact_exact_secret,
};
use colossus_contracts::{
    CredentialReference, DecisionOutcome, IntegrationAuth, IntegrationStatus, ResourceAuthority,
    SandboxBoundaryMode,
};
use colossus_policy::{EffectExecutor, SandboxBoundaryGate, system_actor};
use colossus_ports::{EventJournal, ExtensionRepository};
use colossus_testkit::{InMemoryEventJournal, assert_extension_repository_conformance};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn event_sourced_extension_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::rejecting_global_reads());
    assert_extension_repository_conformance(|| {
        Box::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)))
    });
}

fn document() -> serde_json::Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Demo", "description": "Demo API"},
        "servers": [{"url": "https://api.example.test/v1/"}],
        "paths": {
            "/widgets/{id}": {
                "get": {
                    "operationId": "getWidget",
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "expand", "in": "query", "schema": {"type": "boolean"}}
                    ]
                },
                "patch": {
                    "operationId": "updateWidget",
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {
                            "type": "object",
                            "properties": {"name": {"type": "string", "maxLength": 100}},
                            "required": ["name"]
                        }}}
                    }
                }
            }
        }
    })
}

#[test]
fn openapi_compilation_maps_path_query_and_body_without_auth_arguments() {
    let connection = compile_openapi(
        "demo",
        &document(),
        None,
        IntegrationAuth::Bearer {
            header: "Authorization".into(),
            scheme: "Bearer".into(),
        },
        Some("env:DEMO_TOKEN".into()),
        vec!["widgets:read".into()],
        "2026-01-01T00:00:00Z".into(),
        "2026-01-01T00:00:00Z".into(),
    )
    .expect("compile");
    assert_eq!(connection.status, IntegrationStatus::Connected);
    assert_eq!(connection.operations.len(), 2);
    let read = connection
        .operations
        .iter()
        .find(|operation| operation.method == "GET")
        .expect("read");
    assert_eq!(read.tool.name, "openapi.demo.getwidget");
    assert_eq!(read.path_parameters, ["id"]);
    assert_eq!(read.query_parameters, ["expand"]);
    let schema = serde_json::to_string(&read.tool.input_schema).expect("schema");
    assert!(!schema.contains("credential"));
    assert!(!schema.contains("Authorization"));
    let update = connection
        .operations
        .iter()
        .find(|operation| operation.method == "PATCH")
        .expect("update");
    assert!(update.accepts_body);
    assert!(
        update.tool.input_schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("body")))
    );
}

#[test]
fn openapi_operation_preserves_a_server_path_without_a_trailing_slash() {
    let mut without_trailing_slash = document();
    without_trailing_slash["servers"][0]["url"] = json!("https://api.example.test/v1");
    let connection = compile_openapi(
        "demo",
        &without_trailing_slash,
        None,
        IntegrationAuth::None,
        None,
        Vec::new(),
        "2026-01-01T00:00:00Z".into(),
        "2026-01-01T00:00:00Z".into(),
    )
    .expect("compile");
    let operation = connection
        .operations
        .iter()
        .find(|operation| operation.method == "GET")
        .expect("GET operation");
    let url = operation_url(
        &connection,
        operation,
        &json!({"id": "sdk example", "expand": true}),
    )
    .expect("operation URL");
    assert_eq!(
        url.as_str(),
        "https://api.example.test/v1/widgets/sdk%20example"
    );
}

#[test]
fn extension_repository_reconstructs_reconnect_and_disconnect_history() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository = EventSourcedExtensionRepository::new(journal);
    let connection = compile_openapi(
        "demo",
        &document(),
        None,
        IntegrationAuth::None,
        None,
        Vec::new(),
        "2026-01-01T00:00:00Z".into(),
        "2026-01-01T00:00:00Z".into(),
    )
    .expect("compile");
    repository
        .save_integration(connection, system_actor("test"))
        .expect("save");
    assert_eq!(repository.list_integrations(10).expect("list").len(), 1);
    let disconnected = repository
        .disconnect_integration("demo", system_actor("test"), "2026-01-02T00:00:00Z")
        .expect("disconnect");
    assert_eq!(disconnected.status, IntegrationStatus::Disconnected);
    assert_eq!(
        repository
            .get_integration("demo")
            .expect("get")
            .expect("connection")
            .status,
        IntegrationStatus::Disconnected
    );
}

#[test]
fn importer_rejects_refs_embedded_origins_and_unsupported_schema_references() {
    let mut invalid = document();
    invalid["paths"]["/widgets/{id}"]["get"]["parameters"][0]["schema"] =
        json!({"$ref": "#/components/schemas/Id"});
    assert!(
        compile_openapi(
            "demo",
            &invalid,
            Some("https://user:secret@example.test"),
            IntegrationAuth::None,
            None,
            Vec::new(),
            "now".into(),
            "now".into(),
        )
        .is_err()
    );
    assert!(
        compile_openapi(
            "demo",
            &invalid,
            Some("https://example.test"),
            IntegrationAuth::None,
            None,
            Vec::new(),
            "now".into(),
            "now".into(),
        )
        .is_err()
    );
}

#[test]
fn exact_credential_values_are_removed_from_quarantined_responses() {
    assert_eq!(
        redact_exact_secret(
            br#"{"authorization":"Bearer secret-token"}"#,
            b"secret-token"
        ),
        br#"{"authorization":"Bearer [REDACTED]"}"#
    );
}

#[tokio::test]
async fn canonical_credential_reference_mismatch_fails_before_network_execution() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn ExtensionRepository> =
        Arc::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)));
    let connection = compile_openapi(
        "demo",
        &document(),
        Some("http://127.0.0.1:9/v1/"),
        IntegrationAuth::Bearer {
            header: "Authorization".into(),
            scheme: "Bearer".into(),
        },
        Some("env:PATH".into()),
        Vec::new(),
        "2026-01-01T00:00:00Z".into(),
        "2026-01-01T00:00:00Z".into(),
    )
    .expect("connection");
    repository
        .save_integration(connection, system_actor("test"))
        .expect("save");
    let executor = IntegrationExecutor::new(repository).expect("executor");
    let operation = IntegrationRequest::Invoke {
        connection: "demo".into(),
        tool_name: "openapi.demo.getwidget".into(),
        arguments: json!({"id":"1"}),
    };
    let mut request = colossus_policy::effect_request(
        system_actor("test"),
        operation.action(),
        operation.resource(),
        serde_json::to_value(&operation).expect("request"),
    );
    request.capabilities = vec!["integration.invoke".into()];
    let gateway = colossus_policy::EffectGateway::new(
        journal,
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("openapi.demo.getwidget", DecisionOutcome::Allow)
                .with_network_destination("http://127.0.0.1:9"),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["integration.invoke".into()]),
        [31_u8; 32],
    );
    let error = gateway
        .execute(request, &executor as &dyn EffectExecutor)
        .await
        .expect_err("mismatched disclosure must fail");
    assert!(error.to_string().contains("credential disclosure"));
}

#[tokio::test]
async fn remote_plaintext_integration_requires_ambient_authority_in_the_permit() {
    let credential = "env:COLOSSUS_TEST_MISSING_PLAINTEXT_INTEGRATION_KEY";
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn ExtensionRepository> =
        Arc::new(EventSourcedExtensionRepository::new(Arc::clone(&journal)));
    let connection = compile_openapi(
        "plaintext",
        &document(),
        Some("http://192.0.2.1:9/v1/"),
        IntegrationAuth::Bearer {
            header: "Authorization".into(),
            scheme: "Bearer".into(),
        },
        Some(credential.into()),
        Vec::new(),
        "2026-01-01T00:00:00Z".into(),
        "2026-01-01T00:00:00Z".into(),
    )
    .expect("potential ambient connection");
    repository
        .save_integration(connection, system_actor("test"))
        .expect("save");
    let executor = IntegrationExecutor::new(repository).expect("executor");
    let request = || {
        let operation = IntegrationRequest::Invoke {
            connection: "plaintext".into(),
            tool_name: "openapi.plaintext.getwidget".into(),
            arguments: json!({"id":"1"}),
        };
        let mut request = colossus_policy::effect_request(
            system_actor("test"),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation).expect("request"),
        );
        request.capabilities = vec!["integration.invoke".into()];
        request.credential_references = vec![CredentialReference {
            reference: credential.into(),
            value_hash: None,
        }];
        request
    };

    let declared = colossus_policy::EffectGateway::new(
        Arc::clone(&journal),
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("openapi.plaintext.getwidget", DecisionOutcome::Allow)
                .with_network_destination("http://192.0.2.1:9"),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["integration.invoke".into()]),
        [71_u8; 32],
    );
    let error = declared
        .execute(request(), &executor as &dyn EffectExecutor)
        .await
        .expect_err("declared exact origin must not authorize remote plaintext HTTP");
    assert!(
        error
            .to_string()
            .contains("requires ambient resource authority")
    );

    let ambient = colossus_policy::EffectGateway::new(
        journal,
        Arc::new(
            colossus_policy::BuiltInPolicy::offline_default()
                .with_action("openapi.plaintext.getwidget", DecisionOutcome::Allow)
                .with_sandbox("danger_full_access", "test", false)
                .with_resource_authority(ResourceAuthority::Ambient)
                .with_limits(25, 1024 * 1024, 1, 64 * 1024 * 1024, 1),
        ),
        Arc::new(colossus_policy::DenyApproval),
        colossus_policy::SafetyKernel::new(["integration.invoke".into()])
            .with_sandbox_boundary_gate(Arc::new(SandboxBoundaryGate::new(
                Some(SandboxBoundaryMode::DangerFullAccess),
                true,
            ))),
        [72_u8; 32],
    );
    let error = ambient
        .execute(request(), &executor as &dyn EffectExecutor)
        .await
        .expect_err("missing credential must stop before dispatch");
    assert!(error.to_string().contains("credential"));
    assert!(
        !error
            .to_string()
            .contains("requires ambient resource authority")
    );
}

#[test]
fn native_manifests_cover_github_searxng_and_opensearch_auth_contracts() {
    let github = compile_native(
        "github",
        None,
        IntegrationAuth::Bearer {
            header: "Authorization".into(),
            scheme: "Bearer".into(),
        },
        None,
        BTreeMap::new(),
        Vec::new(),
        "created".into(),
        "updated".into(),
    )
    .expect("GitHub");
    assert_eq!(github.status, IntegrationStatus::PendingAuth);
    assert_eq!(github.operations.len(), 5);
    assert_eq!(github.scopes, ["repo", "workflow"]);

    let searxng = compile_native(
        "searxng",
        Some("https://search.example.test/search"),
        IntegrationAuth::None,
        None,
        BTreeMap::new(),
        Vec::new(),
        "created".into(),
        "updated".into(),
    )
    .expect("SearXNG");
    let prepared = prepare_native_request(
        &searxng,
        "searxng.search",
        &json!({"query":"rust agents","max_results":2}),
    )
    .expect("request");
    assert_eq!(prepared.url.path(), "/search");
    assert_eq!(prepared.url.query(), Some("q=rust+agents&format=json"));
    let normalized = normalize_native_response(
        &searxng,
        "searxng.search",
        &json!({"query":"rust agents","max_results":1}),
        json!({"results":[
            {"title":"One","url":"https://one.test","content":"First","engine":"demo"},
            {"title":"Two","url":"https://two.test","content":"Second"}
        ]}),
    )
    .expect("normalize");
    assert_eq!(normalized["count"], 1);
    assert_eq!(normalized["results"][0]["metadata"]["engine"], "demo");

    let basic = BTreeMap::from([
        ("username".into(), "env:OPENSEARCH_USER".into()),
        ("password".into(), "env:OPENSEARCH_PASSWORD".into()),
    ]);
    let opensearch = compile_native(
        "opensearch",
        Some("https://search.example.test"),
        IntegrationAuth::Basic {
            header: "Authorization".into(),
        },
        None,
        basic,
        Vec::new(),
        "created".into(),
        "updated".into(),
    )
    .expect("OpenSearch");
    assert_eq!(opensearch.status, IntegrationStatus::Connected);
    assert_eq!(opensearch.operations.len(), 9);
    let prepared = prepare_native_request(
        &opensearch,
        "opensearch.update_document",
        &json!({
            "index":"notes-*","id":"a b","doc":{"status":"done"},
            "doc_as_upsert":true,"refresh":"wait_for"
        }),
    )
    .expect("update request");
    assert_eq!(prepared.method, reqwest::Method::POST);
    assert_eq!(prepared.url.path(), "/notes-*/_update/a%20b");
    assert_eq!(prepared.url.query(), Some("refresh=wait_for"));
    assert_eq!(prepared.body.expect("body")["doc_as_upsert"], true);
}
