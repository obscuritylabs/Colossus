use super::*;
use colossus_contracts::{ActorType, DecisionOutcome, SandboxBoundaryMode};
use colossus_policy::{
    AllowApproval, BuiltInPolicy, EffectGateway, SafetyKernel, SandboxBoundaryGate,
};
use colossus_ports::{KeyProvider, StoreError};
use colossus_testkit::InMemoryEventJournal;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Mutex;

fn test_schema_sha256(schema: &Value) -> String {
    let bytes = serde_json::to_vec(schema).expect("schema bytes");
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn request_uses_official_protocol_models_and_no_secret_values() {
    let operation = McpOperation::CallTool {
        server: "local".into(),
        tool: "echo".into(),
        description: Some("Echo one message".into()),
        annotations: None,
        arguments: json!({"text": "hello"}),
        input_schema: Box::new(json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": false
        })),
        schema_sha256: "unused-by-protocol-projection".into(),
    };
    let bytes = protocol_input(&operation).expect("protocol");
    let lines = std::str::from_utf8(&bytes)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .collect::<Vec<_>>();
    assert_eq!(lines[0]["method"], "initialize");
    assert_eq!(lines[0]["params"]["protocolVersion"], "2025-11-25");
    assert_eq!(lines[1]["method"], "notifications/initialized");
    assert_eq!(lines[2]["method"], "tools/call");
    assert_eq!(lines[2]["params"]["name"], "echo");
}

#[test]
fn remote_call_timeout_certainty_follows_dispatch_stage() {
    let call = McpOperation::CallTool {
        server: "fixture".into(),
        tool: "echo".into(),
        description: None,
        annotations: None,
        arguments: json!({}),
        input_schema: Box::new(json!({"type": "object"})),
        schema_sha256: "unused-by-timeout-classification".into(),
    };
    assert!(matches!(
        remote_timeout_error(&call, false),
        ExecutionError::Failed(_)
    ));
    assert!(matches!(
        remote_timeout_error(&call, true),
        ExecutionError::OutcomeUnknown(_)
    ));
    assert!(matches!(
        remote_timeout_error(
            &McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
            false,
        ),
        ExecutionError::Failed(_)
    ));
}

#[test]
fn discovered_schema_is_enforced_before_call() {
    let input_schema = json!({
        "type": "object",
        "properties": {"count": {"type": "integer"}},
        "required": ["count"],
        "additionalProperties": false
    });
    let tool = McpToolSummary {
        server: "local".into(),
        name: "echo".into(),
        title: None,
        description: None,
        annotations: None,
        schema_sha256: test_schema_sha256(&input_schema),
        input_schema,
    };
    assert!(validate_tool_arguments(&tool, &json!({"count": 2})).is_ok());
    assert!(validate_tool_arguments(&tool, &json!({"count": "two"})).is_err());
}

#[test]
fn secret_values_are_redacted_from_nested_results() {
    let mut value = json!({"text": "token=secret-value", "nested": ["secret-value"]});
    redact_value(&mut value, &["secret-value".into()]);
    assert_eq!(value["text"], "token=<redacted>");
    assert_eq!(value["nested"][0], "<redacted>");
}

#[test]
fn discovery_pages_containing_configured_credentials_fail_before_release() {
    let page = McpToolsPage {
        server: "splunk".into(),
        tools: vec![McpToolSummary {
            server: "splunk".into(),
            name: "search".into(),
            title: None,
            description: Some("accidentally echoed hard-secret".into()),
            annotations: None,
            input_schema: json!({"type": "object"}),
            schema_sha256: "hash".into(),
        }],
        next_cursor: None,
    };
    assert!(tools_page_contains_secret(&page, &["hard-secret".into()]));
}

#[test]
fn streamable_http_content_types_require_an_exact_media_type() {
    assert!(content_type_matches("application/json", "application/json"));
    assert!(content_type_matches(
        "Application/JSON; charset=utf-8",
        "application/json"
    ));
    assert!(content_type_matches(
        "text/event-stream; charset=utf-8",
        "text/event-stream"
    ));
    assert!(!content_type_matches(
        "application/json-seq",
        "application/json"
    ));
    assert!(!content_type_matches(
        "text/event-streaming",
        "text/event-stream"
    ));
}

#[test]
fn tool_wildcard_is_exclusive_and_empty_or_duplicate_lists_fail_closed() {
    let wildcard = ToolAllowlist::from_config("remote", &["*".into()]).expect("wildcard");
    assert!(matches!(&wildcard, ToolAllowlist::All));
    assert_eq!(wildcard.summary(), vec!["*"]);
    assert!(ToolAllowlist::from_config("remote", &["*".into(), "search".into()]).is_err());
    assert!(ToolAllowlist::from_config("remote", &[]).is_err());
    assert!(ToolAllowlist::from_config("remote", &["search".into(), "search".into()]).is_err());
}

fn remote_server(endpoint: &str) -> McpServerConfig {
    McpServerConfig {
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        working_directory: None,
        environment: BTreeMap::new(),
        url: Some(endpoint.into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: vec!["*".into()],
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    }
}

fn validation_context(
    resource_authority: ResourceAuthority,
    sandbox_environment: &[String],
) -> McpValidationContext<'_> {
    McpValidationContext {
        resource_authority,
        sandbox_executables: &[],
        sandbox_filesystem: &[],
        sandbox_environment,
        sandbox_timeout_ms: 30_000,
        sandbox_max_output_bytes: 1024 * 1024,
    }
}

#[test]
fn streamable_http_config_accepts_env_credentials_and_rejects_unsafe_http_identity() {
    let mut server = remote_server("https://splunk.example.com/services/mcp");
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    let mut config = McpConfig {
        oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
        servers: BTreeMap::from([("splunk".into(), server.clone())]),
    };
    let sandbox_environment = ["SPLUNK_MCP_TOKEN".into()];
    let validate = |config: &McpConfig| {
        validate_config(
            config,
            Path::new("."),
            validation_context(ResourceAuthority::Declared, &sandbox_environment),
        )
    };
    validate(&config).expect("valid remote config");
    config
        .servers
        .get_mut("splunk")
        .expect("server")
        .credential_headers
        .get_mut("Authorization")
        .expect("credential header")
        .reference = "host:mcp-splunk-token".into();
    validate(&config).expect("injected host credential does not require an environment grant");
    config
        .servers
        .get_mut("splunk")
        .expect("server")
        .credential_headers
        .get_mut("Authorization")
        .expect("credential header")
        .reference = "env:SPLUNK_MCP_TOKEN".into();
    config
        .servers
        .get_mut("splunk")
        .expect("server")
        .allow_stateless = true;
    validate(&config).expect("explicit stateless remote server");
    config.servers.get_mut("splunk").expect("server").url = Some("http://[::1]:8787/mcp".into());
    validate(&config).expect("IPv6 loopback development endpoint");

    config.servers.get_mut("splunk").expect("server").url =
        Some("http://splunk.example.com/services/mcp".into());
    assert!(validate(&config).is_err());
    config.servers.get_mut("splunk").expect("server").url =
        Some("https://user:secret@splunk.example.com/services/mcp".into());
    assert!(validate(&config).is_err());
    config.servers.get_mut("splunk").expect("server").url =
        Some("https://splunk.example.com/services/mcp?token=secret".into());
    assert!(validate(&config).is_err());

    let server = config.servers.get_mut("splunk").expect("server");
    server.url = Some("https://splunk.example.com/services/mcp".into());
    server.credential_headers.clear();
    server
        .headers
        .insert("Authorization".into(), "secret".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.headers.clear();
    server.headers.insert("X-API-Key".into(), "secret".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.headers.clear();
    server.credential_headers.insert(
        "Accept".into(),
        McpCredentialHeaderConfig {
            scheme: None,
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    assert!(validate(&config).is_err());

    let server = config.servers.get_mut("splunk").expect("server");
    server.credential_headers.clear();
    server.oauth = Some(McpOAuthConfig {
        client_id: "colossus".into(),
        client_secret_reference: Some("env:SPLUNK_MCP_TOKEN".into()),
        callback_port: 8787,
        scopes: vec!["openid".into(), "offline_access".into()],
    });
    validate(&config).expect("valid OAuth alternative");
    config
        .servers
        .get_mut("splunk")
        .expect("server")
        .oauth
        .as_mut()
        .expect("OAuth")
        .scopes
        .push("openid".into());
    assert!(validate(&config).is_err());
    let server = config.servers.get_mut("splunk").expect("server");
    server.oauth.as_mut().expect("OAuth").scopes.pop();
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    assert!(validate(&config).is_err());
}

#[test]
fn ambient_streamable_http_accepts_private_http_without_relaxing_url_validation() {
    let mut server = remote_server("http://10.20.30.40:8787/mcp");
    let mut config = McpConfig {
        oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
        servers: BTreeMap::from([("private".into(), server.clone())]),
    };
    let validate = |config: &McpConfig, ambient_resources| {
        validate_config(
            config,
            Path::new("."),
            validation_context(
                if ambient_resources {
                    ResourceAuthority::Ambient
                } else {
                    ResourceAuthority::Declared
                },
                &[],
            ),
        )
    };

    assert!(validate(&config, false).is_err());
    validate(&config, true).expect("acknowledged ambient authority permits private HTTP");

    server.url = Some("http://10.20.30.40:8787/mcp?token=secret".into());
    config.servers.insert("private".into(), server);
    assert!(validate(&config, true).is_err());
}

#[test]
fn ambient_validation_keeps_exact_mcp_declarations_but_omits_duplicate_sandbox_grants() {
    let workspace = tempfile::tempdir().expect("workspace");
    let command = std::env::current_exe().expect("test executable");
    let mut stdio = McpServerConfig {
        transport: McpTransportKind::Stdio,
        command,
        args: Vec::new(),
        working_directory: Some(workspace.path().to_owned()),
        environment: BTreeMap::from([("TOKEN".into(), "env:HOST_TOKEN".into())]),
        url: None,
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: vec!["*".into()],
        research_tools: Vec::new(),
        timeout_ms: None,
        max_output_bytes: None,
        effect_action_prefix: None,
        provenance: None,
    };
    let mut config = McpConfig {
        oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
        servers: BTreeMap::from([("ambient".into(), stdio.clone())]),
    };
    assert!(
        validate_config(
            &config,
            workspace.path(),
            validation_context(ResourceAuthority::Declared, &[]),
        )
        .is_err()
    );
    validate_config(
        &config,
        workspace.path(),
        validation_context(ResourceAuthority::Ambient, &[]),
    )
    .expect("ambient resource authority");

    stdio.command = PathBuf::from("relative-command");
    config.servers.insert("ambient".into(), stdio);
    assert!(
        validate_config(
            &config,
            workspace.path(),
            validation_context(ResourceAuthority::Ambient, &[]),
        )
        .is_err(),
        "ambient authority must not weaken exact server declarations"
    );
}

struct McpEffectShapeExecutor;

#[async_trait]
impl EffectExecutor for McpEffectShapeExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let input: McpEffectInput = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let authorization = input
            .credential_headers
            .get("Authorization")
            .ok_or_else(|| ExecutionError::Failed("authorization reference was removed".into()))?;
        assert_eq!(authorization.scheme.as_deref(), Some("Bearer"));
        assert_eq!(authorization.reference, "env:SPLUNK_MCP_TOKEN");
        assert!(input.allow_stateless);
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: br#"{"ok":true}"#.to_vec(),
            effect_succeeded: true,
        })
    }
}

#[tokio::test]
async fn gateway_preserves_remote_credential_reference_shape_and_session_mode() {
    let endpoint = "http://127.0.0.1:8787/mcp";
    let mut server = remote_server(endpoint);
    server.allow_stateless = true;
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:SPLUNK_MCP_TOKEN".into(),
        },
    );
    let executor = McpExecutor::new(
        &McpConfig {
            oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
            servers: BTreeMap::from([("splunk".into(), server)]),
        },
        Path::new("."),
        "native",
        Arc::new(McpEffectShapeExecutor),
    )
    .expect("MCP executor");
    let policy = BuiltInPolicy::offline_default()
        .with_action("mcp.tools", DecisionOutcome::Allow)
        .with_post_effect(false)
        .with_sandbox("native", "mcp-regression", false)
        .with_action_restrictions(
            "mcp.tools",
            Vec::new(),
            vec!["SPLUNK_MCP_TOKEN".into()],
            vec!["http://127.0.0.1:8787".into()],
        );
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "test".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]),
        [7_u8; 32],
    );
    let request = executor
        .request(
            Actor {
                actor_type: ActorType::System,
                id: "mcp-credential-regression".into(),
            },
            ExecutionContext::default(),
            McpOperation::ListTools {
                server: "splunk".into(),
                cursor: None,
            },
        )
        .expect("effect request");
    assert_eq!(request.content["transport"], "streamable_http");
    gateway
        .execute(request, &McpEffectShapeExecutor)
        .await
        .expect("gateway execution");
}

#[tokio::test]
async fn remote_plaintext_mcp_requires_ambient_authority_in_the_permit() {
    let endpoint = "http://192.0.2.1:9/mcp";
    let mut server = remote_server(endpoint);
    server.credential_headers.insert(
        "Authorization".into(),
        McpCredentialHeaderConfig {
            scheme: Some("Bearer".into()),
            reference: "env:COLOSSUS_TEST_MISSING_PLAINTEXT_MCP_KEY".into(),
        },
    );
    let config = McpConfig {
        oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
        servers: BTreeMap::from([("remote".into(), server)]),
    };
    validate_config(
        &config,
        Path::new("."),
        validation_context(ResourceAuthority::Ambient, &[]),
    )
    .expect("potential ambient config");
    let executor = McpExecutor::new(
        &config,
        Path::new("."),
        "danger_full_access",
        Arc::new(McpEffectShapeExecutor),
    )
    .expect("MCP executor");
    let request = || {
        executor
            .request(
                Actor {
                    actor_type: ActorType::System,
                    id: "mcp-plaintext-transport".into(),
                },
                ExecutionContext::default(),
                McpOperation::ListTools {
                    server: "remote".into(),
                    cursor: None,
                },
            )
            .expect("request")
    };

    let declared = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("mcp.tools", DecisionOutcome::Allow)
                .with_network_destination("http://192.0.2.1:9"),
        ),
        Arc::new(AllowApproval {
            approved_by: "test".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]),
        [67_u8; 32],
    );
    let error = declared
        .execute(request(), &executor)
        .await
        .expect_err("declared exact origin must not authorize remote plaintext HTTP");
    assert!(
        error
            .to_string()
            .contains("requires ambient resource authority")
    );

    let ambient = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(
            BuiltInPolicy::offline_default()
                .with_action("mcp.tools", DecisionOutcome::Allow)
                .with_sandbox("danger_full_access", "test", false)
                .with_resource_authority(ResourceAuthority::Ambient)
                .with_limits(25, 1024 * 1024, 1, 64 * 1024 * 1024, 1),
        ),
        Arc::new(AllowApproval {
            approved_by: "test".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]).with_sandbox_boundary_gate(Arc::new(
            SandboxBoundaryGate::new(Some(SandboxBoundaryMode::DangerFullAccess), true),
        )),
        [68_u8; 32],
    );
    let error = ambient
        .execute(request(), &executor)
        .await
        .expect_err("missing credential must stop before dispatch");
    assert!(error.to_string().contains("credential is unavailable"));
    assert!(
        !error
            .to_string()
            .contains("requires ambient resource authority")
    );
}

#[test]
fn wildcard_releases_new_valid_tools_but_rejects_invalid_discovery_names() {
    let server = ConfiguredServer {
        name: "remote".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some("https://splunk.example.com/services/mcp".into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let first: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "splunk_run_search",
            "description": "Run a search",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    let later: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "splunk_new_tool",
            "description": "Published later",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert_eq!(
        parse_tools_result(first, &server).expect("first").tools[0].name,
        "splunk_run_search"
    );
    assert_eq!(
        parse_tools_result(later, &server).expect("later").tools[0].name,
        "splunk_new_tool"
    );

    let invalid: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "invalid tool",
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert!(parse_tools_result(invalid, &server).is_err());

    let oversized_description: ListToolsResult = serde_json::from_value(json!({
        "tools": [{
            "name": "valid_tool",
            "description": "x".repeat(32 * 1024 + 1),
            "inputSchema": {"type": "object"}
        }]
    }))
    .expect("tools");
    assert!(parse_tools_result(oversized_description, &server).is_err());
}

#[test]
fn wildcard_and_explicit_discovery_preserve_bounded_risk_review_metadata() {
    let base = ConfiguredServer {
        name: "everything".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some("http://127.0.0.1:3001/mcp".into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    for allowlist in [
        ToolAllowlist::All,
        ToolAllowlist::Explicit(BTreeSet::from(["echo".into()])),
    ] {
        let mut server = base.clone();
        server.allowed_tools = allowlist;
        let result: ListToolsResult = serde_json::from_value(json!({
            "tools": [{
                "name": "echo",
                "description": "Echo one bounded message",
                "inputSchema": {
                    "type": "object",
                    "properties": {"message": {"type": "string"}},
                    "required": ["message"],
                    "additionalProperties": false
                },
                "annotations": {
                    "title": "Echo",
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false
                }
            }]
        }))
        .expect("tools");
        let page = parse_tools_result(result, &server).expect("discovery page");
        let tool = page.tools.first().expect("echo tool");
        assert_eq!(
            tool.description.as_deref(),
            Some("Echo one bounded message")
        );
        assert_eq!(
            tool.annotations,
            Some(McpToolAnnotations {
                title: Some("Echo".into()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            })
        );
        assert_eq!(tool.schema_sha256, test_schema_sha256(&tool.input_schema));
        validate_tool_arguments(tool, &json!({"message": "MCP tool test"}))
            .expect("bound arguments");
        let mut mismatched = tool.clone();
        mismatched.schema_sha256 = "0".repeat(64);
        assert!(
            validate_tool_arguments(&mismatched, &json!({"message": "MCP tool test"})).is_err()
        );
    }
}

struct RotatingTestKeys {
    active: Mutex<String>,
    keys: BTreeMap<String, [u8; 32]>,
}

impl KeyProvider for RotatingTestKeys {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        let id = self.active.lock().expect("active key").clone();
        Ok((id.clone(), self.keys[&id]))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        self.keys
            .get(key_id)
            .copied()
            .ok_or_else(|| StoreError::KeyUnavailable(key_id.into()))
    }

    fn store_anchor(&self, _anchor: &colossus_contracts::SecureAnchor) -> Result<(), StoreError> {
        Ok(())
    }

    fn load_anchor(&self) -> Result<Option<colossus_contracts::SecureAnchor>, StoreError> {
        Ok(None)
    }
}

#[tokio::test]
async fn encrypted_oauth_store_is_ciphertext_at_rest_and_reencrypts_after_rotation() {
    use redb::ReadableDatabase as _;
    use rmcp::transport::auth::{CredentialStore as _, StoredCredentials};

    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("oauth.redb");
    let keys = Arc::new(RotatingTestKeys {
        active: Mutex::new("key-1".into()),
        keys: BTreeMap::from([("key-1".into(), [7_u8; 32]), ("key-2".into(), [9_u8; 32])]),
    });
    let factory = OAuthStoreFactory::encrypted_state(
        &path,
        keys.clone() as Arc<dyn KeyProvider>,
        "repository-1".into(),
    )
    .expect("store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    let credentials: StoredCredentials = serde_json::from_value(json!({
        "client_id": "colossus",
        "token_response": null,
        "granted_scopes": ["openid"],
        "token_received_at": 1
    }))
    .expect("credentials");
    store.save(credentials.clone()).await.expect("save");
    assert!(
        factory
            .store("other", "https://splunk.example.com/services/mcp")
            .load()
            .await
            .expect("identity load")
            .is_none()
    );
    assert!(
        factory
            .store("splunk", "https://splunk.example.com/other")
            .load()
            .await
            .expect("endpoint load")
            .is_none()
    );
    drop(store);
    drop(factory);
    let at_rest = fs::read(&path).expect("database bytes");
    assert!(
        !at_rest
            .windows(b"colossus".len())
            .any(|value| value == b"colossus")
    );

    *keys.active.lock().expect("active key") = "key-2".into();
    let factory = OAuthStoreFactory::encrypted_state(
        &path,
        keys.clone() as Arc<dyn KeyProvider>,
        "repository-1".into(),
    )
    .expect("store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    let loaded = store.load().await.expect("load").expect("credentials");
    assert_eq!(
        serde_json::to_value(loaded).expect("loaded JSON"),
        serde_json::to_value(credentials).expect("expected JSON")
    );
    let record = {
        let OAuthCredentialStore::EncryptedState {
            database, identity, ..
        } = &store
        else {
            panic!("encrypted store");
        };
        let read = database.begin_read().expect("read");
        let table = read.open_table(OAUTH_RECORDS).expect("table");
        let value = table
            .get(identity.as_str())
            .expect("record read")
            .expect("record");
        serde_json::from_slice::<Value>(value.value()).expect("record JSON")
    };
    assert_eq!(record["key_id"], "key-2");
}

#[tokio::test]
async fn plaintext_oauth_store_round_trips_in_owner_private_state() {
    use rmcp::transport::auth::{CredentialStore as _, StoredCredentials};

    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("oauth-plaintext.redb");
    let factory =
        OAuthStoreFactory::plaintext_state(&path, "repository-1".into()).expect("plaintext store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    let credentials: StoredCredentials = serde_json::from_value(json!({
        "client_id": "visible-client-id",
        "token_response": null,
        "granted_scopes": ["openid"],
        "token_received_at": 1
    }))
    .expect("credentials");
    store.save(credentials.clone()).await.expect("save");
    let loaded = store.load().await.expect("load").expect("credentials");
    assert_eq!(
        serde_json::to_value(loaded).expect("loaded JSON"),
        serde_json::to_value(credentials).expect("expected JSON")
    );
    drop(store);
    drop(factory);
    assert!(
        fs::read(&path)
            .expect("state bytes")
            .windows(b"visible-client-id".len())
            .any(|window| window == b"visible-client-id")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::symlink_metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let factory =
        OAuthStoreFactory::plaintext_state(&path, "repository-1".into()).expect("plaintext store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    store.clear().await.expect("clear");
    assert!(store.load().await.expect("load after clear").is_none());
}

#[tokio::test]
async fn ephemeral_oauth_store_round_trips_without_a_state_path() {
    use rmcp::transport::auth::{CredentialStore as _, StoredCredentials};

    let factory =
        OAuthStoreFactory::ephemeral_state("repository-1".into()).expect("ephemeral store");
    let store = factory.store("splunk", "https://splunk.example.com/services/mcp");
    let credentials: StoredCredentials = serde_json::from_value(json!({
        "client_id": "ephemeral-client-id",
        "token_response": null,
        "granted_scopes": ["openid"],
        "token_received_at": 1
    }))
    .expect("credentials");
    store.save(credentials.clone()).await.expect("save");
    let loaded = store.load().await.expect("load").expect("credentials");
    assert_eq!(
        serde_json::to_value(loaded).expect("loaded JSON"),
        serde_json::to_value(credentials).expect("expected JSON")
    );
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> (String, Option<Value>) {
    use tokio::io::AsyncReadExt as _;

    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.expect("read request");
        assert!(count > 0, "client disconnected before sending a request");
        request.extend_from_slice(&chunk[..count]);
        if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).expect("request headers");
    let first = headers.lines().next().expect("request line").to_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::trim)
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk).await.expect("read request body");
        assert!(count > 0, "client disconnected before sending its body");
        request.extend_from_slice(&chunk[..count]);
    }
    let body = &request[header_end..header_end + content_length];
    (first, serde_json::from_slice(body).ok())
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    extra_headers: &str,
    body: &str,
) {
    use tokio::io::AsyncWriteExt as _;

    let response = format!(
        "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

fn configured_http_server(endpoint: String) -> ConfiguredServer {
    ConfiguredServer {
        name: "fixture".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(5_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    }
}

/// How the fixture acknowledges the one-way `notifications/initialized` frame.
#[derive(Clone, Copy)]
enum EmptyAck {
    /// Empty `200 OK` with an exact `Content-Length: 0`.
    Measured,
    /// Empty `200 OK` delimited by chunked encoding, so no size hint is exposed.
    Chunked,
}

async fn execute_stateless_discovery(
    allow_stateless: bool,
) -> Option<Result<RemoteOperationResult, ExecutionError>> {
    execute_stateless_discovery_with_ack(allow_stateless, EmptyAck::Measured).await
}

async fn execute_stateless_discovery_with_ack(
    allow_stateless: bool,
    empty_ack: EmptyAck,
) -> Option<Result<RemoteOperationResult, ExecutionError>> {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return None;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let (_, message) = read_http_request(&mut stream).await;
            let method = message
                .as_ref()
                .and_then(|value| value.get("method"))
                .and_then(Value::as_str);
            match method {
                Some("initialize") => {
                    let body = json!({
                        "jsonrpc": "2.0",
                        "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                        "result": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "stateless-fixture", "version": "1.0.0"}
                        }
                    })
                    .to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        &body,
                    )
                    .await;
                    if !allow_stateless {
                        break;
                    }
                }
                Some("notifications/initialized") => match empty_ack {
                    EmptyAck::Measured => {
                        write_http_response(
                            &mut stream,
                            "200 OK",
                            "Content-Type: application/json\r\n",
                            "",
                        )
                        .await;
                    }
                    EmptyAck::Chunked => {
                        use tokio::io::AsyncWriteExt as _;

                        stream
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
                            )
                            .await
                            .expect("chunked acknowledgement");
                    }
                },
                Some("tools/list") => {
                    let body = json!({
                        "jsonrpc": "2.0",
                        "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                        "result": {
                            "tools": [{
                                "name": "splunk_get_info",
                                "inputSchema": {"type": "object"}
                            }]
                        }
                    })
                    .to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        &body,
                    )
                    .await;
                    break;
                }
                _ => panic!("unexpected stateless MCP request: {message:?}"),
            }
        }
    });
    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let mut configured = configured_http_server(endpoint);
    configured.allow_stateless = allow_stateless;
    let result = execute_remote_operation(
        http,
        &configured,
        &McpOperation::ListTools {
            server: "fixture".into(),
            cursor: None,
        },
        HashMap::new(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await;
    server_task.await.expect("server task");
    Some(result)
}

#[tokio::test]
async fn streamable_http_stateless_discovery_requires_explicit_opt_in() {
    let Some(denied) = execute_stateless_discovery(false).await else {
        return;
    };
    assert!(matches!(denied, Err(ExecutionError::Failed(_))));

    let allowed = execute_stateless_discovery(true)
        .await
        .expect("loopback listener")
        .expect("opt-in stateless discovery");
    let RemoteOperationResult::Tools(tools) = allowed else {
        panic!("tools result");
    };
    assert_eq!(tools.tools[0].name, "splunk_get_info");
}

#[tokio::test]
async fn streamable_http_accepts_size_hintless_empty_one_way_acknowledgement() {
    let Some(result) = execute_stateless_discovery_with_ack(true, EmptyAck::Chunked).await else {
        return;
    };
    let RemoteOperationResult::Tools(tools) = result.expect("chunked empty acknowledgement") else {
        panic!("tools result");
    };
    assert_eq!(tools.tools[0].name, "splunk_get_info");
}

#[tokio::test]
async fn streamable_http_call_initialization_failure_has_a_known_outcome() {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let (_, message) = read_http_request(&mut stream).await;
        assert_eq!(
            message
                .and_then(|value| value["method"].as_str().map(str::to_owned))
                .as_deref(),
            Some("initialize")
        );
        write_http_response(&mut stream, "500 Internal Server Error", "", "").await;
    });
    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let dispatched = std::sync::atomic::AtomicBool::new(false);
    let result = execute_remote_operation(
        http,
        &configured_http_server(endpoint),
        &McpOperation::CallTool {
            server: "fixture".into(),
            tool: "echo".into(),
            description: None,
            annotations: None,
            arguments: json!({}),
            input_schema: Box::new(json!({"type": "object"})),
            schema_sha256: "unused-before-dispatch".into(),
        },
        HashMap::new(),
        &dispatched,
    )
    .await;
    let Err(error) = result else {
        panic!("initialization must fail");
    };
    assert!(matches!(error, ExecutionError::Failed(_)));
    assert!(!dispatched.load(std::sync::atomic::Ordering::Acquire));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn streamable_http_call_failure_after_dispatch_has_an_unknown_outcome() {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let (first, message) = read_http_request(&mut stream).await;
            let method = message
                .as_ref()
                .and_then(|value| value.get("method"))
                .and_then(Value::as_str);
            match method {
                Some("initialize") => {
                    let body = json!({
                        "jsonrpc": "2.0",
                        "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                        "result": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fixture", "version": "1.0.0"}
                        }
                    })
                    .to_string();
                    write_http_response(
                        &mut stream,
                        "200 OK",
                        "Content-Type: application/json\r\nMcp-Session-Id: test-session\r\n",
                        &body,
                    )
                    .await;
                }
                Some("notifications/initialized") => {
                    write_http_response(&mut stream, "202 Accepted", "", "").await;
                }
                Some("tools/call") => break,
                None if first.starts_with("GET ") => {
                    write_http_response(&mut stream, "405 Method Not Allowed", "", "").await;
                }
                _ => panic!("unexpected MCP request: {first} {message:?}"),
            }
        }
    });
    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let dispatched = std::sync::atomic::AtomicBool::new(false);
    let result = execute_remote_operation(
        http,
        &configured_http_server(endpoint),
        &McpOperation::CallTool {
            server: "fixture".into(),
            tool: "echo".into(),
            description: None,
            annotations: None,
            arguments: json!({}),
            input_schema: Box::new(json!({"type": "object"})),
            schema_sha256: "unused-after-dispatch".into(),
        },
        HashMap::new(),
        &dispatched,
    )
    .await;
    let Err(error) = result else {
        panic!("dispatched call must fail without a response");
    };
    assert!(matches!(error, ExecutionError::OutcomeUnknown(_)));
    assert!(dispatched.load(std::sync::atomic::Ordering::Acquire));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn streamable_http_json_session_discovery_uses_fresh_stateful_transport() {
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping loopback transport test: sandbox forbids listeners");
            return;
        }
        Err(error) => panic!("listener: {error}"),
    };
    let address = listener.local_addr().expect("address");
    let saw_delete = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_saw_delete = Arc::clone(&saw_delete);
    let server_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 2048];
                let count = stream.read(&mut chunk).await.expect("read");
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if let Some(index) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let (content_length, first) = {
                let headers = std::str::from_utf8(&request[..header_end]).expect("headers");
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let first = headers.lines().next().expect("request line").to_owned();
                (content_length, first)
            };
            while request.len() < header_end + content_length {
                let mut chunk = [0_u8; 2048];
                let count = stream.read(&mut chunk).await.expect("body");
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
            }
            let body = &request[header_end..header_end + content_length];
            let message = serde_json::from_slice::<Value>(body).ok();
            let (status, extra, response_body) = if first.starts_with("GET ") {
                ("405 Method Not Allowed", "", String::new())
            } else if first.starts_with("DELETE ") {
                ("200 OK", "", String::new())
            } else {
                match message
                    .as_ref()
                    .and_then(|value| value.get("method"))
                    .and_then(Value::as_str)
                {
                    Some("initialize") => (
                        "200 OK",
                        "Content-Type: application/json\r\nMcp-Session-Id: test-session\r\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                            "result": {
                                "protocolVersion": "2025-11-25",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "fixture", "version": "1.0.0"}
                            }
                        })
                        .to_string(),
                    ),
                    Some("tools/list") => (
                        "200 OK",
                        "Content-Type: application/json\r\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": message.as_ref().and_then(|value| value.get("id")).cloned().unwrap(),
                            "result": {
                                "tools": [{
                                    "name": "splunk_run_search",
                                    "inputSchema": {"type": "object"}
                                }]
                            }
                        })
                        .to_string(),
                    ),
                    _ => ("202 Accepted", "", String::new()),
                }
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
            if first.starts_with("DELETE ") {
                server_saw_delete.store(true, std::sync::atomic::Ordering::Release);
                break;
            }
        }
    });

    let endpoint = format!("http://{address}/mcp");
    let http = HardenedStreamableHttpClient::for_test(
        endpoint.clone(),
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("client"),
        1024 * 1024,
    );
    let configured = ConfiguredServer {
        name: "splunk".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(5_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let result = execute_remote_operation(
        http,
        &configured,
        &McpOperation::ListTools {
            server: "splunk".into(),
            cursor: None,
        },
        HashMap::new(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await
    .expect("remote discovery");
    let RemoteOperationResult::Tools(result) = result else {
        panic!("tools result");
    };
    assert_eq!(result.tools[0].name, "splunk_run_search");
    tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("session delete")
        .expect("server");
    assert!(saw_delete.load(std::sync::atomic::Ordering::Acquire));
}

#[cfg(feature = "live-splunk")]
#[tokio::test]
#[ignore = "requires COLOSSUS_LIVE_SPLUNK_MCP_URL and SPLUNK_MCP_TOKEN"]
async fn live_splunk_streamable_http_discovery() {
    let endpoint = env::var("COLOSSUS_LIVE_SPLUNK_MCP_URL").expect("COLOSSUS_LIVE_SPLUNK_MCP_URL");
    let token = env::var("SPLUNK_MCP_TOKEN").expect("SPLUNK_MCP_TOKEN");
    let url = url::Url::parse(&endpoint).expect("endpoint URL");
    let client = colossus_network::pinned_reqwest_client(
        &url,
        &AdditionalRootCertificates::default(),
        30_000,
        true,
    )
    .await
    .expect("hardened HTTP client");
    let http = HardenedStreamableHttpClient::for_test(endpoint.clone(), client, 1024 * 1024);
    let server = ConfiguredServer {
        name: "splunk".into(),
        transport: McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        cwd: None,
        environment: BTreeMap::new(),
        url: Some(endpoint),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: true,
        oauth: None,
        allowed_tools: ToolAllowlist::All,
        research_tools: Vec::new(),
        timeout_ms: Some(30_000),
        max_output_bytes: Some(1024 * 1024),
        effect_action_prefix: None,
        provenance: None,
    };
    let authorization = format!("Bearer {token}")
        .parse::<http::HeaderValue>()
        .expect("bearer header");
    let result = execute_remote_operation(
        http,
        &server,
        &McpOperation::ListTools {
            server: "splunk".into(),
            cursor: None,
        },
        HashMap::from([(
            http::HeaderName::from_static("authorization"),
            authorization,
        )]),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .await
    .expect("Splunk discovery");
    assert!(matches!(result, RemoteOperationResult::Tools(_)));
}
