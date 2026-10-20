//! The exact stream owner checks additive resource support before dispatch.
use super::{control_frame, send, status, wire};
use colossus_cloud::storage::{CloudStore, ConnectionLease, RuntimeResourceRequest};
use colossus_cloud_protocol::{ResourceOperation, ResourceReply, encode};
use tokio::sync::mpsc;
use tonic::Status;

pub(super) async fn dispatch(
    store: &dyn CloudStore,
    lease: &ConnectionLease,
    request: RuntimeResourceRequest,
    model_catalog_enabled: bool,
    sender: &mpsc::Sender<Result<wire::ControlFrame, Status>>,
) -> Result<(), Status> {
    if matches!(request.operation, ResourceOperation::ModelCatalog {}) && !model_catalog_enabled {
        // HTTP admission can happen on another replica. Only this connection's
        // authenticated Hello can prove it understands the additive operation.
        return store
            .resource_complete(
                lease,
                &request.request_id,
                ResourceReply::Failed {
                    error: colossus_sdk::ApiError::failed_precondition(
                        colossus_sdk::ApiErrorReason::InvalidArgument,
                        "This runtime connector needs an update before it can load model choices.",
                    ),
                },
                crate::http::now(),
            )
            .await
            .map_err(status);
    }
    send(
        sender,
        control_frame::Body::ResourceRequest(wire::ResourceRequest {
            request_id: request.request_id,
            operation_json: encode(&request.operation)
                .map_err(|_| Status::invalid_argument("invalid resource operation"))?,
        }),
    )
    .await
}

#[cfg(test)]
mod tests;
