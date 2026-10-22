//! Browser credential metadata and writes; storing a secret grants no runtime authority.
use crate::{config::ChatProjectConfig, hosted::HostedExecution, http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudCaller, CloudError, CloudPermission};
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, CredentialStore,
    StoredCredentialKind, StoredCredentialMetadata,
};
use colossus_contracts::{CredentialError, HostSecret};
use colossus_ports::CredentialVault;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod inventory;
use inventory::{Connection, Snapshot, connections, snapshot};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scope {
    Personal,
    Project,
}
impl Scope {
    fn authentication(self) -> ChatAuthentication {
        match self {
            Self::Personal => ChatAuthentication::Personal,
            Self::Project => ChatAuthentication::Shared,
        }
    }
    fn authorize(self, caller: &CloudCaller, write: bool) -> std::result::Result<(), CloudError> {
        match self {
            Self::Project => caller.require(CloudPermission::Administer),
            Self::Personal if write => {
                caller.require(CloudPermission::Execute)?;
                caller.require(CloudPermission::Control)
            }
            Self::Personal => caller.require(CloudPermission::Read),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteCredential {
    revision: u64,
    label: String,
    kind: StoredCredentialKind,
    secret: Option<String>,
    attachment: Option<Attachment>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    kind: ChatConnectionKind,
    connection: String,
    credential_binding: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachCredential {
    revision: u64,
    credential_id: Option<String>,
    credential_binding: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: u64,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route(
            "/api/projects/{project}/chat/credential-store/{scope}",
            get(list).post(create),
        )
        .route(
            "/api/projects/{project}/chat/credential-store/{scope}/{id}",
            post(update).delete(remove),
        )
        .route(
            "/api/projects/{project}/chat/credential-store/{scope}/attachments/{kind}/{connection}",
            post(attach),
        )
        .route("/api/projects/{project}/chat/accounts", get(accounts))
}

fn hosted(state: &State) -> std::result::Result<&HostedExecution, CloudError> {
    state.hosted.as_deref().ok_or(CloudError::NotFound)
}
fn custody(error: CredentialError) -> CloudError {
    match error {
        CredentialError::InvalidInput => CloudError::InvalidArgument,
        CredentialError::Oversized => CloudError::ResourceExhausted,
        _ => CloudError::Storage,
    }
}
fn check_revision(store: &CredentialStore, revision: u64) -> std::result::Result<(), CloudError> {
    if revision == store.revision() {
        Ok(())
    } else {
        Err(CloudError::Conflict)
    }
}
fn adopt(
    store: &mut CredentialStore,
    vault: &dyn CredentialVault,
    rows: &[Connection],
    id: &str,
) -> std::result::Result<(), CloudError> {
    if store.metadata().any(|entry| entry.id == id) {
        return Ok(());
    }
    let row = rows
        .iter()
        .find(|row| row.legacy_id() == id && store.attachment(&row.binding).is_none())
        .ok_or(CloudError::NotFound)?;
    store
        .adopt_legacy(
            vault,
            &row.binding,
            &row.label.chars().take(160).collect::<String>(),
            StoredCredentialKind::ApiKey,
        )
        .map_err(custody)?;
    Ok(())
}

async fn list(
    Extract(state): Extract<Arc<State>>,
    Path((project, scope)): Path<(String, Scope)>,
    headers: HeaderMap,
) -> Result<Json<Snapshot>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    scope.authorize(&caller, false)?;
    let host = hosted(&state)?;
    let config = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let vault = host.selected_vault(&caller, scope.authentication())?;
    let store = CredentialStore::load(vault.as_ref()).map_err(custody)?;
    Ok(Json(snapshot(
        host,
        &caller,
        &config,
        scope,
        vault.as_ref(),
        &store,
    )?))
}

async fn create(
    Extract(state): Extract<Arc<State>>,
    Path((project, scope)): Path<(String, Scope)>,
    headers: HeaderMap,
    Json(input): Json<WriteCredential>,
) -> Result<Json<serde_json::Value>> {
    write(state, project, scope, headers, None, input).await
}
async fn update(
    Extract(state): Extract<Arc<State>>,
    Path((project, scope, id)): Path<(String, Scope, String)>,
    headers: HeaderMap,
    Json(input): Json<WriteCredential>,
) -> Result<Json<serde_json::Value>> {
    write(state, project, scope, headers, Some(id), input).await
}
async fn write(
    state: Arc<State>,
    project: String,
    scope: Scope,
    headers: HeaderMap,
    id: Option<String>,
    input: WriteCredential,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    scope.authorize(&caller, true)?;
    let secret = input
        .secret
        .map(HostSecret::new)
        .transpose()
        .map_err(custody)?;
    if id.is_none() && secret.is_none() {
        return Err(CloudError::InvalidArgument.into());
    }
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let unused_creation = id.is_none() && input.attachment.is_none();
    let edit = if unused_creation {
        host.begin_storage_edit(&caller)?
    } else {
        host.begin_shared_edit(&state, &caller, matches!(scope, Scope::Project))
            .await?
    };
    host.refresh_project(&project).await?;
    let config = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let vault = editing_vault(host, &caller, scope, edit.as_ref())?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(custody)?;
    check_revision(&store, input.revision)?;
    let rows = connections(host, &caller, &config, scope)?;
    let creating = id.is_none();
    let id = id.unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string());
    let metadata = StoredCredentialMetadata {
        id: id.clone(),
        label: input.label.trim().into(),
        kind: input.kind,
    };
    if creating {
        store
            .create(
                metadata,
                secret.as_ref().ok_or(CloudError::InvalidArgument)?,
            )
            .map_err(custody)?;
    } else {
        adopt(&mut store, vault.as_ref(), &rows, &id)?;
        store.update(metadata, secret.as_ref()).map_err(custody)?;
    }
    if let Some(target) = input.attachment {
        let row = rows
            .iter()
            .find(|row| row.kind == target.kind && row.id == target.connection)
            .ok_or(CloudError::NotFound)?;
        if row.binding.record_id() != target.credential_binding {
            return Err(CloudError::Conflict.into());
        }
        store.attach(&row.binding, Some(&id)).map_err(custody)?;
    }
    if !unused_creation {
        stop_local(host, &state, &caller, scope, edit.is_some()).await?;
    }
    store.commit(vault.as_ref()).map_err(custody)?;
    Ok(Json(
        serde_json::json!({"id":id, "revision":store.revision()}),
    ))
}

async fn remove(
    Extract(state): Extract<Arc<State>>,
    Path((project, scope, id)): Path<(String, Scope, String)>,
    headers: HeaderMap,
    Json(input): Json<Revision>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    scope.authorize(&caller, true)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host
        .begin_shared_edit(&state, &caller, matches!(scope, Scope::Project))
        .await?;
    host.refresh_project(&project).await?;
    let config = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let vault = editing_vault(host, &caller, scope, edit.as_ref())?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(custody)?;
    check_revision(&store, input.revision)?;
    let rows = connections(host, &caller, &config, scope)?;
    adopt(&mut store, vault.as_ref(), &rows, &id)?;
    store.remove(&id).map_err(custody)?;
    stop_local(host, &state, &caller, scope, edit.is_some()).await?;
    store.commit(vault.as_ref()).map_err(custody)?;
    Ok(Json(serde_json::json!({"revision":store.revision()})))
}

async fn attach(
    Extract(state): Extract<Arc<State>>,
    Path((project, scope, kind, connection)): Path<(String, Scope, ChatConnectionKind, String)>,
    headers: HeaderMap,
    Json(input): Json<AttachCredential>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    scope.authorize(&caller, true)?;
    let host = hosted(&state)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = host
        .begin_shared_edit(&state, &caller, matches!(scope, Scope::Project))
        .await?;
    host.refresh_project(&project).await?;
    let config = host
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let vault = editing_vault(host, &caller, scope, edit.as_ref())?;
    let mut store = CredentialStore::load(vault.as_ref()).map_err(custody)?;
    check_revision(&store, input.revision)?;
    let rows = connections(host, &caller, &config, scope)?;
    let row = rows
        .iter()
        .find(|row| row.kind == kind && row.id == connection)
        .ok_or(CloudError::NotFound)?;
    if row.binding.record_id() != input.credential_binding {
        return Err(CloudError::Conflict.into());
    }
    if let Some(id) = &input.credential_id {
        adopt(&mut store, vault.as_ref(), &rows, id)?;
    }
    store
        .attach(&row.binding, input.credential_id.as_deref())
        .map_err(custody)?;
    stop_local(host, &state, &caller, scope, edit.is_some()).await?;
    store.commit(vault.as_ref()).map_err(custody)?;
    Ok(Json(serde_json::json!({"revision":store.revision()})))
}

fn editing_vault(
    host: &HostedExecution,
    caller: &CloudCaller,
    scope: Scope,
    edit: Option<&crate::hosted::management::SharedEdit>,
) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
    if let Some(edit) = edit {
        let partition = match scope {
            Scope::Personal => format!(
                "user/{}/{}/connections",
                caller.project_id(),
                caller.subject()
            ),
            Scope::Project => format!("project/{}/connections", caller.project_id()),
        };
        Ok(edit
            .shared
            .editing_vault(&partition, &edit.project, &edit.owner))
    } else {
        host.selected_vault(caller, scope.authentication())
    }
}
async fn stop_local(
    host: &HostedExecution,
    state: &State,
    caller: &CloudCaller,
    scope: Scope,
    shared: bool,
) -> std::result::Result<(), CloudError> {
    if !shared {
        let busy = match scope {
            Scope::Personal => {
                host.user_is_busy(state, caller.project_id(), caller.subject())
                    .await?
            }
            Scope::Project => host.active_project_run_status(caller.project_id()).await?,
        };
        if busy {
            return Err(CloudError::Conflict);
        }
        match scope {
            Scope::Personal => {
                host.stop_user(caller.project_id(), caller.subject())
                    .await?
            }
            Scope::Project => host.stop_project(caller.project_id()).await?,
        }
    }
    Ok(())
}

async fn accounts(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let config = hosted(&state)?
        .project_config(&project)
        .await?
        .ok_or(CloudError::NotFound)?;
    let managed = config.codex_auth_path.is_some() || config.codex_auth_shared;
    Ok(Json(serde_json::json!({"accounts":[{
        "id":"chatgpt", "label":"ChatGPT / Codex", "kind":"chatgpt",
        "status":if managed { "deployment_managed" } else { "setup_required" },
        "can_connect":false,
        "reason":"Personal ChatGPT subscription linking is not implemented on this server. Hosted plan access requires a supported Sign in with ChatGPT integration and integration access; adding an API key does not link a subscription.",
        "setup_url":"https://developers.openai.com/siwc/token-sharing-open-source"
    }]})))
}
