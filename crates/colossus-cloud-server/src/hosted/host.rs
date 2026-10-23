use super::*;
use colossus_cloud_chat::ChatRuntimeOptions;
use std::{sync::atomic::Ordering, time::Duration};

impl HostedExecution {
    pub(crate) fn open(
        deployment: ChatConfig,
        repo: colossus_cloud::CloudRepository,
        database: colossus_cloud_postgres::CloudDatabaseConfig,
    ) -> std::result::Result<Self, &'static str> {
        deployment.validate()?;
        let home = colossus_home::ColossusHome::ensure_at(&deployment.home)
            .map_err(|_| "hosted chat state unavailable")?;
        let shared = shared::SharedChat::open(&deployment, database)?;
        let mut host = Self {
            root: home.confined_root().clone(),
            repo,
            projects: RwLock::new(BTreeMap::new()),
            deployment,
            live: Mutex::new(BTreeMap::new()),
            lifecycle: Mutex::new(()),
            shared,
            closing: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            supervisor: Mutex::new(None),
            oauth: Mutex::new(BTreeMap::new()),
            vaults: std::sync::Mutex::new(BTreeMap::new()),
        };
        let mut retained = BTreeMap::new();
        let global = crate::global_settings::catalog(&host)
            .map_err(|_| "hosted instance catalog unavailable")?;
        for (project, seed) in &host.deployment.projects {
            retained.insert(
                project.clone(),
                crate::global_settings::inheritance::resolve(
                    &crate::chat_settings::retained_configuration(&host, project, seed)
                        .map_err(|_| "hosted chat retained configuration unavailable")?,
                    &global,
                    seed,
                )
                .map_err(|_| "hosted inherited configuration invalid")?,
            );
        }
        host.projects = RwLock::new(retained);
        Ok(host)
    }

    pub(crate) async fn project_config(
        &self,
        project: &str,
    ) -> std::result::Result<Option<ChatProjectConfig>, CloudError> {
        if !self.deployment.projects.contains_key(project) {
            return Ok(None);
        }
        self.refresh_project(project).await?;
        Ok(self.projects.read().await.get(project).cloned())
    }

    pub(crate) async fn refresh_project(
        &self,
        project: &str,
    ) -> std::result::Result<(), CloudError> {
        {
            let seed = self
                .deployment
                .projects
                .get(project)
                .ok_or(CloudError::NotFound)?;
            let raw = crate::chat_settings::retained_configuration(self, project, seed)?;
            let global = crate::global_settings::catalog(self)?;
            let configuration = crate::global_settings::inheritance::resolve(&raw, &global, seed)?;
            self.projects
                .write()
                .await
                .insert(project.into(), configuration);
        }
        Ok(())
    }

    /// Caller holds lifecycle while admitting the corresponding durable cloud task.
    pub(crate) async fn admission(
        &self,
        caller: &CloudCaller,
        node: &str,
    ) -> std::result::Result<Option<tokio::sync::MutexGuard<'_, ()>>, CloudError> {
        if self.node_id(caller)?.as_deref() != Some(node) {
            return Ok(None);
        }
        let guard = self.lifecycle.lock().await;
        self.require_live_submission(caller, node).await?;
        Ok(Some(guard))
    }

    /// Validate a Hosted submission while the caller retains lifecycle admission.
    pub(crate) async fn require_live_submission(
        &self,
        caller: &CloudCaller,
        node: &str,
    ) -> std::result::Result<(), CloudError> {
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        if node != format!("chat-{identity}") {
            return Ok(());
        }
        caller.require(CloudPermission::Execute)?;
        if self.closing.load(Ordering::SeqCst)
            || self.project_config(caller.project_id()).await?.is_none()
        {
            return Err(CloudError::Conflict);
        }
        let configuration = self
            .project_config(caller.project_id())
            .await?
            .ok_or(CloudError::NotFound)?;
        let provisioner = CloudCaller::new(
            "service:hosted-execution".into(),
            caller.project_id().into(),
            [CloudPermission::Administer].into_iter().collect(),
        )?;
        self.repo
            .register_hosted_target(
                &provisioner,
                caller.subject(),
                configuration.runtime.models.roles.keys().cloned().collect(),
            )
            .await?;

        Ok(())
    }

    pub(crate) fn node_id(
        &self,
        caller: &CloudCaller,
    ) -> std::result::Result<Option<String>, CloudError> {
        Ok(Some(format!(
            "chat-{}",
            ChatRuntime::identity_for(caller.project_id(), caller.subject())
                .map_err(|_| CloudError::InvalidArgument)?
        )))
    }

    pub(crate) async fn node(
        &self,
        state: &State,
        caller: &CloudCaller,
    ) -> std::result::Result<Option<serde_json::Value>, CloudError> {
        let node = match state
            .repo
            .get_node(caller, &self.node_id(caller)?.ok_or(CloudError::NotFound)?)
            .await
        {
            Ok(node) => node,
            Err(CloudError::NotFound) => self.configured_target(caller).await?,
            Err(error) => return Err(error),
        };
        if node.owner_user_id.as_deref() != Some(caller.subject()) {
            return Err(CloudError::PermissionDenied);
        }
        let (generation, resources) = self.context(caller).await?;
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        let warm = if let Some(shared) = &self.shared {
            match shared.sql.read_lease(&identity) {
                Ok(lease)
                    if lease.project_id == caller.project_id()
                        && lease.user_id == caller.subject() =>
                {
                    true
                }
                Ok(_) => return Err(CloudError::PermissionDenied),
                Err(CloudError::NotFound) => false,
                Err(error) => return Err(error),
            }
        } else {
            self.live
                .lock()
                .await
                .get(&identity)
                .is_some_and(|live| !*live.shutdown.borrow())
        };
        let presence = crate::server::NodePresence {
            connection_id: generation,
            ready: !node.revoked,
            capabilities: resources.capabilities,
            heartbeat: None,
        };
        Ok(Some(serde_json::json!({"node":node,"presence":presence,
            "execution":{"kind":"hosted","state": if warm {"warm"} else {"on_demand"},"environment":null,"filesystem":false,"process":false,"managed_files":true}})))
    }

    pub(crate) async fn ensure(
        &self,
        state: &Arc<State>,
        caller: &CloudCaller,
    ) -> std::result::Result<String, CloudError> {
        caller.require(CloudPermission::Execute)?;
        self.refresh_project(caller.project_id()).await?;
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        if let Some(shared) = &self.shared {
            match shared.sql.read_lease(&identity) {
                Ok(lease)
                    if lease.project_id == caller.project_id()
                        && lease.user_id == caller.subject() =>
                {
                    return Ok(format!("chat-{identity}"));
                }
                Ok(_) => return Err(CloudError::PermissionDenied),
                Err(CloudError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        match self.ensure_local(state, caller).await {
            Ok(live) => Ok(live.node_id.clone()),
            Err(CloudError::Conflict)
                if self
                    .shared
                    .as_ref()
                    .is_some_and(|shared| shared.sql.read_lease(&identity).is_ok()) =>
            {
                Ok(format!("chat-{identity}"))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn ensure_local(
        &self,
        state: &Arc<State>,
        caller: &CloudCaller,
    ) -> std::result::Result<Arc<LiveRuntime>, CloudError> {
        caller.require(CloudPermission::Read)?;
        caller.require(CloudPermission::Execute)?;
        // Retain this read lease until insertion: an administrative writer cannot
        // inspect idle state and publish a new revision while an old runtime starts.
        let _lifecycle = self.lifecycle.lock().await;
        let projects = self.projects.read().await;
        let configuration = projects
            .get(caller.project_id())
            .cloned()
            .ok_or(CloudError::NotFound)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(CloudError::Storage);
        }
        let identity = ChatRuntime::identity_for(caller.project_id(), caller.subject())
            .map_err(|_| CloudError::InvalidArgument)?;
        let existing = self.live.lock().await.get(&identity).cloned();
        if let Some(live) = existing {
            let expired = if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
                match shared.sql.verify_lease(lease) {
                    Ok(()) => false,
                    Err(CloudError::Conflict | CloudError::NotFound) => true,
                    Err(error) => return Err(error),
                }
            } else {
                false
            };
            if !*live.shutdown.borrow() && !expired {
                live.touch();
                return Ok(live);
            }
            // A failed startup or delayed monitor must never make an expired
            // cached owner eligible for reuse. Retire its exact generation before
            // attempting a fresh claim; storage outages above preserve a live owner.
            self.stop_entry_with_timeout(&identity, &live, Duration::from_secs(2))
                .await?;
        }
        if self.live.lock().await.len() >= self.deployment.max_active_runtimes {
            self.evict_idle(state, true).await?;
            if self.live.lock().await.len() >= self.deployment.max_active_runtimes {
                return Err(CloudError::ResourceExhausted);
            }
        }
        let lease = if let Some(shared) = &self.shared {
            Some(shared.sql.claim_lease(
                &identity,
                caller.project_id(),
                caller.subject(),
                &format!("{}-{}", state.replica_id, uuid::Uuid::now_v7().simple()),
                30,
            )?)
        } else {
            None
        };
        let mut claim = shared::LeaseClaim::new(self.shared.clone(), lease.clone());
        let (mut runtime_config, credentials) =
            self.credentials(caller, &configuration, lease.clone())?;
        // All hosted tenants use the collector accepted at this process startup.
        // Persisting a desired replacement does not change journal disclosure or
        // exporter behavior underneath already resident work.
        runtime_config.observability = state.telemetry.applied_configuration().clone();
        let shared_runtime = if let (Some(shared), Some(lease)) = (&self.shared, &lease) {
            if configuration.codex_auth_path.is_some() {
                return Err(CloudError::InvalidArgument);
            }
            let shared = shared.clone();
            let durable_lease = lease.clone();
            let namespace = identity.clone();
            let (postgres, durability) = tokio::task::spawn_blocking(move || {
                shared.runtime_storage(&namespace, &durable_lease)
            })
            .await
            .map_err(|_| CloudError::Storage)??;
            runtime_config.storage.adapter = colossus_runtime::StorageAdapter::Postgres;
            runtime_config.storage.postgres = Some(postgres);
            Some(colossus_cloud_chat::ChatSharedRuntime {
                durability,
                oauth_vault: self.execution_vault(
                    caller.project_id(),
                    caller.subject(),
                    Some(lease.clone()),
                )?,
            })
        } else {
            None
        };
        let codex_auth = if configuration.codex_auth_shared {
            Some(
                self.shared
                    .as_ref()
                    .ok_or(CloudError::InvalidArgument)?
                    .codex_auth(caller.project_id(), lease.clone())?,
            )
        } else {
            configuration
                .codex_auth_path
                .as_ref()
                .map(colossus_runtime::CodexAuthStore::at_path)
        };
        let options = ChatRuntimeOptions {
            project_id: caller.project_id().into(),
            user_id: caller.subject().into(),
            root: self.root.clone(),
            config: runtime_config,
            wrapping_key_variable: self.deployment.wrapping_key_variable.clone(),
            credentials,
            codex_auth,
            shared: shared_runtime,
        };
        let runtime = Arc::new(
            tokio::task::spawn_blocking(move || ChatRuntime::open(options))
                .await
                .map_err(|_| CloudError::Storage)?
                .map_err(|_| CloudError::Storage)?,
        );
        let node = self
            .register_target_roles(
                state,
                caller,
                configuration.runtime.models.roles.keys().cloned().collect(),
            )
            .await?;
        let node_id = node.node_id.clone();
        let model_profiles = runtime
            .runtime()
            .model_profiles()
            .into_iter()
            .map(|route| colossus_sdk::ConfiguredModelProfile {
                default_roles: configuration
                    .runtime
                    .models
                    .roles
                    .iter()
                    .filter(|(_, profile)| *profile == &route.model_profile)
                    .map(|(role, _)| role.clone())
                    .collect(),
                profile: route.model_profile,
                model: route.model,
                provider: route.provider,
                provider_profile: route.provider_profile,
                reasoning_effort: route.reasoning_effort.map(|effort| effort.as_str().into()),
            })
            .collect();
        // Complete every fallible authority check before any worker task can
        // execute or a cached entry can be published.
        if let (Some(shared), Some(lease)) = (&self.shared, &lease) {
            shared.sql.verify_lease(lease)?;
        }
        let (shutdown, _close) = watch::channel(false);
        let live = Arc::new(LiveRuntime {
            project_id: caller.project_id().into(),
            user_id: caller.subject().into(),
            runtime,
            node_id,
            shutdown,
            worker: Mutex::new(None),
            lease,
            management: Mutex::new(None),
            last_used: std::sync::Mutex::new(std::time::Instant::now()),
            operations: std::sync::atomic::AtomicUsize::new(0),
        });
        let capabilities = super::pool::capabilities();
        self.start_worker(
            state,
            live.clone(),
            node,
            colossus_cloud_executor::ExecutionResources {
                workflows: None,
                plugins: None,
                capabilities,
                model_profiles,
            },
        )
        .await?;
        if self.shared.is_some()
            && let Err(error) = self.start_management(state, live.clone()).await
        {
            let _ = self
                .stop_entry_with_timeout(&identity, &live, Duration::from_secs(2))
                .await;
            return Err(error);
        }
        self.live.lock().await.insert(identity, live.clone());
        claim.retain();
        drop(projects);
        Ok(live)
    }

    pub(crate) async fn active_project_run_status(
        &self,
        project: &str,
    ) -> std::result::Result<bool, CloudError> {
        use colossus_cloud::storage::{EntityKind, EntityQuery};
        // Canonical cloud admission remains authoritative even while its runtime is
        // cold or before a worker has accepted the durable creation command.
        for status in [
            "queued",
            "running",
            "waiting",
            "cancelling",
            "outcome_unknown",
        ] {
            let records = self
                .repo
                .storage()
                .list(&EntityQuery {
                    kind: EntityKind::Task,
                    project_id: project.into(),
                    status: Some(status.into()),
                    limit: 100,
                    ..Default::default()
                })
                .await?;
            if records.len() == 100 {
                return Ok(true);
            }
            for record in records {
                let task: colossus_cloud::CloudTask = record.value.try_into()?;
                if task.node_id.starts_with("chat-") {
                    return Ok(true);
                }
            }
        }
        let live = self
            .live
            .lock()
            .await
            .values()
            .filter(|live| live.project_id == project)
            .cloned()
            .collect::<Vec<_>>();
        for entry in live {
            if entry.operations.load(Ordering::SeqCst) != 0
                || entry.runtime.has_pending_executions()
                || Self::pending_children(&entry)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) async fn user_is_busy(
        &self,
        state: &State,
        project: &str,
        user: &str,
    ) -> std::result::Result<bool, CloudError> {
        use colossus_cloud::storage::{EntityKind, EntityQuery};
        let identity =
            ChatRuntime::identity_for(project, user).map_err(|_| CloudError::InvalidArgument)?;
        for status in [
            "queued",
            "running",
            "waiting",
            "cancelling",
            "outcome_unknown",
        ] {
            if !state
                .repo
                .storage()
                .list(&EntityQuery {
                    kind: EntityKind::Task,
                    project_id: project.into(),
                    node_id: Some(format!("chat-{identity}")),
                    status: Some(status.into()),
                    limit: 1,
                    ..Default::default()
                })
                .await?
                .is_empty()
            {
                return Ok(true);
            }
        }
        let live = self.live.lock().await.get(&identity).cloned();
        if let Some(live) = live
            && (live.operations.load(Ordering::SeqCst) != 0
                || live.runtime.has_pending_executions()
                || Self::pending_children(&live)?)
        {
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn pending_children(live: &LiveRuntime) -> std::result::Result<bool, CloudError> {
        for status in [
            colossus_contracts::SubagentStatus::Queued,
            colossus_contracts::SubagentStatus::Running,
        ] {
            if !live
                .runtime
                .runtime()
                .list_subagents(None, Some(status), 1)
                .map_err(|_| CloudError::Storage)?
                .is_empty()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Caller holds lifecycle admission and has proved this user's work idle.
    pub(crate) async fn stop_user(
        &self,
        project: &str,
        user: &str,
    ) -> std::result::Result<(), CloudError> {
        self.cancel_oauth(project, Some(user)).await;
        let identity =
            ChatRuntime::identity_for(project, user).map_err(|_| CloudError::InvalidArgument)?;
        let live = self.live.lock().await.get(&identity).cloned();
        if let Some(live) = live {
            self.stop_entry(&identity, &live).await?;
        }
        Ok(())
    }

    pub(super) async fn stop_entry(
        &self,
        id: &str,
        live: &Arc<LiveRuntime>,
    ) -> std::result::Result<(), CloudError> {
        self.stop_entry_with_timeout(id, live, Duration::from_secs(10))
            .await
    }

    async fn stop_entry_with_timeout(
        &self,
        id: &str,
        live: &Arc<LiveRuntime>,
        timeout: Duration,
    ) -> std::result::Result<(), CloudError> {
        if self.shared.is_some() {
            live.shutdown.send_replace(true);
        }
        let mut failed = !live.runtime.shutdown(timeout).await;
        if failed && self.shared.is_none() {
            // A local writer has no distributed fence to revoke its authority.
            // Retain its handle until cooperative shutdown is confirmed.
            return Err(CloudError::Storage);
        }
        live.shutdown.send_replace(true);
        if let Some(mut task) = live.worker.lock().await.take() {
            match tokio::time::timeout(timeout, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(_) => failed = true,
                Err(_) => {
                    task.abort();
                    let _ = tokio::time::timeout(timeout, &mut task).await;
                    failed = true;
                }
            }
        }
        if let Some(mut task) = live.management.lock().await.take() {
            task.abort();
            failed |= tokio::time::timeout(timeout, &mut task).await.is_err();
        }
        if let (Some(shared), Some(lease)) = (&self.shared, &live.lease) {
            failed |= shared.sql.release_lease(lease).is_err();
        }
        let mut entries = self.live.lock().await;
        if entries
            .get(id)
            .is_some_and(|entry| Arc::ptr_eq(entry, live))
        {
            entries.remove(id);
        }
        if failed {
            Err(CloudError::Storage)
        } else {
            Ok(())
        }
    }

    /// Caller holds the project configuration write lease, or service shutdown admission.
    pub(crate) async fn stop_project(&self, project: &str) -> std::result::Result<(), CloudError> {
        self.cancel_oauth(project, None).await;
        let entries = self
            .live
            .lock()
            .await
            .iter()
            .filter(|(_, live)| live.project_id == project)
            .map(|(id, live)| (id.clone(), live.clone()))
            .collect::<Vec<_>>();
        for (id, live) in entries {
            self.stop_entry(&id, &live).await?;
        }
        Ok(())
    }

    pub(crate) async fn shutdown(&self) -> std::result::Result<(), CloudError> {
        self.closing.store(true, Ordering::SeqCst);
        self.notify();
        if let Some(mut supervisor) = self.supervisor.lock().await.take() {
            tokio::time::timeout(Duration::from_secs(30), &mut supervisor)
                .await
                .map_err(|_| CloudError::Storage)?
                .map_err(|_| CloudError::Storage)?;
        }
        let _lifecycle = self.lifecycle.lock().await;
        let projects = self
            .live
            .lock()
            .await
            .values()
            .map(|entry| entry.project_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for project in projects {
            self.stop_project(&project).await?;
        }
        self.oauth.lock().await.clear();
        Ok(())
    }
}
