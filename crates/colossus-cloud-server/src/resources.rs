//! Generation-fenced online resource requests. No reconnect replay or task allocation.
use crate::{
    http::{Error, now},
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, State as Extract},
    http::HeaderMap,
    routing::post,
};
use colossus_cloud::{CloudError, CloudPermission};
use colossus_cloud_protocol::{
    MAX_RESOURCE_REQUEST_BYTES, ResourceOperation, ResourcePermission, ResourceReply, encode,
};
use colossus_sdk::{ApiError, ApiErrorCode, ApiErrorReason};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    connection_id: Option<String>,
    operation: ResourceOperation,
}
#[derive(Serialize)]
struct Output {
    connection_id: String,
    #[serde(flatten)]
    reply: ResourceReply,
}
pub(crate) fn router() -> Router<Arc<State>> {
    Router::new().route(
        "/api/projects/{project}/nodes/{node}/resources",
        post(request),
    )
}
async fn request(
    Extract(state): Extract<Arc<State>>,
    Path((project, node)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Output>, Error> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Read)?;
    caller.require(match input.operation.permission() {
        ResourcePermission::Read => CloudPermission::Read,
        ResourcePermission::Execute => CloudPermission::Execute,
        ResourcePermission::Control => CloudPermission::Control,
    })?;
    let identity = state.repo.get_node(&caller, &node).await?;
    if identity.revoked {
        return Err(CloudError::PermissionDenied.into());
    }
    let operation_json = encode(&input.operation).map_err(|_| CloudError::InvalidArgument)?;
    if !input.operation.validate() || operation_json.len() > MAX_RESOURCE_REQUEST_BYTES {
        return Err(CloudError::InvalidArgument.into());
    }
    let lease = state
        .repo
        .storage()
        .read_lease(&project, &node, now())
        .await?;
    if input
        .connection_id
        .as_ref()
        .is_some_and(|expected| expected != &lease.owner_id)
        || (input.connection_id.is_none() && !matches!(input.operation, ResourceOperation::Context))
    {
        return Err(CloudError::Conflict.into());
    }
    let store = state.repo.storage();
    let id = match store
        .resource_submit(&caller, &lease, input.operation.clone(), now())
        .await
    {
        Ok(id) => id,
        Err(CloudError::Conflict) if matches!(input.operation, ResourceOperation::Context) => {
            return Ok(Json(Output {
                connection_id: lease.owner_id,
                reply: ResourceReply::Failed {
                    error: known_error(
                        ApiErrorCode::FailedPrecondition,
                        ApiErrorReason::InvalidArgument,
                        "This runtime connector needs an update before it can load workflows and schedules.",
                        false,
                    ),
                },
            }));
        }
        Err(error) => return Err(error.into()),
    };
    // Any HTTP replica can admit/read; only the exact stream owner claims dispatch.
    // A claimed operation is never reissued to a replacement connection.
    let reply = match tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(reply) = store.resource_read(&caller, &id).await? {
                return Ok::<_, CloudError>(reply);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    {
        Ok(Ok(reply)) => reply,
        _ => ResourceReply::Failed {
            error: if input.operation.is_mutation() {
                unknown_outcome(
                    "The runtime response was lost. Inspect the resource and reconcile the same request before retrying.",
                )
            } else {
                known_error(
                    ApiErrorCode::Unavailable,
                    ApiErrorReason::StorageFailure,
                    "The runtime disconnected before this read completed. Refresh to retry.",
                    true,
                )
            },
        },
    };
    // Recheck user membership, revocation and lease before disclosing a response.
    let caller = state.auth.caller(&headers, &project, true).await?;
    if state.repo.get_node(&caller, &node).await?.revoked {
        return Err(CloudError::PermissionDenied.into());
    }
    if state
        .repo
        .storage()
        .verify_lease(&lease, now())
        .await
        .is_err()
    {
        return Ok(Json(Output {
            connection_id: lease.owner_id,
            reply: ResourceReply::Failed {
                error: if input.operation.is_mutation() {
                    unknown_outcome("The connection changed. Inspect the resource before retrying.")
                } else {
                    known_error(
                        ApiErrorCode::Unavailable,
                        ApiErrorReason::StorageFailure,
                        "The connection changed. Refresh to retry.",
                        true,
                    )
                },
            },
        }));
    }
    Ok(Json(Output {
        connection_id: lease.owner_id,
        reply,
    }))
}

fn unknown_outcome(message: &str) -> ApiError {
    ApiError {
        code: ApiErrorCode::OutcomeUnknown,
        reason: ApiErrorReason::OutcomeUnknown,
        message: message.into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_sdk::ApiOutcomeCertainty::Unknown,
        violations: Vec::new(),
    }
}

fn known_error(
    code: ApiErrorCode,
    reason: ApiErrorReason,
    message: &str,
    retryable: bool,
) -> ApiError {
    ApiError {
        code,
        reason,
        message: message.into(),
        correlation_id: None,
        retryable,
        outcome: colossus_sdk::ApiOutcomeCertainty::Known,
        violations: Vec::new(),
    }
}
