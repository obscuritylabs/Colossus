use super::*;
use colossus_contracts::EventEnvelope;
use colossus_ports::{MAX_STREAM_LIST_BATCH, MAX_STREAM_READ_BATCH};

pub(super) fn workspace_absolute_path(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    }
}

const MAX_PENDING_EFFECT_RECOVERIES: usize = 1_024;

struct PendingEffectRecovery {
    started: EventEnvelope,
    latest_stream_version: u64,
}

pub(super) fn recover_unknown_effects(journal: &dyn EventJournal) -> Result<u64, StoreError> {
    let pending = pending_effect_recoveries(journal)?;
    let mut recovered = 0_u64;
    for effect in pending {
        let started = effect.started;
        journal.append(NewEvent {
            event_version: 1,
            stream_id: started.stream_id,
            expected_stream_version: effect.latest_stream_version,
            classification: EventClassification::Effect,
            event_type: "effect.outcome_unknown.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "startup-recovery".into(),
            },
            context: started.context,
            payload: json!({
                "reason": "process stopped after effect.started without a terminal event",
                "recovered_from_event_id": started.event_id,
                "automatic_retry": false,
            }),
        })?;
        recovered = recovered.saturating_add(1);
    }
    Ok(recovered)
}

fn pending_effect_recoveries(
    journal: &dyn EventJournal,
) -> Result<Vec<PendingEffectRecovery>, StoreError> {
    let mut pending = Vec::new();
    let mut after = None::<String>;
    loop {
        let page = journal.list_stream_ids("effect:", after.as_deref(), MAX_STREAM_LIST_BATCH)?;
        if page.len() > MAX_STREAM_LIST_BATCH {
            return Err(StoreError::Verification(
                "effect stream discovery exceeded its page bound".into(),
            ));
        }
        if page.is_empty() {
            break;
        }
        let mut previous = after.as_deref();
        for stream_id in &page {
            if !stream_id.starts_with("effect:")
                || previous.is_some_and(|previous| stream_id.as_str() <= previous)
            {
                return Err(StoreError::Verification(
                    "effect stream discovery returned an invalid ordered page".into(),
                ));
            }
            previous = Some(stream_id);
            if let Some(effect) = pending_effect_recovery(journal, stream_id)? {
                pending.push(effect);
                if pending.len() > MAX_PENDING_EFFECT_RECOVERIES {
                    return Err(StoreError::Adapter(format!(
                        "startup effect recovery exceeds the safe bound of {MAX_PENDING_EFFECT_RECOVERIES}"
                    )));
                }
            }
        }
        after = page.last().cloned();
    }
    Ok(pending)
}

fn pending_effect_recovery(
    journal: &dyn EventJournal,
    stream_id: &str,
) -> Result<Option<PendingEffectRecovery>, StoreError> {
    let mut before_version = None;
    let mut latest = None;
    loop {
        let page =
            journal.read_stream_backwards(stream_id, before_version, MAX_STREAM_READ_BATCH)?;
        if page.is_empty() {
            return Ok(None);
        }
        let latest_event = latest.get_or_insert_with(|| page[0].clone());
        for event in &page {
            match event.event_type.as_str() {
                "effect.started.v1" => {
                    journal.decrypt_payload(event)?;
                    if event.event_id != latest_event.event_id {
                        journal.decrypt_payload(latest_event)?;
                    }
                    return Ok(Some(PendingEffectRecovery {
                        started: event.clone(),
                        latest_stream_version: latest_event.stream_version,
                    }));
                }
                "effect.completed.v1" | "effect.failed.v1" | "effect.outcome_unknown.v1" => {
                    return Ok(None);
                }
                _ => {}
            }
        }
        before_version = page.last().map(|event| event.stream_version);
    }
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
    let mut serialized_output_bytes = 2_usize;
    for server in servers {
        visit_mcp_server_tools(
            gateway,
            executor,
            effect_executor,
            &actor,
            &context,
            &server,
            |tool| {
                push_bounded_mcp_discovery_tool(&mut tools, &mut serialized_output_bytes, tool)?;
                if tools.len() > MAX_MCP_TOOLS.saturating_mul(executor.server_names().len().max(1))
                {
                    return Err(RuntimeError::Config(
                        "MCP discovery exceeded its aggregate tool bound".into(),
                    ));
                }
                Ok(())
            },
        )
        .await?;
    }
    tools.sort_by(|left, right| {
        left.server
            .cmp(&right.server)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(tools)
}

async fn visit_mcp_server_tools(
    gateway: &EffectGateway,
    executor: &McpExecutor,
    effect_executor: &dyn EffectExecutor,
    actor: &Actor,
    context: &ExecutionContext,
    server: &str,
    mut visit: impl FnMut(McpToolSummary) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    let mut server_names = BTreeSet::new();
    let mut completed = false;
    for _ in 0..MAX_MCP_PAGES {
        let request = executor.request(
            actor.clone(),
            context.clone(),
            McpOperation::ListTools {
                server: server.to_owned(),
                cursor: cursor.clone(),
            },
        )?;
        let released = gateway.execute(request, effect_executor).await?;
        let page: McpToolsPage = serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(format!("invalid MCP tools page: {error}")))?;
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
            visit(tool)?;
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
    Ok(())
}

pub(super) fn push_bounded_mcp_discovery_tool(
    tools: &mut Vec<McpToolSummary>,
    serialized_output_bytes: &mut usize,
    tool: McpToolSummary,
) -> Result<(), RuntimeError> {
    let tool_bytes = serde_json::to_vec(&tool)
        .map_err(|error| RuntimeError::Config(format!("invalid MCP tool summary: {error}")))?
        .len();
    let delimiter_bytes = usize::from(!tools.is_empty());
    let next_output_bytes = serialized_output_bytes
        .checked_add(delimiter_bytes)
        .and_then(|bytes| bytes.checked_add(tool_bytes))
        .ok_or_else(mcp_discovery_output_limit_error)?;
    if next_output_bytes > MCP_TOOLS_MAX_OUTPUT_BYTES {
        return Err(mcp_discovery_output_limit_error());
    }
    *serialized_output_bytes = next_output_bytes;
    tools.push(tool);
    Ok(())
}

pub(super) fn mcp_discovery_output_limit_error() -> RuntimeError {
    RuntimeError::Config(format!(
        "MCP discovery exceeded the mcp.tools {MCP_TOOLS_MAX_OUTPUT_BYTES}-byte output bound"
    ))
}

const MAX_RELATED_MCP_TOOLS: usize = 8;

pub(super) struct McpToolLookup {
    server: String,
    requested_tool: String,
    exact: Option<McpToolSummary>,
    related: Vec<String>,
}

impl McpToolLookup {
    pub(super) fn new(server: &str, requested_tool: &str) -> Self {
        Self {
            server: server.to_owned(),
            requested_tool: requested_tool.to_owned(),
            exact: None,
            related: Vec::new(),
        }
    }

    pub(super) fn visit(&mut self, tool: McpToolSummary) {
        if tool.name == self.requested_tool {
            self.exact = Some(tool);
        } else if self.related.len() < MAX_RELATED_MCP_TOOLS
            && (tool.name.contains(&self.requested_tool)
                || self.requested_tool.contains(&tool.name))
        {
            self.related.push(tool.name);
        }
    }

    pub(super) fn finish(self) -> Result<McpToolSummary, RuntimeError> {
        self.exact.ok_or_else(|| {
            unadvertised_mcp_tool_error(&self.server, &self.requested_tool, &self.related).into()
        })
    }
}

async fn discover_exact_mcp_tool(
    gateway: &EffectGateway,
    executor: &McpExecutor,
    effect_executor: &dyn EffectExecutor,
    actor: &Actor,
    context: &ExecutionContext,
    server: &str,
    tool: &str,
) -> Result<McpToolSummary, RuntimeError> {
    // Calls retain only the exact schema and bounded name guidance. The mcp.tools
    // presentation ceiling must not make an otherwise bounded catalog uncallable.
    let mut lookup = McpToolLookup::new(server, tool);
    visit_mcp_server_tools(
        gateway,
        executor,
        effect_executor,
        actor,
        context,
        server,
        |candidate| {
            lookup.visit(candidate);
            Ok(())
        },
    )
    .await?;
    lookup.finish()
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
    if !executor.allows_tool(server, tool)? {
        return Err(McpError::ToolDenied(format!("{server}:{tool}")).into());
    }
    let tool_spec = discover_exact_mcp_tool(
        gateway,
        executor,
        effect_executor,
        &actor,
        &context,
        server,
        tool,
    )
    .await?;
    validate_tool_arguments(&tool_spec, &arguments)?;
    let request = executor.request(
        actor,
        context,
        McpOperation::CallTool {
            server: server.into(),
            tool: tool.into(),
            description: tool_spec.description.clone(),
            annotations: tool_spec.annotations.clone(),
            arguments,
            input_schema: Box::new(tool_spec.input_schema.clone()),
            schema_sha256: tool_spec.schema_sha256.clone(),
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

pub(super) fn unadvertised_mcp_tool_error(
    server: &str,
    tool: &str,
    related: &[String],
) -> McpError {
    let guidance = if related.is_empty() {
        "call mcp.tools for the exact available names".into()
    } else {
        format!("related available tools: {}", related.join(", "))
    };
    McpError::InvalidArguments(format!(
        "server {server} did not advertise tool {tool}; {guidance}"
    ))
}
