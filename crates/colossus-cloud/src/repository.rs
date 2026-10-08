use crate::storage::{
    CloudStore, CloudTransaction, ConnectionLease, CursorMutation, EntityData, EntityKey,
    EntityKind, EntityMutation, EntityOrder, EntityQuery, ReleasedEvent,
};
use crate::{
    CloudCaller, CloudError, CloudNode, CloudPermission, CloudResult, CloudTask, PendingCommand,
};
use colossus_cloud_protocol::{CloudReply, Command};
use serde::Serialize;
use std::sync::Arc;

mod accounts;
mod delivery;
mod enrollment;
mod history;
mod inventory;
mod nodes;
mod projects;
mod renewal;
mod tasks;
mod threads;

/// Project-scoped application services backed by cloud-owned relational storage.
#[derive(Clone)]
pub struct CloudRepository {
    pub(super) store: Arc<dyn CloudStore>,
    lease: Option<ConnectionLease>,
    ingest: Arc<tokio::sync::Semaphore>,
}
pub(super) enum Write {
    Entity(Box<EntityMutation>),
    Event(ReleasedEvent),
    Cursor(CursorMutation),
}
impl CloudRepository {
    /// Bind independent cloud persistence. Runtime journals never enter this boundary.
    pub fn new(store: Arc<dyn CloudStore>) -> CloudResult<Self> {
        Ok(Self {
            store,
            lease: None,
            ingest: Arc::new(tokio::sync::Semaphore::new(12)),
        })
    }
    /// Bind every mutation to the exact active connection generation.
    pub fn with_lease(&self, lease: ConnectionLease) -> Self {
        Self {
            store: self.store.clone(),
            lease: Some(lease),
            ingest: self.ingest.clone(),
        }
    }
    /// Probe cloud database readiness without releasing project payloads.
    pub async fn check_readiness(&self) -> CloudResult<()> {
        self.store.readiness().await
    }
    /// Retain the cloud persistence authority for host composition and coordination.
    pub fn storage(&self) -> Arc<dyn CloudStore> {
        self.store.clone()
    }
    async fn read<T: EntityData>(&self, identity: &str) -> CloudResult<(T, u64)> {
        let record = self.store.read(&key(identity)?).await?;
        Ok((T::from_entity(record.value)?, record.revision))
    }
    async fn list<T: EntityData>(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<T>> {
        let query = query(prefix, after, limit.min(100))?;
        self.store
            .list(&query)
            .await?
            .into_iter()
            .map(|record| T::from_entity(record.value))
            .collect()
    }
    fn released_event<T: Serialize>(
        &self,
        _subject: &str,
        identity: String,
        version: u64,
        _operation: &str,
        value: &T,
    ) -> CloudResult<Write> {
        let value = serde_json::to_value(value).map_err(|_| CloudError::InvalidArgument)?;
        if let Some(scope) = identity.strip_prefix("cloud.output:") {
            let (project_id, scope_id) =
                scope.split_once(':').ok_or(CloudError::InvalidArgument)?;
            return Ok(Write::Event(ReleasedEvent {
                project_id: project_id.into(),
                scope_id: scope_id.into(),
                sequence: version + 1,
                value,
            }));
        }
        Err(CloudError::InvalidArgument)
    }
    fn event<T: EntityData>(
        &self,
        subject: &str,
        identity: String,
        version: u64,
        operation: &str,
        value: &T,
    ) -> CloudResult<Write> {
        let mut key = key(&identity)?;
        let value = value.entity_value();
        if key.kind == EntityKind::Invitation {
            let crate::storage::EntityValue::Invitation(invitation) = &value else {
                return Err(CloudError::InvalidArgument);
            };
            key.project_id = invitation.project_id.clone();
        }
        if !value.matches_kind(key.kind) {
            return Err(CloudError::InvalidArgument);
        }
        Ok(Write::Entity(Box::new(EntityMutation {
            key,
            expected_revision: version,
            value,
            actor: subject.into(),
            operation: operation.into(),
        })))
    }
    async fn append<T: EntityData>(
        &self,
        subject: &str,
        identity: String,
        version: u64,
        operation: &str,
        value: &T,
    ) -> CloudResult<()> {
        self.commit(vec![
            self.event(subject, identity, version, operation, value)?,
        ])
        .await?;
        Ok(())
    }
    async fn commit(&self, writes: Vec<Write>) -> Result<(), colossus_ports::StoreError> {
        let mut transaction = CloudTransaction {
            lease: self.lease.clone(),
            ..Default::default()
        };
        for write in writes {
            match write {
                Write::Entity(entity) => transaction.entities.push(*entity),
                Write::Event(event) => transaction.events.push(event),
                Write::Cursor(cursor) => transaction.cursors.push(cursor),
            }
        }
        self.store.commit(transaction).await
    }
}
// Application resource names encode an exact project and optional parent.
// Storage receives their explicit table kind, project, parent and entity identities.
pub(super) fn key(identity: &str) -> CloudResult<EntityKey> {
    let (kind, tail) = identity
        .split_once(':')
        .ok_or(CloudError::InvalidArgument)?;
    let kind = match kind {
        "cloud.node" => EntityKind::Node,
        "cloud.host" => EntityKind::Host,
        "cloud.workspace" => EntityKind::Workspace,
        "cloud.thread" => EntityKind::Thread,
        "cloud.task" => EntityKind::Task,
        "cloud.command" => EntityKind::Command,
        "cloud.run" => EntityKind::Run,
        "cloud.session" => EntityKind::SessionMapping,
        "cloud.invitation" => EntityKind::Invitation,
        "cloud.renewal" => EntityKind::Renewal,
        "cloud.admission" => EntityKind::Admission,
        "cloud.node-task" => EntityKind::NodeTask,
        "cloud.message" => EntityKind::ThreadMessage,
        _ => return Err(CloudError::InvalidArgument),
    };
    if kind == EntityKind::Invitation {
        return Ok(EntityKey {
            kind,
            project_id: String::new(),
            parent_id: None,
            id: tail.into(),
        });
    }
    let (project_id, rest) = tail.split_once(':').ok_or(CloudError::InvalidArgument)?;
    let (parent_id, id) = match kind {
        EntityKind::Command
        | EntityKind::Run
        | EntityKind::SessionMapping
        | EntityKind::NodeTask
        | EntityKind::ThreadMessage => {
            let (parent, id) = rest.split_once(':').ok_or(CloudError::InvalidArgument)?;
            (Some(parent.into()), id)
        }
        _ => (None, rest),
    };
    Ok(EntityKey {
        kind,
        project_id: project_id.into(),
        parent_id,
        id: id.into(),
    })
}
fn query(prefix: &str, after: Option<&str>, limit: usize) -> CloudResult<EntityQuery> {
    let key = key(&format!("{prefix}__page"))?;
    Ok(EntityQuery {
        kind: key.kind,
        project_id: key.project_id,
        parent_id: key.parent_id,
        after: after.map(str::to_owned),
        limit,
        node_id: None,
        host_id: None,
        query: None,
        status: None,
        archived: None,
        order: EntityOrder::IdAsc,
    })
}
