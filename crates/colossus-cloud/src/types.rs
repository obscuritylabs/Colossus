use colossus_cloud_protocol::{CloudReply, Command};
use colossus_sdk::{CreateRunRequest, GetRunResponse};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One enrolled node bound to a project, local instance, and TLS client certificate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudNode {
    /// Stable registered machine identity.
    pub node_id: String,
    /// Owning cloud project.
    pub project_id: String,
    /// Exact local Colossus instance independently verified by the connector SDK.
    pub instance_id: String,
    /// Bounded human-readable display label.
    pub label: String,
    /// SHA-256 of the enrolled TLS client leaf, never credential material.
    pub certificate_sha256: String,
    /// Additional cloud role ceiling, intersected with the local application grant.
    pub roles: BTreeSet<String>,
    /// Durable revocation; active streams recheck before accepting commands or output.
    pub revoked: bool,
    /// Journal revision used for exact enrollment/revocation operations.
    pub revision: u64,
}

/// Immutable task allocation. Accepted work is never moved to another node implicitly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudTask {
    /// Stable cloud identity.
    pub task_id: String,
    /// Owning project.
    pub project_id: String,
    /// Exact enrolled execution target.
    pub node_id: String,
    /// Authenticated submitting subject.
    pub subject: String,
    /// Reviewed SDK request with a cloud-derived idempotency key.
    pub request: CreateRunRequest,
    /// Exact local run identity after durable allocation is acknowledged.
    pub run_id: Option<String>,
    /// Latest released caller-visible runtime snapshot.
    pub snapshot: Option<GetRunResponse>,
    /// Released failure settling initial dispatch; unknown outcomes remain explicit.
    pub dispatch_error: Option<colossus_sdk::ApiError>,
    /// Highest contiguous durably received local run sequence.
    pub last_sequence: u64,
    /// Aggregate released event bytes retained for this task.
    pub released_bytes: usize,
    /// Output retention reached its bound; snapshots/control continue on the fixed run.
    #[serde(default)]
    pub output_limited: bool,
    /// Optimistic journal revision.
    pub revision: u64,
}

/// Durable command and receipt; replay carries the same runtime idempotency identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCommand {
    /// Stable cloud-generated dispatch/mutation identity.
    pub command_id: String,
    /// Exact immutable task allocation.
    pub task_id: String,
    /// Exact enrolled target.
    pub node_id: String,
    /// Closed typed SDK operation.
    pub command: Command,
    /// Durable reply, absent until reconciled.
    pub reply: Option<CloudReply>,
    /// Optimistic journal revision.
    pub revision: u64,
}
