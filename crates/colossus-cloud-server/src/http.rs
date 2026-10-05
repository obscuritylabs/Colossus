use crate::server::{State, db};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State as Extract},
    http::{HeaderMap, StatusCode, header},
    response::{
        Html, IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use colossus_cloud::{CertificateRedemption, CloudError, Enrollment};
use colossus_sdk::{CreateRunRequest, RespondInteractionRequest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, convert::Infallible, sync::Arc, time::Duration};

pub(crate) fn router(state: Arc<State>) -> Router {
    Router::new().route("/health/live",get(||async{"ok"})).route("/health/ready",get(ready))
        .route("/auth/login",get(login)).route("/auth/callback",get(callback)).route("/auth/logout",post(logout))
        .route("/api/me",get(me)).route("/api/enroll",post(enroll))
        .route("/api/projects/{project}/nodes",get(nodes)).route("/api/projects/{project}/invitations",post(invite))
        .route("/api/projects/{project}/nodes/{node}/revoke",post(revoke))
        .route("/api/projects/{project}/tasks",get(tasks).post(create))
        .route("/api/projects/{project}/tasks/{task}",get(task))
        .route("/api/projects/{project}/tasks/{task}/commands/{command}",get(command))
        .route("/api/projects/{project}/tasks/{task}/events",get(events))
        .route("/api/projects/{project}/tasks/{task}/cancel",post(cancel))
        .route("/api/projects/{project}/tasks/{task}/respond",post(respond))
        .fallback_service(tower_http::services::ServeDir::new(&state.config.web_root).not_found_service(tower_http::services::ServeFile::new(state.config.web_root.join("index.html"))))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(header::HeaderName::from_static("x-content-type-options"), header::HeaderValue::from_static("nosniff")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(header::HeaderName::from_static("referrer-policy"),header::HeaderValue::from_static("no-referrer")))
        .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(header::HeaderName::from_static("content-security-policy"),header::HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")))
        .layer(DefaultBodyLimit::max(colossus_cloud_protocol::MAX_PAYLOAD_BYTES))
        .layer(axum::middleware::from_fn_with_state(state.clone(), admission)).with_state(state)
}
async fn admission(
    Extract(state): Extract<Arc<State>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Ok(_permit) = state.http_permits.clone().try_acquire_owned() else {
        return Error(CloudError::ResourceExhausted).into_response();
    };
    let mut response = match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await
    {
        Ok(response) => response,
        Err(_) => Error(CloudError::Storage).into_response(),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
async fn ready(Extract(state): Extract<Arc<State>>) -> Result<&'static str> {
    db(state.repo.clone(), |repo| repo.check_readiness()).await?;
    Ok("ok")
}
struct Error(CloudError);
impl From<CloudError> for Error {
    fn from(error: CloudError) -> Self {
        Self(error)
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0 {
            CloudError::PermissionDenied => StatusCode::FORBIDDEN,
            CloudError::NotFound => StatusCode::NOT_FOUND,
            CloudError::Conflict => StatusCode::CONFLICT,
            CloudError::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            CloudError::InvalidArgument => StatusCode::BAD_REQUEST,
            CloudError::Storage => StatusCode::SERVICE_UNAVAILABLE,
        };
        (
            status,
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({"error":self.0})),
        )
            .into_response()
    }
}
type Result<T> = std::result::Result<T, Error>;
async fn login(Extract(state): Extract<Arc<State>>) -> Result<Response> {
    let (url, cookie) = state.auth.login()?;
    let mut response = Redirect::temporary(&url).into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    state: String,
    code: String,
    #[serde(default)]
    session_state: Option<String>,
    #[serde(default)]
    iss: Option<String>,
}
async fn callback(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    query: std::result::Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return sign_in_failed();
    };
    if query
        .iss
        .as_ref()
        .is_some_and(|issuer| issuer != &state.config.oidc.issuer)
    {
        return sign_in_failed();
    }
    let _ = query.session_state;
    let cookie = match state
        .auth
        .callback(&headers, &query.state, query.code)
        .await
    {
        Ok(cookie) => cookie,
        Err(_) => return sign_in_failed(),
    };
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
fn sign_in_failed() -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(include_str!("sign-in-failed.html")),
    )
        .into_response()
}
async fn logout(Extract(state): Extract<Arc<State>>, headers: HeaderMap) -> Result<Response> {
    let cookie = state.auth.logout(&headers)?;
    Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response())
}
async fn me(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({"memberships":state.auth.memberships(&headers)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<usize>,
}
async fn nodes(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false)?;
    let nodes = db(state.repo.clone(), move |repo| {
        repo.list_nodes(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
    })
    .await?;
    let presence = state.presence.lock().map_err(|_| CloudError::Storage)?;
    let nodes = nodes
        .into_iter()
        .map(|node| {
            let online = presence
                .get(&format!("{}:{}", node.project_id, node.node_id))
                .filter(|online| {
                    online
                        .heartbeat
                        .is_some_and(|time| time.elapsed() < Duration::from_secs(30))
                });
            serde_json::json!({"node":node,"presence":online})
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({"nodes":nodes})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    label: String,
    roles: BTreeSet<String>,
}
async fn invite(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Invite>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true)?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| CloudError::Storage)?;
    let token = hex::encode(bytes);
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    let node_id = uuid::Uuid::now_v7().simple().to_string();
    let invitation = Enrollment {
        token_hash,
        project_id: project,
        node_id: node_id.clone(),
        label: input.label,
        roles: input.roles,
        expires_at: now() + 600,
        redeemed_certificate: None,
        redeemed_csr: None,
        certificate_pem: None,
    };
    db(state.repo.clone(), move |repo| {
        repo.invite(&caller, invitation, now())
    })
    .await?;
    Ok(Json(
        serde_json::json!({"token":token,"node_id":node_id,"expires_in":600,"enrollment_url":format!("{}/api/enroll",state.config.public_origin.trim_end_matches('/'))}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Redeem {
    token: String,
    instance_id: String,
    csr_pem: String,
}
async fn enroll(
    Extract(state): Extract<Arc<State>>,
    Json(input): Json<Redeem>,
) -> Result<Json<serde_json::Value>> {
    if input.token.len() != 64 || !input.token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CloudError::PermissionDenied.into());
    }
    let (certificate, fingerprint) = state
        .ca
        .sign(&input.csr_pem)
        .map_err(|_| CloudError::InvalidArgument)?;
    let token_hash = hex::encode(Sha256::digest(input.token.as_bytes()));
    let csr_sha256 = hex::encode(Sha256::digest(input.csr_pem.as_bytes()));
    let (node, certificate) = db(state.repo.clone(), move |repo| {
        repo.redeem(
            &token_hash,
            &input.instance_id,
            CertificateRedemption {
                fingerprint,
                csr_sha256,
                certificate_pem: certificate,
            },
            now(),
        )
    })
    .await?;
    Ok(Json(
        serde_json::json!({"node":node,"certificate_pem":certificate,"ca_pem":state.ca.pem,"grpc_endpoint":state.config.grpc_endpoint}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: u64,
}
async fn revoke(
    Extract(state): Extract<Arc<State>>,
    Path((project, node)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Revision>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true)?;
    let node = db(state.repo.clone(), move |repo| {
        repo.revoke_node(&caller, &node, input.revision)
    })
    .await?;
    Ok(Json(serde_json::json!({"node":node})))
}
async fn tasks(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false)?;
    let tasks = db(state.repo.clone(), move |repo| {
        repo.list_tasks(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
    })
    .await?;
    Ok(Json(serde_json::json!({"tasks":tasks})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    node_id: String,
    request: CreateRunRequest,
}
async fn create(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Create>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true)?;
    let task = db(state.repo.clone(), move |repo| {
        repo.create_task(&caller, &input.node_id, input.request)
    })
    .await?;
    Ok(Json(serde_json::json!({"task":task})))
}
async fn task(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false)?;
    let task = db(state.repo.clone(), move |repo| {
        repo.get_task(&caller, &task)
    })
    .await?;
    Ok(Json(serde_json::json!({"task":task})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mutation {
    mutation_id: String,
}
async fn command(
    Extract(state): Extract<Arc<State>>,
    Path((project, task, command)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false)?;
    let command = db(state.repo.clone(), move |repo| {
        repo.get_command(&caller, &task, &command)
    })
    .await?;
    Ok(Json(serde_json::json!({"command":command})))
}
async fn cancel(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Mutation>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true)?;
    let command = db(state.repo.clone(), move |repo| {
        repo.cancel_task(&caller, &task, &input.mutation_id)
    })
    .await?;
    Ok(Json(serde_json::json!({"command":command})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Respond {
    mutation_id: String,
    request: RespondInteractionRequest,
}
async fn respond(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Respond>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true)?;
    let command = db(state.repo.clone(), move |repo| {
        repo.respond_task(&caller, &task, &input.mutation_id, input.request)
    })
    .await?;
    Ok(Json(serde_json::json!({"command":command})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    #[serde(default)]
    after: u64,
}
async fn events(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
) -> Result<Sse<impl futures::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let permit = state
        .sse_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    let caller = state.auth.caller(&headers, &project, false)?;
    let after = headers
        .get("last-event-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(CloudError::InvalidArgument)
        })
        .transpose()?
        .unwrap_or(cursor.after);
    let first = db(state.repo.clone(), {
        let task = task.clone();
        let caller = caller.clone();
        move |repo| repo.updates(&caller, &task, after, 100)
    })
    .await?;
    let stream = futures::stream::unfold(
        (state, headers, project, task, caller, after, first, permit),
        |(state, headers, project, task, caller, mut after, mut pending, permit)| async move {
            loop {
                if *state.shutdown.borrow() {
                    return None;
                }
                state.auth.caller(&headers, &project, false).ok()?;
                if let Some(update) = pending.first().cloned() {
                    pending.remove(0);
                    after = update.sequence;
                    let event = Event::default()
                        .id(after.to_string())
                        .event("run_update")
                        .json_data(update)
                        .ok()?;
                    return Some((
                        Ok(event),
                        (
                            state, headers, project, task, caller, after, pending, permit,
                        ),
                    ));
                }
                let mut shutdown = state.shutdown.subscribe();
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(300)) => {},
                    _ = shutdown.changed() => return None,
                }
                state.auth.caller(&headers, &project, false).ok()?;
                pending = db(state.repo.clone(), {
                    let task = task.clone();
                    let caller = caller.clone();
                    move |repo| repo.updates(&caller, &task, after, 100)
                })
                .await
                .ok()?;
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
