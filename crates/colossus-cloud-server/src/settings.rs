//! Display settings and monitoring baselines through authenticated administration.
use crate::{
    http::{Error, Result},
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, State as Extract},
    http::HeaderMap,
    routing::get,
};
use colossus_cloud::settings::{ControlPlaneSettings, ProjectPolicyExpectation};
use std::sync::Arc;

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/settings", get(public_settings))
        .route(
            "/api/admin/settings",
            get(admin_settings).patch(update_settings),
        )
        .route(
            "/api/projects/{project}/policy",
            get(project_policy).patch(update_policy),
        )
}
async fn public_settings(
    Extract(state): Extract<Arc<State>>,
) -> Result<Json<ControlPlaneSettings>> {
    Ok(Json(state.repo.display_settings().await?))
}
async fn admin_settings(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    state.auth.admin(&headers, false).await?;
    let settings = state.repo.display_settings().await?;
    Ok(Json(
        serde_json::json!({"revision":settings.revision,"classification":settings.classification,"auth":{"local_enabled":state.config.local_auth.is_some(),"oidc_label":state.config.oidc.as_ref().map(|oidc| &oidc.label)}}),
    ))
}
async fn update_settings(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(settings): Json<ControlPlaneSettings>,
) -> Result<Json<ControlPlaneSettings>> {
    let user = state.auth.admin(&headers, true).await?;
    Ok(Json(
        state
            .repo
            .replace_display_settings(&user.id, settings)
            .await?,
    ))
}
async fn project_policy(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ProjectPolicyExpectation>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    Ok(Json(state.repo.project_policy(&caller).await?))
}
async fn update_policy(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Json(policy): Json<ProjectPolicyExpectation>,
) -> std::result::Result<Json<ProjectPolicyExpectation>, Error> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    Ok(Json(
        state.repo.replace_project_policy(&caller, policy).await?,
    ))
}
