//! Exact personal credential writes, fenced to the endpoint reviewed by this user.
use super::*;
use colossus_cloud::CloudCaller;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PersonalCredential {
    secret: String,
    credential_binding: String,
}

pub(super) async fn providers(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let host = hosted(&state)?;
    let configuration = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let mut providers = Vec::new();
    for (id, profile) in &configuration.runtime.providers.profiles {
        let authentication = configuration
            .provider_authentication
            .get(id)
            .copied()
            .unwrap_or_default();
        let network_key = !matches!(
            profile.kind.generation_action(),
            "provider.echo" | "provider.openai.codex"
        );
        let binding = if network_key && authentication != ChatAuthentication::None {
            host.connection_binding(&caller, &configuration, ChatConnectionKind::Provider, id)?
        } else {
            None
        };
        let authenticated = if let Some(binding) = &binding {
            host.connection_credential_configured(
                &caller,
                binding,
                authentication,
                profile
                    .credential_reference
                    .as_deref()
                    .is_some_and(|value| value.starts_with("env:")),
            )?
        } else {
            true
        };
        providers.push(serde_json::json!({
            "id":id,"label":id,"authentication":authentication,"authenticated":authenticated,
            "requires_credential": network_key && authentication == ChatAuthentication::Personal,
            "endpoint":profile.base_url,"credential_binding":binding.map(|binding| binding.record_id()),
        }));
    }
    Ok(Json(serde_json::json!({"providers":providers})))
}

pub(super) async fn save_provider(
    state: Extract<Arc<State>>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    input: Json<PersonalCredential>,
) -> Result<Json<serde_json::Value>> {
    save(state, path, headers, input, ChatConnectionKind::Provider).await
}
pub(super) async fn save_mcp(
    state: Extract<Arc<State>>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    input: Json<PersonalCredential>,
) -> Result<Json<serde_json::Value>> {
    save(state, path, headers, input, ChatConnectionKind::Mcp).await
}
async fn save(
    Extract(state): Extract<Arc<State>>,
    Path((project, connection)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<PersonalCredential>,
    kind: ChatConnectionKind,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    let secret = HostSecret::new(input.secret).map_err(|_| CloudError::InvalidArgument)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host.begin_shared_edit(&state, &caller, false).await?;
    host.refresh_project(&project).await?;
    let projects = host.projects.read().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    let binding = personal_binding(host, &caller, configuration, kind, &connection)?;
    if binding.record_id() != input.credential_binding {
        return Err(CloudError::Conflict.into());
    }
    if host
        .user_is_busy(&state, &project, caller.subject())
        .await?
    {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_user(&project, caller.subject()).await?;
    }
    let vault = if let Some(edit) = &edit {
        edit.shared.editing_vault(
            &format!("user/{project}/{}/connections", caller.subject()),
            &edit.project,
            &edit.owner,
        )
    } else {
        host.user_vault(&project, caller.subject())?
    };
    ChatCredentialResolver::save(vault.as_ref(), &binding, &secret)
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(serde_json::json!({"authenticated":true})))
}

pub(super) async fn disconnect_provider(
    Extract(state): Extract<Arc<State>>,
    Path((project, connection)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host.begin_shared_edit(&state, &caller, false).await?;
    host.refresh_project(&project).await?;
    let projects = host.projects.read().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    let binding = personal_binding(
        host,
        &caller,
        configuration,
        ChatConnectionKind::Provider,
        &connection,
    )?;
    if host
        .user_is_busy(&state, &project, caller.subject())
        .await?
    {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_user(&project, caller.subject()).await?;
    }
    let vault = if let Some(edit) = &edit {
        edit.shared.editing_vault(
            &format!("user/{project}/{}/connections", caller.subject()),
            &edit.project,
            &edit.owner,
        )
    } else {
        host.user_vault(&project, caller.subject())?
    };
    ChatCredentialResolver::disconnect(vault.as_ref(), &binding)
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(serde_json::json!({"authenticated":false})))
}

fn personal_binding(
    host: &HostedExecution,
    caller: &CloudCaller,
    configuration: &ChatProjectConfig,
    kind: ChatConnectionKind,
    id: &str,
) -> std::result::Result<ChatConnectionBinding, CloudError> {
    let authentication = match kind {
        ChatConnectionKind::Provider => {
            let profile = configuration
                .runtime
                .providers
                .profiles
                .get(id)
                .ok_or(CloudError::NotFound)?;
            if matches!(
                profile.kind.generation_action(),
                "provider.echo" | "provider.openai.codex"
            ) {
                return Err(CloudError::InvalidArgument);
            }
            configuration
                .provider_authentication
                .get(id)
                .copied()
                .unwrap_or_default()
        }
        ChatConnectionKind::Mcp => {
            let server = configuration
                .runtime
                .mcp
                .servers
                .get(id)
                .ok_or(CloudError::NotFound)?;
            if server.oauth.is_some() {
                return Err(CloudError::InvalidArgument);
            }
            configuration
                .mcp_authentication
                .get(id)
                .copied()
                .unwrap_or_default()
        }
        ChatConnectionKind::Search => {
            if !configuration.runtime.search.profiles.contains_key(id) {
                return Err(CloudError::NotFound);
            }
            crate::global_settings::search::authentication(configuration, id)
        }
    };
    if authentication != ChatAuthentication::Personal {
        return Err(CloudError::PermissionDenied);
    }
    host.connection_binding(caller, configuration, kind, id)?
        .ok_or(CloudError::InvalidArgument)
}
