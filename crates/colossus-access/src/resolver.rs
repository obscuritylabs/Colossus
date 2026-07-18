use crate::{
    AccessConfig, AccessContext, AccessDecision, AccessError, AccessProfile, AccessResolution,
    ActionClass, ActionDescriptor, ResolvedAction, ResolvedTool, ToolAvailability, ToolDescriptor,
    ToolPrerequisite, validate_config,
};
use colossus_contracts::ToolSpec;
use std::collections::{BTreeMap, BTreeSet};

/// Resolve trusted tools and actions through one profile.
pub fn resolve_access(
    config: &AccessConfig,
    tool_specs: &[ToolSpec],
    action_descriptors: impl IntoIterator<Item = ActionDescriptor>,
    tool_descriptors: impl IntoIterator<Item = ToolDescriptor>,
    context: &AccessContext,
    external_policy: bool,
) -> Result<AccessResolution, AccessError> {
    validate_config(config, external_policy)?;
    let mut actions = BTreeMap::new();
    for descriptor in action_descriptors {
        let name = descriptor.name.clone();
        if actions.insert(name.clone(), descriptor).is_some() {
            return Err(AccessError::Invalid(format!(
                "duplicate action descriptor {name}"
            )));
        }
    }
    let mut tool_metadata = BTreeMap::new();
    for descriptor in tool_descriptors {
        let name = descriptor.name.clone();
        if tool_metadata.insert(name.clone(), descriptor).is_some() {
            return Err(AccessError::Invalid(format!(
                "duplicate tool descriptor {name}"
            )));
        }
    }
    for action in config
        .actions
        .allow
        .iter()
        .chain(&config.actions.require_approval)
        .chain(&config.actions.deny)
    {
        if !actions.contains_key(action) {
            return Err(AccessError::Unclassified(action.clone()));
        }
    }

    let allow = config.actions.allow.iter().collect::<BTreeSet<_>>();
    let approval = config
        .actions
        .require_approval
        .iter()
        .collect::<BTreeSet<_>>();
    let deny = config.actions.deny.iter().collect::<BTreeSet<_>>();
    let resolved_actions = actions
        .values()
        .map(|descriptor| {
            let (decision, reason) = if external_policy {
                (AccessDecision::ExternalPolicy, "external policy")
            } else if deny.contains(&descriptor.name) {
                (AccessDecision::Deny, "explicit deny")
            } else if approval.contains(&descriptor.name) {
                (AccessDecision::RequireApproval, "explicit approval")
            } else if allow.contains(&descriptor.name) {
                (AccessDecision::Allow, "explicit allow")
            } else {
                (
                    profile_decision(config.profile, descriptor),
                    "profile default",
                )
            };
            ResolvedAction {
                name: descriptor.name.clone(),
                class: descriptor.class,
                source: descriptor.source,
                decision,
                reason: reason.into(),
            }
        })
        .collect::<Vec<_>>();
    let decisions = resolved_actions
        .iter()
        .map(|action| (action.name.as_str(), (action.class, action.decision)))
        .collect::<BTreeMap<_, _>>();

    let include_all = config.tools.include.iter().any(|name| name == "*");
    let includes = config.tools.include.iter().collect::<BTreeSet<_>>();
    let excludes = config.tools.exclude.iter().collect::<BTreeSet<_>>();
    let known_tools = tool_specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<BTreeSet<_>>();
    for name in config
        .tools
        .include
        .iter()
        .chain(&config.tools.exclude)
        .filter(|name| name.as_str() != "*")
    {
        if !known_tools.contains(name.as_str()) {
            return Err(AccessError::Unclassified(name.clone()));
        }
    }

    let mut resolved_tools = Vec::with_capacity(tool_specs.len());
    for spec in tool_specs {
        let metadata = tool_metadata
            .get(&spec.name)
            .ok_or_else(|| AccessError::Unclassified(format!("tool {}", spec.name)))?;
        let exactly_included = includes.contains(&spec.name);
        let explicitly_selected = include_all || exactly_included;
        let selected = match config.profile {
            AccessProfile::Minimal => spec.effect_action.is_none(),
            AccessProfile::Development | AccessProfile::AllowAll => true,
            AccessProfile::Pinned => explicitly_selected,
        } || explicitly_selected;
        let unmet = metadata
            .prerequisites
            .iter()
            .find(|requirement| !prerequisite_met(**requirement, context))
            .copied();
        let (availability, reason) = if excludes.contains(&spec.name) {
            (ToolAvailability::Hidden, "explicit exclude".into())
        } else if !selected {
            (
                ToolAvailability::Hidden,
                "profile does not select tool".into(),
            )
        } else if let Some(requirement) = unmet {
            if exactly_included && requirement != ToolPrerequisite::Interactive {
                return Err(AccessError::Invalid(format!(
                    "explicitly included tool {} has unmet prerequisite {}",
                    spec.name,
                    prerequisite_label(requirement)
                )));
            }
            (
                ToolAvailability::Hidden,
                format!("unmet prerequisite: {}", prerequisite_label(requirement)),
            )
        } else if exactly_included {
            (ToolAvailability::Active, "explicit include".into())
        } else if include_all {
            (ToolAvailability::Active, "wildcard include".into())
        } else {
            (ToolAvailability::Active, "profile selection".into())
        };
        let (action_class, decision) = match spec.effect_action.as_deref() {
            Some(action) => {
                let (class, decision) = decisions
                    .get(action)
                    .copied()
                    .ok_or_else(|| AccessError::Unclassified(action.into()))?;
                (Some(class), Some(decision))
            }
            None => (None, None),
        };
        if let Some(capability) = spec.capability.as_deref()
            && !decisions.contains_key(capability)
        {
            return Err(AccessError::Unclassified(capability.into()));
        }
        resolved_tools.push(ResolvedTool {
            name: spec.name.clone(),
            family: metadata.family.clone(),
            source: metadata.source,
            availability,
            reason,
            unmet_prerequisite: unmet.map(|requirement| prerequisite_label(requirement).into()),
            effect_action: spec.effect_action.clone(),
            capability: spec.capability.clone(),
            action_class,
            decision,
        });
    }
    resolved_tools.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(AccessResolution {
        profile: config.profile,
        external_policy,
        tools: resolved_tools,
        actions: resolved_actions,
    })
}

fn profile_decision(profile: AccessProfile, descriptor: &ActionDescriptor) -> AccessDecision {
    match profile {
        AccessProfile::Minimal => {
            if descriptor.class == ActionClass::Provider {
                AccessDecision::Allow
            } else {
                AccessDecision::Deny
            }
        }
        AccessProfile::Development => match descriptor.class {
            ActionClass::Provider | ActionClass::Read | ActionClass::LocalState => {
                AccessDecision::Allow
            }
            ActionClass::WorkspaceMutation
            | ActionClass::Execution
            | ActionClass::ExternalNetwork
            | ActionClass::Administration => AccessDecision::RequireApproval,
        },
        AccessProfile::AllowAll => AccessDecision::Allow,
        AccessProfile::Pinned => {
            if descriptor.name == "provider.echo" {
                AccessDecision::Allow
            } else {
                AccessDecision::Deny
            }
        }
    }
}

fn prerequisite_met(requirement: ToolPrerequisite, context: &AccessContext) -> bool {
    match requirement {
        ToolPrerequisite::FilesystemRead => context.filesystem_read,
        ToolPrerequisite::FilesystemWrite => context.filesystem_write,
        ToolPrerequisite::GitExecutable => context.git_executable,
        ToolPrerequisite::AnyExecutable => context.any_executable,
        ToolPrerequisite::NetworkDestination => context.network_destination,
        ToolPrerequisite::AgentSearchRoute => context.agent_search_route,
        ToolPrerequisite::Interactive => context.interactive,
        ToolPrerequisite::McpConfigured => context.mcp_configured,
    }
}

fn prerequisite_label(requirement: ToolPrerequisite) -> &'static str {
    match requirement {
        ToolPrerequisite::FilesystemRead => "filesystem read grant",
        ToolPrerequisite::FilesystemWrite => "filesystem write grant",
        ToolPrerequisite::GitExecutable => "exact Git executable",
        ToolPrerequisite::AnyExecutable => "exact sandbox executable",
        ToolPrerequisite::NetworkDestination => "network destination",
        ToolPrerequisite::AgentSearchRoute => "agent search route",
        ToolPrerequisite::Interactive => "interactive user interface",
        ToolPrerequisite::McpConfigured => "configured MCP server",
    }
}
