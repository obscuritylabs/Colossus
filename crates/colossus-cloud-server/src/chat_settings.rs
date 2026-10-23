//! Project-managed chat connections. Browser DTOs never carry host configuration authority.
use crate::{
    config::{ChatProjectConfig, ChatSettingsLabels},
    hosted::HostedExecution,
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
pub(crate) mod availability;
mod credentials;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsInput {
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
    #[serde(default)]
    search: Option<colossus_runtime::SearchConfig>,
    #[serde(default)]
    search_authentication: Option<BTreeMap<String, ChatAuthentication>>,
    #[serde(default)]
    observability: Option<colossus_runtime::ObservabilityConfig>,
    #[serde(default)]
    variables: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing)]
    inheritance: Option<crate::global_settings::types::InheritanceInput>,
    #[serde(default, skip_serializing)]
    global_revision: Option<u64>,
    // Released runtime status is accepted for GET/PUT compatibility and ignored.
    #[serde(default, skip_serializing, rename = "telemetry")]
    _telemetry: Option<serde_json::Value>,
    #[serde(default, skip_serializing, rename = "execution")]
    _execution: Option<serde_json::Value>,
}
#[derive(Serialize)]
struct SettingsOutput {
    #[serde(flatten)]
    configuration: SettingsInput,
    credential_status: CredentialStatus,
    inheritance: crate::global_settings::types::Inheritance,
    execution: availability::ExecutionAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    telemetry: Option<crate::telemetry_host::TelemetryStatus>,
}
#[derive(Default, Serialize)]
struct CredentialStatus {
    providers: BTreeMap<String, bool>,
    mcp: BTreeMap<String, bool>,
    search: BTreeMap<String, bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialInput {
    revision: u64,
    secret: String,
    #[serde(default)]
    credential_binding: Option<String>,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route(
            "/api/projects/{project}/chat/settings/provider-presets",
            get(crate::hosted::catalog::presets),
        )
        .route(
            "/api/projects/{project}/chat/settings/providers/{connection}/models",
            post(crate::hosted::catalog::models),
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

fn hosted(state: &State) -> std::result::Result<&HostedExecution, CloudError> {
    state.hosted.as_deref().ok_or(CloudError::NotFound)
}

async fn settings(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SettingsOutput>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Administer)?;
    let host = hosted(&state)?;
    let Some(configuration) = host.project_config(&project).await? else {
        // Cloud project creation grants no runtime or network authority. Release
        // the reusable definitions so administrators can review inherited setup
        // without confusing an unprovisioned project with a missing resource.
        let catalog = crate::global_settings::catalog(host)?;
        let projection = crate::global_settings::inheritance::catalog_only(&catalog);
        let mut output = public_configuration(projection);
        output.execution = availability::ExecutionAvailability::dormant();
        output.credential_status = CredentialStatus::default();
        output.telemetry = Some(
            state
                .telemetry
                .configure(&catalog.observability, catalog.revision),
        );
        return Ok(Json(output));
    };
    let mut output = public_configuration(configuration.clone());
    output.telemetry = Some(state.telemetry.configure(
        &configuration.runtime.observability,
        configuration.inheritance.global_revision,
    ));
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
        } else if let Some(binding) =
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Provider, id)?
        {
            shared_credential_configured(
                host,
                &project,
                &binding,
                profile
                    .credential_reference
                    .as_deref()
                    .is_some_and(|reference| reference.starts_with("env:")),
            )?
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
        } else if let Some(binding) =
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Mcp, id)?
        {
            shared_credential_configured(
                host,
                &project,
                &binding,
                server
                    .credential_headers
                    .values()
                    .any(|header| header.reference.starts_with("env:")),
            )?
        } else {
            false
        };
        output.credential_status.mcp.insert(id.clone(), configured);
    }
    for (id, profile) in &configuration.runtime.search.profiles {
        let configured = if crate::global_settings::search::authentication(&configuration, id)
            != ChatAuthentication::Shared
        {
            false
        } else if let Some(binding) =
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Search, id)?
        {
            shared_credential_configured(
                host,
                &project,
                &binding,
                crate::global_settings::search::reference(profile)
                    .is_some_and(|reference| reference.starts_with("env:")),
            )?
        } else {
            false
        };
        output
            .credential_status
            .search
            .insert(id.clone(), configured);
    }
    Ok(Json(output))
}

fn shared_credential_configured(
    host: &HostedExecution,
    project: &str,
    binding: &ChatConnectionBinding,
    environment_configured: bool,
) -> std::result::Result<bool, CloudError> {
    if let Some(vault) = crate::global_settings::credentials::assigned_vault(host, binding)? {
        return ChatCredentialResolver::contains(vault.as_ref(), binding)
            .map_err(|_| CloudError::Storage);
    }
    if environment_configured {
        return Ok(true);
    }
    ChatCredentialResolver::contains(host.project_vault(project)?.as_ref(), binding)
        .map_err(|_| CloudError::Storage)
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
            search: Some(crate::global_settings::search::public_search(
                &configuration.runtime.search,
            )),
            search_authentication: Some(configuration.search_authentication),
            observability: Some(configuration.runtime.observability),
            variables: Some(configuration.variables),
            inheritance: None,
            global_revision: None,
            _telemetry: None,
            _execution: None,
        },
        credential_status,
        inheritance: configuration.inheritance,
        execution: availability::ExecutionAvailability::enabled(),
        telemetry: None,
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
    let global = crate::global_settings::catalog(host)?;
    let updated = reviewed_project_update(current, input, &global, ceiling)?;
    if host.active_project_run_status(&project).await? {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_project(&project).await?;
    }
    let encoded = zeroize::Zeroizing::new(
        serde_json::to_vec(&crate::global_settings::inheritance::local_configuration(
            &updated,
        ))
        .map_err(|_| CloudError::Storage)?,
    );
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
    host: &HostedExecution,
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
    // Partial local catalogs are validated after inherited definitions are resolved.
    // Every other operator-controlled runtime field must retain its exact seed value.
    let mut authority = configuration.runtime.clone();
    authority.providers = seed.runtime.providers.clone();
    authority.models = seed.runtime.models.clone();
    authority.mcp = seed.runtime.mcp.clone();
    authority.search = seed.runtime.search.clone();
    authority.observability = seed.runtime.observability.clone();
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
    if let Some(search) = input.search {
        updated.runtime.search = search;
    }
    if let Some(authentication) = input.search_authentication {
        updated.search_authentication = authentication;
    }
    if let Some(variables) = input.variables {
        updated.variables = variables;
    }
    if input
        .observability
        .as_ref()
        .is_some_and(|value| value != &current.runtime.observability)
    {
        return Err(CloudError::InvalidArgument);
    }
    crate::global_settings::inheritance::validate_variables(&updated.variables)?;
    updated.labels = input.labels.unwrap_or_else(|| {
        let mut labels = current.labels.clone();
        labels
            .providers
            .retain(|id, _| updated.runtime.providers.profiles.contains_key(id));
        labels
            .models
            .retain(|id, _| updated.runtime.models.profiles.contains_key(id));
        labels
            .search
            .retain(|id, _| updated.runtime.search.profiles.contains_key(id));
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
    crate::global_settings::search::review_search(current, &mut updated, ceiling)?;
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

/// Validate a fully resolved snapshot without allocating a new project revision.
pub(crate) fn review_effective(
    current: &ChatProjectConfig,
    ceiling: &ChatProjectConfig,
) -> std::result::Result<ChatProjectConfig, CloudError> {
    let input = public_configuration(current.clone()).configuration;
    let mut reviewed = reviewed_configuration(current, input, ceiling)?;
    reviewed.revision = current.revision;
    Ok(reviewed)
}

fn reconcile<T: Clone + Serialize>(
    input: &mut BTreeMap<String, T>,
    global: &BTreeMap<String, T>,
    previous: &std::collections::BTreeSet<String>,
    reset: &std::collections::BTreeSet<String>,
    disabled: &mut std::collections::BTreeSet<String>,
) -> std::result::Result<std::collections::BTreeSet<String>, CloudError> {
    for id in reset {
        input.insert(
            id.clone(),
            global.get(id).ok_or(CloudError::NotFound)?.clone(),
        );
    }
    for id in global.keys() {
        if input.contains_key(id) {
            disabled.remove(id);
        } else {
            disabled.insert(id.clone());
        }
    }
    let mut overrides = std::collections::BTreeSet::new();
    for (id, value) in input {
        if !reset.contains(id)
            && (previous.contains(id)
                || global
                    .get(id)
                    .map(|inherited| {
                        crate::global_settings::inheritance::equal(value, inherited)
                            .map(|same| !same)
                    })
                    .transpose()?
                    .unwrap_or(true))
        {
            overrides.insert(id.clone());
        }
    }
    Ok(overrides)
}

fn reset_auth(
    authentication: &mut BTreeMap<String, ChatAuthentication>,
    global: &BTreeMap<String, ChatAuthentication>,
    reset: &std::collections::BTreeSet<String>,
) {
    for id in reset {
        if let Some(value) = global.get(id) {
            authentication.insert(id.clone(), *value);
        } else {
            authentication.remove(id);
        }
    }
}

pub(crate) fn reviewed_project_update(
    current: &ChatProjectConfig,
    mut input: SettingsInput,
    global: &crate::global_settings::types::HostedCatalog,
    ceiling: &ChatProjectConfig,
) -> std::result::Result<ChatProjectConfig, CloudError> {
    let request = input.inheritance.take();
    let expected = request
        .as_ref()
        .map(|request| request.global_revision)
        .or(input.global_revision);
    if (global.revision > 0 || expected.is_some()) && expected != Some(global.revision) {
        return Err(CloudError::Conflict);
    }
    let reset = request.map(|request| request.reset).unwrap_or_default();
    let mut disabled = current.catalog_disabled.clone();
    let prior = &current.inheritance.overrides;
    let mut keep = crate::global_settings::types::Sections {
        providers: reconcile(
            &mut input.providers.profiles,
            &global.providers.profiles,
            &prior.providers,
            &reset.providers,
            &mut disabled.providers,
        )?,
        models: reconcile(
            &mut input.models.profiles,
            &global.models.profiles,
            &prior.models,
            &reset.models,
            &mut disabled.models,
        )?,
        mcp: reconcile(
            &mut input.mcp.servers,
            &global.mcp.servers,
            &prior.mcp,
            &reset.mcp,
            &mut disabled.mcp,
        )?,
        variables: reconcile(
            input
                .variables
                .get_or_insert_with(|| current.variables.clone()),
            &global.variables,
            &prior.variables,
            &reset.variables,
            &mut disabled.variables,
        )?,
        roles: reconcile(
            &mut input.models.roles,
            &global.models.roles,
            &prior.roles,
            &reset
                .roles
                .iter()
                .filter(|id| !id.starts_with("search:"))
                .cloned()
                .collect(),
            &mut disabled.roles,
        )?,
        search: Default::default(),
    };
    let search = input.search.get_or_insert_with(|| {
        crate::global_settings::search::public_search(&current.runtime.search)
    });
    keep.search = reconcile(
        &mut search.profiles,
        &global.search.profiles,
        &prior.search,
        &reset.search,
        &mut disabled.search,
    )?;
    let role_resets = reset
        .roles
        .iter()
        .filter_map(|id| id.strip_prefix("search:").map(str::to_owned))
        .collect();
    let prior_roles = prior
        .roles
        .iter()
        .filter_map(|id| id.strip_prefix("search:").map(str::to_owned))
        .collect();
    let mut disabled_search = disabled
        .roles
        .iter()
        .filter_map(|id| id.strip_prefix("search:").map(str::to_owned))
        .collect();
    for role in reconcile(
        &mut search.roles,
        &global.search.roles,
        &prior_roles,
        &role_resets,
        &mut disabled_search,
    )? {
        keep.roles.insert(format!("search:{role}"));
    }
    disabled.roles.retain(|id| !id.starts_with("search:"));
    disabled
        .roles
        .extend(disabled_search.into_iter().map(|id| format!("search:{id}")));
    reset_auth(
        &mut input.provider_authentication,
        &global.provider_authentication,
        &reset.providers,
    );
    reset_auth(
        &mut input.mcp_authentication,
        &global.mcp_authentication,
        &reset.mcp,
    );
    let search_auth = input
        .search_authentication
        .get_or_insert_with(|| current.search_authentication.clone());
    reset_auth(search_auth, &global.search_authentication, &reset.search);
    for (profiles, auth, inherited, reset, overrides) in [
        (
            input.providers.profiles.keys().collect::<Vec<_>>(),
            &input.provider_authentication,
            &global.provider_authentication,
            &reset.providers,
            &mut keep.providers,
        ),
        (
            input.mcp.servers.keys().collect(),
            &input.mcp_authentication,
            &global.mcp_authentication,
            &reset.mcp,
            &mut keep.mcp,
        ),
    ] {
        for id in profiles {
            if !reset.contains(id)
                && auth.get(id).copied().unwrap_or_default()
                    != inherited.get(id).copied().unwrap_or_default()
            {
                overrides.insert(id.clone());
            }
        }
    }
    for (id, profile) in &search.profiles {
        let inherited = global.search.profiles.get(id).map(|profile| {
            crate::global_settings::search::profile_authentication(
                profile,
                global.search_authentication.get(id).copied(),
            )
        });
        let selected = crate::global_settings::search::profile_authentication(
            profile,
            search_auth.get(id).copied(),
        );
        if !reset.search.contains(id) && inherited != Some(selected) {
            keep.search.insert(id.clone());
        }
    }
    if let Some(labels) = input.labels.as_mut() {
        for (labels, inherited, reset, overrides) in [
            (
                &mut labels.providers,
                &global.labels.providers,
                &reset.providers,
                &mut keep.providers,
            ),
            (
                &mut labels.models,
                &global.labels.models,
                &reset.models,
                &mut keep.models,
            ),
            (
                &mut labels.search,
                &global.labels.search,
                &reset.search,
                &mut keep.search,
            ),
        ] {
            for id in reset {
                if let Some(label) = inherited.get(id) {
                    labels.insert(id.clone(), label.clone());
                } else {
                    labels.remove(id);
                }
            }
            for (id, label) in labels {
                if !reset.contains(id) && inherited.get(id) != Some(label) {
                    overrides.insert(id.clone());
                }
            }
        }
    }
    let mut updated = reviewed_configuration(current, input, ceiling)?;
    updated.inheritance.overrides = keep;
    updated.catalog_disabled = disabled;
    let raw = crate::global_settings::inheritance::local_configuration(&updated);
    crate::global_settings::inheritance::resolve(&raw, global, ceiling)
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
    let (authentication, _endpoint) = match kind {
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
        ChatConnectionKind::Search => {
            let profile = configuration
                .runtime
                .search
                .profiles
                .get(&connection)
                .ok_or(CloudError::NotFound)?;
            (
                crate::global_settings::search::authentication(configuration, &connection),
                crate::global_settings::search::endpoint(profile),
            )
        }
    };
    if authentication != ChatAuthentication::Shared {
        return Err(CloudError::PermissionDenied.into());
    }
    let binding = host
        .connection_binding(&caller, configuration, kind, &connection)?
        .ok_or(CloudError::InvalidArgument)?;
    let provenance = match kind {
        ChatConnectionKind::Provider => &configuration.inheritance.provenance.providers,
        ChatConnectionKind::Mcp => &configuration.inheritance.provenance.mcp,
        ChatConnectionKind::Search => &configuration.inheritance.provenance.search,
    };
    let inherited = provenance
        .get(&connection)
        .is_some_and(|source| *source == "global");
    // Instance edits can repoint an inherited definition without changing this
    // project's revision. Only an exact reviewed binding identifies that target.
    if input
        .credential_binding
        .as_deref()
        .map_or(inherited, |reviewed| reviewed != binding.record_id())
    {
        return Err(CloudError::Conflict.into());
    }
    if crate::global_settings::credentials::assigned_vault(host, &binding)?.is_some() {
        return Err(CloudError::PermissionDenied.into());
    }
    let vault = host.project_vault(&project)?;
    if host.active_project_run_status(&project).await? {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_project(&project).await?;
    }
    let mut credentials = colossus_cloud_chat::CredentialStore::load(vault.as_ref())
        .map_err(|_| CloudError::Storage)?;
    credentials
        .save_bound(&binding, &secret)
        .map_err(|_| CloudError::Storage)?;
    let (credential_key, credential_record) = credentials
        .prepare_commit()
        .map_err(|_| CloudError::Storage)?;
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
        ChatConnectionKind::Search => {
            crate::global_settings::search::set_reference(
                updated
                    .runtime
                    .search
                    .profiles
                    .get_mut(&connection)
                    .ok_or(CloudError::NotFound)?,
                Some(reference),
            )?;
        }
    }
    let record = VaultRecord::new(
        serde_json::to_vec(&crate::global_settings::inheritance::local_configuration(
            &updated,
        ))
        .map_err(|_| CloudError::Storage)?,
    )
    .map_err(|_| CloudError::InvalidArgument)?;
    if let Some(edit) = &edit {
        edit.shared.save_configuration(
            &project,
            &updated,
            Some((&credential_key, &credential_record)),
            Some((&edit.project, &edit.owner)),
        )?;
    } else {
        vault
            .write(&credential_key, &credential_record)
            .map_err(|_| CloudError::Storage)?;
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
