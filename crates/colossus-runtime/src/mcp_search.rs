use super::*;
use futures::{StreamExt as _, stream};

const MAX_SEARCH_RESULTS: usize = 10;
const MAX_UNAVAILABLE_NAMES: usize = 32;
const PARALLEL_DISCOVERY: usize = 4;

#[derive(Serialize)]
struct McpSearchHit {
    server: String,
    name: String,
    title: Option<String>,
    description: Option<String>,
    score: u32,
}

impl McpSearchHit {
    fn new(tool: McpToolSummary, score: u32) -> Self {
        Self {
            server: tool.server,
            name: tool.name,
            title: tool.title.map(|text| text.chars().take(128).collect()),
            description: tool
                .description
                .map(|text| text.chars().take(512).collect()),
            score,
        }
    }
}

impl GatewayToolExecutor {
    pub(super) async fn search_mcp_tool_output(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        query: &str,
        selected_server: Option<&str>,
        max_results: u64,
    ) -> Result<String, ToolError> {
        let catalog = active_plugin_catalog();
        let executor = self
            .mcp
            .as_deref()
            .ok_or_else(|| ToolError::Failed("MCP adapter is unavailable".into()))?;
        let executor = catalog
            .as_ref()
            .and_then(|catalog| catalog.mcp.as_deref())
            .unwrap_or(executor);
        let bound = self.bound_effects.as_ref().map(|effects| {
            WorkspaceBoundEffectExecutor::new(
                effects.identity.clone(),
                catalog
                    .as_ref()
                    .and_then(|catalog| catalog.mcp.clone())
                    .unwrap_or_else(|| Arc::clone(self.mcp.as_ref().expect("MCP checked"))),
            )
        });
        let effect = self
            .bound_effects
            .as_ref()
            .map_or(executor as &dyn EffectExecutor, |effects| {
                effects.mcp.as_ref()
            });
        let effect = bound
            .as_ref()
            .map_or(effect, |bound| bound as &dyn EffectExecutor);
        let actor = model_actor(call, &context);
        search_mcp_catalog(
            self.gateway.as_ref(),
            executor,
            effect,
            &actor,
            &context,
            query,
            selected_server,
            max_results,
        )
        .await
        .map_err(mcp_runtime_tool_error)
    }
}

#[allow(clippy::too_many_arguments)]
async fn search_mcp_catalog(
    gateway: &EffectGateway,
    executor: &McpExecutor,
    effect: &dyn EffectExecutor,
    actor: &Actor,
    context: &ExecutionContext,
    query: &str,
    selected_server: Option<&str>,
    max_results: u64,
) -> Result<String, RuntimeError> {
    let servers = selected_server.map_or_else(|| executor.server_names(), |name| vec![name.into()]);
    let terms = search_terms(query);
    let results = stream::iter(servers.into_iter().map(|server| {
        let actor = &actor;
        let context = &context;
        let terms = &terms;
        async move {
            let mut hits = Vec::new();
            let result = visit_mcp_server_tools(
                gateway,
                executor,
                effect,
                actor,
                context,
                &server,
                |tool| {
                    let score = mcp_search_score(&tool, terms);
                    if score > 0 || selected_server.is_some() {
                        hits.push(McpSearchHit::new(tool, score));
                        hits.sort_by(|left, right| {
                            right
                                .score
                                .cmp(&left.score)
                                .then_with(|| left.name.cmp(&right.name))
                        });
                        hits.truncate(MAX_SEARCH_RESULTS + 1);
                    }
                    Ok(())
                },
            )
            .await;
            (server, result, hits)
        }
    }))
    .buffer_unordered(PARALLEL_DISCOVERY)
    .collect::<Vec<_>>()
    .await;
    let mut hits = Vec::new();
    let mut unavailable = Vec::new();
    let mut unavailable_count = 0_usize;
    for (server, result, server_hits) in results {
        match result {
            Ok(()) => hits.extend(server_hits),
            Err(error) if selected_server.is_some() => {
                return Err(error);
            }
            Err(_) => {
                unavailable_count += 1;
                if unavailable.len() < MAX_UNAVAILABLE_NAMES {
                    unavailable.push(server);
                }
            }
        }
    }
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.server.cmp(&right.server))
            .then_with(|| left.name.cmp(&right.name))
    });
    let limit = usize::try_from(max_results)
        .unwrap_or(MAX_SEARCH_RESULTS)
        .clamp(1, MAX_SEARCH_RESULTS);
    let truncated = hits.len() > limit;
    hits.truncate(limit);
    let output = serde_json::to_string(&json!({
        "query": query,
        "tools": hits,
        "truncated": truncated,
        "unavailable_servers": unavailable,
        "unavailable_server_count": unavailable_count,
    }))
    .map_err(|error| RuntimeError::Config(error.to_string()))?;
    if output.len() > 32 * 1024 {
        return Err(RuntimeError::Config(
            "MCP search output exceeded its bound".into(),
        ));
    }
    Ok(output)
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn mcp_search_score(tool: &McpToolSummary, terms: &[String]) -> u32 {
    let name = tool.name.to_ascii_lowercase();
    let title = tool.title.as_deref().unwrap_or("").to_ascii_lowercase();
    let description = tool
        .description
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let argument_names = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .keys()
                .map(|name| name.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    terms.iter().fold(0_u32, |score, term| {
        score
            + if name == *term {
                24
            } else if name.contains(term) {
                12
            } else {
                0
            }
            + if title.contains(term) { 5 } else { 0 }
            + if description.contains(term) { 2 } else { 0 }
            + if argument_names.iter().any(|name| name.contains(term)) {
                3
            } else {
                0
            }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::DecisionOutcome;
    use colossus_mcp::{McpOAuthCredentialStoreKind, McpTransportKind};
    use colossus_policy::AllowApproval;
    use colossus_testkit::InMemoryEventJournal;

    struct AdvertisedTools;

    #[async_trait]
    impl EffectExecutor for AdvertisedTools {
        async fn execute(
            &self,
            request: &colossus_contracts::EffectRequest,
            _permit: ExecutionPermit,
        ) -> Result<QuarantinedEffectResult, ExecutionError> {
            assert_eq!(request.action, "mcp.tools");
            let page = McpToolsPage {
                server: "splunk".into(),
                tools: vec![
                    tool("get_alert", "Get one named Splunk alert"),
                    tool("search_events", "Search Splunk events using SPL"),
                ],
                next_cursor: None,
            };
            Ok(QuarantinedEffectResult {
                media_type: "application/json".into(),
                bytes: serde_json::to_vec(&page).expect("page"),
                effect_succeeded: true,
            })
        }
    }

    fn tool(name: &str, description: &str) -> McpToolSummary {
        McpToolSummary {
            server: "splunk".into(),
            name: name.into(),
            title: None,
            description: Some(description.into()),
            annotations: None,
            input_schema: json!({"type": "object"}),
            schema_sha256: "hash".into(),
        }
    }

    #[tokio::test]
    async fn search_is_bounded_and_survives_an_unavailable_server() {
        let workspace = tempfile::tempdir().expect("workspace");
        let remote = McpServerConfig {
            transport: McpTransportKind::StreamableHttp,
            command: PathBuf::new(),
            args: Vec::new(),
            working_directory: None,
            environment: BTreeMap::new(),
            literal_environment: BTreeMap::new(),
            url: Some("http://127.0.0.1:18787/mcp".into()),
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
        let mut missing = remote.clone();
        missing.transport = McpTransportKind::Stdio;
        missing.command = workspace.path().join("missing-server");
        missing.url = None;
        let executor = McpExecutor::new(
            &McpConfig {
                oauth_credential_store: McpOAuthCredentialStoreKind::Auto,
                servers: BTreeMap::from([("missing".into(), missing), ("splunk".into(), remote)]),
            },
            workspace.path(),
            "native",
            Arc::new(AdvertisedTools),
        )
        .expect("adapter");
        let policy = BuiltInPolicy::offline_default()
            .with_action("mcp.tools", DecisionOutcome::Allow)
            .with_post_effect(false)
            .with_sandbox("native", "mcp-search-test", false)
            .with_action_restrictions(
                "mcp.tools",
                Vec::new(),
                Vec::new(),
                vec!["http://127.0.0.1:18787".into()],
            );
        let gateway = EffectGateway::new(
            Arc::new(InMemoryEventJournal::default()),
            Arc::new(policy),
            Arc::new(AllowApproval {
                approved_by: "test".into(),
            }),
            SafetyKernel::new(["mcp.invoke".into()]),
            [5_u8; 32],
        );
        let actor = Actor {
            actor_type: ActorType::System,
            id: "mcp-search-test".into(),
        };
        let output = search_mcp_catalog(
            &gateway,
            &executor,
            &AdvertisedTools,
            &actor,
            &ExecutionContext::default(),
            "search Splunk for opaque incident 123",
            None,
            1,
        )
        .await
        .expect("search");
        let output: Value = serde_json::from_str(&output).expect("JSON");
        assert_eq!(output["tools"][0]["name"], "search_events");
        assert_eq!(output["tools"].as_array().map(Vec::len), Some(1));
        assert_eq!(output["truncated"], true);
        assert_eq!(output["unavailable_servers"], json!(["missing"]));
        assert!(output["tools"][0].get("input_schema").is_none());
    }

    #[test]
    fn search_matches_task_words_without_requiring_payload_words_in_metadata() {
        let tool = McpToolSummary {
            server: "splunk".into(),
            name: "search_events".into(),
            title: Some("Run a search".into()),
            description: Some("Search Splunk events with SPL".into()),
            annotations: None,
            input_schema: json!({"type": "object"}),
            schema_sha256: "hash".into(),
        };
        assert!(
            mcp_search_score(
                &tool,
                &search_terms("Search Splunk for xyz opaque incident 123")
            ) > 0
        );
        assert_eq!(
            mcp_search_score(&tool, &search_terms("calendar meeting")),
            0
        );
        let sparse = McpToolSummary {
            name: "run".into(),
            title: None,
            description: None,
            input_schema: json!({"type": "object", "properties": {"splunk_index": {"type": "string"}}}),
            ..tool
        };
        assert!(mcp_search_score(&sparse, &search_terms("find index")) > 0);
    }
}
