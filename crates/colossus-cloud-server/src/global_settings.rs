//! Instance-owned definitions and explicit project inheritance beneath operator ceilings.
pub(crate) mod credentials;
pub(crate) mod inheritance;
mod intrinsic;
pub(crate) mod search;
mod storage;
pub(crate) mod telemetry;
#[cfg(test)]
mod tests;
pub(crate) mod types;
mod variables;

use crate::{hosted::HostedExecution, http::Result, server::State};
use axum::{Json, Router, extract::State as Extract, http::HeaderMap, routing::get};
use colossus_cloud::{CloudError, CloudPermission};
use std::sync::Arc;
pub(crate) use storage::catalog;
use types::HostedCatalog;

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/admin/hosted-settings", get(read).put(replace))
        .route("/api/admin/hosted-settings/provider-presets", get(presets))
        .merge(credentials::router())
        .merge(telemetry::router())
        .merge(variables::router())
}

fn snapshot(
    state: &State,
    catalog: HostedCatalog,
) -> std::result::Result<serde_json::Value, CloudError> {
    let mut value = serde_json::to_value(&catalog).map_err(|_| CloudError::Storage)?;
    value["credential_status"] = serde_json::json!({"providers":{},"mcp":{},"search":{}});
    value["telemetry"] = serde_json::to_value(
        state
            .telemetry
            .configure(&catalog.observability, catalog.revision),
    )
    .map_err(|_| CloudError::Storage)?;
    Ok(value)
}
async fn read(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    state.auth.admin(&headers, false).await?;
    Ok(Json(snapshot(
        &state,
        catalog(state.hosted.as_deref().ok_or(CloudError::NotFound)?)?,
    )?))
}

async fn replace(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(mut input): Json<HostedCatalog>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let _lifecycle = host.lifecycle.lock().await;
    let edit = storage::InstanceEdit::begin(host, &state, &actor.id, true).await?;
    let current = catalog(host)?;
    if input.revision != current.revision {
        return Err(CloudError::Conflict.into());
    }
    inheritance::review_catalog(host, &current, &mut input)?;
    for (project, ceiling) in &host.deployment.projects {
        let raw = crate::chat_settings::retained_configuration(host, project, ceiling)?;
        inheritance::resolve(&raw, &input, ceiling)?;
    }
    input.revision = current
        .revision
        .checked_add(1)
        .ok_or(CloudError::ResourceExhausted)?;
    storage::save_catalog(host, &edit, &actor.id, &input)?;
    Ok(Json(snapshot(&state, input)?))
}

async fn presets(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    state.auth.admin(&headers, false).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let presets = colossus_contracts::provider_presets();
    let availability: std::collections::BTreeMap<_, _> = presets
        .iter()
        .map(|preset| {
            let status = match (preset.protocol, preset.base_url) {
                (colossus_contracts::ProviderSetupProtocol::Codex, _) => {
                    "deployment_required"
                }
                (_, None) => "configure_endpoint",
                (_, Some(endpoint)) => {
                    if host.deployment.projects.values().all(|ceiling| {
                        crate::chat_settings::allowed_endpoint(endpoint, ceiling, false).is_ok()
                    }) {
                        "available"
                    } else {
                        "deployment_required"
                    }
                }
            };
            (
                preset.id,
                serde_json::json!({
                    "status": status,
                    "reason": "Instance definitions remain subject to each project's operator-managed network ceiling. ChatGPT account enrollment remains deployment-managed."
                }),
            )
        })
        .collect();
    Ok(Json(
        serde_json::json!({"presets":presets,"availability":availability}),
    ))
}

pub(crate) fn administrator(
    project: &str,
    actor: &str,
) -> std::result::Result<colossus_cloud::CloudCaller, CloudError> {
    colossus_cloud::CloudCaller::new(
        actor.into(),
        project.into(),
        [CloudPermission::Read, CloudPermission::Administer]
            .into_iter()
            .collect(),
    )
}
