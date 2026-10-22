//! Browser OAuth coordination shared across HTTP replicas; protocol sessions stay with their fenced owner.
use super::management::Management;
use super::*;
use colossus_cloud_chat::{ChatAuthentication, ChatConnectionKind, ChatCredentialResolver};
use colossus_cloud_postgres::ChatLease;
use sha2::{Digest as _, Sha256};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Flow {
    project: String,
    user: String,
    session: String,
    revision: u64,
    connection: String,
    runtime_connection: String,
    flow_id: String,
    binding: String,
    lease: ChatLease,
    expires_at: u64,
}

pub(super) async fn begin_with_session(
    state: &Arc<State>,
    caller: &CloudCaller,
    connection: &str,
    session: String,
) -> std::result::Result<serde_json::Value, CloudError> {
    let host = service(state)?;
    host.ensure(state, caller).await?;
    let configuration = host
        .project_config(caller.project_id())
        .await?
        .ok_or(CloudError::NotFound)?;
    let server = configuration
        .runtime
        .mcp
        .servers
        .get(connection)
        .ok_or(CloudError::NotFound)?;
    if configuration
        .mcp_authentication
        .get(connection)
        .copied()
        .unwrap_or_default()
        != ChatAuthentication::Personal
        || server.oauth.is_none()
    {
        return Err(CloudError::PermissionDenied);
    }
    let shared = host.shared.as_ref().ok_or(CloudError::Storage)?;
    let namespace = ChatRuntime::identity_for(caller.project_id(), caller.subject())
        .map_err(|_| CloudError::InvalidArgument)?;
    let lease = shared.sql.read_lease(&namespace)?;
    begin_bound(
        state,
        caller,
        connection,
        configuration,
        lease,
        uuid::Uuid::now_v7().simple().to_string(),
        session,
    )
    .await
}

async fn begin_bound(
    state: &Arc<State>,
    caller: &CloudCaller,
    connection: &str,
    configuration: ChatProjectConfig,
    lease: ChatLease,
    flow_id: String,
    session: String,
) -> std::result::Result<serde_json::Value, CloudError> {
    let host = service(state)?;
    let shared = host.shared.as_ref().ok_or(CloudError::Storage)?;
    let binding = hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            caller.project_id(),
            caller.subject(),
            &session,
            configuration.revision,
            connection,
            configuration.generation(ChatConnectionKind::Mcp, connection),
            lease.generation,
        ))
        .map_err(|_| CloudError::Storage)?,
    ));
    let runtime_connection = super::credentials::runtime_mcp_name(&configuration, connection);
    let callback = format!(
        "{}/auth/mcp/callback",
        state.config.public_origin.trim_end_matches('/')
    );
    let login = host
        .management_request(
            &lease,
            Management::OAuthBegin {
                server: runtime_connection.clone(),
                flow_id: flow_id.clone(),
                binding: binding.clone(),
                callback,
                local_development: state.config.local_development,
            },
        )
        .await?;
    let login: colossus_mcp::McpOAuthBrowserLogin =
        serde_json::from_value(login).map_err(|_| CloudError::Storage)?;
    let token = url::Url::parse(&login.authorization_url)
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
        })
        .ok_or(CloudError::Storage)?;
    let id = hex::encode(Sha256::digest(token.as_bytes()));
    let flow = Flow {
        project: caller.project_id().into(),
        user: caller.subject().into(),
        session,
        revision: configuration.revision,
        connection: connection.into(),
        runtime_connection: runtime_connection.clone(),
        flow_id: flow_id.clone(),
        binding: binding.clone(),
        lease: lease.clone(),
        expires_at: login.expires_at,
    };
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(&flow).map_err(|_| CloudError::Storage)?);
    let ciphertext = shared.seal("oauth-browser", &id, 1, &bytes)?;
    if let Err(error) = shared
        .sql
        .create_oauth_flow(&id, &ciphertext, &lease.namespace)
    {
        let _ = host
            .management_request(
                &lease,
                Management::OAuthCancel {
                    server: runtime_connection,
                    flow_id,
                    binding,
                },
            )
            .await;
        return Err(error);
    }
    Ok(serde_json::json!({"authorization_url":login.authorization_url}))
}

pub(super) async fn callback(
    state: &Arc<State>,
    headers: &HeaderMap,
    uri: &axum::http::Uri,
    state_token: &str,
) -> std::result::Result<(), CloudError> {
    let host = service(state)?;
    let shared = host.shared.as_ref().ok_or(CloudError::Storage)?;
    let id = hex::encode(Sha256::digest(state_token.as_bytes()));
    let record = shared.sql.read_record("oauth-browser", &id)?;
    let ciphertext = record.ciphertext.ok_or(CloudError::PermissionDenied)?;
    let plaintext = shared.unseal("oauth-browser", &id, record.revision, &ciphertext)?;
    let flow: Flow = serde_json::from_slice(&plaintext).map_err(|_| CloudError::Storage)?;
    let caller = state.auth.caller(headers, &flow.project, false).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    if caller.subject() != flow.user
        || state.auth.session_binding(headers).await? != flow.session
        || flow.expires_at <= now()
    {
        return Err(CloudError::PermissionDenied);
    }
    let configuration = host
        .project_config(&flow.project)
        .await?
        .ok_or(CloudError::NotFound)?;
    if configuration.revision != flow.revision {
        return Err(CloudError::Conflict);
    }
    shared.sql.verify_lease(&flow.lease)?;
    // Authorization and exact binding checks precede single-use consumption.
    shared
        .sql
        .take_record("oauth-browser", &id, record.revision)?;
    let callback = format!(
        "{}/auth/mcp/callback?{}",
        state.config.public_origin.trim_end_matches('/'),
        uri.query().ok_or(CloudError::InvalidArgument)?
    );
    host.management_request(
        &flow.lease,
        Management::OAuthComplete {
            server: flow.runtime_connection,
            flow_id: flow.flow_id,
            binding: flow.binding,
            callback,
        },
    )
    .await?;
    Ok(())
}

pub(super) async fn disconnect(
    state: &Arc<State>,
    caller: &CloudCaller,
    connection: &str,
    input: super::oauth::Disconnect,
) -> std::result::Result<serde_json::Value, CloudError> {
    let host = service(state)?;
    let configuration = host
        .project_config(caller.project_id())
        .await?
        .ok_or(CloudError::NotFound)?;
    if configuration
        .mcp_authentication
        .get(connection)
        .copied()
        .unwrap_or_default()
        != ChatAuthentication::Personal
    {
        return Err(CloudError::PermissionDenied);
    }
    let server = configuration
        .runtime
        .mcp
        .servers
        .get(connection)
        .ok_or(CloudError::NotFound)?;
    let before = if server.oauth.is_some() {
        host.ensure(state, caller).await?;
        Some(Management::OAuthLogout {
            server: super::credentials::runtime_mcp_name(&configuration, connection),
        })
    } else {
        None
    };
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host
        .begin_shared_edit_with(state, caller, false, before)
        .await?
        .ok_or(CloudError::Storage)?;
    if server.oauth.is_none() {
        let binding = host
            .connection_binding(caller, &configuration, ChatConnectionKind::Mcp, connection)?
            .ok_or(CloudError::InvalidArgument)?;
        if input.credential_binding.as_deref() != Some(binding.record_id().as_str()) {
            return Err(CloudError::Conflict);
        }
        let vault = edit.shared.editing_vault(
            &format!(
                "user/{}/{}/connections",
                caller.project_id(),
                caller.subject()
            ),
            &edit.project,
            &edit.owner,
        );
        ChatCredentialResolver::disconnect(vault.as_ref(), &binding)
            .map_err(|_| CloudError::Storage)?;
    }
    Ok(serde_json::json!({"disconnected":true}))
}
