use super::*;
use colossus_contracts::{CredentialError, UserPromptResponse, VaultRecord};
use colossus_ports::{CredentialKey, CredentialVault};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn config(workspace: &Path) -> RuntimeConfig {
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.use_ephemeral_storage();
    config.browser.enabled = false;
    config.computer.enabled = false;
    config
}

#[test]
fn controlled_chat_template_opens_with_a_pinned_policy() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(runtime.access.profile, AccessProfile::Pinned);
    assert_eq!(
        runtime.access.action_decision("provider.echo"),
        Some(AccessDecision::Allow)
    );
    assert_eq!(
        runtime.access.action_decision("mcp.call"),
        Some(AccessDecision::RequireApproval)
    );
    assert_eq!(
        runtime.access.action_decision("filesystem.read"),
        Some(AccessDecision::Deny)
    );
    assert!(
        runtime
            .tool_specs()
            .iter()
            .all(|spec| allows_tool(&spec.name))
    );
}

#[test]
fn controlled_chat_host_denied_internal_roles_never_fall_back_or_accept_a_model_override() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    config.use_ephemeral_storage();
    let unavailable = [
        "planner",
        "subagent_default",
        "research_planner",
        "research_worker",
        "research_synthesizer",
        "risk_evaluator",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let options = RuntimeOpenOptions::for_controlled_chat(workspace.path())
        .unwrap()
        .with_unavailable_model_roles(unavailable);
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
    let primary = runtime.provider_route("primary").unwrap();
    assert_eq!(
        runtime
            .provider_route("unconfigured_role")
            .unwrap()
            .model_profile,
        primary.model_profile,
        "truly unconfigured roles retain the ordinary Primary fallback"
    );
    for role in [
        "planner",
        "subagent_default",
        "research_planner",
        "research_worker",
        "research_synthesizer",
        "risk_evaluator",
    ] {
        assert!(
            runtime.provider_route(role).is_err(),
            "blocked role fell back: {role}"
        );
        assert!(
            runtime
                .model_route_with_options(
                    role,
                    &colossus_contracts::WorkflowAgentOptions {
                        model_profile: Some(primary.model_profile.clone()),
                        reasoning_effort: None,
                    }
                )
                .is_err(),
            "a model preference bypassed the host denial: {role}"
        );
    }
    assert_eq!(
        runtime.access.action_decision("filesystem.read"),
        Some(AccessDecision::Deny)
    );
}

struct Questions;

#[tokio::test]
async fn completed_controlled_parent_settles_queued_child_without_notification_race() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    config.use_ephemeral_storage();
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
    )
    .unwrap();
    let session = runtime.create_session(Some("controlled child")).unwrap();
    let child = runtime
        .queue_subagent(&session.id, "Complete this controlled task", "primary")
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.run_with_subagent_scheduling(async { Ok::<_, AgentError>("parent complete") }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, "parent complete");
    assert_eq!(
        runtime.get_subagent(&child.id).unwrap().unwrap().status,
        SubagentStatus::Completed
    );
}

#[async_trait]
impl UserPromptProvider for Questions {
    async fn prompt(&self, _request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        Err(ToolError::Failed("no test question is pending".into()))
    }
}

#[tokio::test]
async fn controlled_chat_catalog_and_effects_cannot_inherit_host_authority() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = config(workspace.path());
    config.sandbox.network_destinations = vec!["https://provider.invalid".into()];
    // Even broad operator action overrides must remain beneath the host-mode ceiling.
    config.access.actions.allow = vec!["filesystem.read".into(), "network.http".into()];
    let options = RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap();
    assert!(!options.automatic_agent_instructions);
    assert!(!options.model_network_tools_enabled());
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(colossus_policy::AllowApproval {
            approved_by: "test".into(),
        }),
        Some(Arc::new(Questions)),
        options,
    )
    .unwrap();
    let names = runtime
        .tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    for expected in [
        "user.ask",
        "context.show",
        "context.compact",
        "plan.create",
        "goal.show",
        "goal.update",
        "agent.delegate",
    ] {
        assert!(
            names.contains(expected),
            "missing controlled capability {expected}"
        );
    }
    assert!(names.iter().all(|name| allows_tool(name)));
    for forbidden in [
        "filesystem.read",
        "filesystem.write",
        "shell.run",
        "browser.open",
        "computer.open",
        "network.http",
        "web.fetch",
        "docs.fetch",
        "plugin.skill.read",
        "workflow.schedule.create",
        "workflow.task.schedule",
        "workflow.schedule.set_enabled",
        "workflow.schedule.delete",
    ] {
        assert!(!names.contains(forbidden), "exposed {forbidden}");
    }
    assert_eq!(
        runtime.echo("controlled").await.unwrap().bytes,
        b"controlled"
    );
    assert!(!runtime.plugins_enabled);
    assert!(!runtime.automatic_agent_instructions);
    struct MustNotExecute(AtomicUsize);
    #[async_trait]
    impl EffectExecutor for MustNotExecute {
        async fn execute(
            &self,
            _: &EffectRequest,
            _: ExecutionPermit,
        ) -> Result<QuarantinedEffectResult, ExecutionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ExecutionError::Failed("forbidden adapter reached".into()))
        }
    }
    let adapter = MustNotExecute(AtomicUsize::new(0));
    for (action, resource) in [
        ("filesystem.read", workspace.path().to_str().unwrap()),
        ("filesystem.write", workspace.path().to_str().unwrap()),
        ("process.spawn", workspace.path().to_str().unwrap()),
        ("shell.run", workspace.path().to_str().unwrap()),
        ("network.http", "https://provider.invalid/secret"),
        ("browser.open", "https://provider.invalid"),
        ("computer.open", "computer:host"),
        (
            "plugin.registry.credential_helper",
            workspace.path().to_str().unwrap(),
        ),
        ("integration.invoke", "https://provider.invalid"),
    ] {
        assert_eq!(
            runtime.access.action_decision(action),
            Some(AccessDecision::Deny)
        );
        let mut request = effect_request(terminal_actor(), action, resource, json!({}));
        request.capabilities = vec![action.into()];
        assert!(
            runtime.gateway.execute(request, &adapter).await.is_err(),
            "allowed {action}"
        );
    }
    assert_eq!(adapter.0.load(Ordering::SeqCst), 0);
    std::fs::write(workspace.path().join("host-secret"), "private host bytes").unwrap();
    assert!(runtime.read_text_file("host-secret").await.is_err());
    assert!(
        runtime
            .write_text_file("must-not-exist", "denied")
            .await
            .is_err()
    );
    assert!(!workspace.path().join("must-not-exist").exists());
    assert!(runtime.http_get("https://provider.invalid/").await.is_err());
}

#[test]
fn controlled_chat_rejects_execution_and_unreviewed_mcp_configuration() {
    let workspace = crate::test_support::private_tempdir();
    let options = RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap();
    let base = config(workspace.path());
    let mut candidates = Vec::new();
    let mut candidate = base.clone();
    candidate.sandbox.backend = "danger_full_access".into();
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.sandbox.backend = "external".into();
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.sandbox.profile = "workspace-development".into();
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.sandbox.filesystem.push(FilesystemGrant {
        root: workspace.path().display().to_string(),
        mode: "read".into(),
    });
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate
        .sandbox
        .executables
        .push(std::env::current_exe().unwrap());
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.browser.enabled = true;
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.computer.enabled = true;
    candidates.push(candidate);
    let mut candidate = base.clone();
    candidate.sandbox.network_destinations = vec!["https://*.example.com".into()];
    candidates.push(candidate);
    for server in [
        json!({"command":"/unreviewed/server", "allowedTools":["lookup"]}),
        json!({"transport":"streamable_http", "url":"https://unreviewed.invalid/mcp", "allowedTools":["lookup"]}),
        json!({"transport":"streamable_http", "url":"https://mcp.invalid/mcp", "allowedTools":["lookup"], "oauth":{"clientId":"fixture", "callbackPort":8787}}),
    ] {
        let mut candidate = base.clone();
        candidate.sandbox.network_destinations = vec!["https://mcp.invalid".into()];
        candidate
            .mcp
            .servers
            .insert("fixture".into(), serde_json::from_value(server).unwrap());
        candidates.push(candidate);
    }
    for candidate in candidates {
        let error = prepare_config(&candidate, &options).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("configuration error: controlled chat:")
                || error.to_string().contains("controlled chat:")
        );
    }
    assert!(prepare_config(&base, &options).is_ok());
    assert_eq!(
        base.access.profile,
        AccessProfile::AllowAll,
        "ordinary defaults remain unchanged"
    );
    assert!(base.plugins.enabled);
}

#[test]
fn controlled_chat_mcp_selectors_are_validated_by_the_core_on_runtime_open() {
    let workspace = crate::test_support::private_tempdir();
    for selectors in [
        vec![],
        vec!["*".into(), "read".into()],
        vec!["get_*".into(), "get_*".into()],
        vec!["get_**".into()],
        vec!["^get_.*$".into()],
        vec!["get_?".into()],
        vec!["get_[ab]".into()],
        vec!["a".repeat(129)],
        (0..1025).map(|index| format!("get_{index}")).collect(),
    ] {
        let mut config = config(workspace.path());
        config.sandbox.network_destinations = vec!["https://mcp.invalid".into()];
        config.mcp.servers.insert(
            "fixture".into(),
            serde_json::from_value(json!({
                "transport":"streamable_http", "url":"https://mcp.invalid/mcp",
                "allowedTools":selectors
            }))
            .unwrap(),
        );
        assert!(
            Runtime::open_with_options(
                &config,
                Arc::new(DenyApproval),
                None,
                RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
            )
            .is_err(),
            "invalid selectors reached a controlled runtime: {selectors:?}"
        );
    }
}

#[tokio::test]
async fn controlled_chat_mcp_selectors_filter_discovery_without_widening_authority() {
    for selectors in [vec!["get_*", "*_search", "echo"], vec!["*"]] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut lists = 0;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let result = match request["method"].as_str().unwrap() {
                    "server/discover" => json!({
                        "resultType":"complete", "supportedVersions":["2026-07-28"],
                        "capabilities":{"tools":{}}, "ttlMs":0, "cacheScope":"private"
                    }),
                    "tools/list" => {
                        let mut names = vec![
                            "get_user",
                            "get_",
                            "web_search",
                            "echo",
                            "delete_user",
                            "Get_user",
                            "forget_user",
                        ];
                        if lists > 0 {
                            names.push("get_future_tool");
                        }
                        lists += 1;
                        json!({"tools": names.into_iter().map(|name| json!({
                            "name":name, "inputSchema":{"type":"object"}
                        })).collect::<Vec<_>>()})
                    }
                    "tools/call" => {
                        assert_eq!(request["params"]["name"], "get_future_tool");
                        json!({"content":[{"type":"text","text":"reviewed future tool"}]})
                    }
                    method => panic!("unexpected MCP method {method}"),
                };
                let body =
                    json!({"jsonrpc":"2.0", "id":request["id"], "result":result}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let workspace = crate::test_support::private_tempdir();
        let mut config =
            RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
        config.use_ephemeral_storage();
        config.sandbox.network_destinations = vec![origin.clone()];
        config.mcp.servers.insert(
            "fixture".into(),
            serde_json::from_value(json!({
                "transport":"streamable_http", "url":format!("{origin}/mcp"),
                "protocolVersion":"2026-07-28", "allowedTools":selectors
            }))
            .unwrap(),
        );
        let runtime = Runtime::open_with_options(
            &config,
            Arc::new(colossus_policy::AllowApproval {
                approved_by: "test".into(),
            }),
            None,
            RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            runtime.access.action_decision("mcp.call"),
            Some(AccessDecision::RequireApproval)
        );
        let discovered = runtime.mcp_tools(Some("fixture")).await.unwrap();
        let names = discovered
            .into_iter()
            .map(|tool| tool.name)
            .collect::<BTreeSet<_>>();
        assert!(!names.contains("get_future_tool"));
        for expected in ["get_user", "get_", "web_search", "echo"] {
            assert!(names.contains(expected));
        }
        for nonmatching in ["delete_user", "Get_user", "forget_user"] {
            assert_eq!(names.contains(nonmatching), selectors == ["*"]);
        }
        assert!(
            runtime
                .mcp_tools(Some("fixture"))
                .await
                .unwrap()
                .iter()
                .any(|tool| tool.name == "get_future_tool")
        );
        let result = runtime
            .mcp_call("fixture", "get_future_tool", json!({}))
            .await
            .unwrap();
        assert!(
            serde_json::to_string(&result)
                .unwrap()
                .contains("reviewed future tool")
        );
        assert!(
            runtime
                .mcp_call("fixture", "never_published", json!({}))
                .await
                .is_err()
        );
        assert!(runtime.http_get(&format!("{origin}/mcp")).await.is_err());
        assert!(runtime.read_text_file("private-host-state").await.is_err());
        assert_eq!(
            runtime.access.action_decision("network.http"),
            Some(AccessDecision::Deny)
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn controlled_chat_keeps_narrower_denials_and_mcp_approval_requirements() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = config(workspace.path());
    config.access.actions.deny = vec!["provider.echo".into(), "goal.update".into()];
    config.access.actions.require_approval = vec!["mcp.call".into()];
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
    )
    .unwrap();
    assert!(runtime.echo("denied").await.is_err());
    assert_eq!(
        runtime.access.action_decision("goal.update"),
        Some(AccessDecision::Deny)
    );
    assert_eq!(
        runtime.access.action_decision("mcp.call"),
        Some(AccessDecision::RequireApproval)
    );
}

#[derive(Default)]
struct HostVault(AtomicUsize);

impl CredentialVault for HostVault {
    fn read(&self, _: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    fn write(&self, _: &CredentialKey, _: &VaultRecord) -> Result<(), CredentialError> {
        unreachable!("not authorizing in this test")
    }
    fn delete(&self, _: &CredentialKey) -> Result<(), CredentialError> {
        Ok(())
    }
}

#[tokio::test]
async fn controlled_chat_uses_injected_oauth_vault_across_catalog_snapshots() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = config(workspace.path());
    config.sandbox.network_destinations = vec!["https://mcp.invalid".into()];
    config.mcp.oauth_credential_store = McpOAuthCredentialStoreKind::Platform;
    config.mcp.servers.insert("fixture".into(), serde_json::from_value(json!({"transport":"streamable_http", "url":"https://mcp.invalid/mcp", "allowedTools":["lookup"], "oauth":{"clientId":"fixture", "callbackPort":8787}})).unwrap());
    let vault = Arc::new(HostVault::default());
    let options = RuntimeOpenOptions::for_controlled_chat(workspace.path())
        .unwrap()
        .with_oauth_credential_vault(vault.clone());
    assert!(format!("{options:?}").contains("SuppliedOAuthVault([REDACTED])"));
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
    for _ in 0..2 {
        assert!(
            !runtime
                .mcp_oauth_status("fixture")
                .await
                .unwrap()
                .authenticated
        );
    }
    assert_eq!(
        vault.0.load(Ordering::SeqCst),
        2,
        "fresh catalogs retain supplied custody"
    );
    assert!(!workspace.path().join("state.mcp-oauth.redb").exists());
}

#[tokio::test]
async fn controlled_chat_remote_mcp_still_discovers_and_calls_only_reviewed_tools() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut methods = Vec::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_owned());
            let result = match method {
                "server/discover" => {
                    json!({"resultType":"complete", "supportedVersions":["2026-07-28"], "capabilities":{"tools":{}}, "ttlMs":0, "cacheScope":"private"})
                }
                "tools/list" => {
                    json!({"tools":[{"name":"lookup", "inputSchema":{"type":"object", "properties":{}}}, {"name":"unreviewed", "inputSchema":{"type":"object"}}]})
                }
                "tools/call" => {
                    assert_eq!(request["params"]["name"], "lookup");
                    json!({"content":[{"type":"text", "text":"reviewed remote result"}]})
                }
                other => panic!("unexpected MCP method {other}"),
            };
            let body = json!({"jsonrpc":"2.0", "id":request["id"], "result":result}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            if method == "tools/call" {
                return methods;
            }
        }
    });
    let workspace = crate::test_support::private_tempdir();
    let mut config = config(workspace.path());
    config.sandbox.network_destinations = vec![origin.clone()];
    config.mcp.servers.insert("fixture".into(), serde_json::from_value(json!({"transport":"streamable_http", "url":format!("{origin}/mcp"), "protocolVersion":"2026-07-28", "allowedTools":["lookup"]})).unwrap());
    let runtime = Runtime::open_with_options(
        &config,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_controlled_chat(workspace.path()).unwrap(),
    )
    .unwrap();
    let result = runtime
        .mcp_call("fixture", "lookup", json!({}))
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&result)
            .unwrap()
            .contains("reviewed remote result")
    );
    assert!(
        runtime
            .mcp_call("fixture", "unreviewed", json!({}))
            .await
            .is_err()
    );
    let methods = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        methods,
        [
            "server/discover",
            "tools/list",
            "server/discover",
            "tools/call"
        ]
    );
    assert!(
        runtime.http_get(&format!("{origin}/mcp")).await.is_err(),
        "MCP origin grants do not activate generic fetch"
    );
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 2048];
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0 && bytes.len() + read <= 64 * 1024);
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(index) = bytes.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let length = std::str::from_utf8(&bytes[..header_end])
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap();
    assert!(length <= 64 * 1024);
    while bytes.len() < header_end + length {
        let mut buffer = [0_u8; 2048];
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
    }
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}
