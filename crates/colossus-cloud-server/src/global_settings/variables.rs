//! Personal reusable nonsecret values; private to the signed-in account in one project.
use crate::{http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, State as Extract},
    http::HeaderMap,
    routing::get,
};
use colossus_cloud::{CloudError, CloudPermission};
use colossus_contracts::VaultRecord;
use colossus_ports::CredentialKey;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Variables {
    revision: u64,
    variables: BTreeMap<String, String>,
}
pub(super) fn router() -> Router<Arc<State>> {
    Router::new().route(
        "/api/projects/{project}/chat/variables/personal",
        get(read).put(replace),
    )
}
fn key() -> std::result::Result<CredentialKey, CloudError> {
    CredentialKey::new("cloud-hosted-variables", "personal-v1").map_err(|_| CloudError::Storage)
}
fn load(vault: &dyn colossus_ports::CredentialVault) -> std::result::Result<Variables, CloudError> {
    let Some(record) = vault.read(&key()?).map_err(|_| CloudError::Storage)? else {
        return Ok(Variables::default());
    };
    serde_json::from_slice(record.expose()).map_err(|_| CloudError::Storage)
}
async fn read(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Result<Json<Variables>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    if host.project_config(&project).await?.is_none() {
        return Err(CloudError::NotFound.into());
    }
    Ok(Json(load(
        host.user_vault(&project, caller.subject())?.as_ref(),
    )?))
}
async fn replace(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(project): Path<String>,
    Json(mut input): Json<Variables>,
) -> Result<Json<Variables>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    super::inheritance::validate_variables(&input.variables)?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let _lifecycle = host.lifecycle.lock().await;
    if host.project_config(&project).await?.is_none() {
        return Err(CloudError::NotFound.into());
    }
    let edit = host.begin_storage_edit(&caller)?;
    let vault = if let Some(edit) = &edit {
        edit.shared.editing_vault(
            &format!("user/{project}/{}/connections", caller.subject()),
            &project,
            &edit.owner,
        )
    } else {
        host.user_vault(&project, caller.subject())?
    };
    let current = load(vault.as_ref())?;
    if input.revision != current.revision {
        return Err(CloudError::Conflict.into());
    }
    input.revision = input
        .revision
        .checked_add(1)
        .ok_or(CloudError::ResourceExhausted)?;
    let bytes = serde_json::to_vec(&input).map_err(|_| CloudError::Storage)?;
    let record = VaultRecord::new(bytes).map_err(|_| CloudError::ResourceExhausted)?;
    vault
        .write(&key()?, &record)
        .map_err(|_| CloudError::Storage)?;
    Ok(Json(input))
}
