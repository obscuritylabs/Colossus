//! Instance secrets are usable only through an administrator's exact project/connection grant.
use super::{administrator, storage::InstanceEdit};
use crate::{hosted::HostedExecution, http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudError, CloudResult};
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, CredentialStore,
    StoredCredentialKind, StoredCredentialMetadata,
};
use colossus_contracts::HostSecret;
use colossus_ports::CredentialVault;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) fn assigned_vault(
    host: &HostedExecution,
    binding: &ChatConnectionBinding,
) -> CloudResult<Option<Arc<dyn CredentialVault>>> {
    let vault = host.instance_vault()?;
    let store = CredentialStore::load(vault.as_ref()).map_err(|_| CloudError::Storage)?;
    Ok(store.attachment(binding).is_some().then_some(vault))
}
#[derive(Clone, Serialize)]
struct Destination {
    id: String,
    kind: ChatConnectionKind,
    label: String,
    endpoint: String,
    project_id: String,
    credential_binding: String,
    credential_id: Option<String>,
    credential_source: &'static str,
}
#[derive(Serialize)]
struct Credential {
    #[serde(flatten)]
    metadata: StoredCredentialMetadata,
    legacy: bool,
    used_by: Vec<Destination>,
}
#[derive(Serialize)]
struct Snapshot {
    revision: u64,
    credentials: Vec<Credential>,
    connections: Vec<Destination>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    project: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    project_id: String,
    kind: ChatConnectionKind,
    connection: String,
    credential_binding: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    revision: u64,
    label: String,
    kind: StoredCredentialKind,
    secret: Option<String>,
    attachment: Option<Attachment>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    revision: u64,
    credential_id: Option<String>,
    credential_binding: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: u64,
}

pub(super) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/admin/credential-store", get(list).post(create))
        .route(
            "/api/admin/credential-store/{id}",
            post(update).delete(remove),
        )
        .route(
            "/api/admin/credential-store/attachments/{project}/{kind}/{connection}",
            post(attach),
        )
}
async fn destinations(
    host: &HostedExecution,
    project: &str,
    actor: &str,
    store: &CredentialStore,
) -> CloudResult<Vec<(Destination, ChatConnectionBinding)>> {
    host.repo.project(project).await?;
    let Some(config) = host.project_config(project).await? else {
        // A real Cloud project need not have operator-provisioned hosted authority.
        // Its global inventory stays readable, with no eligible grant targets.
        return Ok(Vec::new());
    };
    let caller = administrator(project, actor)?;
    let mut result = Vec::new();
    for (kind, profiles, authentication) in [
        (
            ChatConnectionKind::Provider,
            config.runtime.providers.profiles.keys().collect::<Vec<_>>(),
            &config.provider_authentication,
        ),
        (
            ChatConnectionKind::Mcp,
            config.runtime.mcp.servers.keys().collect(),
            &config.mcp_authentication,
        ),
        (
            ChatConnectionKind::Search,
            config.runtime.search.profiles.keys().collect(),
            &config.search_authentication,
        ),
    ] {
        for id in profiles {
            if !super::network::connection_enabled(&config, kind, id) {
                continue;
            }
            if authentication.get(id).copied().unwrap_or_default() != ChatAuthentication::Shared {
                continue;
            }
            let (endpoint, label, eligible) = match kind {
                ChatConnectionKind::Provider => {
                    let profile = &config.runtime.providers.profiles[id];
                    (
                        profile.base_url.as_deref(),
                        config.labels.providers.get(id).unwrap_or(id),
                        !matches!(
                            profile.kind.generation_action(),
                            "provider.echo" | "provider.openai.codex"
                        ) && !profile
                            .credential_reference
                            .as_deref()
                            .is_some_and(|reference| reference.starts_with("env:")),
                    )
                }
                ChatConnectionKind::Mcp => {
                    let profile = &config.runtime.mcp.servers[id];
                    (
                        profile.url.as_deref(),
                        id,
                        profile.oauth.is_none()
                            && !profile
                                .credential_headers
                                .values()
                                .any(|header| header.reference.starts_with("env:")),
                    )
                }
                ChatConnectionKind::Search => {
                    let profile = &config.runtime.search.profiles[id];
                    (
                        Some(super::search::endpoint(profile)),
                        config.labels.search.get(id).unwrap_or(id),
                        !super::search::reference(profile)
                            .is_some_and(|reference| reference.starts_with("env:")),
                    )
                }
            };
            if !eligible {
                continue;
            }
            let Some(endpoint) = endpoint else {
                continue;
            };
            let binding = host
                .connection_binding(&caller, &config, kind, id)?
                .ok_or(CloudError::InvalidArgument)?;
            let assigned = store.attachment(&binding);
            result.push((
                Destination {
                    id: id.clone(),
                    kind,
                    label: label.clone(),
                    endpoint: endpoint.into(),
                    project_id: project.into(),
                    credential_binding: binding.record_id(),
                    credential_id: assigned.flatten().map(str::to_owned),
                    credential_source: if assigned.is_some() {
                        "global"
                    } else {
                        "project"
                    },
                },
                binding,
            ));
        }
    }
    Ok(result)
}
async fn list(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(selection): Query<Selection>,
) -> Result<Json<Snapshot>> {
    let actor = state.auth.admin(&headers, false).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let vault = host.instance_vault()?;
    let store = CredentialStore::load(vault.as_ref()).map_err(|_| CloudError::Storage)?;
    let connections = if let Some(project) = selection.project {
        destinations(host, &project, &actor.id, &store)
            .await?
            .into_iter()
            .map(|(row, _)| row)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let credentials = store
        .metadata()
        .map(|metadata| Credential {
            metadata: metadata.clone(),
            legacy: false,
            used_by: connections
                .iter()
                .filter(|row| row.credential_id.as_deref() == Some(metadata.id.as_str()))
                .cloned()
                .collect(),
        })
        .collect();
    Ok(Json(Snapshot {
        revision: store.revision(),
        credentials,
        connections,
    }))
}
async fn create(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(input): Json<Write>,
) -> Result<Json<serde_json::Value>> {
    write(state, headers, None, input).await
}
async fn update(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Write>,
) -> Result<Json<serde_json::Value>> {
    write(state, headers, Some(id), input).await
}
async fn write(
    state: Arc<State>,
    headers: HeaderMap,
    id: Option<String>,
    input: Write,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = InstanceEdit::begin(
        host,
        &state,
        &actor.id,
        id.is_some() || input.attachment.is_some(),
    )
    .await?;
    let vault = edit.vault(host, &actor.id, "instance.credential.write")?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(|_| CloudError::Storage)?;
    if store.revision() != input.revision {
        return Err(CloudError::Conflict.into());
    }
    let secret = input
        .secret
        .map(HostSecret::new)
        .transpose()
        .map_err(|_| CloudError::InvalidArgument)?;
    let creating = id.is_none();
    let id = id.unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string());
    let metadata = StoredCredentialMetadata {
        id: id.clone(),
        label: input.label.trim().into(),
        kind: input.kind,
    };
    if creating {
        store.create(
            metadata,
            secret.as_ref().ok_or(CloudError::InvalidArgument)?,
        )
    } else {
        store.update(metadata, secret.as_ref())
    }
    .map_err(|_| CloudError::InvalidArgument)?;
    if let Some(target) = input.attachment {
        let binding = target_binding(
            host,
            &actor.id,
            &store,
            &target.project_id,
            target.kind,
            &target.connection,
            &target.credential_binding,
        )
        .await?;
        store
            .attach(&binding, Some(&id))
            .map_err(|_| CloudError::InvalidArgument)?;
    }
    store
        .commit(vault.as_ref())
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(
        serde_json::json!({"id":id,"revision":store.revision()}),
    ))
}
async fn target_binding(
    host: &HostedExecution,
    actor: &str,
    store: &CredentialStore,
    project: &str,
    kind: ChatConnectionKind,
    connection: &str,
    expected: &str,
) -> CloudResult<ChatConnectionBinding> {
    let (_, binding) = destinations(host, project, actor, store)
        .await?
        .into_iter()
        .find(|(row, _)| row.kind == kind && row.id == connection)
        .ok_or(CloudError::NotFound)?;
    if binding.record_id() != expected {
        return Err(CloudError::Conflict);
    }
    Ok(binding)
}
async fn attach(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path((project, kind, connection)): Path<(String, ChatConnectionKind, String)>,
    Json(input): Json<Grant>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = InstanceEdit::begin(host, &state, &actor.id, true).await?;
    let vault = edit.vault(host, &actor.id, "instance.credential.assign")?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(|_| CloudError::Storage)?;
    if store.revision() != input.revision {
        return Err(CloudError::Conflict.into());
    }
    let binding = target_binding(
        host,
        &actor.id,
        &store,
        &project,
        kind,
        &connection,
        &input.credential_binding,
    )
    .await?;
    store
        .attach(&binding, input.credential_id.as_deref())
        .map_err(|_| CloudError::InvalidArgument)?;
    store
        .commit(vault.as_ref())
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(serde_json::json!({"revision":store.revision()})))
}
async fn remove(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Revision>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = InstanceEdit::begin(host, &state, &actor.id, true).await?;
    let vault = edit.vault(host, &actor.id, "instance.credential.remove")?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(|_| CloudError::Storage)?;
    if store.revision() != input.revision {
        return Err(CloudError::Conflict.into());
    }
    store.remove(&id).map_err(|_| CloudError::NotFound)?;
    store
        .commit(vault.as_ref())
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(serde_json::json!({"revision":store.revision()})))
}
