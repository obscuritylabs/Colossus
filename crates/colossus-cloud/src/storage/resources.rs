//! Separate short-lived online resource admission, fenced to one connection generation.
use super::*;
use colossus_cloud_protocol::{
    MAX_RESOURCE_REQUEST_BYTES, MAX_RESOURCE_REQUESTS, ResourceOperation, ResourcePermission,
    ResourceReply,
};

/// Durable online request. Dispatched writes are never reissued after a disconnect.
#[derive(Clone, Debug)]
pub struct RuntimeResourceRequest {
    /// Server-assigned correlation identity, never a runtime mutation idempotency key.
    pub request_id: String,
    /// Exact live connection captured at admission.
    pub lease: ConnectionLease,
    /// Authenticated user for metadata-only audit correlation.
    pub actor: String,
    /// Closed public SDK operation; no caller-selected owner or executable.
    pub operation: ResourceOperation,
    /// UTC admission instant.
    pub created_at: u64,
    /// Deadline for dispatch/response, independent of runtime execution lifetime.
    pub expires_at: u64,
    /// At-most-once dispatch in the captured connection.
    pub dispatched: bool,
    /// Released response; never private runtime state.
    pub reply: Option<ResourceReply>,
}
/// Admission validates independent project authority before selecting any runtime.
pub fn validate_resource_request(
    caller: &crate::CloudCaller,
    lease: &ConnectionLease,
    operation: &ResourceOperation,
) -> CloudResult<()> {
    caller.require(crate::CloudPermission::Read)?;
    caller.require(match operation.permission() {
        ResourcePermission::Read => crate::CloudPermission::Read,
        ResourcePermission::Execute => crate::CloudPermission::Execute,
        ResourcePermission::Control => crate::CloudPermission::Control,
    })?;
    if caller.project_id() != lease.project_id {
        return Err(CloudError::PermissionDenied);
    }
    if !operation.validate()
        || colossus_cloud_protocol::encode(operation)
            .map_err(|_| CloudError::InvalidArgument)?
            .len()
            > MAX_RESOURCE_REQUEST_BYTES
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
impl MemoryCloudStore {
    pub(super) async fn memory_resource_connect(
        &self,
        lease: &ConnectionLease,
        enabled: bool,
    ) -> CloudResult<()> {
        let mut state = self.state.lock().await;
        let key = (lease.project_id.clone(), lease.node_id.clone());
        if !state.leases.get(&key).is_some_and(|live| {
            live.owner_id == lease.owner_id && live.generation == lease.generation
        }) {
            return Err(CloudError::Conflict);
        }
        if enabled {
            state.resource_connections.insert(key, lease.clone());
        } else {
            state.resource_connections.remove(&key);
        }
        Ok(())
    }
    pub(super) async fn memory_resource_submit(
        &self,
        caller: &crate::CloudCaller,
        lease: &ConnectionLease,
        operation: ResourceOperation,
        now: u64,
    ) -> CloudResult<String> {
        validate_resource_request(caller, lease, &operation)?;
        let mut state = self.state.lock().await;
        let key = (lease.project_id.clone(), lease.node_id.clone());
        if !enrolled_live(&state, &lease.project_id, &lease.node_id)
            || !state.leases.get(&key).is_some_and(|live| {
                live.owner_id == lease.owner_id
                    && live.generation == lease.generation
                    && live.expires_at > now
            })
            || !state.resource_connections.get(&key).is_some_and(|live| {
                live.owner_id == lease.owner_id && live.generation == lease.generation
            })
        {
            return Err(CloudError::Conflict);
        }
        state
            .resource_requests
            .retain(|_, request| request.expires_at.saturating_add(60) > now);
        if state.resource_requests.len() >= 4096
            || state
                .resource_requests
                .values()
                .filter(|request| {
                    request.lease.project_id == lease.project_id
                        && request.lease.node_id == lease.node_id
                })
                .count()
                >= 256
        {
            return Err(CloudError::ResourceExhausted);
        }
        let active: Vec<_> = state
            .resource_requests
            .values()
            .filter(|request| request.reply.is_none() && request.expires_at > now)
            .collect();
        if active.len() >= 128
            || active
                .iter()
                .filter(|request| {
                    request.lease.project_id == lease.project_id
                        && request.lease.node_id == lease.node_id
                })
                .count()
                >= MAX_RESOURCE_REQUESTS
        {
            return Err(CloudError::ResourceExhausted);
        }
        let id = uuid::Uuid::now_v7().simple().to_string();
        state.resource_requests.insert(
            id.clone(),
            RuntimeResourceRequest {
                request_id: id.clone(),
                lease: lease.clone(),
                actor: caller.subject().into(),
                operation,
                created_at: now,
                expires_at: now + 20,
                dispatched: false,
                reply: None,
            },
        );
        Ok(id)
    }
    pub(super) async fn memory_resource_take(
        &self,
        lease: &ConnectionLease,
        now: u64,
    ) -> CloudResult<Vec<RuntimeResourceRequest>> {
        let mut state = self.state.lock().await;
        if !enrolled_live(&state, &lease.project_id, &lease.node_id)
            || !state
                .leases
                .get(&(lease.project_id.clone(), lease.node_id.clone()))
                .is_some_and(|live| {
                    live.owner_id == lease.owner_id
                        && live.generation == lease.generation
                        && live.expires_at > now
                })
        {
            return Err(CloudError::Conflict);
        }
        let mut result = Vec::new();
        for request in state
            .resource_requests
            .values_mut()
            .filter(|request| {
                request.lease.project_id == lease.project_id
                    && request.lease.node_id == lease.node_id
                    && request.lease.owner_id == lease.owner_id
                    && request.lease.generation == lease.generation
                    && request.expires_at > now
                    && !request.dispatched
                    && request.reply.is_none()
            })
            .take(MAX_RESOURCE_REQUESTS)
        {
            request.dispatched = true;
            result.push(request.clone());
        }
        Ok(result)
    }
    pub(super) async fn memory_resource_complete(
        &self,
        lease: &ConnectionLease,
        id: &str,
        reply: ResourceReply,
        now: u64,
    ) -> CloudResult<()> {
        let mut state = self.state.lock().await;
        if !enrolled_live(&state, &lease.project_id, &lease.node_id)
            || !state
                .leases
                .get(&(lease.project_id.clone(), lease.node_id.clone()))
                .is_some_and(|live| {
                    live.owner_id == lease.owner_id
                        && live.generation == lease.generation
                        && live.expires_at > now
                })
        {
            return Err(CloudError::Conflict);
        }
        let Some(request) = state.resource_requests.get_mut(id).filter(|request| {
            request.lease.project_id == lease.project_id
                && request.lease.node_id == lease.node_id
                && request.lease.owner_id == lease.owner_id
                && request.lease.generation == lease.generation
                && request.dispatched
                && request.expires_at > now
        }) else {
            return Ok(());
        };
        if request.reply.is_none() {
            request.reply = Some(reply);
        }
        Ok(())
    }
    pub(super) async fn memory_resource_read(
        &self,
        caller: &crate::CloudCaller,
        id: &str,
    ) -> CloudResult<Option<ResourceReply>> {
        caller.require(crate::CloudPermission::Read)?;
        let state = self.state.lock().await;
        state
            .resource_requests
            .get(id)
            .filter(|request| {
                request.lease.project_id == caller.project_id() && request.actor == caller.subject()
            })
            .map(|request| request.reply.clone())
            .ok_or(CloudError::NotFound)
    }
}
