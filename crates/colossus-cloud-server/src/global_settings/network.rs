//! Project connection policy is separate from process, filesystem and credential authority.
use crate::config::ChatProjectConfig;
use colossus_cloud::{CloudError, CloudResult};
use colossus_cloud_chat::ChatConnectionKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionNetworkMode {
    #[default]
    AllowAll,
    AllowList,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionNetworkPolicy {
    pub mode: ConnectionNetworkMode,
    #[serde(default)]
    pub allowed_origins: BTreeSet<String>,
}

impl ConnectionNetworkPolicy {
    pub(crate) fn validate(&self) -> CloudResult<()> {
        if self.allowed_origins.len() > 256 {
            return Err(CloudError::ResourceExhausted);
        }
        for origin in &self.allowed_origins {
            let url = url::Url::parse(origin).map_err(|_| CloudError::InvalidArgument)?;
            if origin.len() > 2048
                || !matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.host_str().is_none_or(|host| host.contains('*'))
                || url.query().is_some()
                || url.fragment().is_some()
                || *origin != url.origin().ascii_serialization()
            {
                return Err(CloudError::InvalidArgument);
            }
        }
        Ok(())
    }

    pub(crate) fn permits(&self, endpoint: &str) -> bool {
        let Ok(url) = url::Url::parse(endpoint) else {
            return false;
        };
        self.mode == ConnectionNetworkMode::AllowAll
            || self
                .allowed_origins
                .contains(&url.origin().ascii_serialization())
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct ConnectionAvailability {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}
impl ConnectionAvailability {
    fn available() -> Self {
        Self {
            enabled: true,
            reason: None,
        }
    }
    fn blocked(reason: &'static str) -> Self {
        Self {
            enabled: false,
            reason: Some(reason),
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct ConnectionAvailabilities {
    pub providers: BTreeMap<String, ConnectionAvailability>,
    pub mcp: BTreeMap<String, ConnectionAvailability>,
    pub search: BTreeMap<String, ConnectionAvailability>,
    pub models: BTreeMap<String, ConnectionAvailability>,
    pub roles: BTreeMap<String, ConnectionAvailability>,
}

fn provider_origins(profile: &colossus_runtime::ProviderProfileConfig) -> BTreeSet<String> {
    if profile.kind.generation_action() == "provider.openai.codex" {
        [
            colossus_codex_auth::CODEX_API_BASE_URL,
            colossus_codex_auth::CODEX_AUTH_ORIGIN,
        ]
        .into_iter()
        .filter_map(|endpoint| {
            url::Url::parse(endpoint)
                .ok()
                .map(|url| url.origin().ascii_serialization())
        })
        .collect()
    } else {
        profile
            .base_url
            .as_deref()
            .and_then(|endpoint| url::Url::parse(endpoint).ok())
            .map(|url| url.origin().ascii_serialization())
            .into_iter()
            .collect()
    }
}

pub(crate) fn configured_origins(config: &ChatProjectConfig) -> BTreeSet<String> {
    config
        .runtime
        .providers
        .profiles
        .values()
        .flat_map(provider_origins)
        .chain(
            config
                .runtime
                .mcp
                .servers
                .values()
                .filter_map(|server| server.url.as_deref())
                .chain(
                    config
                        .runtime
                        .search
                        .profiles
                        .values()
                        .map(super::search::endpoint),
                )
                .filter_map(|endpoint| {
                    url::Url::parse(endpoint)
                        .ok()
                        .map(|url| url.origin().ascii_serialization())
                }),
        )
        .collect()
}

const POLICY_BLOCKED: &str =
    "This connection's origin is blocked by the project's Network access policy.";

fn connection_availability(enabled: bool, reason: &'static str) -> ConnectionAvailability {
    if enabled {
        ConnectionAvailability::available()
    } else {
        ConnectionAvailability::blocked(reason)
    }
}

fn provider_enabled(
    configuration: &ChatProjectConfig,
    profile: &colossus_runtime::ProviderProfileConfig,
) -> bool {
    provider_origins(profile)
        .iter()
        .all(|origin| configuration.connection_network_policy.permits(origin))
}

pub(crate) fn assess(configuration: &ChatProjectConfig) -> ConnectionAvailabilities {
    let mut output = ConnectionAvailabilities::default();
    for (id, profile) in &configuration.runtime.providers.profiles {
        output.providers.insert(
            id.clone(),
            connection_availability(provider_enabled(configuration, profile), POLICY_BLOCKED),
        );
    }
    for (id, server) in &configuration.runtime.mcp.servers {
        let enabled = server
            .url
            .as_deref()
            .is_some_and(|url| configuration.connection_network_policy.permits(url));
        output
            .mcp
            .insert(id.clone(), connection_availability(enabled, POLICY_BLOCKED));
    }
    for (id, profile) in &configuration.runtime.search.profiles {
        let enabled = configuration
            .connection_network_policy
            .permits(super::search::endpoint(profile));
        output
            .search
            .insert(id.clone(), connection_availability(enabled, POLICY_BLOCKED));
    }
    for (id, model) in &configuration.runtime.models.profiles {
        let enabled = output
            .providers
            .get(&model.provider_profile)
            .is_some_and(|provider| provider.enabled);
        output.models.insert(id.clone(), connection_availability(enabled, "This model's provider is missing or blocked by the project's Network access policy."));
    }
    for (role, model) in &configuration.runtime.models.roles {
        let enabled = output.models.get(model).is_some_and(|model| model.enabled);
        output.roles.insert(role.clone(), connection_availability(enabled, "This role's model is missing or unavailable. Choose an available model in Model routing."));
    }
    for (role, profile) in &configuration.runtime.search.roles {
        let enabled = output
            .search
            .get(profile)
            .is_some_and(|profile| profile.enabled);
        output.roles.insert(format!("search:{role}"), connection_availability(enabled, "This Search role's connection is missing or blocked by the project's Network access policy."));
    }
    output
}

pub(crate) fn connection_enabled(
    configuration: &ChatProjectConfig,
    kind: ChatConnectionKind,
    id: &str,
) -> bool {
    match kind {
        ChatConnectionKind::Provider => configuration
            .runtime
            .providers
            .profiles
            .get(id)
            .is_some_and(|profile| provider_enabled(configuration, profile)),
        ChatConnectionKind::Mcp => configuration
            .runtime
            .mcp
            .servers
            .get(id)
            .and_then(|server| server.url.as_deref())
            .is_some_and(|url| configuration.connection_network_policy.permits(url)),
        ChatConnectionKind::Search => {
            configuration
                .runtime
                .search
                .profiles
                .get(id)
                .is_some_and(|profile| {
                    configuration
                        .connection_network_policy
                        .permits(super::search::endpoint(profile))
                })
        }
    }
}

/// Only this projection can enter credential resolution and Core runtime composition.
/// The full catalog remains intact for administrative edits, provenance and reset.
pub(crate) fn execution(configuration: &ChatProjectConfig) -> CloudResult<ChatProjectConfig> {
    configuration.connection_network_policy.validate()?;
    let availability = assess(configuration);
    if !availability
        .roles
        .get("primary")
        .is_some_and(|role| role.enabled)
    {
        return Err(CloudError::PermissionDenied);
    }
    let mut output = configuration.clone();
    output
        .runtime
        .providers
        .profiles
        .retain(|id, _| availability.providers[id].enabled);
    output
        .runtime
        .models
        .profiles
        .retain(|id, _| availability.models[id].enabled);
    output
        .runtime
        .models
        .roles
        .retain(|id, _| availability.roles[id].enabled);
    output
        .runtime
        .mcp
        .servers
        .retain(|id, _| availability.mcp[id].enabled);
    output
        .runtime
        .search
        .profiles
        .retain(|id, _| availability.search[id].enabled);
    output
        .runtime
        .search
        .roles
        .retain(|id, _| availability.roles[&format!("search:{id}")].enabled);
    let configured = configured_origins(configuration);
    let mut origins = output
        .runtime
        .sandbox
        .network_destinations
        .iter()
        .filter(|origin| {
            !configured.contains(*origin) && configuration.connection_network_policy.permits(origin)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    origins.extend(configured_origins(&output));
    if configuration.connection_network_policy.mode == ConnectionNetworkMode::AllowList {
        // Explicit auth/metadata origins are connection authority, not a wildcard.
        // In AllowAll mode the retained list is inactive and grants nothing.
        origins.extend(
            configuration
                .connection_network_policy
                .allowed_origins
                .iter()
                .cloned(),
        );
    }
    output.runtime.sandbox.network_destinations = origins.into_iter().collect();
    Ok(output)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod oauth_tests;
