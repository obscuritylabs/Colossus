//! Process-local request routing. Terminal bytes never enter relational outboxes.
use crate::{connection::FrameSender, server::State};
use colossus_cloud::storage::ConnectionLease;
use colossus_cloud_protocol::{
    MAX_TERMINAL_CONTROL_REQUESTS, MAX_TERMINAL_PAYLOAD_BYTES, MAX_TERMINAL_READ_REQUESTS,
    TerminalReply, decode, encode,
    v1alpha1::{ControlFrame, TerminalRequest, TerminalResponse, control_frame},
    valid_terminal_request_id,
};
use colossus_sdk::{
    ApiError, ApiErrorReason, ApiResult, HumanTerminalOperation, HumanTerminalReply,
    HumanTerminalRequest,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};
use tonic::Status;

const MAX_CONNECTIONS: usize = 1024;
const MAX_PENDING: usize = 256;
const MAX_PENDING_PER_LANE: usize = MAX_PENDING / 2;
type NodeKey = (String, String);
#[derive(Clone)]
struct Connection {
    lease: ConnectionLease,
    sender: FrameSender,
}
struct Pending {
    lease: ConnectionLease,
    read: bool,
    reply: oneshot::Sender<TerminalReply>,
}
struct Inner {
    connections: Mutex<HashMap<NodeKey, Connection>>,
    pending: Mutex<HashMap<String, Pending>>,
}
/// Independent terminal transport capacity shared by browser and internal relay.
#[derive(Clone)]
pub(crate) struct TerminalHub {
    inner: Arc<Inner>,
    pub(crate) browsers: Arc<Semaphore>,
}
impl Default for TerminalHub {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                connections: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
            }),
            browsers: Arc::new(Semaphore::new(64)),
        }
    }
}
impl TerminalHub {
    pub(crate) fn connect(&self, lease: ConnectionLease, sender: FrameSender) -> Result<(), ()> {
        let mut connections = self.inner.connections.lock().map_err(|_| ())?;
        let key = (lease.project_id.clone(), lease.node_id.clone());
        if connections.len() >= MAX_CONNECTIONS && !connections.contains_key(&key) {
            return Err(());
        }
        connections.insert(key, Connection { lease, sender });
        Ok(())
    }
    pub(crate) fn disconnect(&self, lease: &ConnectionLease) {
        let key = (lease.project_id.clone(), lease.node_id.clone());
        if let Ok(mut connections) = self.inner.connections.lock()
            && connections
                .get(&key)
                .is_some_and(|connection| same_lease(&connection.lease, lease))
        {
            connections.remove(&key);
        }
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.retain(|_, request| !same_lease(&request.lease, lease));
        }
    }
    pub(crate) fn complete(
        &self,
        lease: &ConnectionLease,
        response: TerminalResponse,
    ) -> Result<(), Status> {
        if !valid_terminal_request_id(&response.request_id)
            || response.reply_json.len() > MAX_TERMINAL_PAYLOAD_BYTES
        {
            return Err(Status::invalid_argument("invalid terminal response"));
        }
        let reply: TerminalReply = decode(&response.reply_json)
            .map_err(|_| Status::invalid_argument("invalid terminal response"))?;
        let mut pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| Status::unavailable("terminal routing unavailable"))?;
        if let Some(request) = pending.get(&response.request_id) {
            if !same_lease(&request.lease, lease) {
                return Err(Status::permission_denied(
                    "terminal response owner mismatch",
                ));
            }
            if let Some(request) = pending.remove(&response.request_id) {
                let _ = request.reply.send(reply);
            }
        }
        // A cancelled browser may have removed the exact pending request already.
        Ok(())
    }
    async fn request(
        &self,
        lease: ConnectionLease,
        request: HumanTerminalRequest,
    ) -> ApiResult<HumanTerminalReply> {
        let mutation = is_mutation(&request.operation);
        let read = matches!(&request.operation, HumanTerminalOperation::Read { .. });
        if !request.validate() {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "operation",
                "Invalid terminal operation.",
            ));
        }
        let bytes = encode(&request).map_err(|_| super::unavailable(false))?;
        if bytes.len() > MAX_TERMINAL_PAYLOAD_BYTES {
            return Err(ApiError::bounded_resource_exhausted(
                ApiErrorReason::InvalidArgument,
                "Terminal request exceeds its bound.",
            ));
        }
        let connection = self
            .inner
            .connections
            .lock()
            .map_err(|_| super::unavailable(false))?
            .get(&(lease.project_id.clone(), lease.node_id.clone()))
            .filter(|connection| same_lease(&connection.lease, &lease))
            .cloned()
            .ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::ScopeDenied,
                    "This connection has no human terminal capability.",
                )
            })?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .map_err(|_| super::unavailable(false))?;
            let node_limit = if read {
                MAX_TERMINAL_READ_REQUESTS
            } else {
                MAX_TERMINAL_CONTROL_REQUESTS
            };
            if pending.len() >= MAX_PENDING
                || pending.values().filter(|entry| entry.read == read).count()
                    >= MAX_PENDING_PER_LANE
                || pending
                    .values()
                    .filter(|entry| entry.read == read && same_lease(&entry.lease, &lease))
                    .count()
                    >= node_limit
            {
                return Err(ApiError::resource_exhausted(
                    ApiErrorReason::InvalidArgument,
                    "The terminal request limit is active.",
                ));
            }
            pending.insert(
                id.clone(),
                Pending {
                    lease,
                    read,
                    reply: sender,
                },
            );
        }
        let _registration = Registration {
            hub: self.clone(),
            id: id.clone(),
        };
        connection
            .sender
            .try_terminal(ControlFrame {
                body: Some(control_frame::Body::TerminalRequest(TerminalRequest {
                    request_id: id,
                    request_json: bytes,
                })),
            })
            .map_err(|_| {
                ApiError::resource_exhausted(
                    ApiErrorReason::InvalidArgument,
                    "The terminal transport queue is full.",
                )
            })?;
        match tokio::time::timeout(Duration::from_secs(5), receiver).await {
            Ok(Ok(TerminalReply::Result { reply })) => Ok(reply),
            Ok(Ok(TerminalReply::Failed { error })) => Err(error),
            _ => Err(super::unavailable(mutation)),
        }
    }
}
struct Registration {
    hub: TerminalHub,
    id: String,
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.hub.inner.pending.lock() {
            pending.remove(&self.id);
        }
    }
}
fn same_lease(left: &ConnectionLease, right: &ConnectionLease) -> bool {
    left.project_id == right.project_id
        && left.node_id == right.node_id
        && left.owner_id == right.owner_id
        && left.generation == right.generation
}
pub(crate) fn is_mutation(operation: &HumanTerminalOperation) -> bool {
    !matches!(
        operation,
        HumanTerminalOperation::Context | HumanTerminalOperation::Read { .. }
    )
}
/// Dispatch only to this replica's exact live connector; authenticated relay calls here.
pub(crate) async fn dispatch_local(
    state: &Arc<State>,
    lease: ConnectionLease,
    request: HumanTerminalRequest,
) -> ApiResult<HumanTerminalReply> {
    state
        .repo
        .storage()
        .verify_lease(&lease, crate::http::now())
        .await
        .map_err(|_| super::unavailable(false))?;
    let mutation = is_mutation(&request.operation);
    let reply = state.terminal.request(lease.clone(), request).await?;
    state
        .repo
        .storage()
        .verify_lease(&lease, crate::http::now())
        .await
        .map_err(|_| super::unavailable(mutation))?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
