//! Safety checks and authorization lineage for internal remote MCP Tasks effects.

use super::*;

#[cfg(test)]
mod tests;

pub(super) const UNSUPPORTED_REDACTIONS: &str =
    "MCP Tasks cannot satisfy policy-specified output redactions";

pub(super) fn is_follow_up(action: &str) -> bool {
    matches!(action, "mcp.tasks.get" | "mcp.tasks.cancel")
}

pub(super) fn is_task_step(request: &EffectRequest) -> bool {
    is_follow_up(&request.action)
        || (request.action == "mcp.call" && request.content.get("task_step").is_some())
}

pub(super) fn validate_pre_effect(request: &EffectRequest) -> Result<(), GatewayError> {
    if !is_task_step(request) {
        return Ok(());
    }
    let invalid = || GatewayError::Safety("invalid internal MCP Tasks effect".into());
    let step = request
        .content
        .get("task_step")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let kind = match request.action.as_str() {
        "mcp.call" => "start",
        "mcp.tasks.get" => "get",
        "mcp.tasks.cancel" => "cancel",
        _ => return Err(invalid()),
    };
    let binding_valid = if kind == "start" {
        step.len() == 1
    } else {
        // Only the local, opaque commitment crosses policy. The actual task ID
        // and its routing authority stay in the adapter's active invocation.
        step.len() == 2
            && step
                .get("binding")
                .and_then(Value::as_str)
                .is_some_and(|binding| {
                    binding.len() == 64
                        && binding
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                })
    };
    if step.get("kind").and_then(Value::as_str) != Some(kind)
        || !binding_valid
        || request.content.get("transport").and_then(Value::as_str) != Some("streamable_http")
        || !matches!(
            request
                .content
                .get("protocol_version")
                .and_then(Value::as_str),
            Some("auto" | "2026-07-28")
        )
        || request.content.get("url").and_then(Value::as_str) != Some(request.resource.as_str())
        || !request.content.get("provenance").is_none_or(Value::is_null)
        || request
            .content
            .pointer("/operation/kind")
            .and_then(Value::as_str)
            != Some("call_tool")
        || !request
            .capabilities
            .iter()
            .any(|capability| capability == "mcp.invoke")
        || !matches!(
            request.actor.actor_type,
            ActorType::Model | ActorType::Subagent
        )
        || request.context.run_id.as_deref().is_none_or(str::is_empty)
        || request
            .context
            .session_id
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(invalid());
    }
    Ok(())
}

/// Evaluate the original action as a policy-only veto on a fresh follow-up.
/// The original one-use approval does not move to this request.
pub(super) fn parent_policy_request(
    request: &EffectRequest,
) -> Result<Option<EffectRequest>, GatewayError> {
    if !is_follow_up(&request.action) {
        return Ok(None);
    }
    let mut parent = request.clone();
    parent.request_id = format!("{}:mcp-call-authority", request.request_id);
    parent.action = "mcp.call".into();
    parent.approval = None;
    parent
        .content
        .as_object_mut()
        .ok_or_else(|| GatewayError::Contract("MCP Tasks policy content is not an object".into()))?
        .insert("task_step".into(), json!({"kind": "start"}));
    Ok(Some(parent))
}

pub(super) fn fits_live_parent_ceiling(
    follow_up: &PolicyObligations,
    parent: &PolicyObligations,
) -> bool {
    // Compare declared grants literally. Recognizing a larger set of allowed
    // origins as equivalent could transfer credentials to a wider authority.
    follow_up.resource_authority == parent.resource_authority
        && follow_up
            .network_destinations
            .iter()
            .all(|grant| parent.network_destinations.contains(grant))
        && follow_up
            .allowed_environment
            .iter()
            .all(|name| parent.allowed_environment.contains(name))
        && follow_up.timeout_ms <= parent.timeout_ms
        && follow_up.max_output_bytes <= parent.max_output_bytes
        && parent.required_redactions.is_empty()
}

pub(super) fn release_denial_reason(
    request: &EffectRequest,
    decision: &PolicyDecision,
    output_size: usize,
) -> Option<String> {
    if decision.outcome != DecisionOutcome::Allow {
        return Some(decision.reason.clone());
    }
    if !is_task_step(request) {
        return None;
    }
    if !decision.obligations.required_redactions.is_empty() {
        return Some(UNSUPPORTED_REDACTIONS.into());
    }
    if output_size > usize::try_from(decision.obligations.max_output_bytes).unwrap_or(usize::MAX) {
        return Some("MCP Tasks output exceeded the current release policy limit".into());
    }
    None
}
