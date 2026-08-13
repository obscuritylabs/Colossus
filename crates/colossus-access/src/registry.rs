use crate::{
    AccessError, ActionClass, ActionDescriptor, CapabilitySource, ToolDescriptor, ToolPrerequisite,
};

/// Return the single first-party descriptor for one built-in tool.
///
/// Unknown names fail closed so adding a built-in tool also requires an explicit
/// source, family, and prerequisite classification.
pub fn builtin_tool_descriptor(name: &str) -> Result<ToolDescriptor, AccessError> {
    let read = vec![ToolPrerequisite::FilesystemRead];
    let write = vec![ToolPrerequisite::FilesystemWrite];
    let (family, prerequisites) = match name {
        "echo" | "user.ask" | "tool.search" | "trace.show" => (
            "utility",
            if name == "user.ask" {
                vec![ToolPrerequisite::Interactive]
            } else {
                Vec::new()
            },
        ),
        "filesystem.list" | "filesystem.read" | "filesystem.search" => ("filesystem", read),
        "filesystem.write" | "filesystem.replace" => ("filesystem", write),
        "git.status" | "git.diff" | "git.show" => (
            "git",
            vec![
                ToolPrerequisite::FilesystemRead,
                ToolPrerequisite::GitExecutable,
            ],
        ),
        "shell.run" => ("process", vec![ToolPrerequisite::AnyExecutable]),
        "repo.map" | "repo.symbol_search" | "repo.references" | "repo.file_summary" => {
            ("repository", vec![ToolPrerequisite::FilesystemRead])
        }
        "patch.preview" => ("patch", vec![ToolPrerequisite::FilesystemRead]),
        "patch.apply" | "patch.reverse" => ("patch", vec![ToolPrerequisite::FilesystemWrite]),
        "trace.export" => ("trace", vec![ToolPrerequisite::FilesystemWrite]),
        "task.create" | "task.update" | "task.list" => simple_tool("tasks"),
        "decision.create" | "decision.update" | "decision.list" | "decision.archive"
        | "decision.supersede" => simple_tool("decisions"),
        "plan.create" | "plan.update" | "plan.show" | "plan.approve_request" => {
            simple_tool("plans")
        }
        "goal.show" | "goal.update" => simple_tool("goals"),
        "agent.delegate" | "agent.result" | "agent.list" => simple_tool("subagents"),
        "memory.create" | "memory.update" | "memory.list" | "memory.search" | "memory.archive"
        | "memory.supersede" => simple_tool("memory"),
        "context.show" | "context.compact" | "context.snapshots" | "context.restore" => {
            simple_tool("context")
        }
        "skill.scaffold"
        | "skill.inspect"
        | "skill.read"
        | "skill.write"
        | "skill.validate"
        | "skill.install"
        | "skill.resource.list"
        | "skill.resource.read" => simple_tool("skills"),
        "web.search" => ("web", vec![ToolPrerequisite::AgentSearchRoute]),
        "web.fetch" | "docs.fetch" | "network.http" => (
            "web",
            vec![
                ToolPrerequisite::NetworkDestination,
                ToolPrerequisite::ModelNetworkTools,
            ],
        ),
        "mcp.servers" | "mcp.tools" | "mcp.call" => ("mcp", vec![ToolPrerequisite::McpConfigured]),
        _ => return Err(AccessError::Unclassified(format!("tool {name}"))),
    };
    let source = if family == "mcp" {
        CapabilitySource::Mcp
    } else {
        CapabilitySource::Core
    };
    Ok(ToolDescriptor::new(name, family, source, prerequisites))
}

fn simple_tool(family: &'static str) -> (&'static str, Vec<ToolPrerequisite>) {
    (family, Vec::new())
}

/// Complete first-party action and Safety Kernel capability metadata.
pub fn builtin_action_descriptors() -> Vec<ActionDescriptor> {
    let mut descriptors = Vec::new();
    push_actions(
        &mut descriptors,
        ActionClass::Provider,
        &[
            "provider.echo",
            "provider.openai.responses",
            "provider.openai.codex",
            "provider.openai.chat",
            "provider.models",
            "provider.call",
        ],
    );
    push_actions(
        &mut descriptors,
        ActionClass::Read,
        &[
            "filesystem.read",
            "filesystem.list",
            "filesystem.metadata",
            "filesystem.search",
            "git.status",
            "git.diff",
            "git.show",
            "repo.map",
            "repo.symbol_search",
            "repo.references",
            "repo.file_summary",
            "context.show",
            "context.snapshots",
            "patch.preview",
            "task.list",
            "decision.list",
            "plan.show",
            "goal.show",
            "subagent.read",
            "subagent.list",
            "memory.read",
            "memory.list",
            "memory.search",
            "memory.index.status",
            "skill.inspect",
            "skill.read",
            "skill.validate",
            "skill.resource.list",
            "skill.resource.read",
            "pack.verify",
            "bundle.verify",
            "bundle.key.inspect",
            "collection.verify",
        ],
    );
    push_actions(
        &mut descriptors,
        ActionClass::LocalState,
        &[
            "context.compact",
            "context.restore",
            "presentation.preferences.update",
            "presentation.history.append",
            "task.create",
            "task.update",
            "decision.create",
            "decision.update",
            "decision.archive",
            "decision.supersede",
            "plan.create",
            "plan.update",
            "plan.discard",
            "goal.create",
            "goal.update",
            "goal.iteration.record",
            "subagent.create",
            "subagent.start",
            "subagent.complete",
            "subagent.fail",
            "subagent.cancel",
            "subagent.interrupt",
            "subagent.requeue",
            "memory.create",
            "memory.update",
            "memory.archive",
            "memory.supersede",
            "memory.index.sync",
            "memory.index.rebuild",
            "workflow.webhook.ingest",
            "workflow.subscription.dispatch",
        ],
    );
    push_actions(
        &mut descriptors,
        ActionClass::WorkspaceMutation,
        &[
            "filesystem.write",
            "patch.apply",
            "patch.reverse",
            "trace.export",
            "skill.scaffold",
            "skill.write",
            "skill.install",
            "audit.export.write",
        ],
    );
    push_actions(
        &mut descriptors,
        ActionClass::Execution,
        &[
            "process.spawn",
            "shell.run",
            "workflow.execute",
            "workflow.start",
            "agent.run",
            "plan.execute",
        ],
    );
    push_actions(
        &mut descriptors,
        ActionClass::ExternalNetwork,
        &[
            "network.http",
            "web.search",
            "embedding.openai.create",
            "memory.index.chroma.search",
            "memory.index.chroma.status",
            "memory.index.chroma.upsert",
            "memory.index.chroma.remove",
            "memory.index.chroma.reset",
            "research.run",
            "integration.openapi.import",
            "integration.connect",
            "integration.disconnect",
            "integration.invoke",
        ],
    );
    push_sourced_actions(
        &mut descriptors,
        ActionClass::Read,
        CapabilitySource::Mcp,
        &["mcp.tools"],
    );
    push_sourced_actions(
        &mut descriptors,
        ActionClass::ExternalNetwork,
        CapabilitySource::Mcp,
        &["mcp.invoke", "mcp.call"],
    );
    push_actions(
        &mut descriptors,
        ActionClass::Administration,
        &[
            "plan.approve_request",
            "audit.export.worm.write",
            "pack.install",
            "pack.enable",
            "pack.disable",
            "pack.uninstall",
            "pack.trust.add",
            "bundle.build",
            "bundle.install",
            "collection.build",
            "collection.install",
            "registry.pull",
            "registry.push",
        ],
    );
    descriptors.sort_by(|left, right| left.name.cmp(&right.name));
    descriptors
}

fn push_actions(descriptors: &mut Vec<ActionDescriptor>, class: ActionClass, names: &[&str]) {
    push_sourced_actions(descriptors, class, CapabilitySource::Core, names);
}

fn push_sourced_actions(
    descriptors: &mut Vec<ActionDescriptor>,
    class: ActionClass,
    source: CapabilitySource,
    names: &[&str],
) {
    descriptors.extend(
        names
            .iter()
            .map(|name| ActionDescriptor::new(*name, class, source)),
    );
}
