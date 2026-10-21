//! Configuration discovery never requires a resident runtime or a Fleet connection.
use super::*;
use colossus_cloud::CloudNode;
use colossus_cloud_executor::ExecutionResources;

pub(super) async fn discovery(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let Some(host) = &state.hosted else {
        return Ok(Json(serde_json::json!({"enabled":false,"target":null})));
    };
    if host.project_config(&project).await?.is_none() {
        return Ok(Json(serde_json::json!({"enabled":false,"target":null})));
    }
    let target = host.node(&state, &caller).await?;
    let activity = super::inventory::activity(host, &caller).await?;
    Ok(Json(serde_json::json!({"enabled":true,"target":target,
        "activity":activity,
        "capabilities":{"managed_files":true,"filesystem":false,"process":false},"environment":null})))
}

impl HostedExecution {
    pub(crate) async fn context(
        &self,
        caller: &CloudCaller,
    ) -> std::result::Result<(String, ExecutionResources), CloudError> {
        caller.require(CloudPermission::Read)?;
        let configuration = self
            .project_config(caller.project_id())
            .await?
            .ok_or(CloudError::NotFound)?;
        let profiles = configuration
            .runtime
            .models
            .profiles
            .iter()
            .map(|(id, model)| colossus_sdk::ConfiguredModelProfile {
                profile: id.clone(),
                model: model.model.clone(),
                provider: configuration
                    .labels
                    .providers
                    .get(&model.provider_profile)
                    .cloned()
                    .unwrap_or_else(|| model.provider_profile.clone()),
                provider_profile: model.provider_profile.clone(),
                reasoning_effort: model.reasoning_effort.map(|effort| effort.as_str().into()),
                default_roles: configuration
                    .runtime
                    .models
                    .roles
                    .iter()
                    .filter(|(_, profile)| *profile == id)
                    .map(|(role, _)| role.clone())
                    .collect(),
            })
            .collect();
        Ok((
            format!("hosted-config-{}", configuration.revision),
            ExecutionResources {
                workflows: None,
                plugins: None,
                capabilities: super::pool::capabilities(),
                model_profiles: profiles,
            },
        ))
    }

    pub(super) async fn configured_target(
        &self,
        caller: &CloudCaller,
    ) -> std::result::Result<CloudNode, CloudError> {
        let configuration = self
            .project_config(caller.project_id())
            .await?
            .ok_or(CloudError::NotFound)?;
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        Ok(CloudNode {
            execution_kind: colossus_cloud::ExecutionKind::Hosted,
            owner_user_id: Some(caller.subject().into()),
            node_id: format!("chat-{identity}"),
            project_id: caller.project_id().into(),
            instance_id: format!("hosted-chat-{identity}"),
            label: "Colossus Hosted".into(),
            certificate_sha256: String::new(),
            roles: configuration.runtime.models.roles.keys().cloned().collect(),
            revoked: false,
            host_id: None,
            workspace_id: None,
            workspace_label: None,
            runtime_ready: true,
            policy: None,
            policy_observed_at: None,
            revision: 0,
        })
    }
}
