use super::*;
use colossus_contracts::{ModelLimits, ResearchLaneStatus, ResearchStatus};
use colossus_testkit::InMemoryEventJournal;
use sha2::{Digest as _, Sha256};
use std::sync::Mutex;

struct Tools {
    advertised: Vec<McpToolSummary>,
    calls: Mutex<Vec<Value>>,
    discoveries: Mutex<usize>,
    change_schema: bool,
    delay_ms: u64,
    timeout_ms: u64,
}

#[async_trait]
impl EffectExecutor for Tools {
    async fn execute(
        &self,
        request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        let operation = &request.content["operation"];
        let output = if request.action == "mcp.tools" {
            let mut discoveries = self.discoveries.lock().expect("discoveries");
            *discoveries += 1;
            let server = operation["server"].as_str().expect("server");
            let mut tools = self
                .advertised
                .iter()
                .filter(|tool| tool.server == server)
                .cloned()
                .collect::<Vec<_>>();
            if self.change_schema && *discoveries > 1 {
                for tool in &mut tools {
                    tool.input_schema["properties"]["query"]["type"] = json!("integer");
                    tool.schema_sha256 = hex::encode(Sha256::digest(
                        serde_json::to_vec(&tool.input_schema).expect("schema"),
                    ));
                }
            }
            json!({"server": server, "tools": tools, "next_cursor": null})
        } else {
            assert_eq!(request.action, "mcp.call");
            self.calls.lock().expect("calls").push(operation.clone());
            json!({"server": operation["server"], "tool": operation["tool"],
                "result": {"content": [{"type": "text", "text": "Released MCP research evidence"}]}})
        };
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&output).expect("output"),
            effect_succeeded: true,
        })
    }
}

struct Model {
    events: Vec<ProviderEvent>,
    requests: Mutex<Vec<ModelRequest>>,
    available: bool,
    supports_tools: bool,
}

#[async_trait]
impl ModelProvider for Model {
    fn route(&self, role: &str) -> Result<ProviderRoute, ModelProviderError> {
        if !self.available {
            return Err(ModelProviderError::Configuration("no model".into()));
        }
        assert_eq!(role, "research_worker");
        Ok(ProviderRoute {
            role: role.into(),
            profile: "fixture".into(),
            model_profile: "fixture".into(),
            provider_profile: "fixture".into(),
            provider: "fixture".into(),
            model: "fixture".into(),
            limits: ModelLimits {
                context_window_tokens: 32768,
                max_output_tokens: 4096,
                safety_margin_tokens: 3276,
                input_budget_tokens: 25396,
            },
            capabilities: ModelCapabilities {
                tool_calls: self.supports_tools,
                streaming: false,
                image_inputs: false,
            },
            reasoning_effort: None,
        })
    }

    async fn turn(
        &self,
        role: &str,
        request: ModelRequest,
        context: ExecutionContext,
    ) -> Result<ProviderTurn, ModelProviderError> {
        assert_eq!(role, "research_worker");
        assert_eq!(context.run_id.as_deref(), Some("research-fixture"));
        self.requests.lock().expect("requests").push(request);
        Ok(ProviderTurn {
            profile: "fixture".into(),
            model_profile: "fixture".into(),
            provider_profile: "fixture".into(),
            provider: "fixture".into(),
            model: "fixture".into(),
            response_id: None,
            events: self.events.clone(),
        })
    }
}

fn selected(name: &str, arguments: Value) -> ProviderEvent {
    ProviderEvent::ToolCallRequested {
        call_id: "research-call".into(),
        name: name.into(),
        arguments,
    }
}

fn model(events: Vec<ProviderEvent>) -> Arc<Model> {
    Arc::new(Model {
        events,
        requests: Mutex::new(Vec::new()),
        available: true,
        supports_tools: true,
    })
}

fn tools() -> Arc<Tools> {
    Arc::new(Tools {
        advertised: vec![tool("search_events"), tool("blocked")],
        calls: Mutex::new(Vec::new()),
        discoveries: Mutex::new(0),
        change_schema: false,
        delay_ms: 0,
        timeout_ms: 30_000,
    })
}

fn tool(name: &str) -> McpToolSummary {
    let input_schema = json!({"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["query"], "additionalProperties": false});
    McpToolSummary {
        server: "fixture".into(),
        name: name.into(),
        title: None,
        description: Some("Search events using the query".into()),
        annotations: None,
        schema_sha256: hex::encode(Sha256::digest(
            serde_json::to_vec(&input_schema).expect("schema"),
        )),
        input_schema,
        output_schema: None,
    }
}

fn server(allowed: &[&str], projections: Vec<McpResearchToolConfig>) -> McpServerConfig {
    McpServerConfig {
        transport: colossus_mcp::McpTransportKind::StreamableHttp,
        command: PathBuf::new(),
        args: Vec::new(),
        working_directory: None,
        environment: BTreeMap::new(),
        literal_environment: BTreeMap::new(),
        url: Some("http://127.0.0.1:18787/mcp".into()),
        headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        allow_stateless: false,
        protocol_version: colossus_contracts::McpProtocolVersion::Auto,
        oauth: None,
        allowed_tools: allowed.iter().map(|name| (*name).into()).collect(),
        research_tools: projections,
        timeout_ms: None,
        max_output_bytes: None,
        effect_action_prefix: None,
        provenance: None,
    }
}

fn gateway(allow_call: bool, timeout_ms: u64) -> EffectGateway {
    let decision = if allow_call {
        DecisionOutcome::Allow
    } else {
        DecisionOutcome::Deny
    };
    let policy = BuiltInPolicy::offline_default()
        .with_action("mcp.tools", DecisionOutcome::Allow)
        .with_action("mcp.call", decision)
        .with_action_timeout("mcp.tools", timeout_ms)
        .with_action_timeout("mcp.call", timeout_ms)
        .with_post_effect(false)
        .with_sandbox("native", "mcp-research-test", false)
        .with_action_restrictions(
            "mcp.tools",
            Vec::new(),
            Vec::new(),
            vec!["http://127.0.0.1:18787".into()],
        )
        .with_action_restrictions(
            "mcp.call",
            Vec::new(),
            Vec::new(),
            vec!["http://127.0.0.1:18787".into()],
        );
    EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(DenyApproval),
        SafetyKernel::new(["mcp.invoke".into()]),
        [5_u8; 32],
    )
}

fn run() -> ResearchRun {
    ResearchRun {
        id: "research-fixture".into(),
        session_id: "session-fixture".into(),
        question: "Find event evidence".into(),
        depth: ResearchDepth::Quick,
        source_kinds: vec![ResearchSourceKind::Mcp],
        status: ResearchStatus::Running,
        queries: Vec::new(),
        lanes: Vec::new(),
        progress: Vec::new(),
        limitations: Vec::new(),
        report: String::new(),
        error: String::new(),
        created_at: "2026-10-07T00:00:00Z".into(),
        updated_at: "2026-10-07T00:00:00Z".into(),
        completed_at: None,
    }
}

async fn collect(
    servers: BTreeMap<String, McpServerConfig>,
    tools: Arc<Tools>,
    model: Arc<Model>,
    allow_call: bool,
    limit: usize,
) -> ResearchCollection {
    let workspace = tempfile::tempdir().expect("workspace");
    let executor = McpExecutor::new(
        &McpConfig {
            servers,
            ..McpConfig::default()
        },
        workspace.path(),
        "native",
        Arc::clone(&tools) as Arc<dyn EffectExecutor>,
    )
    .expect("executor");
    let gateway = gateway(allow_call, tools.timeout_ms);
    let model = GatewayResearchModel { provider: model };
    McpResearchCollector {
        gateway: &gateway,
        executor: &executor,
        effect: tools.as_ref(),
        model: &model,
    }
    .collect(&run(), "event evidence", limit)
    .await
}

#[tokio::test]
async fn slow_inherited_servers_and_invocation_fit_the_outer_research_deadline() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.timeout_ms = 1_000;
    config.research.max_sources = 1;
    config.research.max_workers = 1;
    let mut tools = tools();
    let fixture = Arc::get_mut(&mut tools).expect("exclusive fixture");
    fixture.advertised.clear();
    fixture.delay_ms = 50;
    fixture.timeout_ms = config.sandbox.timeout_ms;
    // Each discovery is comfortably inside its own one-second deadline, but
    // together these exceed the old two-second outer collection allowance.
    for index in 0..64 {
        let name = format!("fixture-{index:02}");
        let mut advertised = tool("search_events");
        advertised.server = name.clone();
        fixture.advertised.push(advertised);
        config
            .mcp
            .servers
            .insert(name, server(&["search_events"], Vec::new()));
    }
    let timeout_ms = crate::composition::research_run_timeout_ms(0, &config);
    let output = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        collect(
            config.mcp.servers,
            Arc::clone(&tools),
            model(vec![selected(
                "research_mcp_0",
                json!({"query": "event evidence"}),
            )]),
            true,
            1,
        ),
    )
    .await
    .expect("valid inner MCP effects must complete within the outer deadline");
    assert_eq!(
        output.status,
        ResearchLaneStatus::Completed,
        "{}",
        output.message
    );
    assert_eq!(output.sources.len(), 1);
    assert_eq!(*tools.discoveries.lock().expect("discoveries"), 65);
    assert_eq!(tools.calls.lock().expect("calls").len(), 1);
}

#[tokio::test]
async fn empty_projections_inherit_explicit_pattern_and_wildcard_tool_selection() {
    for allowed in [vec!["search_events"], vec!["search_*"], vec!["*"]] {
        let tools = tools();
        let model = model(vec![selected(
            "research_mcp_0",
            json!({"query": "event evidence", "limit": 10}),
        )]);
        let output = collect(
            BTreeMap::from([("fixture".into(), server(&allowed, Vec::new()))]),
            Arc::clone(&tools),
            Arc::clone(&model),
            true,
            2,
        )
        .await;
        assert_eq!(
            output.status,
            ResearchLaneStatus::Completed,
            "{}",
            output.message
        );
        assert_eq!(output.sources.len(), 1);
        assert_eq!(output.sources[0].uri, "mcp://fixture/search_events");
        let requests = model.requests.lock().expect("requests");
        let offered = &requests[0].tools;
        assert_eq!(offered.len(), if allowed == ["*"] { 2 } else { 1 });
        assert_eq!(offered[0].input_schema, tools.advertised[0].input_schema);
        assert_eq!(
            tools.calls.lock().expect("calls")[0]["arguments"],
            json!({"query": "event evidence", "limit": 10})
        );
        assert_eq!(
            *tools.discoveries.lock().expect("discoveries"),
            2,
            "invocation must rediscover the schema"
        );
    }
}

#[tokio::test]
async fn explicit_projections_override_the_servers_normal_selection_without_a_model() {
    let tools = tools();
    let mut model = model(Vec::new());
    Arc::get_mut(&mut model)
        .expect("exclusive fixture")
        .available = false;
    let projection = McpResearchToolConfig {
        tool: "search_events".into(),
        title: Some("Projected source".into()),
        arguments: json!({"query": "prefix {query}", "limit": 3}),
    };
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["*"], vec![projection]))]),
        Arc::clone(&tools),
        Arc::clone(&model),
        true,
        3,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Completed);
    assert_eq!(output.sources[0].title, "Projected source");
    assert_eq!(
        tools.calls.lock().expect("calls")[0]["arguments"],
        json!({"query": "prefix event evidence", "limit": 3})
    );
    assert_eq!(tools.calls.lock().expect("calls").len(), 1);
    assert!(model.requests.lock().expect("requests").is_empty());
}

#[tokio::test]
async fn projections_on_one_server_do_not_disable_inheritance_on_another() {
    let mut tools = tools();
    let mut other = tool("search_events");
    other.server = "other".into();
    Arc::get_mut(&mut tools)
        .expect("exclusive fixture")
        .advertised
        .push(other);
    let model = model(vec![selected(
        "research_mcp_0",
        json!({"query": "automatic"}),
    )]);
    let projection = McpResearchToolConfig {
        tool: "search_events".into(),
        title: None,
        arguments: json!({"query": "projected"}),
    };
    let servers = BTreeMap::from([
        ("fixture".into(), server(&["*"], vec![projection])),
        ("other".into(), server(&["search_events"], Vec::new())),
    ]);
    let output = collect(servers, Arc::clone(&tools), Arc::clone(&model), true, 2).await;
    assert_eq!(output.sources.len(), 2, "{}", output.message);
    let calls = tools.calls.lock().expect("calls");
    assert_eq!(calls[0]["server"], "fixture");
    assert_eq!(calls[1]["server"], "other");
    assert_eq!(model.requests.lock().expect("requests")[0].tools.len(), 1);
}

#[tokio::test]
async fn invalid_selections_never_dispatch_a_call() {
    let valid = selected("research_mcp_0", json!({"query": "evidence"}));
    for events in [
        Vec::new(),
        vec![ProviderEvent::FinalOutput {
            text: String::new(),
        }],
        vec![
            valid.clone(),
            selected("research_mcp_99", json!({"query": "evidence"})),
        ],
        vec![selected("research_mcp_0", json!({"query": 42}))],
        vec![selected(
            "research_mcp_0",
            json!({"query": "evidence", "extra": true}),
        )],
        vec![selected("blocked", json!({"query": "evidence"}))],
        vec![valid.clone(), valid.clone()],
    ] {
        let tools = tools();
        let output = collect(
            BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
            Arc::clone(&tools),
            model(events),
            true,
            2,
        )
        .await;
        assert_eq!(output.status, ResearchLaneStatus::Failed);
        assert!(tools.calls.lock().expect("calls").is_empty());
    }
    let tools = tools();
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["*"], Vec::new()))]),
        Arc::clone(&tools),
        model(vec![
            valid,
            selected("research_mcp_1", json!({"query": "evidence"})),
        ]),
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Failed);
    assert!(tools.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn inherited_calls_preserve_policy_denials_and_fresh_schema_checks() {
    for changed in [false, true] {
        let mut tools = tools();
        Arc::get_mut(&mut tools)
            .expect("exclusive fixture")
            .change_schema = changed;
        let output = collect(
            BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
            Arc::clone(&tools),
            model(vec![selected(
                "research_mcp_0",
                json!({"query": "evidence"}),
            )]),
            changed,
            1,
        )
        .await;
        assert_eq!(
            output.status,
            if changed {
                ResearchLaneStatus::Failed
            } else {
                ResearchLaneStatus::Denied
            }
        );
        assert!(tools.calls.lock().expect("calls").is_empty());
    }
}

#[tokio::test]
async fn no_selected_tools_is_completed_and_no_enabled_tools_is_disabled() {
    let tools = tools();
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
        Arc::clone(&tools),
        model(vec![ProviderEvent::FinalOutput {
            text: "No relevant tools".into(),
        }]),
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Completed);
    assert!(output.sources.is_empty());
    let model = model(Vec::new());
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["unadvertised"], Vec::new()))]),
        Arc::clone(&tools),
        Arc::clone(&model),
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Disabled);
    assert!(model.requests.lock().expect("requests").is_empty());
    assert!(tools.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn unavailable_model_reports_a_limitation_without_guessing_tool_arguments() {
    let tools = tools();
    let mut model = model(Vec::new());
    Arc::get_mut(&mut model)
        .expect("exclusive fixture")
        .available = false;
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
        Arc::clone(&tools),
        model,
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Failed);
    assert!(output.message.contains("research_worker model"));
    assert!(tools.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn a_model_without_tool_support_does_not_receive_or_invoke_inherited_tools() {
    let tools = tools();
    let mut model = model(Vec::new());
    Arc::get_mut(&mut model)
        .expect("exclusive fixture")
        .supports_tools = false;
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
        Arc::clone(&tools),
        Arc::clone(&model),
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Failed);
    assert!(output.message.contains("tool-call support"));
    assert!(model.requests.lock().expect("requests").is_empty());
    assert!(tools.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn inherited_catalog_is_ranked_and_bounded_before_model_disclosure() {
    let mut tools = tools();
    for index in 0..100 {
        let mut unrelated = tool(&format!("unrelated_{index}"));
        unrelated.description = None;
        Arc::get_mut(&mut tools)
            .expect("exclusive fixture")
            .advertised
            .push(unrelated);
    }
    let model = model(vec![selected(
        "research_mcp_0",
        json!({"query": "evidence"}),
    )]);
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["*"], Vec::new()))]),
        Arc::clone(&tools),
        Arc::clone(&model),
        true,
        1,
    )
    .await;
    assert_eq!(output.sources.len(), 1, "{}", output.message);
    let requests = model.requests.lock().expect("requests");
    assert_eq!(requests[0].tools.len(), MAX_RESEARCH_MCP_TOOLS);
    assert!(requests[0].tools[0].description.contains("search_events"));
}

#[tokio::test]
async fn schema_budget_keeps_exact_schemas_and_rejects_unoffered_tools() {
    for name in ["research_mcp_1", "research_mcp_0"] {
        let mut tools = tools();
        let oversized = &mut Arc::get_mut(&mut tools)
            .expect("exclusive fixture")
            .advertised[0];
        oversized.input_schema["description"] = json!("x".repeat(128 * 1024));
        oversized.schema_sha256 = hex::encode(Sha256::digest(
            serde_json::to_vec(&oversized.input_schema).expect("schema"),
        ));
        let model = model(vec![selected(name, json!({"query": "evidence"}))]);
        let output = collect(
            BTreeMap::from([("fixture".into(), server(&["*"], Vec::new()))]),
            Arc::clone(&tools),
            Arc::clone(&model),
            true,
            1,
        )
        .await;
        let requests = model.requests.lock().expect("requests");
        assert_eq!(requests[0].tools.len(), 1);
        assert_eq!(requests[0].tools[0].name, "research_mcp_1");
        assert_eq!(
            requests[0].tools[0].input_schema,
            tools.advertised[1].input_schema
        );
        if name == "research_mcp_1" {
            assert_eq!(output.sources.len(), 1, "{}", output.message);
        } else {
            assert_eq!(output.status, ResearchLaneStatus::Failed);
            assert!(tools.calls.lock().expect("calls").is_empty());
        }
    }
}

#[tokio::test]
async fn an_unavailable_server_does_not_hide_enabled_tools_on_healthy_servers() {
    let tools = tools();
    let model = model(vec![selected(
        "research_mcp_0",
        json!({"query": "evidence"}),
    )]);
    let mut missing = server(&["*"], Vec::new());
    missing.transport = colossus_mcp::McpTransportKind::Stdio;
    missing.command = std::env::temp_dir().join("colossus-unavailable-research-server");
    missing.url = None;
    let output = collect(
        BTreeMap::from([
            ("missing".into(), missing),
            ("fixture".into(), server(&["search_events"], Vec::new())),
        ]),
        Arc::clone(&tools),
        model,
        true,
        1,
    )
    .await;
    assert_eq!(output.status, ResearchLaneStatus::Completed);
    assert_eq!(output.sources.len(), 1, "{}", output.message);
    assert!(output.message.contains("failed=1"));
}

#[tokio::test]
async fn provider_object_declaration_preserves_the_original_mcp_schema_and_hash() {
    let mut tools = tools();
    let schema = &mut Arc::get_mut(&mut tools)
        .expect("exclusive fixture")
        .advertised[0];
    schema
        .input_schema
        .as_object_mut()
        .expect("object schema")
        .remove("type");
    schema.schema_sha256 = hex::encode(Sha256::digest(
        serde_json::to_vec(&schema.input_schema).expect("schema"),
    ));
    let model = model(vec![selected(
        "research_mcp_0",
        json!({"query": "evidence"}),
    )]);
    let output = collect(
        BTreeMap::from([("fixture".into(), server(&["search_events"], Vec::new()))]),
        Arc::clone(&tools),
        Arc::clone(&model),
        true,
        1,
    )
    .await;
    assert_eq!(output.sources.len(), 1, "{}", output.message);
    let requests = model.requests.lock().expect("requests");
    assert_eq!(requests[0].tools[0].input_schema["type"], "object");
    let calls = tools.calls.lock().expect("calls");
    assert_eq!(calls[0]["input_schema"], tools.advertised[0].input_schema);
    assert_eq!(calls[0]["schema_sha256"], tools.advertised[0].schema_sha256);
}
