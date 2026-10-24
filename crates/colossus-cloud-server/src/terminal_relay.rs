//! Authenticated bounded forwarding between independently load-balanced replicas.
mod identity;
#[cfg(test)]
mod tests;
use crate::server::State;
use colossus_api::{ApiError, ApiErrorReason, ApiResult, HumanTerminalReply, HumanTerminalRequest};
use colossus_cloud::storage::ConnectionLease;
use colossus_cloud_protocol::v1alpha1::{
    ReplicaTerminalRequest, ReplicaTerminalResponse,
    replica_terminal_relay_client::ReplicaTerminalRelayClient,
    replica_terminal_relay_server::{ReplicaTerminalRelay, ReplicaTerminalRelayServer},
};
pub(crate) use identity::RelayTransport;
use std::sync::Arc;
use tonic::service::{Interceptor, interceptor::InterceptedService};
use tonic::{Request, Response, Status};

const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_REPLY_BYTES: usize = 512 * 1024;

fn unavailable() -> ApiError {
    crate::terminal::unavailable(false)
}

/// Dispatch only to the fenced owner. Connection failure never retries a mutation.
pub(crate) async fn dispatch(
    state: &Arc<State>,
    lease: ConnectionLease,
    request: HumanTerminalRequest,
) -> ApiResult<HumanTerminalReply> {
    if is_local(&state.replica_id, &lease.owner_id) {
        return crate::terminal::dispatch_local(state, lease, request).await;
    }
    let mutation = crate::terminal::is_mutation(&request.operation);
    let _permit = state.relay.admit(&request.operation).map_err(|_| {
        ApiError::resource_exhausted(
            ApiErrorReason::CapacityExceeded,
            "Terminal relay capacity reached.",
        )
    })?;
    let replica = replica_owner(&lease.owner_id).ok_or_else(unavailable)?;
    let route = state
        .repo
        .storage()
        .read_replica_route(replica, crate::http::now())
        .await
        .map_err(|_| unavailable())?;
    state
        .repo
        .storage()
        .verify_lease(&lease, crate::http::now())
        .await
        .map_err(|_| unavailable())?;
    let request_json = serde_json::to_vec(&request).map_err(|_| unavailable())?;
    if request_json.len() > MAX_REQUEST_BYTES {
        return Err(unavailable());
    }
    let channel = state
        .relay
        .channel(state, &route)
        .await
        .map_err(|_| unavailable())?;
    let mut rpc = Request::new(ReplicaTerminalRequest {
        project_id: lease.project_id.clone(),
        node_id: lease.node_id.clone(),
        connection_id: lease.owner_id.clone(),
        generation: lease.generation,
        request_json,
    });
    rpc.metadata_mut().insert(
        "x-colossus-terminal-relay",
        state.relay.token().parse().map_err(|_| unavailable())?,
    );
    rpc.set_timeout(std::time::Duration::from_secs(6));
    let mut client = ReplicaTerminalRelayClient::new(channel)
        .max_decoding_message_size(MAX_REPLY_BYTES + 1024)
        .max_encoding_message_size(MAX_REQUEST_BYTES + 1024);
    let response = client
        .dispatch(rpc)
        .await
        .map_err(|_| crate::terminal::unavailable(mutation))?
        .into_inner();
    if response.reply_json.len() > MAX_REPLY_BYTES {
        return Err(crate::terminal::unavailable(mutation));
    }
    state
        .repo
        .storage()
        .verify_lease(&lease, crate::http::now())
        .await
        .map_err(|_| crate::terminal::unavailable(mutation))?;
    serde_json::from_slice(&response.reply_json)
        .map_err(|_| crate::terminal::unavailable(mutation))?
}

fn replica_owner(connection: &str) -> Option<&str> {
    let (replica, suffix) = connection.split_once('-')?;
    (replica.len() == 32
        && replica.bytes().all(|c| c.is_ascii_hexdigit())
        && suffix.len() == 32
        && suffix.bytes().all(|c| c.is_ascii_hexdigit()))
    .then_some(replica)
}
fn is_local(replica: &str, connection: &str) -> bool {
    replica_owner(connection) == Some(replica)
}

pub(crate) fn service(
    state: Arc<State>,
) -> InterceptedService<ReplicaTerminalRelayServer<RelayService>, RelayAuthentication> {
    let authentication = RelayAuthentication(state.clone());
    let server = ReplicaTerminalRelayServer::new(RelayService(state))
        .max_decoding_message_size(MAX_REQUEST_BYTES + 1024)
        .max_encoding_message_size(MAX_REPLY_BYTES + 1024);
    InterceptedService::new(server, authentication)
}
pub(crate) struct RelayService(Arc<State>);

/// Reject ordinary fleet identities before allocating or decoding relay messages.
#[derive(Clone)]
pub(crate) struct RelayAuthentication(Arc<State>);

impl Interceptor for RelayAuthentication {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        if !self.0.relay.authenticate(
            request
                .metadata()
                .get("x-colossus-terminal-relay")
                .and_then(|value| value.to_str().ok()),
        ) {
            return Err(Status::unauthenticated("replica authentication required"));
        }
        Ok(request)
    }
}

#[tonic::async_trait]
impl ReplicaTerminalRelay for RelayService {
    async fn dispatch(
        &self,
        request: Request<ReplicaTerminalRequest>,
    ) -> Result<Response<ReplicaTerminalResponse>, Status> {
        if !self.0.relay.authenticate(
            request
                .metadata()
                .get("x-colossus-terminal-relay")
                .and_then(|v| v.to_str().ok()),
        ) {
            return Err(Status::unauthenticated("replica authentication required"));
        }
        let request = request.into_inner();
        if !is_local(&self.0.replica_id, &request.connection_id)
            || request.request_json.len() > MAX_REQUEST_BYTES
        {
            return Err(Status::failed_precondition("terminal owner changed"));
        }
        let lease = self
            .0
            .repo
            .storage()
            .read_lease(&request.project_id, &request.node_id, crate::http::now())
            .await
            .map_err(|_| Status::failed_precondition("terminal connection unavailable"))?;
        if lease.owner_id != request.connection_id || lease.generation != request.generation {
            return Err(Status::failed_precondition("terminal owner changed"));
        }
        let operation: HumanTerminalRequest = serde_json::from_slice(&request.request_json)
            .map_err(|_| Status::invalid_argument("invalid terminal operation"))?;
        let _permit = self
            .0
            .relay
            .admit(&operation.operation)
            .map_err(|_| Status::resource_exhausted("terminal relay capacity"))?;
        let result = crate::terminal::dispatch_local(&self.0, lease, operation).await;
        let reply_json = serde_json::to_vec(&result)
            .map_err(|_| Status::internal("terminal response unavailable"))?;
        if reply_json.len() > MAX_REPLY_BYTES {
            return Err(Status::resource_exhausted("terminal response limit"));
        }
        Ok(Response::new(ReplicaTerminalResponse { reply_json }))
    }
}
