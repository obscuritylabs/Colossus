//! Personal Chat presentation over the independent Hosted execution service.

use crate::hosted::oauth;
use crate::{hosted::HostedExecution, http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudError, CloudPermission};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

fn service(state: &State) -> std::result::Result<&Arc<HostedExecution>, CloudError> {
    state.hosted.as_ref().ok_or(CloudError::NotFound)
}

async fn status(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ChatStatus>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let Some(chat) = &state.hosted else {
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
    let node_id = service(&state)?
        .register_target(&state, &caller)
        .await?
        .node_id;
    let _lifecycle = service(&state)?.lifecycle.lock().await;
    service(&state)?
        .require_live_submission(&caller, &node_id)
        .await?;
    let (thread, task) = state
        .repo
        .create_personal_thread(&caller, &node_id, None, input.request)
        .await?;
    service(&state)?.notify();
    Ok(Json(serde_json::json!({"thread":thread,"task":task})))
}
