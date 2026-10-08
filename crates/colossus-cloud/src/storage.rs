//! Cloud-owned relational persistence contracts. Runtime journals do not implement this port.
//!
//! Each transaction commits domain records, released output, audit metadata and delivery
//! outbox entries together. Versions and cursors are local to their entity or stream.

use crate::{CloudError, CloudResult};
use colossus_ports::StoreError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::sync::Mutex;

mod records;
pub use records::{Admission, BootstrapMarker, CertificateRenewal, EntityData, EntityValue};

/// Independently versioned cloud domain tables.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Persisted human account metadata and revocation epoch.
    User,
    /// Issuer-bound OIDC subject to stable account mapping.
    OidcIdentity,
    /// Salted local password credential, host-only.
    LocalCredential,
    /// Global control-plane product settings.
    Setting,
    /// Cloud project authority boundary.
    Project,
    /// Explicit subject membership and project permission ceiling.
    Membership,
    /// Single-use encrypted OIDC authorization flow.
    AuthFlow,
    /// Registered machine or container environment.
    Host,
    /// Independently enrolled runtime.
    Node,
    /// Runtime workspace scope.
    Workspace,
    /// Human-operated conversation.
    Thread,
    /// Ordered visible conversation message.
    ThreadMessage,
    /// Exact mapping from enrolled runtime session to a shared cloud thread.
    SessionMapping,
    /// Cloud-originated work request.
    Task,
    /// Closed operation and its reconciled reply.
    Command,
    /// Immutable runtime run allocation.
    Run,
    /// Fixed-node task placement.
    NodeTask,
    /// Per-node admission counter.
    Admission,
    /// Single-use credential-hash enrollment invitation.
    Invitation,
    /// Runtime certificate renewal receipt.
    Renewal,
}

/// Project-scoped domain identity; parent separates node/thread child resources.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EntityKey {
    /// Owning domain table.
    pub kind: EntityKind,
    /// Authenticated project, empty only for invitation-hash lookup.
    pub project_id: String,
    /// Optional runtime or thread parent identity.
    pub parent_id: Option<String>,
    /// Stable domain identity, never a credential value.
    pub id: String,
}

impl EntityKey {
    /// Stable diagnostic identity used by optimistic concurrency failures.
    pub fn conflict_id(&self) -> String {
        let kind = match self.kind {
            EntityKind::User => "user",
            EntityKind::OidcIdentity => "oidc-identity",
            EntityKind::LocalCredential => "local-credential",
            EntityKind::Setting => "setting",
            EntityKind::Project => "project",
            EntityKind::Membership => "membership",
            EntityKind::AuthFlow => "auth-flow",
            EntityKind::Host => "host",
            EntityKind::Node => "node",
            EntityKind::Workspace => "workspace",
            EntityKind::Thread => "thread",
            EntityKind::ThreadMessage => "message",
            EntityKind::SessionMapping => "session",
            EntityKind::Task => "task",
            EntityKind::Command => "command",
            EntityKind::Run => "run",
            EntityKind::NodeTask => "node-task",
            EntityKind::Admission => "admission",
            EntityKind::Invitation => "invitation",
            EntityKind::Renewal => "renewal",
        };
        if self.kind == EntityKind::Invitation {
            return format!("cloud.invitation:{}", self.id);
        }
        match &self.parent_id {
            Some(parent) => format!("cloud.{kind}:{}:{parent}:{}", self.project_id, self.id),
            None => format!("cloud.{kind}:{}:{}", self.project_id, self.id),
        }
    }
}

/// Versioned cloud resource with its typed service representation.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityRecord {
    /// Project and resource identity.
    pub key: EntityKey,
    /// Monotonically increasing aggregate version.
    pub revision: u64,
    /// Typed domain value reconstructed by the persistence adapter.
    pub value: EntityValue,
    /// Stable returned pagination position; absent for point reads.
    pub page_cursor: Option<String>,
}

/// Atomic create or compare-and-swap mutation.
#[derive(Clone, Debug)]
pub struct EntityMutation {
    /// Exact resource identity.
    pub key: EntityKey,
    /// Zero creates, otherwise the exact currently observed version.
    pub expected_revision: u64,
    /// New typed domain value mapped to storage by the adapter.
    pub value: EntityValue,
    /// Authenticated actor identifier, without credential data.
    pub actor: String,
    /// Bounded domain operation name used for audit and delivery.
    pub operation: String,
}

/// Released output received from an independently authenticated source.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleasedEvent {
    /// Owning authenticated project.
    pub project_id: String,
    /// Run/task or workspace feed identity.
    pub scope_id: String,
    /// Source stream sequence, starting at one.
    pub sequence: u64,
    /// Caller-released, bounded payload only.
    pub value: Value,
}

/// Exact acknowledgement advancement for a retained source feed.
#[derive(Clone, Debug)]
pub struct CursorMutation {
    /// Owning project.
    pub project_id: String,
    /// Runtime or feed owner.
    pub source_id: String,
    /// Source feed identity.
    pub scope_id: String,
    /// Exact current committed source cursor.
    pub expected_sequence: u64,
    /// New source cursor, never lower than the previous one.
    pub sequence: u64,
}

/// One atomic cloud mutation; no cloud-wide journal-head serialization.
#[derive(Clone, Debug, Default)]
pub struct CloudTransaction {
    /// Versioned domain changes.
    pub entities: Vec<EntityMutation>,
    /// Released stream additions.
    pub events: Vec<ReleasedEvent>,
    /// Synchronization acknowledgements advanced only after retention commits.
    pub cursors: Vec<CursorMutation>,
    /// Exact replica connection generation checked under the same transaction lock.
    pub lease: Option<ConnectionLease>,
}

/// Stable bounded listing order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityOrder {
    /// Identifier ascending with an exclusive identifier cursor.
    #[default]
    IdAsc,
    /// Identifier descending with an exclusive identifier cursor.
    IdDesc,
    /// Most recently updated first; cursor resolves the resource timestamp.
    UpdatedDesc,
    /// Oldest creation first with the resource identifier breaking timestamp ties.
    CreatedAsc,
    /// Newest creation first with the resource identifier breaking timestamp ties.
    CreatedDesc,
}

/// Validated stable timestamp/resource position decoded from a collection cursor.
#[derive(Clone, Debug)]
pub struct CloudPagePosition {
    /// Exact timestamp returned by the storage adapter for the previous page boundary.
    pub timestamp: String,
    /// Boundary entity identifier.
    pub id: String,
    /// Parent used to deterministically break equal identifier/timestamp ties.
    pub parent_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedCloudCursor {
    version: u8,
    query_sha256: String,
    timestamp: String,
    id: String,
    parent_id: String,
}
fn cursor_query_hash(query: &EntityQuery) -> CloudResult<String> {
    use sha2::Digest;
    let base = (
        query.kind,
        &query.project_id,
        &query.parent_id,
        &query.node_id,
        &query.query,
        &query.status,
        query.archived,
        query.order,
    );
    // Preserve existing cursors when no new host filter is selected.
    let bytes = if let Some(host) = &query.host_id {
        serde_json::to_vec(&(base, host))
    } else {
        serde_json::to_vec(&base)
    }
    .map_err(|_| CloudError::InvalidArgument)?;
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}
/// Encode a stable, query-bound pagination position. This is not an authorization token.
pub fn encode_page_cursor(
    query: &EntityQuery,
    timestamp: &str,
    id: &str,
    parent: &str,
) -> CloudResult<String> {
    if matches!(query.order, EntityOrder::IdAsc | EntityOrder::IdDesc) {
        return Ok(id.into());
    }
    time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
        .map_err(|_| CloudError::Storage)?;
    let value = EncodedCloudCursor {
        version: 1,
        query_sha256: cursor_query_hash(query)?,
        timestamp: timestamp.into(),
        id: id.into(),
        parent_id: parent.into(),
    };
    Ok(format!(
        "cp1_{}",
        hex::encode(serde_json::to_vec(&value).map_err(|_| CloudError::Storage)?)
    ))
}
/// Decode and bind an opaque cursor to the exact project/filter/order collection.
pub fn decode_page_cursor(query: &EntityQuery) -> CloudResult<Option<CloudPagePosition>> {
    let Some(after) = &query.after else {
        return Ok(None);
    };
    if matches!(query.order, EntityOrder::IdAsc | EntityOrder::IdDesc) {
        return Ok(None);
    }
    if after.len() > 2048 {
        return Err(CloudError::InvalidArgument);
    }
    let bytes = hex::decode(
        after
            .strip_prefix("cp1_")
            .ok_or(CloudError::InvalidArgument)?,
    )
    .map_err(|_| CloudError::InvalidArgument)?;
    let value: EncodedCloudCursor =
        serde_json::from_slice(&bytes).map_err(|_| CloudError::InvalidArgument)?;
    if value.version != 1
        || value.query_sha256 != cursor_query_hash(query)?
        || value.id.is_empty()
        || value.id.len() > 128
        || value.parent_id.len() > 128
    {
        return Err(CloudError::InvalidArgument);
    }
    time::OffsetDateTime::parse(
        &value.timestamp,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|_| CloudError::InvalidArgument)?;
    Ok(Some(CloudPagePosition {
        timestamp: value.timestamp,
        id: value.id,
        parent_id: value.parent_id,
    }))
}

/// Bounded server-side project query. Hosts supply all authority decisions.
#[derive(Clone, Debug)]
pub struct EntityQuery {
    /// Domain collection.
    pub kind: EntityKind,
    /// Exact authenticated project.
    pub project_id: String,
    /// Optional node/thread parent filter.
    pub parent_id: Option<String>,
    /// Exclusive resource identifier cursor.
    pub after: Option<String>,
    /// At most 100 records; zero returns no records.
    pub limit: usize,
    /// Optional immutable runtime placement filter.
    pub node_id: Option<String>,
    /// Optional native host grouping filter, beneath the authenticated project.
    pub host_id: Option<String>,
    /// Literal substring search against the projected title/label.
    pub query: Option<String>,
    /// Exact domain status filter.
    pub status: Option<String>,
    /// Conversation archive state filter.
    pub archived: Option<bool>,
    /// Deterministic ordering.
    pub order: EntityOrder,
}
impl Default for EntityQuery {
    fn default() -> Self {
        Self {
            kind: EntityKind::Node,
            project_id: String::new(),
            parent_id: None,
            after: None,
            limit: 100,
            node_id: None,
            host_id: None,
            query: None,
            status: None,
            archived: None,
            order: EntityOrder::IdAsc,
        }
    }
}
impl EntityQuery {
    /// Create a bounded default query for one explicitly selected collection.
    pub fn new(kind: EntityKind, project_id: String) -> Self {
        Self {
            kind,
            project_id,
            ..Default::default()
        }
    }
}

/// Credential-free persistent browser session; raw cookie and CSRF tokens stay transient.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthSession {
    /// SHA-256 of the random browser cookie.
    pub session_hash: String,
    /// Verified OIDC subject identifier.
    pub subject: String,
    /// SHA-256 of the independently generated CSRF secret.
    pub csrf_hash: String,
    /// UTC Unix creation time.
    pub created_at: u64,
    /// UTC Unix expiry time.
    pub expires_at: u64,
    /// Account credential/disablement generation at authentication.
    #[serde(default)]
    pub security_epoch: u64,
}

/// Replica connection ownership with a monotonic fencing generation.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionLease {
    /// Owning project.
    pub project_id: String,
    /// Exact enrolled runtime.
    pub node_id: String,
    /// Cloud replica identity.
    pub owner_id: String,
    /// Fencing generation; stale owners never pass verification.
    pub generation: u64,
    /// UTC Unix expiry.
    pub expires_at: u64,
}

/// Bounded operational metadata retention, independent of canonical history/audit retention.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudMaintenancePolicy {
    /// At most this many records per collection per maintenance transaction.
    pub batch_limit: usize,
    /// Keep published wakeup metadata for this many seconds before deletion.
    pub delivered_outbox_retention_seconds: u64,
}
impl Default for CloudMaintenancePolicy {
    fn default() -> Self {
        Self {
            batch_limit: 256,
            delivered_outbox_retention_seconds: 86400,
        }
    }
}
/// Credential-free result of a bounded operational metadata maintenance pass.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CloudMaintenanceReport {
    /// Durable hints published and marked, independently of command receipts.
    pub published_outbox: usize,
    /// Old published hint metadata removed.
    pub removed_outbox: usize,
    /// Expired browser sessions removed.
    pub expired_sessions: usize,
    /// Expired OIDC flows consumed and removed; unexpired maintenance metadata remains.
    pub expired_auth_flows: usize,
}

/// Object-safe asynchronous cloud persistence boundary.
#[async_trait::async_trait]
pub trait CloudStore: Send + Sync {
    /// Publish durable wakeup hints and prune only bounded expired operational metadata.
    async fn maintain(
        &self,
        _now: u64,
        _policy: &CloudMaintenancePolicy,
    ) -> CloudResult<CloudMaintenanceReport> {
        Ok(CloudMaintenanceReport::default())
    }
    /// Subscribe to bounded project-only wakeups; always resume from durable rows.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String>;
    /// Verify storage is available without releasing domain payloads.
    async fn readiness(&self) -> CloudResult<()>;
    /// Scoped operational projections, independent of permission and policy decisions.
    async fn statistics(
        &self,
        project: &str,
        node: Option<&str>,
        now: u64,
        days: u16,
    ) -> CloudResult<crate::observability::OperationalStatistics> {
        crate::observability::collect(self, project, node, now, days).await
    }
    /// Read an exact project resource and its revision.
    async fn read(&self, key: &EntityKey) -> CloudResult<EntityRecord>;
    /// Consume an exact entity revision atomically, retaining its tombstone and audit.
    async fn delete_entity(&self, key: &EntityKey, expected_revision: u64) -> CloudResult<()>;
    /// Read a bounded project collection using indexed domain columns.
    async fn list(&self, query: &EntityQuery) -> CloudResult<Vec<EntityRecord>>;
    /// Bounded project catalog for verified global administrators only.
    async fn list_projects(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<EntityRecord>>;
    /// Host-only lookup of account bindings; callers must strip local credential values.
    async fn user_identities(&self, user_id: &str) -> CloudResult<Vec<EntityRecord>>;
    /// Bounded audited account batch for already authorized membership rendering.
    async fn user_accounts(&self, user_ids: &[String]) -> CloudResult<Vec<EntityRecord>>;
    /// Read bounded explicit project memberships for an already verified OIDC subject.
    async fn memberships(&self, subject: &str) -> CloudResult<Vec<EntityRecord>>;
    /// Check all retained turns using an indexed incomplete-source existence query.
    async fn thread_incomplete(&self, project: &str, thread: &str) -> CloudResult<bool>;
    /// Commit all effects atomically; preserve conflicts and uncertain commit outcomes.
    async fn commit(&self, transaction: CloudTransaction) -> Result<(), StoreError>;
    /// Read retained released events after an exclusive committed source cursor.
    async fn events(
        &self,
        project: &str,
        scope: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<ReleasedEvent>>;
    /// Read the committed acknowledgement for a runtime feed.
    async fn cursor(&self, project: &str, source: &str, scope: &str) -> CloudResult<u64>;
    /// Persist only opaque token hashes and verified identity metadata.
    async fn put_session(&self, session: AuthSession) -> CloudResult<()>;
    /// Read a nonexpired browser session; expired or unknown sessions are absent.
    async fn read_session(&self, hash: &str, now: u64) -> CloudResult<AuthSession>;
    /// Revoke an exact browser session.
    async fn delete_session(&self, hash: &str) -> CloudResult<()>;
    /// Claim a free/expired connection; concurrent live owners conflict.
    async fn claim_lease(
        &self,
        project: &str,
        node: &str,
        owner: &str,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease>;
    /// Read live ownership for cross-replica fleet presence.
    async fn read_lease(&self, project: &str, node: &str, now: u64)
    -> CloudResult<ConnectionLease>;
    /// Extend the exact live ownership generation.
    async fn renew_lease(
        &self,
        lease: &ConnectionLease,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease>;
    /// Release only the exact ownership generation.
    async fn release_lease(&self, lease: &ConnectionLease) -> CloudResult<()>;
    /// Refuse an expired or superseded owner before issuing/accepting operations.
    async fn verify_lease(&self, lease: &ConnectionLease, now: u64) -> CloudResult<()>;
}

/// Deterministic test adapter. Production compositions must use PostgreSQL.
pub struct MemoryCloudStore {
    state: Mutex<MemoryState>,
    notifications: tokio::sync::broadcast::Sender<String>,
}
impl Default for MemoryCloudStore {
    fn default() -> Self {
        let (notifications, _) = tokio::sync::broadcast::channel(256);
        Self {
            state: Mutex::new(MemoryState::default()),
            notifications,
        }
    }
}

#[derive(Clone, Default)]
struct MemoryState {
    entities: BTreeMap<EntityKey, EntityRecord>,
    deleted: std::collections::BTreeSet<EntityKey>,
    events: BTreeMap<(String, String, u64), ReleasedEvent>,
    cursors: BTreeMap<(String, String, String), u64>,
    sessions: BTreeMap<String, AuthSession>,
    leases: BTreeMap<(String, String), ConnectionLease>,
}
fn enrolled_live(state: &MemoryState, project: &str, node: &str) -> bool {
    let key = EntityKey {
        kind: EntityKind::Node,
        project_id: project.into(),
        parent_id: None,
        id: node.into(),
    };
    !state.deleted.contains(&key)
        && state.entities.get(&key).is_some_and(
            |record| matches!(&record.value, EntityValue::Node(value) if !value.revoked),
        )
}

#[async_trait::async_trait]
impl CloudStore for MemoryCloudStore {
    async fn user_accounts(&self, user_ids: &[String]) -> CloudResult<Vec<EntityRecord>> {
        if user_ids.len() > 100 {
            return Err(CloudError::InvalidArgument);
        }
        let state = self.state.lock().await;
        Ok(user_ids
            .iter()
            .filter_map(|id| {
                let key = crate::identity_key(EntityKind::User, id);
                if state.deleted.contains(&key) {
                    None
                } else {
                    state.entities.get(&key).cloned()
                }
            })
            .collect())
    }
    async fn user_identities(&self, user_id: &str) -> CloudResult<Vec<EntityRecord>> {
        let state = self.state.lock().await;
        Ok(state
            .entities
            .values()
            .filter(|r| {
                matches!(
                    r.key.kind,
                    EntityKind::LocalCredential | EntityKind::OidcIdentity
                ) && !state.deleted.contains(&r.key)
                    && r.value.user_id() == Some(user_id)
            })
            .take(16)
            .cloned()
            .collect())
    }
    async fn list_projects(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<EntityRecord>> {
        if !(1..=100).contains(&limit) {
            return Err(CloudError::InvalidArgument);
        }
        let state = self.state.lock().await;
        Ok(state
            .entities
            .values()
            .filter(|r| {
                r.key.kind == EntityKind::Project
                    && !state.deleted.contains(&r.key)
                    && after.is_none_or(|id| r.key.id.as_str() > id)
            })
            .take(limit)
            .cloned()
            .collect())
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.notifications.subscribe()
    }
    async fn readiness(&self) -> CloudResult<()> {
        Ok(())
    }
    async fn read(&self, key: &EntityKey) -> CloudResult<EntityRecord> {
        let state = self.state.lock().await;
        state
            .entities
            .get(key)
            .or_else(|| {
                (key.kind == EntityKind::Invitation && key.project_id.is_empty())
                    .then(|| {
                        state
                            .entities
                            .values()
                            .find(|r| r.key.kind == key.kind && r.key.id == key.id)
                    })
                    .flatten()
            })
            .filter(|r| !state.deleted.contains(&r.key))
            .cloned()
            .ok_or(CloudError::NotFound)
    }
    async fn delete_entity(&self, key: &EntityKey, expected_revision: u64) -> CloudResult<()> {
        let mut state = self.state.lock().await;
        let current = state.entities.get_mut(key).ok_or(CloudError::NotFound)?;
        if current.revision != expected_revision {
            return Err(CloudError::Conflict);
        }
        current.revision += 1;
        state.deleted.insert(key.clone());
        Ok(())
    }
    async fn list(&self, query: &EntityQuery) -> CloudResult<Vec<EntityRecord>> {
        let state = self.state.lock().await;
        let mut records: Vec<_> = state
            .entities
            .values()
            .filter(|r| {
                !state.deleted.contains(&r.key)
                    && r.key.kind == query.kind
                    && r.key.project_id == query.project_id
                    && query.parent_id.as_ref().is_none_or(|p| {
                        if query.kind == EntityKind::Task {
                            r.value.thread_id() == Some(p)
                        } else {
                            r.key.parent_id.as_ref() == Some(p)
                        }
                    })
                    && query.node_id.as_ref().is_none_or(|n| {
                        r.value.node_id().or(match r.key.kind {
                            EntityKind::Run | EntityKind::NodeTask | EntityKind::SessionMapping => {
                                r.key.parent_id.as_deref()
                            }
                            EntityKind::Admission | EntityKind::Renewal => Some(r.key.id.as_str()),
                            _ => None,
                        }) == Some(n)
                    })
                    && query.query.as_ref().is_none_or(|q| {
                        r.value
                            .search_text()
                            .to_lowercase()
                            .contains(&q.to_lowercase())
                    })
                    && query.host_id.as_ref().is_none_or(|host| match &r.value {
                        EntityValue::Node(value) => value.host_id.as_ref() == Some(host),
                        EntityValue::Workspace(value) => &value.host_id == host,
                        EntityValue::Thread(value) => value.host_id.as_ref() == Some(host),
                        _ => false,
                    })
                    && query.status.as_ref().is_none_or(|status| {
                        if query.kind == EntityKind::User && status == "administrator" {
                            r.value.active_administrator()
                        } else {
                            r.value.status() == status
                        }
                    })
                    && query
                        .archived
                        .is_none_or(|archived| r.value.archived() == archived)
            })
            .cloned()
            .collect();
        let timestamp = |record: &EntityRecord| {
            match query.order {
                EntityOrder::UpdatedDesc => record.value.updated_at(),
                EntityOrder::CreatedAsc | EntityOrder::CreatedDesc => record.value.created_at(),
                _ => None,
            }
            .filter(|value| !value.is_empty())
            .unwrap_or("1970-01-01T00:00:00Z")
            .to_owned()
        };
        let epoch = |value: &str| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                .map(|time| time.unix_timestamp_nanos())
                .unwrap_or(0)
        };
        let key = |record: &EntityRecord| {
            (
                epoch(&timestamp(record)),
                record.key.id.clone(),
                record.key.parent_id.clone().unwrap_or_default(),
            )
        };
        records.sort_by_key(|record| key(record));
        if matches!(
            query.order,
            EntityOrder::IdDesc | EntityOrder::UpdatedDesc | EntityOrder::CreatedDesc
        ) {
            records.reverse();
        }
        let position = decode_page_cursor(query)?;
        let records = records
            .into_iter()
            .filter(|record| match query.order {
                EntityOrder::IdAsc => query
                    .after
                    .as_ref()
                    .is_none_or(|after| record.key.id > *after),
                EntityOrder::IdDesc => query
                    .after
                    .as_ref()
                    .is_none_or(|after| record.key.id < *after),
                EntityOrder::CreatedAsc => position.as_ref().is_none_or(|position| {
                    key(record)
                        > (
                            epoch(&position.timestamp),
                            position.id.clone(),
                            position.parent_id.clone(),
                        )
                }),
                _ => position.as_ref().is_none_or(|position| {
                    key(record)
                        < (
                            epoch(&position.timestamp),
                            position.id.clone(),
                            position.parent_id.clone(),
                        )
                }),
            })
            .take(query.limit.min(100));
        records
            .map(|mut record| {
                record.page_cursor = Some(encode_page_cursor(
                    query,
                    &timestamp(&record),
                    &record.key.id,
                    record.key.parent_id.as_deref().unwrap_or_default(),
                )?);
                Ok(record)
            })
            .collect()
    }

    async fn commit(&self, transaction: CloudTransaction) -> Result<(), StoreError> {
        let mut guard = self.state.lock().await;
        let mut state = guard.clone();
        let structural = transaction
            .entities
            .iter()
            .any(|m| matches!(m.key.kind, EntityKind::User | EntityKind::Project));
        let had_admin = state.entities.values().any(|r| {
            r.key.kind == EntityKind::User
                && !state.deleted.contains(&r.key)
                && r.value.active_administrator()
        });
        let projects: std::collections::BTreeSet<_> = transaction
            .entities
            .iter()
            .map(|m| m.key.project_id.clone())
            .chain(transaction.events.iter().map(|e| e.project_id.clone()))
            .chain(transaction.cursors.iter().map(|c| c.project_id.clone()))
            .collect();
        if let Some(lease) = &transaction.lease {
            if !enrolled_live(&state, &lease.project_id, &lease.node_id) {
                return Err(StoreError::WriterLeaseHeld);
            }
            if projects.iter().any(|project| *project != lease.project_id) {
                return Err(StoreError::Adapter(
                    "cloud connection cannot mutate another project".into(),
                ));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| StoreError::Adapter("cloud clock unavailable".into()))?
                .as_secs();
            if !state
                .leases
                .get(&(lease.project_id.clone(), lease.node_id.clone()))
                .is_some_and(|l| {
                    l.generation == lease.generation
                        && l.owner_id == lease.owner_id
                        && l.expires_at > now
                })
            {
                return Err(StoreError::WriterLeaseHeld);
            }
        }
        if had_admin
            && transaction.entities.iter().any(|m| {
                m.key.kind == EntityKind::User
                    && m.actor == "operator-bootstrap"
                    && matches!(&m.value, EntityValue::User(value) if value.user.is_admin)
            })
        {
            return Err(StoreError::Conflict {
                stream_id: "cloud.administrator-bootstrap".into(),
                expected: 0,
                actual: 1,
            });
        }
        for mut mutation in transaction.entities {
            if mutation.value.validate_key(&mutation.key).is_err() {
                return Err(StoreError::Adapter("cloud entity kind mismatch".into()));
            }
            mutation.value.set_revision(
                mutation
                    .expected_revision
                    .checked_add(1)
                    .ok_or_else(|| StoreError::Adapter("cloud revision bound exceeded".into()))?,
            );
            if mutation.key.kind == EntityKind::Invitation
                && state.entities.keys().any(|key| {
                    key.kind == EntityKind::Invitation
                        && key.id == mutation.key.id
                        && key.project_id != mutation.key.project_id
                })
            {
                return Err(conflict(&mutation.key, mutation.expected_revision, 1));
            }
            let actual = state.entities.get(&mutation.key).map_or(0, |r| r.revision);
            if actual != mutation.expected_revision {
                return Err(conflict(&mutation.key, mutation.expected_revision, actual));
            }
            state.entities.insert(
                mutation.key.clone(),
                EntityRecord {
                    key: mutation.key,
                    revision: actual + 1,
                    value: mutation.value,
                    page_cursor: None,
                },
            );
        }
        for event in transaction.events {
            let key = (
                event.project_id.clone(),
                event.scope_id.clone(),
                event.sequence,
            );
            if let Some(prior) = state.events.get(&key) {
                if prior != &event {
                    return Err(StoreError::Conflict {
                        stream_id: event.scope_id,
                        expected: event.sequence,
                        actual: event.sequence,
                    });
                }
                continue;
            }
            let last = state
                .events
                .keys()
                .filter(|k| k.0 == key.0 && k.1 == key.1)
                .map(|k| k.2)
                .max()
                .unwrap_or(0);
            if event.sequence != last + 1 {
                return Err(StoreError::Conflict {
                    stream_id: event.scope_id,
                    expected: last + 1,
                    actual: event.sequence,
                });
            }
            state.events.insert(key, event);
        }
        for cursor in transaction.cursors {
            let key = (cursor.project_id, cursor.source_id, cursor.scope_id.clone());
            let actual = state.cursors.get(&key).copied().unwrap_or(0);
            if actual != cursor.expected_sequence || cursor.sequence < actual {
                return Err(StoreError::Conflict {
                    stream_id: cursor.scope_id,
                    expected: cursor.expected_sequence,
                    actual,
                });
            }
            state.cursors.insert(key, cursor.sequence);
        }
        if structural {
            let has_admin = state.entities.values().any(|r| {
                r.key.kind == EntityKind::User
                    && !state.deleted.contains(&r.key)
                    && r.value.active_administrator()
            });
            if had_admin && !has_admin {
                return Err(StoreError::Conflict {
                    stream_id: "cloud.last-administrator".into(),
                    expected: 1,
                    actual: 0,
                });
            }
            for project in state
                .entities
                .values()
                .filter(|r| r.key.kind == EntityKind::Project && !state.deleted.contains(&r.key))
            {
                let mut path = std::collections::BTreeSet::new();
                let mut next = Some(project.key.id.clone());
                while let Some(id) = next {
                    if !path.insert(id.clone()) || path.len() > 64 {
                        return Err(StoreError::Conflict {
                            stream_id: "cloud.project-hierarchy".into(),
                            expected: 0,
                            actual: 1,
                        });
                    }
                    next = state
                        .entities
                        .get(&crate::project_key(&id))
                        .and_then(|r| match &r.value {
                            EntityValue::Project(project) => project.parent_project_id.as_deref(),
                            _ => None,
                        })
                        .map(str::to_owned);
                }
            }
        }
        *guard = state;
        for project in projects {
            let _ = self.notifications.send(project);
        }
        Ok(())
    }
    async fn memberships(&self, subject: &str) -> CloudResult<Vec<EntityRecord>> {
        let state = self.state.lock().await;
        Ok(state
            .entities
            .values()
            .filter(|r| {
                !state.deleted.contains(&r.key)
                    && r.key.kind == EntityKind::Membership
                    && matches!(&r.value, EntityValue::Membership(value) if value.subject == subject)
            })
            .take(1024)
            .cloned()
            .collect())
    }
    async fn thread_incomplete(&self, project: &str, thread: &str) -> CloudResult<bool> {
        let state = self.state.lock().await;
        Ok(state.entities.values().any(|record| {
            !state.deleted.contains(&record.key)
                && record.key.project_id == project
                && record.value.thread_id() == Some(thread)
                && record.value.incomplete()
        }))
    }
    async fn events(
        &self,
        project: &str,
        scope: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<ReleasedEvent>> {
        Ok(self
            .state
            .lock()
            .await
            .events
            .values()
            .filter(|e| e.project_id == project && e.scope_id == scope && e.sequence > after)
            .take(limit.min(100))
            .cloned()
            .collect())
    }
    async fn cursor(&self, project: &str, source: &str, scope: &str) -> CloudResult<u64> {
        Ok(self
            .state
            .lock()
            .await
            .cursors
            .get(&(project.into(), source.into(), scope.into()))
            .copied()
            .unwrap_or(0))
    }
    async fn put_session(&self, session: AuthSession) -> CloudResult<()> {
        validate_session(&session)?;
        self.state
            .lock()
            .await
            .sessions
            .insert(session.session_hash.clone(), session);
        Ok(())
    }
    async fn read_session(&self, hash: &str, now: u64) -> CloudResult<AuthSession> {
        self.state
            .lock()
            .await
            .sessions
            .get(hash)
            .filter(|s| s.expires_at > now)
            .cloned()
            .ok_or(CloudError::NotFound)
    }
    async fn delete_session(&self, hash: &str) -> CloudResult<()> {
        self.state.lock().await.sessions.remove(hash);
        Ok(())
    }
    async fn claim_lease(
        &self,
        project: &str,
        node: &str,
        owner: &str,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        validate_lease_ttl(ttl)?;
        let mut state = self.state.lock().await;
        if !enrolled_live(&state, project, node) {
            return Err(CloudError::PermissionDenied);
        }
        let key = (project.into(), node.into());
        let old = state.leases.get(&key);
        if old.is_some_and(|l| l.expires_at > now && l.owner_id != owner) {
            return Err(CloudError::Conflict);
        }
        let lease = ConnectionLease {
            project_id: project.into(),
            node_id: node.into(),
            owner_id: owner.into(),
            generation: old.map_or(1, |l| l.generation + 1),
            expires_at: now.saturating_add(ttl),
        };
        state.leases.insert(key, lease.clone());
        Ok(lease)
    }
    async fn read_lease(
        &self,
        project: &str,
        node: &str,
        now: u64,
    ) -> CloudResult<ConnectionLease> {
        let state = self.state.lock().await;
        if !enrolled_live(&state, project, node) {
            return Err(CloudError::NotFound);
        }
        state
            .leases
            .get(&(project.into(), node.into()))
            .filter(|l| l.expires_at > now)
            .cloned()
            .ok_or(CloudError::NotFound)
    }
    async fn renew_lease(
        &self,
        lease: &ConnectionLease,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        validate_lease_ttl(ttl)?;
        let mut state = self.state.lock().await;
        if !enrolled_live(&state, &lease.project_id, &lease.node_id) {
            return Err(CloudError::Conflict);
        }
        let current = state
            .leases
            .get_mut(&(lease.project_id.clone(), lease.node_id.clone()))
            .ok_or(CloudError::Conflict)?;
        if current.owner_id != lease.owner_id
            || current.generation != lease.generation
            || current.expires_at <= now
        {
            return Err(CloudError::Conflict);
        }
        current.expires_at = now.saturating_add(ttl);
        Ok(current.clone())
    }
    async fn release_lease(&self, lease: &ConnectionLease) -> CloudResult<()> {
        let mut state = self.state.lock().await;
        if let Some(current) = state
            .leases
            .get_mut(&(lease.project_id.clone(), lease.node_id.clone()))
            && current.owner_id == lease.owner_id
            && current.generation == lease.generation
        {
            current.expires_at = 0;
        }
        Ok(())
    }
    async fn verify_lease(&self, lease: &ConnectionLease, now: u64) -> CloudResult<()> {
        let state = self.state.lock().await;
        if !enrolled_live(&state, &lease.project_id, &lease.node_id) {
            return Err(CloudError::Conflict);
        }
        state
            .leases
            .get(&(lease.project_id.clone(), lease.node_id.clone()))
            .filter(|l| {
                l.owner_id == lease.owner_id
                    && l.generation == lease.generation
                    && l.expires_at > now
            })
            .map(|_| ())
            .ok_or(CloudError::Conflict)
    }
}

/// Literal user-facing title/label projection used consistently by storage adapters.
pub fn search_text(value: &EntityValue) -> String {
    value.search_text().into()
}
/// Released domain status projection, including tasks with an SDK run snapshot.
pub fn status(value: &EntityValue) -> String {
    value.status().into()
}
/// Categorical conflict retaining the entity's exact observed revision.
pub fn conflict(key: &EntityKey, expected: u64, actual: u64) -> StoreError {
    StoreError::Conflict {
        stream_id: key.conflict_id(),
        expected,
        actual,
    }
}
/// Reject raw tokens in browser-session storage and enforce bounded identity/expiry.
pub fn validate_session(session: &AuthSession) -> CloudResult<()> {
    let hash = |value: &str| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
    if !hash(&session.session_hash)
        || (!session.csrf_hash.is_empty() && !hash(&session.csrf_hash))
        || session.subject.is_empty()
        || session.subject.len() > 512
        || session.expires_at <= session.created_at
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
/// Connection ownership expires promptly and renewal cannot create unbounded leases.
pub fn validate_lease_ttl(ttl: u64) -> CloudResult<()> {
    if !(5..=120).contains(&ttl) {
        Err(CloudError::InvalidArgument)
    } else {
        Ok(())
    }
}
