use colossus_cloud_protocol::{CloudReply, Command};
use colossus_sdk::{CreateRunRequest, GetRunResponse};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Explicit native host grouping; never grants execution authority.
    #[serde(default)]
    pub host_id: Option<String>,
    /// Opaque workspace identity advertised by the enrolled runtime.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Sanitized display label, without a filesystem path.
    #[serde(default)]
    pub workspace_label: Option<String>,
    /// Fresh authenticated heartbeat says the runtime is usable.
    #[serde(default)]
    pub runtime_ready: bool,
    /// Metadata released by this authenticated runtime, never independent attestation.
    #[serde(default)]
    pub policy: Option<colossus_sdk::RuntimePolicyPosture>,
    /// Last successful authenticated policy observation, UTC Unix seconds.
    #[serde(default)]
    pub policy_observed_at: Option<u64>,
    /// Optimistic cloud database revision.
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
    /// UTC RFC3339 allocation time.
    #[serde(default)]
    pub created_at: String,
    /// UTC RFC3339 latest durable cloud change.
    #[serde(default)]
    pub updated_at: String,
    /// Reviewed SDK request with a cloud-derived idempotency key.
    pub request: CreateRunRequest,
    /// Owning human conversation; legacy tasks are mapped on discovery.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Imported source runs retain read-only control authority.
    #[serde(default)]
    pub source_read_only: bool,
    /// Public canonical history completed its bounded synchronization.
    #[serde(default)]
    pub history_complete: bool,
    /// Source public history applied its explicit content bound.
    #[serde(default)]
    pub history_bounded: bool,
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

/// Project-visible native host grouping, independent of child runtime grants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudHost {
    /// Stable native installation identity.
    pub host_id: String,
    /// Owning project.
    pub project_id: String,
    /// Sanitized display name.
    pub label: String,
    /// Native operating system name.
    pub platform: String,
    /// Desktop or independent CLI deployment.
    pub deployment_kind: String,
    /// Last authenticated contact, UTC Unix seconds.
    pub last_seen_at: u64,
    /// Optimistic metadata revision.
    pub revision: u64,
}
/// One independently scoped workspace advertised by a native runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudWorkspace {
    /// Native opaque workspace identity.
    pub workspace_id: String,
    /// Owning project.
    pub project_id: String,
    /// Host installation grouping.
    pub host_id: String,
    /// Independently enrolled execution target.
    pub node_id: String,
    /// Sanitized label without local paths.
    pub label: String,
    /// Locally selected sharing mode.
    pub sharing: String,
    /// Optimistic metadata revision.
    pub revision: u64,
}
/// Human-operated cloud conversation mapped to one exact runtime session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudThread {
    /// Stable cloud conversation identity.
    pub thread_id: String,
    /// Owning authenticated project.
    pub project_id: String,
    /// Immutable enrolled runtime placement.
    pub node_id: String,
    /// Native host grouping if advertised.
    pub host_id: Option<String>,
    /// Native workspace identity if advertised.
    pub workspace_id: Option<String>,
    /// Human-editable bounded conversation title.
    pub title: String,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// UTC RFC3339 last conversation change.
    pub updated_at: String,
    /// Optimistic user-visible metadata revision.
    pub revision: u64,
    /// Human archive preference.
    pub archived: bool,
    /// Exact local session; absent until first durable receipt.
    pub session_id: Option<String>,
    /// Current or incomplete released history projection.
    pub sync_status: String,
    /// Cloud-created or runtime-discovered source.
    pub source: String,
    /// Runtime-declared continuation authority.
    pub can_continue: bool,
    /// Current cloud turn, including durable queued allocation.
    pub active_task_id: Option<String>,
    /// Ordered bounded human submission queue.
    #[serde(default)]
    pub queued_task_ids: Vec<String>,
}
/// Ordered released conversation message retained for offline cloud viewing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudMessage {
    /// Stable source or submitted-message identity.
    pub message_id: String,
    /// Owning cloud thread.
    pub thread_id: String,
    /// Authenticated project.
    pub project_id: String,
    /// User or assistant message role.
    pub role: String,
    /// Released human-visible text.
    pub text: String,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// Associated cloud task/run mapping.
    pub task_id: String,
    /// Optimistic representation revision.
    pub revision: u64,
}
/// Complete released conversation view with bounded pages for older history.
#[derive(Clone, Debug, Serialize)]
pub struct CloudThreadDetail {
    /// Conversation metadata.
    pub thread: CloudThread,
    /// Bounded newest-first task page.
    pub tasks: Vec<CloudTask>,
    /// Bounded ordered message page.
    pub messages: Vec<CloudMessage>,
    /// Current display profiles for recorded account authors, keyed by message identity.
    /// Runtime imports without an authenticated account binding are omitted.
    pub message_authors: BTreeMap<String, CloudMessageAuthor>,
    /// Older tasks require an explicit next page.
    pub next_task_cursor: Option<String>,
    /// Older messages require an explicit next page.
    pub next_message_cursor: Option<String>,
}

/// Minimal account presentation released with an authorized conversation read.
#[derive(Clone, Debug, Serialize)]
pub struct CloudMessageAuthor {
    /// Authenticated account identity; never a login or credential binding.
    pub user_id: String,
    /// Current account display name, including for retained disabled accounts.
    pub display_name: String,
}
