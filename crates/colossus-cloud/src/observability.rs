//! Read-only operational statistics. These projections never grant execution authority.

use crate::storage::{CloudStore, EntityKind, EntityOrder, EntityQuery, EntityRecord};
use crate::{CloudCaller, CloudError, CloudPermission, CloudRepository, CloudResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Controller-retained counts; run outcomes remain distinct from policy violations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OperationalCounts {
    /// Visible project namespaces; populated by the global dashboard.
    pub projects: u64,
    /// Retained host groupings in the selected scope.
    pub hosts: u64,
    /// Enrolled, non-revoked runtime registrations.
    pub agents: u64,
    /// Ready runtimes with a live fenced connection lease.
    pub online_agents: u64,
    /// Retained conversations, including archived history.
    pub threads: u64,
    /// Controller-known runs created inside the selected UTC window.
    pub runs: u64,
    /// Runs with a known completed outcome.
    pub completed: u64,
    /// Runs with a known failed outcome, not a policy-violation count.
    pub failed: u64,
    /// Runs with a known cancellation outcome.
    pub cancelled: u64,
    /// Runs interrupted before completion.
    pub interrupted: u64,
    /// Runs whose external effect outcome remains uncertain.
    pub outcome_unknown: u64,
    /// Current running, waiting or cancelling runs, independent of the window.
    pub active: u64,
    /// Current accepted runs awaiting execution.
    pub queued: u64,
}

/// One UTC day of retained run activity, including days without runs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DailyActivity {
    /// UTC calendar date in YYYY-MM-DD format.
    pub date: String,
    /// Controller-known runs created inside the selected UTC window.
    pub runs: u64,
    /// Runs with a known completed outcome.
    pub completed: u64,
    /// Runs with a known failed outcome, not a policy-violation count.
    pub failed: u64,
    /// Sum of retained provider input-token reports; absent means unknown.
    pub input_tokens: Option<u64>,
    /// Sum of retained provider output-token reports; absent means unknown.
    pub output_tokens: Option<u64>,
}

/// Provider-reported token accounting; missing reports and prices remain unknown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationalUsage {
    /// Sum of retained provider input-token reports; absent means unknown.
    pub input_tokens: Option<u64>,
    /// Sum of retained provider output-token reports; absent means unknown.
    pub output_tokens: Option<u64>,
    /// Absent until explicit provider pricing is configured.
    pub estimated_cost: Option<f64>,
    /// Human-readable accounting provenance and coverage limits.
    pub coverage: String,
}
impl Default for OperationalUsage {
    fn default() -> Self {
        Self {
            input_tokens: None,
            output_tokens: None,
            estimated_cost: None,
            coverage: "Only retained provider usage reports are counted; imported or bounded history may omit usage. Pricing is not configured.".into(),
        }
    }
}

/// Database projection from one scoped, read-only query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OperationalStatistics {
    /// Scoped metadata counts.
    pub counts: OperationalCounts,
    /// Ordered UTC daily buckets.
    pub activity: Vec<DailyActivity>,
    /// Retained provider-reported usage only.
    pub usage: OperationalUsage,
    /// False when a bounded fallback query reached its collection limit.
    pub complete: bool,
}

impl CloudRepository {
    /// Read project or agent statistics beneath authenticated project visibility.
    pub async fn statistics(
        &self,
        caller: &CloudCaller,
        node: Option<&str>,
        now: u64,
        days: u16,
    ) -> CloudResult<OperationalStatistics> {
        caller.require(CloudPermission::Read)?;
        if let Some(node) = node {
            crate::validate_identifier(node)?;
            self.get_node(caller, node).await?;
        }
        if !(1..=90).contains(&days) {
            return Err(CloudError::InvalidArgument);
        }
        self.store
            .statistics(caller.project_id(), node, now, days)
            .await
    }
}

/// Bounded deterministic adapter fallback; PostgreSQL uses aggregate SQL instead.
pub(crate) async fn collect<S: CloudStore + ?Sized>(
    store: &S,
    project: &str,
    node: Option<&str>,
    now: u64,
    days: u16,
) -> CloudResult<OperationalStatistics> {
    if !(1..=90).contains(&days) {
        return Err(CloudError::InvalidArgument);
    }
    let today = i64::try_from(now / 86_400).map_err(|_| CloudError::InvalidArgument)?;
    let first = today - i64::from(days) + 1;
    let mut activity = BTreeMap::new();
    for day in first..=today {
        let timestamp = time::OffsetDateTime::from_unix_timestamp(day * 86_400)
            .map_err(|_| CloudError::InvalidArgument)?;
        let date = timestamp.date().to_string();
        activity.insert(
            date.clone(),
            DailyActivity {
                date,
                ..Default::default()
            },
        );
    }
    let mut result = OperationalStatistics {
        complete: true,
        ..Default::default()
    };
    let (nodes, complete) = records(store, project, EntityKind::Node, node).await?;
    result.complete &= complete;
    for record in &nodes {
        if !record
            .value
            .get("revoked")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            result.counts.agents += 1;
            if record
                .value
                .get("runtime_ready")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && store.read_lease(project, &record.key.id, now).await.is_ok()
            {
                result.counts.online_agents += 1;
            }
        }
    }
    if node.is_some() {
        result.counts.hosts = nodes
            .iter()
            .filter_map(|r| r.value.get("host_id").and_then(serde_json::Value::as_str))
            .collect::<std::collections::BTreeSet<_>>()
            .len() as u64;
    } else {
        let (hosts, complete) = records(store, project, EntityKind::Host, None).await?;
        result.counts.hosts = hosts.len() as u64;
        result.complete &= complete;
    }
    let (threads, complete) = records(store, project, EntityKind::Thread, node).await?;
    result.counts.threads = threads.len() as u64;
    result.complete &= complete;
    let (tasks, complete) = records(store, project, EntityKind::Task, node).await?;
    result.complete &= complete;
    for record in tasks {
        let value = &record.value;
        let status = if value
            .pointer("/dispatch_error/code")
            .and_then(serde_json::Value::as_str)
            == Some("outcome_unknown")
        {
            "outcome_unknown"
        } else if value
            .get("dispatch_error")
            .is_some_and(|value| !value.is_null())
        {
            "failed"
        } else {
            value
                .pointer("/snapshot/run/status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("queued")
        };
        match status {
            "running" | "waiting" | "cancelling" => result.counts.active += 1,
            "queued" => result.counts.queued += 1,
            _ => {}
        }
        let Some(created) = value.get("created_at").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(created) =
            time::OffsetDateTime::parse(created, &time::format_description::well_known::Rfc3339)
        else {
            continue;
        };
        if created.unix_timestamp() > now as i64 {
            continue;
        }
        let Some(day) = activity.get_mut(&created.date().to_string()) else {
            continue;
        };
        result.counts.runs += 1;
        day.runs += 1;
        match status {
            "completed" => {
                result.counts.completed += 1;
                day.completed += 1;
            }
            "failed" => {
                result.counts.failed += 1;
                day.failed += 1;
            }
            "cancelled" => result.counts.cancelled += 1,
            "interrupted" => result.counts.interrupted += 1,
            "outcome_unknown" => result.counts.outcome_unknown += 1,
            _ => {}
        }
        let mut after = 0;
        loop {
            let events = store.events(project, &record.key.id, after, 100).await?;
            for event in &events {
                if let Some(usage) = event.value.pointer("/update/usage") {
                    for (name, daily, total) in [
                        (
                            "input_tokens",
                            &mut day.input_tokens,
                            &mut result.usage.input_tokens,
                        ),
                        (
                            "output_tokens",
                            &mut day.output_tokens,
                            &mut result.usage.output_tokens,
                        ),
                    ] {
                        if let Some(tokens) = usage.get(name).and_then(serde_json::Value::as_u64) {
                            *daily = Some(
                                daily
                                    .unwrap_or(0)
                                    .checked_add(tokens)
                                    .ok_or(CloudError::Storage)?,
                            );
                            *total = Some(
                                total
                                    .unwrap_or(0)
                                    .checked_add(tokens)
                                    .ok_or(CloudError::Storage)?,
                            );
                        }
                    }
                }
            }
            if events.len() < 100 {
                break;
            }
            after = events.last().ok_or(CloudError::Storage)?.sequence;
            if after >= 10_000 {
                result.complete = false;
                break;
            }
        }
    }
    result.activity = activity.into_values().collect();
    Ok(result)
}

async fn records<S: CloudStore + ?Sized>(
    store: &S,
    project: &str,
    kind: EntityKind,
    node: Option<&str>,
) -> CloudResult<(Vec<EntityRecord>, bool)> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = store
            .list(&EntityQuery {
                kind,
                project_id: project.into(),
                node_id: node.map(str::to_owned),
                after,
                limit: 100,
                order: EntityOrder::IdAsc,
                ..Default::default()
            })
            .await?;
        let done = page.len() < 100;
        after = page.last().map(|r| r.key.id.clone());
        all.extend(page);
        if done {
            return Ok((all, true));
        }
        if all.len() >= 10_000 {
            return Ok((all, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        CloudTransaction, EntityKey, EntityMutation, MemoryCloudStore, ReleasedEvent,
    };
    #[tokio::test]
    async fn scoped_counts_distinguish_unknown_usage_and_ignore_thread_feed_duplicates() {
        let store = std::sync::Arc::new(MemoryCloudStore::default());
        let entity = |project: &str, id: &str, value: serde_json::Value| EntityMutation {
            key: EntityKey {
                kind: EntityKind::Task,
                project_id: project.into(),
                parent_id: None,
                id: id.into(),
            },
            expected_revision: 0,
            value,
            actor: "fixture".into(),
            operation: "fixture.task.created".into(),
        };
        let usage = serde_json::json!({"update":{"usage":{"input_tokens":17,"output_tokens":0}}});
        store.commit(CloudTransaction { entities:vec![
            entity("project-a","task-a",serde_json::json!({"created_at":"2026-10-06T12:00:00Z","node_id":"node-a","snapshot":{"run":{"status":"completed"}}})),
            entity("project-a","task-b",serde_json::json!({"created_at":"2026-10-06T12:00:00Z","node_id":"node-b","snapshot":{"run":{"status":"outcome_unknown"}}})),
            entity("project-b","task-other",serde_json::json!({"created_at":"2026-10-06T12:00:00Z","node_id":"node-a","snapshot":{"run":{"status":"failed"}}})),
        ], events:vec![
            ReleasedEvent{project_id:"project-a".into(),scope_id:"task-a".into(),sequence:1,value:usage.clone()},
            ReleasedEvent{project_id:"project-a".into(),scope_id:"thread-copy".into(),sequence:1,value:usage},
        ], ..Default::default() }).await.unwrap();
        let repo = CloudRepository::new(store.clone()).unwrap();
        let caller = CloudCaller::new(
            "reader".into(),
            "project-a".into(),
            [CloudPermission::Read].into(),
        )
        .unwrap();
        let now = time::OffsetDateTime::parse(
            "2026-10-06T13:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap()
        .unix_timestamp() as u64;
        let stats = repo.statistics(&caller, None, now, 7).await.unwrap();
        assert_eq!(stats.counts.runs, 2);
        assert_eq!(stats.counts.completed, 1);
        assert_eq!(stats.counts.failed, 0);
        assert_eq!(stats.counts.outcome_unknown, 1);
        assert_eq!(stats.usage.input_tokens, Some(17));
        assert_eq!(stats.usage.output_tokens, Some(0));
        assert_eq!(stats.activity.len(), 7);
        assert!(stats.activity[0].input_tokens.is_none());
        let node_stats = store
            .statistics("project-a", Some("node-b"), now, 7)
            .await
            .unwrap();
        assert_eq!(node_stats.counts.runs, 1);
        assert!(node_stats.usage.input_tokens.is_none());
        let denied =
            CloudCaller::new("reader".into(), "project-a".into(), Default::default()).unwrap();
        assert_eq!(
            repo.statistics(&denied, None, now, 7).await.unwrap_err(),
            CloudError::PermissionDenied
        );
        assert_eq!(
            repo.statistics(&caller, None, now, 91).await.unwrap_err(),
            CloudError::InvalidArgument
        );
    }
}
