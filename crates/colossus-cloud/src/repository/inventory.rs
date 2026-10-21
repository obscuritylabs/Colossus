use super::*;
use crate::{CloudHost, CloudThread, CloudWorkspace, validation::fingerprint};
use colossus_cloud_protocol::{ReleasedRunInventory, RuntimeInventory};
use colossus_sdk::{CreateRunRequest, GetRunResponse, IdempotencyKey, InputContentPart};

impl CloudRepository {
    /// Publish sanitized grouping without widening any runtime grant.
    pub async fn register_inventory(
        &self,
        node: &CloudNode,
        inventory: RuntimeInventory,
        now: u64,
    ) -> CloudResult<CloudNode> {
        inventory
            .validate()
            .map_err(|_| CloudError::InvalidArgument)?;
        for _ in 0..8 {
            self.live_node(node).await?;
            let mut fresh = self.node(&node.project_id, &node.node_id).await?;
            if fresh
                .host_id
                .as_ref()
                .is_some_and(|host| host != &inventory.host_id)
                || fresh
                    .workspace_id
                    .as_ref()
                    .is_some_and(|workspace| workspace != &inventory.workspace_id)
            {
                return Err(CloudError::Conflict);
            }
            let host_key = format!("cloud.host:{}:{}", node.project_id, inventory.host_id);
            let (host_version, previous_host) = match self.read::<CloudHost>(&host_key).await {
                Ok((host, revision)) => (revision, Some(host)),
                Err(CloudError::NotFound) => (0, None),
                Err(error) => return Err(error),
            };
            let workspace_key = format!(
                "cloud.workspace:{}:{}",
                node.project_id, inventory.workspace_id
            );
            let (workspace_version, previous_workspace) = match self
                .read::<CloudWorkspace>(&workspace_key)
                .await
            {
                Ok((workspace, revision)) => {
                    if workspace.node_id != node.node_id || workspace.host_id != inventory.host_id {
                        return Err(CloudError::Conflict);
                    }
                    (revision, Some(workspace))
                }
                Err(CloudError::NotFound) => (0, None),
                Err(error) => return Err(error),
            };
            let mut host = CloudHost {
                host_id: inventory.host_id.clone(),
                project_id: node.project_id.clone(),
                label: inventory.host_label.clone(),
                platform: inventory.platform.clone(),
                deployment_kind: serde_json::to_value(inventory.deployment_kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "cli".into()),
                // Existing presence belongs to heartbeat_node, not repeated
                // static inventory samples accompanying policy observations.
                last_seen_at: previous_host.as_ref().map_or(now, |host| host.last_seen_at),
                revision: host_version,
            };
            let mut workspace = CloudWorkspace {
                workspace_id: inventory.workspace_id.clone(),
                project_id: node.project_id.clone(),
                host_id: inventory.host_id.clone(),
                node_id: node.node_id.clone(),
                label: inventory.workspace_label.clone(),
                sharing: serde_json::to_value(inventory.sharing)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or(CloudError::InvalidArgument)?,
                revision: workspace_version,
            };
            let previous_node = fresh.clone();
            let node_version = fresh.revision;
            fresh.host_id = Some(inventory.host_id.clone());
            fresh.workspace_id = Some(inventory.workspace_id.clone());
            fresh.workspace_label = Some(inventory.workspace_label.clone());
            fresh.runtime_ready = true;
            if let Some(policy) = inventory.policy.clone() {
                fresh.policy = Some(policy);
                fresh.policy_observed_at = Some(now);
            }
            let mut writes = Vec::new();
            if previous_host.as_ref() != Some(&host) {
                host.revision += 1;
                writes.push(self.event(
                    &node.node_id,
                    host_key,
                    host_version,
                    "cloud.host.observed.v2",
                    &host,
                )?);
            }
            if previous_workspace.as_ref() != Some(&workspace) {
                workspace.revision += 1;
                writes.push(self.event(
                    &node.node_id,
                    workspace_key,
                    workspace_version,
                    "cloud.workspace.observed.v2",
                    &workspace,
                )?);
            }
            if fresh != previous_node {
                fresh.revision += 1;
                writes.push(self.event(
                    &node.node_id,
                    format!("cloud.node:{}:{}", node.project_id, node.node_id),
                    node_version,
                    "cloud.node.inventory.v2",
                    &fresh,
                )?);
            }
            // A no-op still checks the bound connection generation atomically;
            // it emits no entity audit/outbox entries or changed notifications.
            match self.commit(writes).await {
                Ok(()) => return Ok(fresh),
                Err(colossus_ports::StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(CloudError::Conflict)
    }
    /// List host groups within the authenticated project.
    pub async fn list_hosts(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudHost>> {
        caller.require(CloudPermission::Read)?;
        self.query_entities(
            caller,
            EntityQuery {
                kind: EntityKind::Host,
                status: Some("connected".into()),
                after: after.map(str::to_owned),
                limit,
                ..Default::default()
            },
        )
        .await?
        .into_iter()
        .map(|record| record.value.try_into())
        .collect()
    }
    /// Read one native host beneath the caller's project visibility.
    pub async fn get_host(&self, caller: &CloudCaller, host_id: &str) -> CloudResult<CloudHost> {
        caller.require(CloudPermission::Read)?;
        crate::validate_identifier(host_id)?;
        let host: CloudHost = self
            .read(&format!("cloud.host:{}:{host_id}", caller.project_id()))
            .await?
            .0;
        // Host labels belong to the native environment; hide groups containing only another owner's nodes.
        let nodes = self
            .store
            .list(&EntityQuery {
                kind: EntityKind::Node,
                project_id: caller.project_id().into(),
                host_id: Some(host_id.into()),
                limit: 1,
                ..Default::default()
            })
            .await?;
        if !nodes.is_empty()
            && self
                .store
                .list(&EntityQuery {
                    kind: EntityKind::Node,
                    project_id: caller.project_id().into(),
                    host_id: Some(host_id.into()),
                    visible_to: Some(caller.subject().into()),
                    status: Some("connected".into()),
                    limit: 1,
                    ..Default::default()
                })
                .await?
                .is_empty()
        {
            return Err(CloudError::NotFound);
        }
        Ok(host)
    }

    /// Read only this project's runtime placements on one exact native host.
    pub async fn list_host_nodes(
        &self,
        caller: &CloudCaller,
        host_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudNode>> {
        self.get_host(caller, host_id).await?;
        if !(1..=100).contains(&limit) {
            return Err(CloudError::InvalidArgument);
        }
        if let Some(after) = after {
            crate::validate_identifier(after)?;
        }
        let query = crate::storage::EntityQuery {
            kind: crate::storage::EntityKind::Node,
            project_id: caller.project_id().into(),
            host_id: Some(host_id.into()),
            visible_to: Some(caller.subject().into()),
            status: Some("connected".into()),
            after: after.map(str::to_owned),
            limit,
            ..Default::default()
        };
        self.storage()
            .list(&query)
            .await?
            .into_iter()
            .map(|record| {
                if let crate::storage::EntityValue::Node(node) = record.value {
                    Ok(node)
                } else {
                    Err(CloudError::Storage)
                }
            })
            .collect()
    }

    /// List advertised workspace identities without releasing local paths.
    pub async fn list_workspaces(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudWorkspace>> {
        caller.require(CloudPermission::Read)?;
        self.query_entities(
            caller,
            EntityQuery {
                kind: EntityKind::Workspace,
                status: Some("connected".into()),
                after: after.map(str::to_owned),
                limit,
                ..Default::default()
            },
        )
        .await?
        .into_iter()
        .map(|record| record.value.try_into())
        .collect()
    }
    /// Upsert an authenticated bounded discovery page, preserving source placement.
    pub async fn discover_runs(
        &self,
        node: &CloudNode,
        runs: Vec<ReleasedRunInventory>,
        sync_id: &str,
    ) -> CloudResult<()> {
        self.live_node(node).await?;
        if runs.len() > colossus_cloud_protocol::MAX_DISCOVERY_PAGE_SIZE as usize {
            return Err(CloudError::ResourceExhausted);
        }
        let current_node = self.node(&node.project_id, &node.node_id).await?;
        for discovered in runs {
            let run = discovered.run;
            crate::validate_identifier(&run.run_id)?;
            crate::validate_identifier(&run.session_id)?;
            let session_key = format!(
                "cloud.session:{}:{}:{}",
                node.project_id, node.node_id, run.session_id
            );
            let (thread_id, session_version) = match self.read::<String>(&session_key).await {
                Ok((id, version)) => (id, version),
                Err(CloudError::NotFound) => (
                    format!(
                        "t{}",
                        &fingerprint(
                            format!("{}:{}:{}", node.project_id, node.node_id, run.session_id)
                                .as_bytes()
                        )[..32]
                    ),
                    0,
                ),
                Err(error) => return Err(error),
            };
            let thread_key = super::threads::thread_stream(&node.project_id, &thread_id);
            let (mut thread, thread_version) = match self.thread(&node.project_id, &thread_id).await
            {
                Ok(thread) => {
                    let version = thread.revision;
                    (thread, version)
                }
                Err(CloudError::NotFound) => (
                    CloudThread {
                        owner_user_id: current_node.owner_user_id.clone(),
                        thread_id: thread_id.clone(),
                        project_id: node.project_id.clone(),
                        node_id: node.node_id.clone(),
                        host_id: current_node.host_id.clone(),
                        workspace_id: current_node.workspace_id.clone(),
                        title: run.title.chars().take(256).collect(),
                        created_at: run.created_at.clone(),
                        updated_at: run.updated_at.clone(),
                        revision: 0,
                        archived: run.archived,
                        session_id: Some(run.session_id.clone()),
                        sync_status: if run.last_sequence > 0 {
                            "incomplete"
                        } else {
                            "current"
                        }
                        .into(),
                        source: "runtime".into(),
                        can_continue: discovered.continuable,
                        active_task_id: None,
                        queued_task_ids: vec![],
                    },
                    0,
                ),
                Err(error) => return Err(error),
            };
            let before_thread = thread.clone();
            if thread.node_id != node.node_id
                || thread
                    .session_id
                    .as_deref()
                    .is_some_and(|id| id != run.session_id)
            {
                return Err(CloudError::PermissionDenied);
            }
            let run_key = format!(
                "cloud.run:{}:{}:{}",
                node.project_id, node.node_id, run.run_id
            );
            let (task_id, run_version) = match self.read::<String>(&run_key).await {
                Ok(value) => value,
                Err(CloudError::NotFound) => (
                    fingerprint(
                        format!("{}:{}:{}", node.project_id, node.node_id, run.run_id).as_bytes(),
                    )[..32]
                        .into(),
                    0,
                ),
                Err(error) => return Err(error),
            };
            let task_key = tasks::stream(&node.project_id, &task_id);
            let mut writes = Vec::new();
            let task = match self.task(&node.project_id, &task_id).await {
                Ok(mut task) => {
                    if task.node_id != node.node_id || task.run_id.as_deref() != Some(&run.run_id) {
                        return Err(CloudError::PermissionDenied);
                    }
                    let revision = task.revision;
                    let before = task.clone();
                    task.thread_id = Some(thread_id.clone());
                    task.source_read_only = !discovered.controllable;
                    if task
                        .snapshot
                        .as_ref()
                        .is_none_or(|snapshot| snapshot.run.last_sequence < run.last_sequence)
                    {
                        if task.subject == "runtime" {
                            task.history_complete = false;
                        }
                        task.snapshot = Some(GetRunResponse {
                            run: run.clone(),
                            pending_interactions: task
                                .snapshot
                                .as_ref()
                                .map_or_else(Vec::new, |snapshot| {
                                    snapshot.pending_interactions.clone()
                                }),
                        });
                    }
                    if task != before {
                        task.revision += 1;
                        task.updated_at = super::threads::timestamp()?;
                        writes.push(self.event(
                            &node.node_id,
                            task_key,
                            revision,
                            "cloud.task.discovered.v2",
                            &task,
                        )?);
                    }
                    task
                }
                Err(CloudError::NotFound) => {
                    let task = CloudTask {
                        task_id: task_id.clone(),
                        project_id: node.project_id.clone(),
                        node_id: node.node_id.clone(),
                        subject: "runtime".into(),
                        created_at: run.created_at.clone(),
                        updated_at: run.updated_at.clone(),
                        request: CreateRunRequest {
                            model_profile: None,
                            reasoning_effort: None,
                            plugin_skill_ids: vec![],
                            input: vec![InputContentPart::Text(run.title.clone())],
                            session_id: Some(run.session_id.clone()),
                            end_user_id: None,
                            role: run.role.clone(),
                            mode: run.mode,
                            goal_max_iterations: 0,
                            research_depth: None,
                            research_sources: vec![],
                            plan_action: None,
                            branch: None,
                            max_turns: 0,
                            idempotency_key: IdempotencyKey::new(format!("discovered-{task_id}"))
                                .map_err(|_| CloudError::InvalidArgument)?,
                        },
                        thread_id: Some(thread_id.clone()),
                        source_read_only: !discovered.controllable,
                        history_complete: false,
                        history_bounded: false,
                        run_id: Some(run.run_id.clone()),
                        snapshot: Some(GetRunResponse {
                            run: run.clone(),
                            pending_interactions: vec![],
                        }),
                        dispatch_error: None,
                        last_sequence: 0,
                        released_bytes: 0,
                        output_limited: false,
                        revision: 1,
                    };
                    writes.push(self.event(
                        &node.node_id,
                        task_key,
                        0,
                        "cloud.task.discovered.v2",
                        &task,
                    )?);
                    writes.push(self.event(
                        &node.node_id,
                        format!(
                            "cloud.node-task:{}:{}:{task_id}",
                            node.project_id, node.node_id
                        ),
                        0,
                        "cloud.node-task.discovered.v2",
                        &task_id,
                    )?);
                    task
                }
                Err(error) => return Err(error),
            };
            if run_version == 0 {
                writes.push(self.event(
                    &node.node_id,
                    run_key,
                    0,
                    "cloud.run.discovered.v2",
                    &task_id,
                )?);
            }
            if session_version == 0 {
                writes.push(self.event(
                    &node.node_id,
                    session_key,
                    0,
                    "cloud.session.mapped.v2",
                    &thread_id,
                )?);
            }
            thread.session_id = Some(run.session_id.clone());
            thread.can_continue = discovered.continuable;
            thread.updated_at = thread.updated_at.max(run.updated_at);
            if task.last_sequence < run.last_sequence {
                thread.sync_status = "incomplete".into();
            }
            if thread_version == 0 || thread != before_thread {
                thread.revision = thread_version + 1;
                writes.push(self.event(
                    &node.node_id,
                    thread_key,
                    thread_version,
                    "cloud.thread.discovered.v2",
                    &thread,
                )?);
            }
            if !writes.is_empty() {
                self.commit(writes).await?;
            }
            if task.subject == "runtime" && !task.history_complete {
                self.queue_history(node, &task, sync_id, None).await?;
            }
        }
        Ok(())
    }
}

impl CloudRepository {
    /// Persist readiness independently of connection presence and refresh host contact.
    pub async fn heartbeat_node(&self, node: &CloudNode, ready: bool, now: u64) -> CloudResult<()> {
        self.live_node(node).await?;
        let mut current = self.node(&node.project_id, &node.node_id).await?;
        let mut writes = Vec::new();
        if current.runtime_ready != ready {
            let revision = current.revision;
            current.revision += 1;
            current.runtime_ready = ready;
            writes.push(self.event(
                &node.node_id,
                format!("cloud.node:{}:{}", node.project_id, node.node_id),
                revision,
                "cloud.node.readiness.v2",
                &current,
            )?);
        }
        if let Some(host_id) = &current.host_id {
            let key = format!("cloud.host:{}:{host_id}", node.project_id);
            let (mut host, revision) = self.read::<CloudHost>(&key).await?;
            if now > host.last_seen_at {
                host.last_seen_at = now;
                host.revision = revision + 1;
                writes.push(self.event(
                    &node.node_id,
                    key,
                    revision,
                    "cloud.host.heartbeat.v2",
                    &host,
                )?);
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.commit(writes).await?;
        Ok(())
    }
}

impl CloudRepository {
    /// Reconcile current sources in one phase; released history remains retained.
    /// Streaming controllers use `begin_discovery` and `finish_discovery_snapshot`
    /// to fence sessions created while an earlier runtime page is in flight.
    pub async fn finish_discovery(
        &self,
        node: &CloudNode,
        visible_sessions: &std::collections::BTreeMap<String, bool>,
    ) -> CloudResult<()> {
        let snapshot = self.begin_discovery(node).await?;
        self.finish_discovery_snapshot(node, visible_sessions, snapshot)
            .await
    }
}
