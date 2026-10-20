//! Provider discovery uses the caller's fenced runtime and its saved credentials.
use super::management::Management;
use super::*;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogRequest {
    revision: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum PresetStatus {
    Available,
    ConfigureEndpoint,
    ExistingOnly,
    DeploymentRequired,
}

#[derive(Serialize)]
struct PresetAvailability {
    status: PresetStatus,
    reason: &'static str,
}

/// Availability describes the existing write policy; it grants no new connection authority.
fn preset_availability(
    preset: &colossus_contracts::ProviderPreset,
    current: &ChatProjectConfig,
    ceiling: &ChatProjectConfig,
) -> PresetAvailability {
    use colossus_contracts::ProviderSetupProtocol;
    if preset.protocol == ProviderSetupProtocol::Codex {
        let existing = (current.codex_auth_shared || current.codex_auth_path.is_some())
            && current
                .runtime
                .providers
                .profiles
                .values()
                .any(|profile| profile.kind.generation_action() == "provider.openai.codex");
        return PresetAvailability {
            status: if existing {
                PresetStatus::ExistingOnly
            } else {
                PresetStatus::DeploymentRequired
            },
            reason: if existing {
                "An existing deployment-managed Codex connection can be edited. New ChatGPT account enrollment is not available in Web."
            } else {
                "A deployment administrator must configure the Codex account and connection. ChatGPT account enrollment is not available in Web."
            },
        };
    }
    let Some(endpoint) = preset.base_url else {
        return PresetAvailability {
            status: PresetStatus::ConfigureEndpoint,
            reason: "Enter an HTTPS endpoint allowed by this deployment, then configure its authentication.",
        };
    };
    if crate::chat_settings::allowed_endpoint(endpoint, ceiling, false).is_ok() {
        return PresetAvailability {
            status: PresetStatus::Available,
            reason: "This preset endpoint is allowed by the deployment. Configure its authentication before use.",
        };
    }
    let action = match preset.protocol {
        ProviderSetupProtocol::ChatCompletions => "provider.openai.chat",
        ProviderSetupProtocol::Responses => "provider.openai.responses",
        ProviderSetupProtocol::AnthropicMessages => "provider.anthropic.messages",
        ProviderSetupProtocol::Codex => unreachable!(),
    };
    let existing = current
        .runtime
        .providers
        .profiles
        .iter()
        .any(|(id, profile)| {
            profile.kind.generation_action() == action
                && profile.base_url.as_deref() == Some(endpoint)
                && ceiling
                    .runtime
                    .providers
                    .profiles
                    .get(id)
                    .is_some_and(|seed| {
                        seed.kind == profile.kind && seed.base_url == profile.base_url
                    })
                && crate::chat_settings::allowed_endpoint(endpoint, ceiling, true).is_ok()
        });
    PresetAvailability {
        status: if existing {
            PresetStatus::ExistingOnly
        } else {
            PresetStatus::DeploymentRequired
        },
        reason: if existing {
            "An existing deployment-managed local connection can be edited. New local endpoints must be configured by a deployment administrator and refer to the server, not your browser device."
        } else if endpoint.starts_with("http:") {
            "A deployment administrator must configure this local endpoint. Local addresses refer to the server, not your browser device."
        } else {
            "A deployment administrator must allow this provider endpoint before it can be saved. You can choose a custom endpoint already allowed by the deployment."
        },
    }
}

fn preset_catalog(current: &ChatProjectConfig, ceiling: &ChatProjectConfig) -> serde_json::Value {
    let presets = colossus_contracts::provider_presets();
    let availability: BTreeMap<_, _> = presets
        .iter()
        .map(|preset| (preset.id, preset_availability(preset, current, ceiling)))
        .collect();
    serde_json::json!({ "presets": presets, "availability": availability })
}

pub(crate) async fn presets(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Administer)?;
    let host = state.chat.as_ref().ok_or(CloudError::NotFound)?;
    let configuration = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let ceiling = host
        .deployment
        .projects
        .get(&project)
        .ok_or(CloudError::NotFound)?;
    Ok(Json(preset_catalog(&configuration, ceiling)))
}

pub(crate) async fn models(
    Extract(state): Extract<Arc<State>>,
    Path((project, profile)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CatalogRequest>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Administer)?;
    caller.require(CloudPermission::Execute)?;
    let host = state.chat.as_ref().ok_or(CloudError::NotFound)?;
    host.check_catalog_revision(&project, &profile, input.revision)
        .await?;
    host.ensure(&state, &caller).await?;
    let identity = ChatRuntime::identity_for(&project, caller.subject())
        .map_err(|_| CloudError::InvalidArgument)?;
    let value = if let Some(shared) = &host.shared {
        let lease = shared.sql.read_lease(&identity)?;
        if lease.project_id != project || lease.user_id != caller.subject() {
            return Err(CloudError::PermissionDenied.into());
        }
        host.management_request(
            &lease,
            Management::ProviderModels {
                profile,
                revision: input.revision,
            },
        )
        .await?
    } else {
        let live = host
            .live
            .lock()
            .await
            .get(&identity)
            .cloned()
            .ok_or(CloudError::Conflict)?;
        host.provider_catalog(&live, &profile, input.revision)
            .await?
    };
    Ok(Json(value))
}

impl HostedChat {
    async fn check_catalog_revision(
        &self,
        project: &str,
        profile: &str,
        revision: u64,
    ) -> std::result::Result<(), CloudError> {
        let configuration = self
            .project_config(project)
            .await?
            .ok_or(CloudError::NotFound)?;
        if configuration.revision != revision {
            return Err(CloudError::Conflict);
        }
        if !configuration
            .runtime
            .providers
            .profiles
            .contains_key(profile)
        {
            return Err(CloudError::NotFound);
        }
        Ok(())
    }

    pub(super) async fn provider_catalog(
        &self,
        live: &LiveChat,
        profile: &str,
        revision: u64,
    ) -> std::result::Result<serde_json::Value, CloudError> {
        let _lifecycle = self
            .lifecycle
            .try_lock()
            .map_err(|_| CloudError::Conflict)?;
        self.check_catalog_revision(&live.project_id, profile, revision)
            .await?;
        if *live.shutdown.borrow() {
            return Err(CloudError::Conflict);
        }
        let models = tokio::time::timeout(
            Duration::from_secs(20),
            live.runtime.runtime().provider_models(Some(profile)),
        )
        .await
        .map_err(|_| CloudError::Storage)?
        .map_err(|_| CloudError::Storage)?;
        // The runtime gateway checks the execution fence before releasing catalog output.
        if let Some(shared) = &self.shared {
            shared
                .sql
                .verify_lease(live.lease.as_ref().ok_or(CloudError::Storage)?)?;
        }
        Ok(serde_json::json!({ "models": models }))
    }
}

#[cfg(test)]
mod tests;
