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

mod spa;

pub(crate) fn router(state: Arc<State>) -> Router {
    Router::new().route("/health/live",get(||async{"ok"})).route("/health/ready",get(ready))
        .route("/auth/login",get(login)).route("/auth/local",post(local_login)).route("/auth/callback",get(callback)).route("/auth/logout",post(logout))
        .route("/api/auth/config",get(auth_configuration))
        .route("/api/me",get(me)).route("/api/enroll",post(enroll))
        .route("/api/projects/{project}/nodes",get(nodes)).route("/api/projects/{project}/invitations",post(invite))
        .route("/api/projects/{project}/nodes/{node}",get(node_detail))
        .route("/api/projects/{project}/nodes/{node}/revoke",post(revoke))
        .route("/api/projects/{project}/hosts",get(hosts))
        .route("/api/projects/{project}/workspaces",get(workspaces))
        .route("/api/projects/{project}/threads",get(threads).post(create_thread))
        .route("/api/projects/{project}/threads/{thread}",get(thread_detail).patch(edit_thread))
        .route("/api/projects/{project}/threads/{thread}/messages",post(send_message))
        .route("/api/projects/{project}/threads/{thread}/events",get(thread_events))
        .route("/api/projects/{project}/tasks",get(tasks).post(create))
        .route("/api/projects/{project}/tasks/{task}",get(task))
        .route("/api/projects/{project}/tasks/{task}/commands/{command}",get(command))
        .route("/api/projects/{project}/tasks/{task}/events",get(events))
        .route("/api/projects/{project}/tasks/{task}/cancel",post(cancel))
        .route("/api/projects/{project}/tasks/{task}/respond",post(respond))
        .route("/api/projects/{project}/tasks/{task}/inboxes",post(inspect_inboxes))
        .merge(crate::admin::router()).merge(crate::observability::router()).merge(crate::settings::router())
        .fallback_service(spa::service(&state.config.web_root))
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
    db(state.repo.clone(), |repo| async move {
        repo.check_readiness().await
    })
    .await?;
    Ok("ok")
}
pub(crate) struct Error(pub(crate) CloudError);
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
pub(crate) type Result<T> = std::result::Result<T, Error>;
async fn auth_configuration(
    Extract(state): Extract<Arc<State>>,
) -> Result<Json<serde_json::Value>> {
    let mut value = state.auth.public_configuration();
    value["classification"] =
        serde_json::to_value(state.repo.display_settings().await?.classification)
            .map_err(|_| CloudError::Storage)?;
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalLogin {
    username: String,
    password: String,
}
async fn local_login(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(input): Json<LocalLogin>,
) -> Result<Response> {
    let cookie = state
        .auth
        .local_login(
            &headers,
            input.username,
            zeroize::Zeroizing::new(input.password),
        )
        .await?;
    Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response())
}
async fn login(Extract(state): Extract<Arc<State>>) -> Result<Response> {
    let (url, cookie) = state.auth.login().await?;
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
    if query.iss.as_ref().is_some_and(|issuer| {
        state
            .config
            .oidc
            .as_ref()
            .is_none_or(|provider| issuer != &provider.issuer)
    }) {
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
    let cookie = state.auth.logout(&headers).await?;
    Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response())
}
async fn me(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({"user":state.auth.user(&headers).await?,"memberships":state.auth.memberships(&headers).await?,"projects":state.auth.projects(&headers).await?}),
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
    let caller = state.auth.caller(&headers, &project, false).await?;
    let nodes = db(state.repo.clone(), move |repo| async move {
        repo.list_nodes(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
            .await
    })
    .await?;
    let local_presence = state
        .presence
        .lock()
        .map_err(|_| CloudError::Storage)?
        .clone();
    let next_cursor = (nodes.len() == page.limit.unwrap_or(100).min(100))
        .then(|| nodes.last().map(|node| node.node_id.clone()))
        .flatten();
    let mut fleet = Vec::with_capacity(nodes.len());
    for node in nodes {
        let online = local_presence
            .get(&format!("{}:{}", node.project_id, node.node_id))
            .filter(|online| {
                online
                    .heartbeat
                    .is_some_and(|time| time.elapsed() < Duration::from_secs(30))
            })
            .cloned();
        let presence = if node.revoked {
            None
        } else if online.is_some() {
            online
        } else {
            match state
                .repo
                .storage()
                .read_lease(&node.project_id, &node.node_id, now())
                .await
            {
                Ok(lease) => Some(crate::server::NodePresence {
                    connection_id: lease.owner_id,
                    ready: node.runtime_ready,
                    capabilities: Vec::new(),
                    heartbeat: None,
                }),
                Err(CloudError::NotFound) => None,
                Err(error) => return Err(error.into()),
            }
        };
        fleet.push(serde_json::json!({"node":node,"presence":presence}));
    }
    Ok(Json(
        serde_json::json!({"nodes":fleet,"next_cursor":next_cursor}),
    ))
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
    let caller = state.auth.caller(&headers, &project, true).await?;
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
    db(state.repo.clone(), move |repo| async move {
        repo.invite(&caller, invitation, now()).await
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
    let (node, certificate) = db(state.repo.clone(), move |repo| async move {
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
        .await
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
    let caller = state.auth.caller(&headers, &project, true).await?;
    let node = db(state.repo.clone(), move |repo| async move {
        repo.revoke_node(&caller, &node, input.revision).await
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
    let caller = state.auth.caller(&headers, &project, false).await?;
    let tasks = db(state.repo.clone(), move |repo| async move {
        repo.list_tasks(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
            .await
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
    let caller = state.auth.caller(&headers, &project, true).await?;
    let task = db(state.repo.clone(), move |repo| async move {
        repo.create_task(&caller, &input.node_id, input.request)
            .await
    })
    .await?;
    Ok(Json(serde_json::json!({"task":task})))
}
async fn task(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let task = db(state.repo.clone(), move |repo| async move {
        repo.get_task(&caller, &task).await
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
    let caller = state.auth.caller(&headers, &project, false).await?;
    let command = db(state.repo.clone(), move |repo| async move {
        repo.get_command(&caller, &task, &command).await
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
    let caller = state.auth.caller(&headers, &project, true).await?;
    let command = db(state.repo.clone(), move |repo| async move {
        repo.cancel_task(&caller, &task, &input.mutation_id).await
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
    let caller = state.auth.caller(&headers, &project, true).await?;
    let command = db(state.repo.clone(), move |repo| async move {
        repo.respond_task(&caller, &task, &input.mutation_id, input.request)
            .await
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
    stream_events(state, headers, project, task, cursor.after, false).await
}
async fn thread_events(
    Extract(state): Extract<Arc<State>>,
    Path((project, thread)): Path<(String, String)>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
) -> Result<Sse<impl futures::Stream<Item = std::result::Result<Event, Infallible>>>> {
    stream_events(state, headers, project, thread, cursor.after, true).await
}
async fn stream_events(
    state: Arc<State>,
    headers: HeaderMap,
    project: String,
    scope: String,
    after: u64,
    thread: bool,
) -> Result<Sse<impl futures::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let permit = state
        .sse_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| CloudError::ResourceExhausted)?;
    let caller = state.auth.caller(&headers, &project, false).await?;
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
        .unwrap_or(after);
    let hints = state.repo.storage().subscribe();
    let first = if thread {
        state
            .repo
            .thread_updates(&caller, &scope, after, 100)
            .await?
    } else {
        state
            .repo
            .updates(&caller, &scope, after, 100)
            .await?
            .into_iter()
            .map(|update| {
                Ok(colossus_cloud::storage::ReleasedEvent {
                    project_id: project.clone(),
                    scope_id: scope.clone(),
                    sequence: update.sequence,
                    value: serde_json::to_value(update).map_err(|_| CloudError::Storage)?,
                })
            })
            .collect::<colossus_cloud::CloudResult<Vec<_>>>()?
    };
    let stream = futures::stream::unfold(
        (
            state, headers, project, scope, after, first, permit, hints, thread, 0u64, false,
        ),
        |(
            state,
            headers,
            project,
            scope,
            mut after,
            mut pending,
            permit,
            mut hints,
            thread,
            mut revision,
            mut hint_pending,
        )| async move {
            loop {
                if *state.shutdown.borrow() {
                    return None;
                }
                let caller = state.auth.caller(&headers, &project, false).await.ok()?;
                if let Some(update) = pending.first().cloned() {
                    pending.remove(0);
                    after = update.sequence;
                    let event = Event::default()
                        .id(after.to_string())
                        .event("run_update")
                        .json_data(update.value)
                        .ok()?;
                    return Some((
                        Ok(event),
                        (
                            state,
                            headers,
                            project,
                            scope,
                            after,
                            pending,
                            permit,
                            hints,
                            thread,
                            revision,
                            hint_pending,
                        ),
                    ));
                }
                if thread && !hint_pending {
                    let detail = state.repo.get_thread(&caller, &scope).await.ok()?;
                    if detail.thread.revision != revision {
                        revision = detail.thread.revision;
                        hint_pending = true;
                        let event = Event::default()
                            .event("thread_changed")
                            .json_data(&detail.thread)
                            .ok()?;
                        return Some((
                            Ok(event),
                            (
                                state,
                                headers,
                                project,
                                scope,
                                after,
                                pending,
                                permit,
                                hints,
                                thread,
                                revision,
                                hint_pending,
                            ),
                        ));
                    }
                }
                let mut shutdown = state.shutdown.subscribe();
                tokio::select! { _=hints.recv()=>{},_=tokio::time::sleep(Duration::from_secs(5))=>{},_=shutdown.changed()=>return None }
                hint_pending = false;
                pending = if thread {
                    state
                        .repo
                        .thread_updates(&caller, &scope, after, 100)
                        .await
                        .ok()?
                } else {
                    state
                        .repo
                        .updates(&caller, &scope, after, 100)
                        .await
                        .ok()?
                        .into_iter()
                        .filter_map(|update| {
                            serde_json::to_value(&update).ok().map(|value| {
                                colossus_cloud::storage::ReleasedEvent {
                                    project_id: project.clone(),
                                    scope_id: scope.clone(),
                                    sequence: update.sequence,
                                    value,
                                }
                            })
                        })
                        .collect()
                };
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadPage {
    node_id: Option<String>,
    query: Option<String>,
    archived: Option<bool>,
    after: Option<String>,
    limit: Option<usize>,
}
async fn hosts(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let hosts = state
        .repo
        .list_hosts(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
        .await?;
    let next_cursor = (hosts.len() == page.limit.unwrap_or(100).min(100))
        .then(|| hosts.last().map(|host| host.host_id.clone()))
        .flatten();
    Ok(Json(
        serde_json::json!({"hosts":hosts,"next_cursor":next_cursor}),
    ))
}
async fn workspaces(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let workspaces = state
        .repo
        .list_workspaces(&caller, page.after.as_deref(), page.limit.unwrap_or(100))
        .await?;
    Ok(Json(serde_json::json!({"workspaces":workspaces})))
}
async fn threads(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<ThreadPage>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let (threads, next_cursor) = state
        .repo
        .query_threads(
            &caller,
            page.node_id,
            page.query,
            page.archived,
            page.after,
            page.limit.unwrap_or(100),
        )
        .await?;
    Ok(Json(
        serde_json::json!({"threads":threads,"next_cursor":next_cursor}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewThread {
    node_id: String,
    title: Option<String>,
    request: CreateRunRequest,
}
async fn create_thread(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(input): Json<NewThread>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    let (thread, task) = state
        .repo
        .create_thread(&caller, &input.node_id, input.title, input.request)
        .await?;
    Ok(Json(serde_json::json!({"thread":thread,"task":task})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryPage {
    task_after: Option<String>,
    message_after: Option<String>,
}
async fn thread_detail(
    Extract(state): Extract<Arc<State>>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(page): Query<HistoryPage>,
) -> Result<Json<colossus_cloud::CloudThreadDetail>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    Ok(Json(
        state
            .repo
            .thread_detail(
                &caller,
                &id,
                page.task_after.as_deref(),
                page.message_after.as_deref(),
            )
            .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadMessage {
    expected_revision: u64,
    request: CreateRunRequest,
}
async fn send_message(
    Extract(state): Extract<Arc<State>>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<ThreadMessage>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    let (thread, task) = state
        .repo
        .send_message(&caller, &id, input.expected_revision, input.request)
        .await?;
    Ok(Json(serde_json::json!({"thread":thread,"task":task})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadEdit {
    revision: u64,
    title: Option<String>,
    archived: Option<bool>,
}
async fn edit_thread(
    Extract(state): Extract<Arc<State>>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<ThreadEdit>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    let thread = state
        .repo
        .update_thread(&caller, &id, input.revision, input.title, input.archived)
        .await?;
    Ok(Json(serde_json::json!({"thread":thread})))
}
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

async fn node_detail(
    Extract(state): Extract<Arc<State>>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let node = state.repo.get_node(&caller, &id).await?;
    let local = state
        .presence
        .lock()
        .map_err(|_| CloudError::Storage)?
        .get(&format!("{}:{}", node.project_id, node.node_id))
        .filter(|presence| {
            presence
                .heartbeat
                .is_some_and(|time| time.elapsed() < Duration::from_secs(30))
        })
        .cloned();
    let presence = if node.revoked {
        None
    } else if local.is_some() {
        local
    } else {
        match state
            .repo
            .storage()
            .read_lease(&node.project_id, &node.node_id, now())
            .await
        {
            Ok(lease) => Some(crate::server::NodePresence {
                connection_id: lease.owner_id,
                ready: node.runtime_ready,
                capabilities: Vec::new(),
                heartbeat: None,
            }),
            Err(CloudError::NotFound) => None,
            Err(error) => return Err(error.into()),
        }
    };
    Ok(Json(serde_json::json!({"node":node,"presence":presence})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectInboxesInput {
    request_id: String,
    participant_id: Option<String>,
    #[serde(default)]
    after_sequence: u64,
}
async fn inspect_inboxes(
    Extract(state): Extract<Arc<State>>,
    Path((project, task)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<InspectInboxesInput>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    let command = db(state.repo.clone(), move |repo| async move {
        repo.inspect_inboxes(
            &caller,
            &task,
            &input.request_id,
            input.participant_id,
            input.after_sequence,
        )
        .await
    })
    .await?;
    Ok(Json(serde_json::json!({"command": command})))
}
