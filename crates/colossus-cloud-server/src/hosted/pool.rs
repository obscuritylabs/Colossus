//! Bounded on-demand tenant placement; durable cloud work survives the supervisor.
use super::*;
use colossus_cloud::{
    CloudNode,
    storage::{EntityKind, EntityQuery},
};
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(super) fn capabilities() -> Vec<String> {
    [
        "agent_runs.create",
        "agent_runs.read",
        "agent_runs.cancel",
        "prompts.respond",
        "approvals.respond",
        "agent_runs.model_selection",
        "plans.continue",
        "artifacts",
        "attachments.text_input",
        colossus_cloud_protocol::RESOURCE_CAPABILITY,
        colossus_cloud_protocol::MODEL_CATALOG_CAPABILITY,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl HostedExecution {
    pub(crate) fn notify(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn register_target(
        &self,
        state: &State,
        caller: &CloudCaller,
    ) -> std::result::Result<CloudNode, CloudError> {
        caller.require(CloudPermission::Execute)?;
        let configuration = self
            .project_config(caller.project_id())
            .await?
            .ok_or(CloudError::NotFound)?;
        self.register_target_roles(
            state,
            caller,
            configuration.runtime.models.roles.keys().cloned().collect(),
        )
        .await
    }

    pub(super) async fn register_target_roles(
        &self,
        state: &State,
        caller: &CloudCaller,
        roles: std::collections::BTreeSet<String>,
    ) -> std::result::Result<CloudNode, CloudError> {
        let provisioner = CloudCaller::new(
            "service:hosted-execution".into(),
            caller.project_id().into(),
            [CloudPermission::Administer].into_iter().collect(),
        )?;
        state
            .repo
            .register_hosted_target(&provisioner, caller.subject(), roles)
            .await
    }

    pub(super) async fn unfinished(
        &self,
        project: &str,
        node: &str,
    ) -> std::result::Result<bool, CloudError> {
        let tasks = !self
            .repo
            .storage()
            .list(&EntityQuery {
                kind: EntityKind::Task,
                project_id: project.into(),
                node_id: Some(node.into()),
                status: Some("unfinished".into()),
                limit: 1,
                ..Default::default()
            })
            .await?
            .is_empty();
        if tasks {
            return Ok(true);
        }
        Ok(!self
            .repo
            .storage()
            .list(&EntityQuery {
                kind: EntityKind::Command,
                project_id: project.into(),
                parent_id: Some(node.into()),
                status: Some("pending".into()),
                limit: 1,
                ..Default::default()
            })
            .await?
            .is_empty())
    }

    /// Caller holds lifecycle admission. Never retire uncertain active work or resources.
    pub(crate) async fn evict_idle(
        &self,
        _state: &State,
        pressure: bool,
    ) -> std::result::Result<(), CloudError> {
        let mut candidates = self
            .live
            .lock()
            .await
            .iter()
            .map(|(id, live)| {
                (
                    id.clone(),
                    live.clone(),
                    live.last_used.lock().ok().map(|last| *last),
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(_, _, last)| *last);
        for (id, live, last) in candidates {
            if last.is_none_or(|last| {
                !pressure && last.elapsed() < Duration::from_secs(self.deployment.idle_seconds)
            }) || live.operations.load(Ordering::SeqCst) != 0
                || live.runtime.has_pending_executions()
                || Self::pending_children(&live)?
                || self
                    .oauth
                    .lock()
                    .await
                    .values()
                    .any(|flow| Arc::ptr_eq(&flow.live, &live) && flow.expires_at > now())
                || live.runtime.runtime().has_pending_mcp_browser_oauth().await
                || self.unfinished(&live.project_id, &live.node_id).await?
            {
                continue;
            }
            self.stop_entry(&id, &live).await?;
            if pressure {
                break;
            }
        }
        Ok(())
    }

    async fn migrate_legacy_page(
        &self,
        state: &State,
        project: &str,
        after: Option<String>,
    ) -> std::result::Result<Option<String>, CloudError> {
        let records = self
            .repo
            .storage()
            .list(&EntityQuery {
                kind: EntityKind::Node,
                project_id: project.into(),
                status: Some("connected".into()),
                after,
                limit: 100,
                ..Default::default()
            })
            .await?;
        let next = if records.len() == 100 {
            records.last().map(|record| record.key.id.clone())
        } else {
            None
        };
        for record in records {
            let node: CloudNode = record.value.try_into()?;
            let Some(user) = node.owner_user_id.as_ref() else {
                continue;
            };
            let Ok(identity) = ChatRuntime::identity_for(project, user) else {
                continue;
            };
            if node.revoked
                || node.node_id != format!("chat-{identity}")
                || node.instance_id != format!("hosted-chat-{identity}")
            {
                continue;
            }
            let caller = CloudCaller::new(
                user.clone(),
                project.into(),
                [CloudPermission::Read, CloudPermission::Execute]
                    .into_iter()
                    .collect(),
            )?;
            self.register_target(state, &caller).await?;
        }
        Ok(next)
    }

    /// One bounded supervisor per worker process. Database records, rather than wakeups,
    /// own dispatch; polling also discovers accepted work after a complete restart.
    pub(crate) fn start_supervisor(state: &Arc<State>) -> tokio::task::JoinHandle<()> {
        let state = Arc::downgrade(state);
        tokio::spawn(async move {
            let mut cursors = BTreeMap::<String, String>::new();
            let mut migration_cursors = BTreeMap::<String, String>::new();
            let mut migrated = std::collections::BTreeSet::new();
            loop {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let Some(host) = &state.hosted else {
                    return;
                };
                if host.closing.load(Ordering::SeqCst) || *state.shutdown.borrow() {
                    return;
                }
                {
                    let _lifecycle = host.lifecycle.lock().await;
                    let _ = host.evict_idle(&state, false).await;
                }
                for project in host.deployment.projects.keys() {
                    if !migrated.contains(project) {
                        match host
                            .migrate_legacy_page(
                                &state,
                                project,
                                migration_cursors.get(project).cloned(),
                            )
                            .await
                        {
                            Ok(Some(after)) => {
                                migration_cursors.insert(project.clone(), after);
                            }
                            Ok(None) => {
                                migrated.insert(project.clone());
                                migration_cursors.remove(project);
                            }
                            Err(_) => {}
                        }
                    }
                    let records = match host
                        .repo
                        .storage()
                        .list(&EntityQuery {
                            kind: EntityKind::Node,
                            project_id: project.clone(),
                            status: Some("hosted_pending".into()),
                            after: cursors.get(project).cloned(),
                            limit: 100,
                            ..Default::default()
                        })
                        .await
                    {
                        Ok(records) => records,
                        Err(_) => continue,
                    };
                    if records.len() == 100 {
                        if let Some(last) = records.last() {
                            cursors.insert(project.clone(), last.key.id.clone());
                        }
                    } else {
                        cursors.remove(project);
                    }
                    for record in records {
                        let Ok(node) = CloudNode::try_from(record.value) else {
                            continue;
                        };
                        let Some(user) = node.owner_user_id.as_ref() else {
                            continue;
                        };
                        if node.revoked || host.closing.load(Ordering::SeqCst) {
                            continue;
                        }
                        let Ok(caller) = CloudCaller::new(
                            user.clone(),
                            project.clone(),
                            [CloudPermission::Read, CloudPermission::Execute]
                                .into_iter()
                                .collect(),
                        ) else {
                            continue;
                        };
                        let _ = host.ensure(&state, &caller).await;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                    _ = host.wake.notified() => {},
                }
            }
        })
    }
}
