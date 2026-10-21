//! Reconcile exactly the source sessions known before a bounded discovery cycle.
use super::*;
use crate::CloudThread;
use std::collections::BTreeMap;

const MAX_DISCOVERY_THREADS: usize = 16_384;

/// Controller-owned discovery boundary; never supplied by a runtime or browser.
/// Newly bound sessions cannot become omissions of an earlier runtime snapshot.
pub struct DiscoverySnapshot {
    project: String,
    node: String,
    instance: String,
    sessions: BTreeMap<String, String>,
}

impl CloudRepository {
    /// Capture bounded canonical source bindings before asking a runtime for discovery.
    /// History remains untouched if the node is stale or the scan exceeds its bound.
    pub async fn begin_discovery(&self, node: &CloudNode) -> CloudResult<DiscoverySnapshot> {
        self.live_node(node).await?;
        let mut sessions = BTreeMap::new();
        let mut after = None;
        let mut scanned = 0usize;
        loop {
            let records = self
                .store
                .list(&EntityQuery {
                    kind: EntityKind::Thread,
                    project_id: node.project_id.clone(),
                    node_id: Some(node.node_id.clone()),
                    after: after.clone(),
                    limit: 100,
                    order: EntityOrder::IdAsc,
                    ..Default::default()
                })
                .await?;
            if records.is_empty() {
                break;
            }
            scanned = scanned.saturating_add(records.len());
            if scanned > MAX_DISCOVERY_THREADS {
                return Err(CloudError::ResourceExhausted);
            }
            after = records.last().map(|record| record.key.id.clone());
            for record in records {
                let thread: CloudThread = record.value.try_into()?;
                if thread.project_id != node.project_id || thread.node_id != node.node_id {
                    return Err(CloudError::Storage);
                }
                if let Some(session) = thread.session_id {
                    sessions.insert(thread.thread_id, session);
                }
            }
        }
        Ok(DiscoverySnapshot {
            project: node.project_id.clone(),
            node: node.node_id.clone(),
            instance: node.instance_id.clone(),
            sessions,
        })
    }

    /// Withdraw omitted sources only within the exact cycle's captured session bindings.
    /// Known shared and cloud-created sources retain the same withdrawal semantics.
    pub async fn finish_discovery_snapshot(
        &self,
        node: &CloudNode,
        visible_sessions: &BTreeMap<String, bool>,
        snapshot: DiscoverySnapshot,
    ) -> CloudResult<()> {
        self.live_node(node).await?;
        if snapshot.project != node.project_id
            || snapshot.node != node.node_id
            || snapshot.instance != node.instance_id
        {
            return Err(CloudError::InvalidArgument);
        }
        if visible_sessions.len() > MAX_DISCOVERY_THREADS {
            return Err(CloudError::ResourceExhausted);
        }
        for (id, session) in snapshot.sessions {
            for attempt in 0..8 {
                let mut thread = self.thread(&node.project_id, &id).await?;
                if thread.node_id != node.node_id {
                    return Err(CloudError::Storage);
                }
                if thread.session_id.as_deref() != Some(session.as_str()) {
                    break;
                }
                let can_continue = visible_sessions.get(&session).copied().unwrap_or(false);
                if thread.can_continue == can_continue {
                    break;
                }
                let revision = thread.revision;
                thread.revision += 1;
                thread.can_continue = can_continue;
                thread.updated_at = super::threads::timestamp()?;
                match self
                    .append(
                        &node.node_id,
                        super::threads::thread_stream(&node.project_id, &id),
                        revision,
                        "cloud.thread.visibility-reconciled.v2",
                        &thread,
                    )
                    .await
                {
                    Ok(()) => break,
                    Err(CloudError::Conflict) if attempt < 7 => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
}
