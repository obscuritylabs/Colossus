//! Trusted composition for chat backed only by controlled runtime capabilities.

use super::*;

impl RuntimeConfig {
    /// Build a deny-by-default baseline for a controlled chat host.
    ///
    /// Add reviewed provider/model, remote MCP, search and exact-origin declarations
    /// before opening with [`RuntimeOpenOptions::for_controlled_chat`]. The host must
    /// separately select durable storage protection and user-bound credential custody.
    pub fn controlled_chat_template(state_path: impl Into<PathBuf>) -> Self {
        let mut config = Self::offline_template(state_path);
        config.browser.enabled = false;
        config.computer.enabled = false;
        config.plugins.enabled = false;
        config.plugins.workspace_discovery = false;
        config.access.profile = AccessProfile::Pinned;
        config.access.tools.include = builtin_specs()
            .into_iter()
            .filter(|spec| allows_tool(&spec.name))
            // Pattern selection keeps optional capabilities dormant when no prompt,
            // MCP server or search route exists. The host-mode catalog remains closed.
            .map(|spec| match spec.name.as_str() {
                "user.ask" | "web.search" => format!("{}*", spec.name),
                name if name.starts_with("mcp.") => format!("{name}*"),
                _ => spec.name,
            })
            .collect();
        config.access.actions.require_approval = ["mcp.call", "plan.approve_request"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        config.access.actions.allow = permitted_actions()
            .into_iter()
            .filter(|action| !config.access.actions.require_approval.contains(action))
            .collect();
        config
    }
}

/// Reject resource authority that belongs to a provisioned execution environment.
/// The returned configuration only narrows activation; action policy is constrained
/// independently below, including calls made outside the model's tool catalog.
pub(super) fn prepare_config(
    config: &RuntimeConfig,
    options: &RuntimeOpenOptions,
) -> Result<RuntimeConfig, RuntimeError> {
    let reject = |reason: &str| RuntimeError::Config(format!("controlled chat: {reason}"));
    if !matches!(
        config.sandbox.backend.as_str(),
        "native" | "windows_job" | "oci"
    ) || config.sandbox.profile != "offline-default"
        || !config.sandbox.filesystem.is_empty()
        || !config.sandbox.executables.is_empty()
    {
        return Err(reject(
            "execution environments and filesystem/process grants are unavailable",
        ));
    }
    if !matches!(config.policy, PolicyConfig::BuiltIn { .. }) {
        return Err(reject("the built-in constrained policy is required"));
    }
    if matches!(config.storage.keys, KeyConfig::Platform { .. }) {
        return Err(reject("journal keys must use explicit headless custody"));
    }
    if config.browser.enabled
        || config.computer.enabled
        || options.browser_host.is_some()
        || options.computer_host.is_some()
    {
        return Err(reject("native browser and computer hosts are unavailable"));
    }
    let origins = config
        .sandbox
        .network_destinations
        .iter()
        .map(|destination| {
            if destination.contains('*') {
                return Err(reject("network destinations must be exact origins"));
            }
            let origin = canonical_network_origin(destination)
                .map_err(|_| reject("network destinations must be exact origins"))?;
            if origin != destination.trim_end_matches('/') {
                return Err(reject("network destinations must be exact origins"));
            }
            Ok(origin)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    for server in config.mcp.servers.values() {
        if server.transport != colossus_mcp::McpTransportKind::StreamableHttp
            || !server.command.as_os_str().is_empty()
            || !server.args.is_empty()
            || server.working_directory.is_some()
            || !server.environment.is_empty()
            || !server.literal_environment.is_empty()
            || (server.oauth.is_some()
                && (options.oauth_credential_vault.is_none()
                    || config.mcp.oauth_credential_store != McpOAuthCredentialStoreKind::Platform))
            || server.allowed_tools.is_empty()
            || server.allowed_tools.iter().any(|tool| tool.contains('*'))
        {
            return Err(reject(
                "MCP requires remote HTTP, exact tool names, and host-bound credentials",
            ));
        }
        let origin = server
            .url
            .as_deref()
            .and_then(|url| canonical_network_origin(url).ok())
            .ok_or_else(|| reject("MCP requires an explicitly reviewed endpoint"))?;
        if !origins.contains(&origin) {
            return Err(reject("MCP endpoint origin is not explicitly permitted"));
        }
    }
    if config
        .plugins
        .mcp_servers
        .values()
        .any(|server| server.enabled)
    {
        return Err(reject("plugin-provided MCP servers are unavailable"));
    }
    let mut constrained = config.clone();
    constrained.plugins.enabled = false;
    constrained.plugins.workspace_discovery = false;
    Ok(constrained)
}

/// Exact names make future tools unavailable until their authority is reviewed.
pub(super) fn allows_tool(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "user.ask"
            | "tool.search"
            | "context.show"
            | "context.compact"
            | "context.snapshots"
            | "context.restore"
            | "trace.show"
            | "session.set_title"
            | "task.create"
            | "task.update"
            | "task.list"
            | "decision.create"
            | "decision.update"
            | "decision.list"
            | "decision.archive"
            | "decision.supersede"
            | "plan.create"
            | "plan.update"
            | "plan.show"
            | "plan.approve_request"
            | "goal.show"
            | "goal.update"
            | "agent.delegate"
            | "agent.participants"
            | "agent.send_message"
            | "agent.inbox"
            | "agent.await_message"
            | "agent.result"
            | "agent.list"
            | "memory.create"
            | "memory.update"
            | "memory.list"
            | "memory.search"
            | "memory.archive"
            | "memory.supersede"
            | "mcp.servers"
            | "mcp.search"
            | "mcp.tools"
            | "mcp.call"
            | "web.search"
            | "workflow.definition.list"
            | "workflow.definition.get"
            | "workflow.schedule.list"
            | "workflow.schedule.get"
    )
}

/// The policy ceiling is separate from catalog visibility and application grants.
pub(super) fn constrain_access(access: &mut AccessResolution) {
    let permitted = permitted_actions();
    for action in &mut access.actions {
        if !permitted.contains(&action.name) {
            action.decision = AccessDecision::Deny;
            action.reason = "unavailable without a controlled chat execution environment".into();
        }
    }
}

fn permitted_actions() -> BTreeSet<String> {
    let mut permitted = builtin_specs()
        .into_iter()
        .filter(|spec| allows_tool(&spec.name))
        .flat_map(|spec| spec.effect_action.into_iter().chain(spec.capability))
        .collect::<BTreeSet<_>>();
    permitted.extend(
        [
            "provider.echo",
            "provider.openai.responses",
            "provider.openai.codex",
            "provider.openai.chat",
            "provider.anthropic.messages",
            "provider.models",
            "provider.call",
            "agent.run",
            "research.run",
            "plan.execute",
            "plan.discard",
            "goal.create",
            "goal.iteration.record",
            "subagent.start",
            "subagent.complete",
            "subagent.fail",
            "subagent.cancel",
            "subagent.interrupt",
            "subagent.requeue",
            "memory.read",
            "memory.index.status",
            "memory.index.sync",
            "memory.index.rebuild",
            "embedding.openai.create",
            "memory.index.chroma.search",
            "memory.index.chroma.status",
            "memory.index.chroma.upsert",
            "memory.index.chroma.remove",
            "memory.index.chroma.reset",
            "presentation.preferences.update",
            "presentation.history.append",
            "workflow.definition.register",
            "workflow.run.read",
            "workflow.run.start",
            "workflow.execute",
            "workflow.start",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    permitted
}

#[cfg(test)]
mod tests;
