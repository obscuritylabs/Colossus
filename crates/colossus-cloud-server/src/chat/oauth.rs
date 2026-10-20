use super::*;
use axum::{
    extract::OriginalUri,
    response::{Html, IntoResponse, Response},
};
use colossus_cloud_chat::{ChatAuthentication, ChatConnectionKind, ChatCredentialResolver};
use colossus_ports::CredentialKey;
use sha2::{Digest as _, Sha256};

pub(super) struct PendingOAuth {
    project: String,
    user: String,
    connection: String,
    runtime_connection: String,
    revision: u64,
    session: String,
    flow_id: String,
    binding: String,
    expires_at: u64,
    live: Arc<LiveChat>,
}

pub(super) async fn connections(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let host = service(&state)?;
    host.refresh_project(&project).await?;
    let _lifecycle = host.lifecycle.lock().await;
    let projects = host.projects.read().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    let identity = ChatRuntime::identity_for(&project, caller.subject())
        .map_err(|_| CloudError::InvalidArgument)?;
    let live = host.live.lock().await.get(&identity).cloned();
    let mut connections = Vec::new();
    for (id, server) in &configuration.runtime.mcp.servers {
        let authentication = configuration
            .mcp_authentication
            .get(id)
            .copied()
            .unwrap_or_default();
        let binding = if server.oauth.is_some() {
            None
        } else {
            host.connection_binding(&caller, configuration, ChatConnectionKind::Mcp, id)?
        };
        let authenticated = if authentication == ChatAuthentication::None {
            true
        } else if server.oauth.is_some() {
            if let Some(shared) = &host.shared {
                match shared.sql.read_lease(&identity) {
                    Ok(lease) => host
                        .management_request(
                            &lease,
                            super::management::Management::OAuthStatus {
                                server: super::credentials::runtime_mcp_name(configuration, id),
                            },
                        )
                        .await?
                        .get("authenticated")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    Err(CloudError::NotFound) => false,
                    Err(error) => return Err(error.into()),
                }
            } else if let Some(live) = &live {
                live.runtime
                    .runtime()
                    .mcp_oauth_status(&super::credentials::runtime_mcp_name(configuration, id))
                    .await
                    .map_err(|_| CloudError::Storage)?
                    .authenticated
            } else {
                false
            }
        } else if let Some(binding) = &binding {
            let vault = host.selected_vault(&caller, authentication)?;
            vault
                .contains(
                    &CredentialKey::new("cloud-chat", &binding.record_id())
                        .map_err(|_| CloudError::Storage)?,
                )
                .map_err(|_| CloudError::Storage)?
                || (authentication == ChatAuthentication::Shared
                    && server
                        .credential_headers
                        .values()
                        .any(|header| header.reference.starts_with("env:")))
        } else {
            false
        };
        connections.push(serde_json::json!({"id":id,"label":id,"transport":"streamable_http","authentication":authentication,"authenticated":authenticated,"oauth":server.oauth.is_some(),"allowed_tools":server.allowed_tools,"endpoint":server.url,"credential_binding":binding.map(|binding| binding.record_id())}));
    }
    Ok(Json(serde_json::json!({"connections":connections})))
}

pub(super) async fn begin(
    Extract(state): Extract<Arc<State>>,
    Path((project, connection)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    let host = service(&state)?;
    if host.shared.is_some() {
        let session = state.auth.session_binding(&headers).await?;
        return Ok(Json(
            super::oauth_shared::begin_with_session(&state, &caller, &connection, session).await?,
        ));
    }
    let node_id = host.ensure(&state, &caller).await?;
    let _lifecycle = host.lifecycle.lock().await;
    host.require_live_submission(&caller, &node_id).await?;
    let identity = ChatRuntime::identity_for(&project, caller.subject())
        .map_err(|_| CloudError::InvalidArgument)?;
    let live = host
        .live
        .lock()
        .await
        .get(&identity)
        .cloned()
        .ok_or(CloudError::Conflict)?;
    let projects = host.projects.read().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    let server = configuration
        .runtime
        .mcp
        .servers
        .get(&connection)
        .ok_or(CloudError::NotFound)?;
    if configuration
        .mcp_authentication
        .get(&connection)
        .copied()
        .unwrap_or_default()
        != ChatAuthentication::Personal
        || server.oauth.is_none()
    {
        return Err(CloudError::PermissionDenied.into());
    }
    let session = state.auth.session_binding(&headers).await?;
    let flow_id = uuid::Uuid::now_v7().simple().to_string();
    let binding = hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            &project,
            caller.subject(),
            &session,
            configuration.revision,
            &connection,
            configuration.generation(ChatConnectionKind::Mcp, &connection),
        ))
        .map_err(|_| CloudError::Storage)?,
    ));
    {
        let mut pending = host.oauth.lock().await;
        pending.retain(|_, flow| flow.expires_at > now());
        if pending.len() >= 128
            || pending
                .values()
                .filter(|flow| flow.user == caller.subject())
                .count()
                >= 4
        {
            return Err(CloudError::ResourceExhausted.into());
        }
    }
    let callback = format!(
        "{}/auth/mcp/callback",
        state.config.public_origin.trim_end_matches('/')
    );
    let runtime_server = super::credentials::runtime_mcp_name(configuration, &connection);
    let callback_policy = if state.config.local_development {
        colossus_runtime::McpOAuthBrowserCallbackPolicy::LocalDevelopmentLoopback
    } else {
        colossus_runtime::McpOAuthBrowserCallbackPolicy::HttpsOnly
    };
    let login = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        live.runtime
            .runtime()
            .mcp_oauth_browser_login_begin_with_policy(
                &runtime_server,
                &flow_id,
                &binding,
                &callback,
                callback_policy,
            ),
    )
    .await
    .map_err(|_| CloudError::Storage)?
    .map_err(|_| CloudError::Storage)?;
    let state_token = url::Url::parse(&login.authorization_url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
        })
        .ok_or(CloudError::Storage)?;
    let key = hex::encode(Sha256::digest(state_token.as_bytes()));
    let mut pending = host.oauth.lock().await;
    if pending.len() >= 128
        || pending
            .values()
            .filter(|flow| flow.user == caller.subject())
            .count()
            >= 4
    {
        drop(pending);
        let _ = live
            .runtime
            .runtime()
            .mcp_oauth_browser_login_cancel(&runtime_server, &flow_id, &binding)
            .await;
        return Err(CloudError::ResourceExhausted.into());
    }
    pending.insert(
        key,
        PendingOAuth {
            project,
            user: caller.subject().into(),
            connection,
            runtime_connection: runtime_server,
            revision: configuration.revision,
            session,
            flow_id,
            binding,
            expires_at: login.expires_at,
            live,
        },
    );
    Ok(Json(
        serde_json::json!({"authorization_url":login.authorization_url}),
    ))
}

#[derive(Deserialize)]
pub(super) struct Callback {
    state: String,
}

pub(super) async fn callback(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    query: std::result::Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let result = async {
        let Query(query) = query.map_err(|_| CloudError::InvalidArgument)?;
        if query.state.is_empty() || query.state.len() > 4096 {
            return Err(CloudError::InvalidArgument);
        }
        let host = service(&state)?;
        if host.shared.is_some() {
            return super::oauth_shared::callback(&state, &headers, &uri, &query.state).await;
        }
        // Fence configuration publication and runtime eviction through the exact
        // credential commit. Settings and admission use this same lock order.
        let _lifecycle = host.lifecycle.lock().await;
        let key = hex::encode(Sha256::digest(query.state.as_bytes()));
        // Peek without consuming: another account cannot destroy the owner's flow.
        let (project, user, session, revision) = {
            let pending = host.oauth.lock().await;
            let flow = pending.get(&key).ok_or(CloudError::PermissionDenied)?;
            (
                flow.project.clone(),
                flow.user.clone(),
                flow.session.clone(),
                flow.revision,
            )
        };
        let caller = state.auth.caller(&headers, &project, false).await?;
        caller.require(CloudPermission::Execute)?;
        caller.require(CloudPermission::Control)?;
        if caller.subject() != user || state.auth.session_binding(&headers).await? != session {
            return Err(CloudError::PermissionDenied);
        }
        let projects = host.projects.read().await;
        let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
        if configuration.revision != revision {
            return Err(CloudError::Conflict);
        }
        let flow = host
            .oauth
            .lock()
            .await
            .remove(&key)
            .ok_or(CloudError::PermissionDenied)?;
        if flow.expires_at <= now() {
            return Err(CloudError::PermissionDenied);
        }
        let redirected = format!(
            "{}/auth/mcp/callback?{}",
            state.config.public_origin.trim_end_matches('/'),
            uri.query().ok_or(CloudError::InvalidArgument)?
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            flow.live
                .runtime
                .runtime()
                .mcp_oauth_browser_login_complete(
                    &flow.runtime_connection,
                    &flow.flow_id,
                    &flow.binding,
                    &redirected,
                ),
        )
        .await
        .map_err(|_| CloudError::Storage)?
        .map_err(|_| CloudError::Storage)?;
        Ok::<_, CloudError>(())
    }
    .await;
    let (status, message) = if result.is_ok() {
        (
            axum::http::StatusCode::OK,
            "Connection authenticated. You can close this tab and return to Colossus Chat.",
        )
    } else {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "Connection authentication could not be completed. Return to Colossus Chat and connect again.",
        )
    };
    (status,Html(format!("<!doctype html><html><head><meta name=\"referrer\" content=\"no-referrer\"><title>Colossus connection</title></head><body><p>{message}</p></body></html>"))).into_response()
}

impl HostedChat {
    /// Caller holds lifecycle. Drop retained runtime leases before cache eviction.
    pub(super) async fn cancel_oauth(&self, project: &str, user: Option<&str>) {
        let flows = {
            let mut pending = self.oauth.lock().await;
            let keys = pending
                .iter()
                .filter(|(_, flow)| {
                    flow.project == project && user.is_none_or(|user| flow.user == user)
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| pending.remove(&key))
                .collect::<Vec<_>>()
        };
        for flow in flows {
            let _ = flow
                .live
                .runtime
                .runtime()
                .mcp_oauth_browser_login_cancel(
                    &flow.runtime_connection,
                    &flow.flow_id,
                    &flow.binding,
                )
                .await;
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disconnect {
    pub(super) credential_binding: Option<String>,
}

pub(super) async fn disconnect(
    Extract(state): Extract<Arc<State>>,
    Path((project, connection)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Disconnect>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    let host = service(&state)?;
    if host.shared.is_some() {
        return Ok(Json(
            super::oauth_shared::disconnect(&state, &caller, &connection, input).await?,
        ));
    }
    let _lifecycle = host.lifecycle.lock().await;
    let projects = host.projects.read().await;
    let configuration = projects.get(&project).ok_or(CloudError::NotFound)?;
    if configuration
        .mcp_authentication
        .get(&connection)
        .copied()
        .unwrap_or_default()
        != ChatAuthentication::Personal
    {
        return Err(CloudError::PermissionDenied.into());
    }
    if host
        .user_is_busy(&state, &project, caller.subject())
        .await?
    {
        return Err(CloudError::Conflict.into());
    }
    let server = configuration
        .runtime
        .mcp
        .servers
        .get(&connection)
        .ok_or(CloudError::NotFound)?;
    if server.oauth.is_some() {
        let identity = ChatRuntime::identity_for(&project, caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        let live = host
            .live
            .lock()
            .await
            .get(&identity)
            .cloned()
            .ok_or(CloudError::Conflict)?;
        live.runtime
            .runtime()
            .mcp_oauth_logout(&super::credentials::runtime_mcp_name(
                configuration,
                &connection,
            ))
            .await
            .map_err(|_| CloudError::Storage)?;
        host.oauth.lock().await.retain(|_, flow| {
            !(flow.project == project
                && flow.user == caller.subject()
                && flow.connection == connection)
        });
    } else {
        let binding = host
            .connection_binding(&caller, configuration, ChatConnectionKind::Mcp, &connection)?
            .ok_or(CloudError::InvalidArgument)?;
        if input.credential_binding.as_deref() != Some(binding.record_id().as_str()) {
            return Err(CloudError::Conflict.into());
        }
        host.stop_user(&project, caller.subject()).await?;
        ChatCredentialResolver::disconnect(
            host.user_vault(&project, caller.subject())?.as_ref(),
            &binding,
        )
        .map_err(|_| CloudError::Storage)?;
    }
    Ok(Json(serde_json::json!({"disconnected":true})))
}
