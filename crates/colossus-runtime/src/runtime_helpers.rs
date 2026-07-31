use super::*;

pub(super) fn workspace_absolute_path(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    }
}

pub(super) fn recover_unknown_effects(journal: &dyn EventJournal) -> Result<u64, StoreError> {
    let mut last_by_stream = std::collections::BTreeMap::new();
    for event in journal.read_global(1, usize::MAX)? {
        if event.stream_id.starts_with("effect:") {
            last_by_stream.insert(event.stream_id.clone(), event);
        }
    }
    let mut recovered = 0_u64;
    for event in last_by_stream.into_values() {
        if event.event_type != "effect.started.v1" {
            continue;
        }
        journal.append(NewEvent {
            event_version: 1,
            stream_id: event.stream_id,
            expected_stream_version: event.stream_version,
            classification: EventClassification::Effect,
            event_type: "effect.outcome_unknown.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "startup-recovery".into(),
            },
            context: event.context,
            payload: json!({
                "reason": "process stopped after effect.started without a terminal event",
                "recovered_from_event_id": event.event_id,
                "automatic_retry": false,
            }),
        })?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

pub(super) fn recover_interrupted_subagents(
    repository: &dyn WorkRepository,
    service: &WorkService,
) -> Result<u64, StoreError> {
    let running = repository.list_subagents(None, Some(SubagentStatus::Running), 1_000)?;
    for job in &running {
        service.stop_subagent(
            &job.id,
            SubagentStatus::Interrupted,
            "Subagent process exited before the job completed.",
            system_actor("subagent-recovery"),
        )?;
    }
    u64::try_from(running.len()).map_err(|error| StoreError::Adapter(error.to_string()))
}

pub(super) fn sha2_compat(secret: &[u8; 32], label: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    // The journal signing secret is already random. This local KDF only domain-separates
    // the permit MAC key without persisting another environment credential.
    Sha256::new()
        .chain_update(label)
        .chain_update(secret)
        .finalize()
        .into()
}

pub(super) fn repository_identity(workspace: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "repo-{}",
        hex::encode(Sha256::digest(workspace.to_string_lossy().as_bytes()))
    )
}

pub(super) async fn discover_mcp_tools(
    gateway: &EffectGateway,
    executor: &McpExecutor,
    effect_executor: &dyn EffectExecutor,
    actor: Actor,
    context: ExecutionContext,
    selected_server: Option<&str>,
) -> Result<Vec<McpToolSummary>, RuntimeError> {
    let servers =
        selected_server.map_or_else(|| executor.server_names(), |server| vec![server.to_owned()]);
    let mut tools = Vec::new();
    for server in servers {
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        let mut server_names = BTreeSet::new();
        let mut completed = false;
        for _ in 0..MAX_MCP_PAGES {
            let request = executor.request(
                actor.clone(),
                context.clone(),
                McpOperation::ListTools {
                    server: server.clone(),
                    cursor: cursor.clone(),
                },
            )?;
            let released = gateway.execute(request, effect_executor).await?;
            let page: McpToolsPage = serde_json::from_slice(&released.bytes).map_err(|error| {
                RuntimeError::Config(format!("invalid MCP tools page: {error}"))
            })?;
            if page.server != server {
                return Err(RuntimeError::Config(
                    "released MCP tools page names another server".into(),
                ));
            }
            for tool in page.tools {
                if !server_names.insert(tool.name.clone()) {
                    return Err(RuntimeError::Config(format!(
                        "MCP server {server} returned duplicate tool {} across pages",
                        tool.name
                    )));
                }
                if server_names.len() > MAX_MCP_TOOLS {
                    return Err(RuntimeError::Config(format!(
                        "MCP server {server} exceeded {MAX_MCP_TOOLS} discovered tools"
                    )));
                }
                tools.push(tool);
                if tools.len() > MAX_MCP_TOOLS.saturating_mul(executor.server_names().len().max(1))
                {
                    return Err(RuntimeError::Config(
                        "MCP discovery exceeded its aggregate tool bound".into(),
                    ));
                }
            }
            let Some(next) = page.next_cursor else {
                completed = true;
                break;
            };
            if next.is_empty() || !cursors.insert(next.clone()) {
                return Err(RuntimeError::Config(format!(
                    "MCP server {server} returned an empty or cyclic pagination cursor"
                )));
            }
            cursor = Some(next);
        }
        if !completed {
            return Err(RuntimeError::Config(format!(
                "MCP server {server} exceeded {MAX_MCP_PAGES} discovery pages"
            )));
        }
    }
    tools.sort_by(|left, right| {
        left.server
            .cmp(&right.server)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(tools)
}

// Keep the typed request builder and the separately identity-bound effect adapter
// explicit: combining them would make it easier to accidentally dispatch the raw MCP
// executor without the permit-time workspace revalidation.
#[allow(clippy::too_many_arguments)]
pub(super) async fn invoke_mcp_tool(
    gateway: &EffectGateway,
    executor: &McpExecutor,
    effect_executor: &dyn EffectExecutor,
    actor: Actor,
    context: ExecutionContext,
    server: &str,
    tool: &str,
    arguments: Value,
) -> Result<McpCallOutput, RuntimeError> {
    let discovered = discover_mcp_tools(
        gateway,
        executor,
        effect_executor,
        actor.clone(),
        context.clone(),
        Some(server),
    )
    .await?;
    let tool_spec = discovered
        .iter()
        .find(|candidate| candidate.name == tool)
        .ok_or_else(|| McpError::ToolDenied(format!("{server}:{tool}")))?;
    validate_tool_arguments(tool_spec, &arguments)?;
    let request = executor.request(
        actor,
        context,
        McpOperation::CallTool {
            server: server.into(),
            tool: tool.into(),
            arguments,
            input_schema: tool_spec.input_schema.clone(),
        },
    )?;
    let released = gateway.execute(request, effect_executor).await?;
    let output: McpCallOutput = serde_json::from_slice(&released.bytes)
        .map_err(|error| RuntimeError::Config(format!("invalid MCP call output: {error}")))?;
    if output.server != server || output.tool != tool {
        return Err(RuntimeError::Config(
            "released MCP result does not match its requested server and tool".into(),
        ));
    }
    Ok(output)
}
