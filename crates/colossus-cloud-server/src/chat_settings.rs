//! Project-managed chat connections. Browser DTOs never carry host configuration authority.
use crate::{
    chat::HostedChat,
    config::{ChatProjectConfig, ChatSettingsLabels},
    http::Result,
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudError, CloudPermission};
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, ChatCredentialResolver,
};
use colossus_contracts::{HostSecret, VaultRecord};
use colossus_mcp::{McpConfig, McpTransportKind};
use colossus_ports::CredentialKey;
use colossus_runtime::{ModelsConfig, ProvidersConfig};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

const MANAGED_REFERENCE: &str = "host:managed";
mod credentials;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    revision: u64,
    #[serde(default)]
    labels: Option<ChatSettingsLabels>,
    providers: ProvidersConfig,
    models: ModelsConfig,
    mcp: McpConfig,
    #[serde(default)]
    provider_authentication: BTreeMap<String, ChatAuthentication>,
    #[serde(default)]
    mcp_authentication: BTreeMap<String, ChatAuthentication>,
}
#[derive(Serialize)]
struct SettingsOutput {
    #[serde(flatten)]
    configuration: SettingsInput,
    credential_status: CredentialStatus,
}
#[derive(Default, Serialize)]
struct CredentialStatus {
    providers: BTreeMap<String, bool>,
    mcp: BTreeMap<String, bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialInput {
    revision: u64,
    secret: String,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route(
            "/api/projects/{project}/chat/settings/provider-presets",
            get(crate::chat::catalog::presets),
        )
        .route(
            "/api/projects/{project}/chat/settings/providers/{connection}/models",
            post(crate::chat::catalog::models),
        )
        .route(
            "/api/projects/{project}/chat/settings",
            get(settings).put(replace),
        )
        .route(
            "/api/projects/{project}/chat/settings/providers/{connection}/credential",
            post(save_provider_credential),
        )
        .route(
            "/api/projects/{project}/chat/settings/mcp/{connection}/credential",
            post(save_mcp_credential),
        )
        .route(
            "/api/projects/{project}/chat/providers",
            get(credentials::providers),
        )
        .route(
            "/api/projects/{project}/chat/providers/{connection}/credential",
            post(credentials::save_provider),
        )
        .route(
            "/api/projects/{project}/chat/providers/{connection}/disconnect",
            post(credentials::disconnect_provider),
        )
        .route(
            "/api/projects/{project}/chat/connections/{connection}/credential",
            post(credentials::save_mcp),
        )
}

fn project_record(project: &str) -> std::result::Result<CredentialKey, CloudError> {
    colossus_cloud::validate_identifier(project)?;
    CredentialKey::new("cloud-chat-config", "project").map_err(|_| CloudError::Storage)
}

fn hosted(state: &State) -> std::result::Result<&HostedChat, CloudError> {
    state.chat.as_deref().ok_or(CloudError::NotFound)
}

async fn settings(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SettingsOutput>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Administer)?;
    let host = hosted(&state)?;
    let configuration = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let mut output = public_configuration(configuration.clone());
    for (id, profile) in &configuration.runtime.providers.profiles {
        let authentication = configuration
            .provider_authentication
            .get(id)
            .copied()
            .unwrap_or_default();
        let configured = if authentication != ChatAuthentication::Shared
            || profile.kind.generation_action() == "provider.echo"
        {
            false
        } else if profile.kind.generation_action() == "provider.openai.codex" {
            configuration.codex_auth_path.is_some() || configuration.codex_auth_shared
        } else if profile
            .credential_reference
            .as_deref()
            .is_some_and(|reference| reference.starts_with("env:"))
        {
            true
        } else if let Some(binding) =
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Provider, id)?
        {
            host.project_vault(&project)?
                .contains(
                    &CredentialKey::new("cloud-chat", &binding.record_id())
                        .map_err(|_| CloudError::Storage)?,
                )
                .map_err(|_| CloudError::Storage)?
        } else {
            false
        };
        output
            .credential_status
            .providers
            .insert(id.clone(), configured);
    }
    for (id, server) in &configuration.runtime.mcp.servers {
        let authentication = configuration
            .mcp_authentication
            .get(id)
            .copied()
            .unwrap_or_default();
        let configured = if authentication != ChatAuthentication::Shared {
            false
        } else if server
            .credential_headers
            .values()
            .any(|header| header.reference.starts_with("env:"))
        {
            true
        } else if let Some(binding) =
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Mcp, id)?
        {
            host.project_vault(&project)?
                .contains(
                    &CredentialKey::new("cloud-chat", &binding.record_id())
                        .map_err(|_| CloudError::Storage)?,
                )
                .map_err(|_| CloudError::Storage)?
        } else {
            false
        };
        output.credential_status.mcp.insert(id.clone(), configured);
    }
    Ok(Json(output))
}

fn public_configuration(configuration: ChatProjectConfig) -> SettingsOutput {
    let mut providers = configuration.runtime.providers.clone();
    let mut mcp = configuration.runtime.mcp.clone();
    let mut credential_status = CredentialStatus::default();
    for (name, profile) in &mut providers.profiles {
        credential_status
            .providers
            .insert(name.clone(), profile.credential_reference.is_some());
        profile.credential_reference = None;
    }
    for (name, server) in &mut mcp.servers {
        server.command = std::path::PathBuf::new();
        server.args.clear();
        server.working_directory = None;
        server.environment.clear();
        credential_status
            .mcp
            .insert(name.clone(), !server.credential_headers.is_empty());
        for value in server.credential_headers.values_mut() {
            value.reference = MANAGED_REFERENCE.into();
        }
        if let Some(oauth) = server.oauth.as_mut() {
            oauth.client_secret_reference = None;
        }
    }
    SettingsOutput {
        configuration: SettingsInput {
            revision: configuration.revision,
            labels: Some(configuration.labels),
            providers,
            models: configuration.runtime.models,
            mcp,
            provider_authentication: configuration.provider_authentication,
            mcp_authentication: configuration.mcp_authentication,
        },
        credential_status,
    }
}

async fn replace(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<SettingsInput>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Administer)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host.begin_shared_edit(&state, &caller, true).await?;
    host.refresh_project(&project).await?;
    let mut projects = host.projects.write().await;
    let current = projects.get(&project).ok_or(CloudError::NotFound)?;
    let ceiling = host
        .deployment
        .projects
        .get(&project)
        .ok_or(CloudError::NotFound)?;
    let updated = reviewed_configuration(current, input, ceiling)?;
    if host.active_project_run_status(&project).await? {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_project(&project).await?;
    }
    let encoded =
        zeroize::Zeroizing::new(serde_json::to_vec(&updated).map_err(|_| CloudError::Storage)?);
    let record = VaultRecord::new(encoded.to_vec()).map_err(|_| CloudError::InvalidArgument)?;
    if let Some(edit) = &edit {
        edit.shared.save_configuration(
            &project,
            &updated,
            None,
            Some((&edit.project, &edit.owner)),
        )?;
    } else {
        host.project_vault(&project)?
            .write(&project_record(&project)?, &record)
            .map_err(|_| CloudError::Storage)?;
    }
    let revision = updated.revision;
    projects.insert(project, updated);
    Ok(Json(serde_json::json!({"revision":revision})))
}

/// Load only an authenticated, tenant-bound retained project configuration over its deployment seed.
pub(crate) fn retained_configuration(
    host: &HostedChat,
    project: &str,
    seed: &ChatProjectConfig,
) -> std::result::Result<ChatProjectConfig, CloudError> {
    if let Some(shared) = &host.shared {
        return shared.configuration(project, seed);
    }
    let vault = host.project_vault(project)?;
    let Some(record) = vault
        .read(&project_record(project)?)
        .map_err(|_| CloudError::Storage)?
    else {
        return Ok(seed.clone());
    };
    let configuration: ChatProjectConfig =
        serde_json::from_slice(record.expose()).map_err(|_| CloudError::Storage)?;
    validate_retained(&configuration, seed)?;
    Ok(configuration)
}

pub(crate) fn validate_retained(
    configuration: &ChatProjectConfig,
    seed: &ChatProjectConfig,
) -> std::result::Result<(), CloudError> {
    configuration.validate().map_err(|_| CloudError::Storage)?;
    colossus_runtime::RuntimeConfig::from_yaml(
        &configuration
            .runtime
            .to_yaml()
            .map_err(|_| CloudError::Storage)?,
    )
    .map_err(|_| CloudError::Storage)?;
    // Only the three browser-managed sections may differ from operator authority.
    let mut authority = configuration.runtime.clone();
    authority.providers = seed.runtime.providers.clone();
    authority.models = seed.runtime.models.clone();
    authority.mcp = seed.runtime.mcp.clone();
    if serde_json::to_value(&authority).map_err(|_| CloudError::Storage)?
        != serde_json::to_value(&seed.runtime).map_err(|_| CloudError::Storage)?
        || configuration.codex_auth_path != seed.codex_auth_path
        || configuration.codex_auth_shared != seed.codex_auth_shared
    {
        return Err(CloudError::Storage);
    }
    Ok(())
}

fn reviewed_configuration(
    current: &ChatProjectConfig,
    input: SettingsInput,
    ceiling: &ChatProjectConfig,
) -> std::result::Result<ChatProjectConfig, CloudError> {
    if input.revision != current.revision {
        return Err(CloudError::Conflict);
    }
    if input.providers.profiles.is_empty()
        || input.providers.profiles.len() > 64
        || input.models.profiles.is_empty()
        || input.models.profiles.len() > 256
        || input.models.roles.is_empty()
        || input.models.roles.len() > 32
        || !input.models.roles.contains_key("primary")
        || input.mcp.servers.len() > 64
        || input
            .provider_authentication
            .keys()
            .any(|id| !input.providers.profiles.contains_key(id))
        || input
            .mcp_authentication
            .keys()
            .any(|id| !input.mcp.servers.contains_key(id))
    {
        return Err(CloudError::InvalidArgument);
    }
    let mut updated = current.clone();
    updated.revision = current
        .revision
        .checked_add(1)
        .ok_or(CloudError::ResourceExhausted)?;
    updated.runtime.providers = input.providers;
    updated.runtime.models = input.models;
    updated.runtime.mcp = input.mcp;
    updated.labels = input.labels.unwrap_or_else(|| {
        let mut labels = current.labels.clone();
        labels
            .providers
            .retain(|id, _| updated.runtime.providers.profiles.contains_key(id));
        labels
            .models
            .retain(|id, _| updated.runtime.models.profiles.contains_key(id));
        labels
    });
    updated
        .labels
        .validate(&updated.runtime)
        .map_err(|_| CloudError::InvalidArgument)?;
    updated.runtime.mcp.oauth_credential_store =
        colossus_mcp::McpOAuthCredentialStoreKind::Platform;
    updated.provider_authentication = input.provider_authentication;
    updated.mcp_authentication = input.mcp_authentication;
    for (name, profile) in &mut updated.runtime.providers.profiles {
        colossus_cloud::validate_identifier(name)?;
        if profile
            .credential_reference
            .as_deref()
            .is_some_and(|reference| reference != MANAGED_REFERENCE)
        {
            return Err(CloudError::InvalidArgument);
        }
        let prior = current.runtime.providers.profiles.get(name);
        let same_destination = prior
            .is_some_and(|old| old.kind == profile.kind && old.base_url == profile.base_url)
            && current
                .provider_authentication
                .get(name)
                .copied()
                .unwrap_or_default()
                == updated
                    .provider_authentication
                    .get(name)
                    .copied()
                    .unwrap_or_default();
        profile.credential_reference = if same_destination {
            prior.and_then(|old| old.credential_reference.clone())
        } else {
            None
        };
        if profile.kind.generation_action() == "provider.openai.codex" {
            // Browser administration cannot enroll, repoint or import a workstation account.
            if !same_destination
                || (current.codex_auth_path.is_none() && !current.codex_auth_shared)
            {
                return Err(CloudError::InvalidArgument);
            }
            profile.credential_reference = Some("codex:default".into());
        } else if profile.kind.generation_action() != "provider.echo" {
            allowed_endpoint(
                profile
                    .base_url
                    .as_deref()
                    .ok_or(CloudError::InvalidArgument)?,
                ceiling,
                prior.is_some_and(|old| old.base_url == profile.base_url)
                    && ceiling
                        .runtime
                        .providers
                        .profiles
                        .get(name)
                        .is_some_and(|seed| seed.base_url == profile.base_url),
            )?;
        }
        if !same_destination {
            let old = current.generation(ChatConnectionKind::Provider, name);
            updated.provider_generations.insert(
                name.clone(),
                old.checked_add(1).ok_or(CloudError::ResourceExhausted)?,
            );
        }
    }
    for (name, model) in &updated.runtime.models.profiles {
        colossus_cloud::validate_identifier(name)?;
        if !updated
            .runtime
            .providers
            .profiles
            .contains_key(&model.provider_profile)
            || model.model.trim().is_empty()
            || model.model.len() > 1024
            || model.model.chars().any(char::is_control)
            || model.context_window_tokens < 1024
            || model.max_output_tokens == 0
            || model.max_output_tokens >= model.context_window_tokens
        {
            return Err(CloudError::InvalidArgument);
        }
    }
    for (role, profile) in &updated.runtime.models.roles {
        colossus_cloud::validate_identifier(role)?;
        if !updated.runtime.models.profiles.contains_key(profile) {
            return Err(CloudError::InvalidArgument);
        }
    }
    for (name, server) in &mut updated.runtime.mcp.servers {
        colossus_cloud::validate_identifier(name)?;
        if server.transport != McpTransportKind::StreamableHttp
            || !server.command.as_os_str().is_empty()
            || !server.args.is_empty()
            || server.working_directory.is_some()
            || !server.environment.is_empty()
            || server.allowed_tools.is_empty()
            || server.allowed_tools.len() > 1024
            || server.allowed_tools.iter().any(|tool| {
                tool.is_empty()
                    || tool.len() > 256
                    || tool.contains('*')
                    || tool.chars().any(char::is_control)
            })
        {
            return Err(CloudError::InvalidArgument);
        }
        allowed_endpoint(
            server.url.as_deref().ok_or(CloudError::InvalidArgument)?,
            ceiling,
            current
                .runtime
                .mcp
                .servers
                .get(name)
                .is_some_and(|old| old.url == server.url)
                && ceiling
                    .runtime
                    .mcp
                    .servers
                    .get(name)
                    .is_some_and(|seed| seed.url == server.url),
        )?;
        if server.headers.iter().any(|(name, value)| {
            let name = name.to_ascii_lowercase();
            name.contains("auth")
                || name.contains("token")
                || name.contains("key")
                || name == "cookie"
                || value.len() > 4096
                || value.chars().any(char::is_control)
        }) {
            return Err(CloudError::InvalidArgument);
        }
        let prior = current.runtime.mcp.servers.get(name);
        let same_destination = prior.is_some_and(|old| {
            old.url == server.url
                && oauth_identity(old.oauth.as_ref()) == oauth_identity(server.oauth.as_ref())
        }) && current
            .mcp_authentication
            .get(name)
            .copied()
            .unwrap_or_default()
            == updated
                .mcp_authentication
                .get(name)
                .copied()
                .unwrap_or_default()
            && prior.is_some_and(|old| {
                old.credential_headers
                    .keys()
                    .eq(server.credential_headers.keys())
                    && old.credential_headers.iter().all(|(header, value)| {
                        server
                            .credential_headers
                            .get(header)
                            .is_some_and(|new| new.scheme == value.scheme)
                    })
            });
        for (header, credential) in &mut server.credential_headers {
            if credential.reference != MANAGED_REFERENCE {
                return Err(CloudError::InvalidArgument);
            }
            credential.reference = if same_destination {
                prior
                    .and_then(|old| old.credential_headers.get(header))
                    .map(|old| old.reference.clone())
                    .ok_or(CloudError::InvalidArgument)?
            } else {
                MANAGED_REFERENCE.into()
            };
        }
        if let Some(oauth) = server.oauth.as_mut() {
            if oauth.client_secret_reference.is_some() {
                return Err(CloudError::InvalidArgument);
            }
            if same_destination {
                oauth.client_secret_reference = prior
                    .and_then(|old| old.oauth.as_ref())
                    .and_then(|old| old.client_secret_reference.clone());
            }
        }
        if !same_destination {
            let old = current.generation(ChatConnectionKind::Mcp, name);
            updated.mcp_generations.insert(
                name.clone(),
                old.checked_add(1).ok_or(CloudError::ResourceExhausted)?,
            );
        }
    }
    let serialized = updated
        .runtime
        .to_yaml()
        .map_err(|_| CloudError::InvalidArgument)?;
    updated.runtime = colossus_runtime::RuntimeConfig::from_yaml(&serialized)
        .map_err(|_| CloudError::InvalidArgument)?;
    updated
        .validate()
        .map_err(|_| CloudError::InvalidArgument)?;
    Ok(updated)
}

fn oauth_identity(oauth: Option<&colossus_mcp::McpOAuthConfig>) -> Option<(&str, &[String])> {
    oauth.map(|oauth| (oauth.client_id.as_str(), oauth.scopes.as_slice()))
}
pub(crate) fn allowed_endpoint(
    endpoint: &str,
    ceiling: &ChatProjectConfig,
    retained_operator_endpoint: bool,
) -> std::result::Result<(), CloudError> {
    let parsed = url::Url::parse(endpoint).map_err(|_| CloudError::InvalidArgument)?;
    let retained_loopback = retained_operator_endpoint
        && parsed.scheme() == "http"
        && match parsed.host() {
            Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
    if (parsed.scheme() != "https" && !retained_loopback)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !ceiling
            .runtime
            .sandbox
            .network_destinations
            .contains(&parsed.origin().ascii_serialization())
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}

async fn save_provider_credential(
    state: Extract<Arc<State>>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    input: Json<CredentialInput>,
) -> Result<Json<serde_json::Value>> {
    save_credential(state, path, headers, input, ChatConnectionKind::Provider).await
}
async fn save_mcp_credential(
    state: Extract<Arc<State>>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    input: Json<CredentialInput>,
) -> Result<Json<serde_json::Value>> {
    save_credential(state, path, headers, input, ChatConnectionKind::Mcp).await
}
async fn save_credential(
    Extract(state): Extract<Arc<State>>,
    Path((project, connection)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CredentialInput>,
    kind: ChatConnectionKind,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Administer)?;
    let secret = HostSecret::new(input.secret).map_err(|_| CloudError::InvalidArgument)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host.begin_shared_edit(&state, &caller, true).await?;
    host.refresh_project(&project).await?;
    let mut projects = host.projects.write().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    if input.revision != configuration.revision {
        return Err(CloudError::Conflict.into());
    }
    let (authentication, endpoint) = match kind {
        ChatConnectionKind::Provider => {
            let profile = configuration
                .runtime
                .providers
                .profiles
                .get(&connection)
                .ok_or(CloudError::NotFound)?;
            (
                configuration
                    .provider_authentication
                    .get(&connection)
                    .copied()
                    .unwrap_or_default(),
                profile
                    .base_url
                    .as_deref()
                    .ok_or(CloudError::InvalidArgument)?,
            )
        }
        ChatConnectionKind::Mcp => {
            let server = configuration
                .runtime
                .mcp
                .servers
                .get(&connection)
                .ok_or(CloudError::NotFound)?;
            if server.oauth.is_some() || server.credential_headers.len() != 1 {
                return Err(CloudError::InvalidArgument.into());
            }
            (
                configuration
                    .mcp_authentication
                    .get(&connection)
                    .copied()
                    .unwrap_or_default(),
                server.url.as_deref().ok_or(CloudError::InvalidArgument)?,
            )
        }
    };
    if authentication != ChatAuthentication::Shared {
        return Err(CloudError::PermissionDenied.into());
    }
    let binding = ChatConnectionBinding::new(
        &project,
        caller.subject(),
        kind,
        &connection,
        configuration.generation(kind, &connection),
        endpoint,
        authentication,
    )
    .map_err(|_| CloudError::InvalidArgument)?
    .ok_or(CloudError::InvalidArgument)?;
    let vault = host.project_vault(&project)?;
    if host.active_project_run_status(&project).await? {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_project(&project).await?;
        ChatCredentialResolver::save(vault.as_ref(), &binding, &secret)
            .map_err(|_| CloudError::Storage)?;
    }
    let reference = ChatCredentialResolver::reference(&binding);
    let mut updated = configuration.clone();
    updated.revision = updated
        .revision
        .checked_add(1)
        .ok_or(CloudError::ResourceExhausted)?;
    match kind {
        ChatConnectionKind::Provider => {
            updated
                .runtime
                .providers
                .profiles
                .get_mut(&connection)
                .ok_or(CloudError::NotFound)?
                .credential_reference = Some(reference)
        }
        ChatConnectionKind::Mcp => {
            updated
                .runtime
                .mcp
                .servers
                .get_mut(&connection)
                .ok_or(CloudError::NotFound)?
                .credential_headers
                .values_mut()
                .next()
                .ok_or(CloudError::InvalidArgument)?
                .reference = reference
        }
    }
    let record = VaultRecord::new(serde_json::to_vec(&updated).map_err(|_| CloudError::Storage)?)
        .map_err(|_| CloudError::InvalidArgument)?;
    if let Some(edit) = &edit {
        let key = CredentialKey::new("cloud-chat", &binding.record_id())
            .map_err(|_| CloudError::Storage)?;
        let secret_record = VaultRecord::new(secret.expose().as_bytes().to_vec())
            .map_err(|_| CloudError::InvalidArgument)?;
        edit.shared.save_configuration(
            &project,
            &updated,
            Some((&key, &secret_record)),
            Some((&edit.project, &edit.owner)),
        )?;
    } else {
        vault
            .write(&project_record(&project)?, &record)
            .map_err(|_| CloudError::Storage)?;
    }
    let revision = updated.revision;
    projects.insert(project, updated);
    Ok(Json(serde_json::json!({"revision":revision})))
}

#[cfg(test)]
mod tests;
