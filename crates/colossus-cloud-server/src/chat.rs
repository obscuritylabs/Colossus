//! Personal hosted chat, using the ordinary enrolled runtime transport.

use crate::{
    config::{ChatConfig, ChatProjectConfig},
    http::{Result, now},
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudCaller, CloudError, CloudPermission};
use colossus_cloud_chat::ChatRuntime;
use colossus_home::ConfinedRoot;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::{Mutex, RwLock, watch};

pub(crate) mod catalog;
mod credentials;
mod host;
mod management;
mod oauth;
mod oauth_shared;
pub(crate) mod shared;

pub(crate) struct HostedChat {
    pub(crate) deployment: ChatConfig,
    pub(crate) root: ConfinedRoot,
    pub(crate) repo: colossus_cloud::CloudRepository,
    pub(crate) projects: RwLock<BTreeMap<String, ChatProjectConfig>>,
    pub(crate) live: Mutex<BTreeMap<String, Arc<LiveChat>>>,
    pub(crate) lifecycle: Mutex<()>,
    pub(super) shared: Option<Arc<shared::SharedChat>>,
    closing: AtomicBool,
    oauth: Mutex<BTreeMap<String, oauth::PendingOAuth>>,
    vaults:
        std::sync::Mutex<BTreeMap<String, std::sync::Weak<dyn colossus_ports::CredentialVault>>>,
}

pub(crate) struct LiveChat {
    pub(crate) project_id: String,
    pub(crate) runtime: Arc<ChatRuntime>,
    pub(crate) node_id: String,
    shutdown: watch::Sender<bool>,
    connector: Mutex<Option<tokio::task::JoinHandle<std::result::Result<(), &'static str>>>>,
    lease: Option<colossus_cloud_postgres::ChatLease>,
    management: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Serialize)]
struct ChatStatus {
    enabled: bool,
    node: Option<serde_json::Value>,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/projects/{project}/chat", get(status))
        .route("/api/projects/{project}/chat/connect", post(connect))
        .route(
            "/api/projects/{project}/chat/threads",
            get(threads).post(create_thread),
        )
        .route(
            "/api/projects/{project}/chat/connections",
            get(oauth::connections),
        )
        .route(
            "/api/projects/{project}/chat/connections/{connection}/auth",
            post(oauth::begin),
        )
        .route(
            "/api/projects/{project}/chat/connections/{connection}/disconnect",
            post(oauth::disconnect),
        )
        .route("/auth/mcp/callback", get(oauth::callback))
}

fn service(state: &State) -> std::result::Result<&Arc<HostedChat>, CloudError> {
    state.chat.as_ref().ok_or(CloudError::NotFound)
}

async fn status(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ChatStatus>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let Some(chat) = &state.chat else {
        return Ok(Json(ChatStatus {
            enabled: false,
            node: None,
        }));
    };
    let enabled = chat.project_config(&project).await?.is_some();
    let node = if enabled {
        chat.node(&state, &caller).await?
    } else {
        None
    };
    Ok(Json(ChatStatus { enabled, node }))
}

async fn connect(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ChatStatus>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    let chat = service(&state)?;
    chat.ensure(&state, &caller).await?;
    Ok(Json(ChatStatus {
        enabled: true,
        node: chat.node(&state, &caller).await?,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadPage {
    after: Option<String>,
    query: Option<String>,
    archived: Option<bool>,
    limit: Option<usize>,
}

async fn threads(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<ThreadPage>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let chat = service(&state)?;
    let Some(node) = chat.node_id(&caller)? else {
        return Ok(Json(serde_json::json!({"threads":[],"next_cursor":null})));
    };
    let (threads, next_cursor) = state
        .repo
        .query_threads(
            &caller,
            Some(node),
            page.query,
            Some(page.archived.unwrap_or(false)),
            page.after,
            page.limit.unwrap_or(25),
        )
        .await?;
    // A hosted personal node can only create owner-bound history. Assert the
    // retained records again instead of treating a query filter as ownership proof.
    if threads
        .iter()
        .any(|thread| thread.owner_user_id.as_deref() != Some(caller.subject()))
    {
        return Err(CloudError::Storage.into());
    }
    Ok(Json(
        serde_json::json!({"threads":threads,"next_cursor":next_cursor}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewThread {
    request: colossus_sdk::CreateRunRequest,
}

async fn create_thread(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<NewThread>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    let node_id = service(&state)?.ensure(&state, &caller).await?;
    let _lifecycle = service(&state)?.lifecycle.lock().await;
    service(&state)?
        .require_live_submission(&caller, &node_id)
        .await?;
    let (thread, task) = state
        .repo
        .create_personal_thread(&caller, &node_id, None, input.request)
        .await?;
    Ok(Json(serde_json::json!({"thread":thread,"task":task})))
}
