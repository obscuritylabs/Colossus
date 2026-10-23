//! Reject unavailable routes before durable admission or private runtime composition.
use super::*;
use crate::global_settings::network;
use colossus_sdk::{CreateRunRequest, ResearchSourceKind, RunMode};

pub(super) fn review_request(
    configuration: &ChatProjectConfig,
    request: &CreateRunRequest,
) -> std::result::Result<(), CloudError> {
    let availability = network::assess(configuration);
    let role = if request.role.is_empty() {
        "primary"
    } else {
        &request.role
    };
    if !availability
        .roles
        .get("primary")
        .is_some_and(|entry| entry.enabled)
        || availability
            .roles
            .get(role)
            .is_some_and(|entry| !entry.enabled)
        || request.model_profile.as_ref().is_some_and(|profile| {
            !availability
                .models
                .get(profile)
                .is_some_and(|entry| entry.enabled)
        })
    {
        return Err(CloudError::PermissionDenied);
    }
    if request.mode == RunMode::Research {
        // Public run validation requires explicit, nonempty evidence lanes. There are
        // no implicit Web or MCP defaults to authorize here.
        for role in [
            "research_planner",
            "research_worker",
            "research_synthesizer",
        ] {
            // A genuinely unconfigured specialized role retains Core's Primary default.
            // An explicitly configured but blocked role never acquires that fallback.
            if availability
                .roles
                .get(role)
                .is_some_and(|entry| !entry.enabled)
            {
                return Err(CloudError::PermissionDenied);
            }
        }
        if request.research_sources.contains(&ResearchSourceKind::Web)
            && availability
                .roles
                .get("search:research")
                .is_some_and(|entry| !entry.enabled)
            || request.research_sources.contains(&ResearchSourceKind::Mcp)
                && !availability.mcp.values().any(|entry| entry.enabled)
        {
            return Err(CloudError::PermissionDenied);
        }
    }
    Ok(())
}

impl HostedExecution {
    /// Caller retains lifecycle admission so definitions and policy cannot change mid-review.
    pub(crate) async fn require_request(
        &self,
        caller: &CloudCaller,
        request: &CreateRunRequest,
    ) -> std::result::Result<(), CloudError> {
        let configuration = self
            .project_config(caller.project_id())
            .await?
            .ok_or(CloudError::NotFound)?;
        review_request(&configuration, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_settings::network::{ConnectionNetworkMode, ConnectionNetworkPolicy};
    use colossus_sdk::{IdempotencyKey, InputContentPart, ResearchDepth};
    use serde_json::json;

    fn configuration(blocked_role: Option<&str>) -> ChatProjectConfig {
        let mut runtime =
            colossus_runtime::RuntimeConfig::controlled_chat_template("/unused/state.redb");
        let provider = serde_json::from_value(json!({
            "kind":"open_ai_compatible", "baseUrl":"https://blocked.example/v1"
        }))
        .unwrap();
        runtime
            .providers
            .profiles
            .insert("blocked".into(), provider);
        let mut model = runtime.models.profiles.values().next().unwrap().clone();
        model.provider_profile = "blocked".into();
        runtime.models.profiles.insert("blocked".into(), model);
        if let Some(role) = blocked_role {
            runtime.models.roles.insert(role.into(), "blocked".into());
        }
        let mut configuration = ChatProjectConfig::new(runtime);
        configuration.connection_network_policy = ConnectionNetworkPolicy {
            mode: ConnectionNetworkMode::AllowList,
            allowed_origins: Default::default(),
        };
        configuration
    }

    fn request() -> CreateRunRequest {
        CreateRunRequest {
            model_profile: None,
            reasoning_effort: None,
            plugin_skill_ids: vec![],
            input: vec![InputContentPart::Text("Test policy admission".into())],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            goal_max_iterations: 0,
            research_depth: None,
            research_sources: vec![],
            plan_action: None,
            branch: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new("network-admission").unwrap(),
        }
    }

    fn research_request() -> CreateRunRequest {
        let mut input = request();
        input.mode = RunMode::Research;
        input.research_depth = Some(ResearchDepth::Quick);
        input.research_sources = vec![ResearchSourceKind::Repo];
        input
    }

    #[test]
    fn blocked_models_and_explicit_roles_cannot_be_admitted_or_overridden() {
        let configuration = configuration(Some("planner"));
        assert!(review_request(&configuration, &request()).is_ok());
        let mut input = request();
        input.model_profile = Some("blocked".into());
        assert_eq!(
            review_request(&configuration, &input),
            Err(CloudError::PermissionDenied)
        );
        input.model_profile = None;
        input.role = "planner".into();
        assert_eq!(
            review_request(&configuration, &input),
            Err(CloudError::PermissionDenied)
        );
        input.model_profile = configuration.runtime.models.roles.get("primary").cloned();
        assert_eq!(
            review_request(&configuration, &input),
            Err(CloudError::PermissionDenied)
        );
    }

    #[test]
    fn research_rejects_blocked_internal_routes_before_execution() {
        for role in [
            "research_planner",
            "research_worker",
            "research_synthesizer",
        ] {
            let configuration = configuration(Some(role));
            let mut input = request();
            assert!(review_request(&configuration, &input).is_ok());
            input = research_request();
            assert_eq!(
                review_request(&configuration, &input),
                Err(CloudError::PermissionDenied)
            );
        }
    }

    #[test]
    fn requested_research_lanes_cannot_use_blocked_connections() {
        let mut configuration = configuration(None);
        configuration.runtime.search = serde_json::from_value(json!({
            "profiles":{"blocked":{"kind":"searxng","endpoint":"https://blocked.example/search"}},
            "roles":{"research":"blocked"}
        }))
        .unwrap();
        configuration.runtime.mcp.servers = serde_json::from_value(json!({
            "blocked":{"transport":"streamable_http","url":"https://blocked.example/mcp","allowedTools":["*"]}
        })).unwrap();
        let mut input = research_request();
        for source in [ResearchSourceKind::Web, ResearchSourceKind::Mcp] {
            input.research_sources = vec![source];
            assert_eq!(
                review_request(&configuration, &input),
                Err(CloudError::PermissionDenied)
            );
        }
    }

    #[test]
    fn unconfigured_roles_are_left_to_existing_transport_validation() {
        let configuration = configuration(None);
        for role in ["planner", "research_planner", "unknown_role"] {
            let mut input = request();
            input.role = role.into();
            assert!(review_request(&configuration, &input).is_ok());
        }
        // The public API separately validates explicit Research depth and lanes.
        // Only the requested lanes participate in this network policy review.
        assert!(review_request(&configuration, &research_request()).is_ok());
    }

    #[test]
    fn an_unreferenced_blocked_profile_does_not_poison_other_routes() {
        let current = configuration(None);
        assert!(review_request(&current, &request()).is_ok());
        let mut input = request();
        input.role.clear();
        assert!(review_request(&current, &input).is_ok());
        let blocked_primary = configuration(Some("primary"));
        assert_eq!(
            review_request(&blocked_primary, &request()),
            Err(CloudError::PermissionDenied)
        );
    }
}
