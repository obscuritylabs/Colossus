use super::*;

/// Build a minimal effect request for trusted callers without losing provenance.
pub fn effect_request(
    actor: Actor,
    action: impl Into<String>,
    resource: impl Into<String>,
    content: Value,
) -> EffectRequest {
    EffectRequest {
        schema_version: 1,
        request_id: Uuid::now_v7().to_string(),
        actor,
        action: action.into(),
        resource: resource.into(),
        capabilities: Vec::new(),
        risk: colossus_contracts::RiskInput {
            status: colossus_contracts::RiskStatus::Unavailable,
            level: None,
            reason: None,
        },
        content,
        credential_references: Vec::new(),
        context: colossus_contracts::ExecutionContext {
            correlation_id: Uuid::now_v7().to_string(),
            ..colossus_contracts::ExecutionContext::default()
        },
        idempotency_id: None,
        phase: EffectPhase::PreEffect,
        approval: None,
    }
}

/// Trusted system actor used by kernel services and offline smoke adapters.
pub fn system_actor(id: impl Into<String>) -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: id.into(),
    }
}
