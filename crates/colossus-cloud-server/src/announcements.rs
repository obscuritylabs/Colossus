//! Narrow authenticated instance communication routes; no runtime dispatch.
use crate::{http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::announcements::{
    AcknowledgeAnnouncement, AdminAnnouncementPage, Announcement, AnnouncementAcknowledgement,
    AnnouncementPage, CreateAnnouncement, UpdateAnnouncement,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/announcements", get(feed))
        .route("/api/announcements/{id}/receipt", post(acknowledge))
        .route("/api/admin/announcements", get(catalog).post(create))
        .route("/api/admin/announcements/{id}", get(detail).patch(update))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<usize>,
}
#[derive(Serialize)]
struct Response {
    announcement: Announcement,
}

async fn feed(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<AnnouncementPage>> {
    let user = state.auth.user(&headers).await?;
    Ok(Json(
        state
            .repo
            .announcements(&user, page.after.as_deref(), page.limit.unwrap_or(20))
            .await?,
    ))
}
async fn catalog(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<AdminAnnouncementPage>> {
    let actor = state.auth.admin(&headers, false).await?;
    Ok(Json(
        state
            .repo
            .admin_announcements(&actor, page.after.as_deref(), page.limit.unwrap_or(20))
            .await?,
    ))
}
async fn detail(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Response>> {
    let actor = state.auth.admin(&headers, false).await?;
    Ok(Json(Response {
        announcement: state.repo.admin_announcement(&actor, &id).await?,
    }))
}
async fn create(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(input): Json<CreateAnnouncement>,
) -> Result<Json<Response>> {
    let actor = state.auth.admin(&headers, true).await?;
    Ok(Json(Response {
        announcement: state.repo.create_announcement(&actor, input).await?,
    }))
}
async fn update(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateAnnouncement>,
) -> Result<Json<Response>> {
    let actor = state.auth.admin(&headers, true).await?;
    Ok(Json(Response {
        announcement: state.repo.update_announcement(&actor, &id, input).await?,
    }))
}
async fn acknowledge(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<AcknowledgeAnnouncement>,
) -> Result<Json<AnnouncementAcknowledgement>> {
    state.auth.csrf(&headers)?;
    let user = state.auth.user(&headers).await?;
    Ok(Json(
        state
            .repo
            .acknowledge_announcement(&user, &id, input)
            .await?,
    ))
}
