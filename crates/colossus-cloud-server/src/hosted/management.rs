//! Closed owner-routed management over shared, encrypted, at-most-once SQL requests.
use super::*;
use colossus_cloud_postgres::ChatLease;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Management {
    File {
        file: super::files::FileOperation,
    },
    Busy,
    ProviderModels {
        profile: String,
        revision: u64,
    },
    Stop {
        handoff_owner: String,
    },
    OAuthBegin {
        server: String,
        flow_id: String,
        binding: String,
        callback: String,
        local_development: bool,
    },
    OAuthComplete {
        server: String,
        flow_id: String,
        binding: String,
        callback: String,
    },
    OAuthStatus {
        server: String,
    },
    OAuthLogout {
        server: String,
    },
    OAuthCancel {
        server: String,
        flow_id: String,
        binding: String,
    },
}

#[derive(Serialize, Deserialize)]
struct Reply {
    value: Option<serde_json::Value>,
    error: Option<String>,
}

impl HostedExecution {
    pub(super) async fn management_request(
        &self,
        lease: &ChatLease,
        operation: Management,
    ) -> std::result::Result<serde_json::Value, CloudError> {
        let shared = self.shared.as_ref().ok_or(CloudError::Storage)?;
        let id = uuid::Uuid::now_v7().simple().to_string();
        let plaintext = zeroize::Zeroizing::new(
            serde_json::to_vec(&operation).map_err(|_| CloudError::Storage)?,
        );
        let ciphertext = shared.seal(
            &lease.namespace,
            &format!("request-{id}"),
            lease.generation,
            &plaintext,
        )?;
        shared.sql.submit_management(lease, &id, &ciphertext)?;
        tokio::time::timeout(Duration::from_secs(23), async {
            loop {
                if let Some(ciphertext) = shared.sql.management_response(&id)? {
                    let plaintext = shared.unseal(
                        &lease.namespace,
                        &format!("response-{id}"),
                        lease.generation,
                        &ciphertext,
                    )?;
                    let reply: Reply =
                        serde_json::from_slice(&plaintext).map_err(|_| CloudError::Storage)?;
                    return match reply.error.as_deref() {
                        None => reply.value.ok_or(CloudError::Storage),
                        Some("conflict") => Err(CloudError::Conflict),
                        Some("invalid_argument") => Err(CloudError::InvalidArgument),
                        Some("not_found") => Err(CloudError::NotFound),
                        Some("permission_denied") => Err(CloudError::PermissionDenied),
                        _ => Err(CloudError::Storage),
                    };
                }
                // Check a result first: successful stop intentionally handed off the owner.
                shared.sql.verify_lease(lease)?;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| CloudError::Storage)?
    }

    pub(super) async fn start_management(
        &self,
        state: &Arc<State>,
        live: Arc<LiveRuntime>,
    ) -> std::result::Result<(), CloudError> {
        let host = Arc::downgrade(state.hosted.as_ref().ok_or(CloudError::Storage)?);
        let shared = self.shared.clone().ok_or(CloudError::Storage)?;
        let lease = live.lease.clone().ok_or(CloudError::Storage)?;
        let task_live = live.clone();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut renewed = std::time::Instant::now();
            let mut jobs = tokio::task::JoinSet::new();
            let mut drain = false;
            loop {
                tick.tick().await;
                if *task_live.shutdown.borrow() {
                    drain = true;
                    break;
                }
                if renewed.elapsed() >= Duration::from_secs(5) {
                    if shared.sql.renew_lease(&lease, 30).is_err() {
                        let _ = task_live.runtime.shutdown(Duration::from_secs(2)).await;
                        task_live.shutdown.send_replace(true);
                        break;
                    }
                    renewed = std::time::Instant::now();
                }
                while jobs.try_join_next().is_some() {}
                if jobs.len() >= 4 {
                    continue;
                }
                let Some(host) = host.upgrade() else {
                    break;
                };
                let requests = match shared.sql.take_management(&lease) {
                    Ok(requests) => requests,
                    Err(_) => continue,
                };
                for request in requests {
                    let host = host.clone();
                    let shared = shared.clone();
                    let live = task_live.clone();
                    let lease = lease.clone();
                    jobs.spawn(async move {
                        let operation = shared
                            .unseal(
                                &lease.namespace,
                                &format!("request-{}", request.id),
                                lease.generation,
                                &request.ciphertext,
                            )
                            .and_then(|bytes| {
                                serde_json::from_slice::<Management>(&bytes)
                                    .map_err(|_| CloudError::Storage)
                            });
                        let handoff = if let Ok(Management::Stop { handoff_owner }) = &operation {
                            Some(handoff_owner.clone())
                        } else {
                            None
                        };
                        let result = match operation {
                            Ok(operation) => host.handle_management(&live, operation).await,
                            Err(error) => Err(error),
                        };
                        let stopped = handoff.is_some() && result.is_ok();
                        let reply = match result {
                            Ok(value) => Reply {
                                value: Some(value),
                                error: None,
                            },
                            Err(error) => Reply {
                                value: None,
                                error: Some(
                                    match error {
                                        CloudError::Conflict => "conflict",
                                        CloudError::InvalidArgument => "invalid_argument",
                                        CloudError::NotFound => "not_found",
                                        CloudError::PermissionDenied => "permission_denied",
                                        _ => "storage",
                                    }
                                    .into(),
                                ),
                            },
                        };
                        if let Ok(plaintext) =
                            serde_json::to_vec(&reply).map(zeroize::Zeroizing::new)
                            && let Ok(ciphertext) = shared.seal(
                                &lease.namespace,
                                &format!("response-{}", request.id),
                                lease.generation,
                                &plaintext,
                            )
                        {
                            let _ = shared.sql.complete_management(
                                &lease,
                                &request.id,
                                &ciphertext,
                                if stopped { handoff.as_deref() } else { None },
                            );
                        }
                        if stopped {
                            live.shutdown.send_replace(true);
                        }
                    });
                }
            }
            if drain {
                let _ = tokio::time::timeout(Duration::from_secs(21), async {
                    while jobs.join_next().await.is_some() {}
                })
                .await;
            }
            jobs.abort_all();
            if let Some(host) = host.upgrade() {
                let mut entries = host.live.lock().await;
                if entries
                    .get(&lease.namespace)
                    .is_some_and(|entry| Arc::ptr_eq(entry, &task_live))
                {
                    entries.remove(&lease.namespace);
                }
            }
            let _ = shared.sql.release_lease(&lease);
        });
        *live.management.lock().await = Some(task);
        Ok(())
    }

    async fn handle_management(
        &self,
        live: &LiveRuntime,
        operation: Management,
    ) -> std::result::Result<serde_json::Value, CloudError> {
        let shared = self.shared.as_ref().ok_or(CloudError::Storage)?;
        shared
            .sql
            .verify_lease(live.lease.as_ref().ok_or(CloudError::Storage)?)?;
        let timeout = Duration::from_secs(20);
        let _file_lifecycle = if matches!(&operation, Management::File { .. }) {
            Some(self.lifecycle.lock().await)
        } else {
            None
        };
        let _operation = if matches!(&operation, Management::Busy | Management::Stop { .. }) {
            None
        } else {
            Some(live.pin())
        };
        match operation {
            Management::File { file } => Self::handle_file_operation(live, file).await,
            Management::ProviderModels { profile, revision } => {
                self.provider_catalog(live, &profile, revision).await
            }
            Management::Busy => Ok(
                serde_json::json!({"busy":live.operations.load(std::sync::atomic::Ordering::SeqCst) != 0 || live.runtime.has_pending_executions()||Self::pending_children(live)?}),
            ),
            Management::Stop { .. } => {
                if live.operations.load(std::sync::atomic::Ordering::SeqCst) != 0
                    || live.runtime.has_pending_executions()
                    || Self::pending_children(live)?
                {
                    return Err(CloudError::Conflict);
                }
                self.cancel_oauth(
                    &live.project_id,
                    Some(&live.lease.as_ref().ok_or(CloudError::Storage)?.user_id),
                )
                .await;
                if !live.runtime.shutdown(Duration::from_secs(10)).await {
                    return Err(CloudError::Storage);
                }
                live.shutdown.send_replace(true);
                super::worker::join_stopped_worker(&live.worker, Duration::from_secs(10)).await?;
                Ok(serde_json::json!({"stopped":true}))
            }
            Management::OAuthBegin {
                server,
                flow_id,
                binding,
                callback,
                local_development,
            } => {
                let policy = if local_development {
                    colossus_runtime::McpOAuthBrowserCallbackPolicy::LocalDevelopmentLoopback
                } else {
                    colossus_runtime::McpOAuthBrowserCallbackPolicy::HttpsOnly
                };
                let login = tokio::time::timeout(
                    timeout,
                    live.runtime
                        .runtime()
                        .mcp_oauth_browser_login_begin_with_policy(
                            &server, &flow_id, &binding, &callback, policy,
                        ),
                )
                .await
                .map_err(|_| CloudError::Storage)?
                .map_err(|_| CloudError::Storage)?;
                serde_json::to_value(login).map_err(|_| CloudError::Storage)
            }
            Management::OAuthComplete {
                server,
                flow_id,
                binding,
                callback,
            } => {
                tokio::time::timeout(
                    timeout,
                    live.runtime
                        .runtime()
                        .mcp_oauth_browser_login_complete(&server, &flow_id, &binding, &callback),
                )
                .await
                .map_err(|_| CloudError::Storage)?
                .map_err(|_| CloudError::Storage)?;
                Ok(serde_json::json!({"authenticated":true}))
            }
            Management::OAuthStatus { server } => {
                let status =
                    tokio::time::timeout(timeout, live.runtime.runtime().mcp_oauth_status(&server))
                        .await
                        .map_err(|_| CloudError::Storage)?
                        .map_err(|_| CloudError::Storage)?;
                Ok(serde_json::json!({"authenticated":status.authenticated}))
            }
            Management::OAuthLogout { server } => {
                tokio::time::timeout(timeout, live.runtime.runtime().mcp_oauth_logout(&server))
                    .await
                    .map_err(|_| CloudError::Storage)?
                    .map_err(|_| CloudError::Storage)?;
                Ok(serde_json::json!({"authenticated":false}))
            }
            Management::OAuthCancel {
                server,
                flow_id,
                binding,
            } => {
                tokio::time::timeout(
                    timeout,
                    live.runtime
                        .runtime()
                        .mcp_oauth_browser_login_cancel(&server, &flow_id, &binding),
                )
                .await
                .map_err(|_| CloudError::Storage)?
                .map_err(|_| CloudError::Storage)?;
                Ok(serde_json::json!({"cancelled":true}))
            }
        }
    }

    pub(crate) async fn begin_shared_edit(
        &self,
        state: &State,
        caller: &CloudCaller,
        project_wide: bool,
    ) -> std::result::Result<Option<SharedEdit>, CloudError> {
        self.begin_shared_edit_with(state, caller, project_wide, None)
            .await
    }

    /// Serialize encrypted store edits without retiring execution when authority is unchanged.
    pub(crate) fn begin_storage_edit(
        &self,
        caller: &CloudCaller,
    ) -> std::result::Result<Option<SharedEdit>, CloudError> {
        let Some(shared) = &self.shared else {
            return Ok(None);
        };
        let owner = format!("edit-{}", uuid::Uuid::now_v7().simple());
        shared
            .sql
            .claim_project_gate(caller.project_id(), &owner, 300)?;
        Ok(Some(SharedEdit {
            shared: shared.clone(),
            project: caller.project_id().into(),
            owner,
            reservations: Vec::new(),
        }))
    }

    pub(super) async fn begin_shared_edit_with(
        &self,
        state: &State,
        caller: &CloudCaller,
        project_wide: bool,
        before_stop: Option<Management>,
    ) -> std::result::Result<Option<SharedEdit>, CloudError> {
        let Some(mut edit) = self.begin_storage_edit(caller)? else {
            return Ok(None);
        };
        let shared = edit.shared.clone();
        let busy = if project_wide {
            self.active_project_run_status(caller.project_id()).await?
        } else {
            self.user_is_busy(state, caller.project_id(), caller.subject())
                .await?
        };
        if busy {
            return Err(CloudError::Conflict);
        }
        let leases = shared.sql.project_leases(caller.project_id())?;
        if leases.len() > 1024 {
            return Err(CloudError::ResourceExhausted);
        }
        for lease in leases
            .into_iter()
            .filter(|lease| project_wide || lease.user_id == caller.subject())
        {
            let busy = self.management_request(&lease, Management::Busy).await?;
            if busy.get("busy").and_then(serde_json::Value::as_bool) != Some(false) {
                return Err(CloudError::Conflict);
            }
            if let Some(operation) = &before_stop {
                let bytes = zeroize::Zeroizing::new(
                    serde_json::to_vec(operation).map_err(|_| CloudError::Storage)?,
                );
                self.management_request(
                    &lease,
                    serde_json::from_slice(&bytes).map_err(|_| CloudError::Storage)?,
                )
                .await?;
            }
            self.management_request(
                &lease,
                Management::Stop {
                    handoff_owner: edit.owner.clone(),
                },
            )
            .await?;
            let reservation = shared.sql.read_lease(&lease.namespace)?;
            if reservation.owner_id != edit.owner {
                return Err(CloudError::Conflict);
            }
            edit.reservations.push(reservation);
        }
        Ok(Some(edit))
    }
}

pub(crate) struct SharedEdit {
    pub shared: Arc<shared::SharedChat>,
    pub project: String,
    pub owner: String,
    reservations: Vec<ChatLease>,
}
impl Drop for SharedEdit {
    fn drop(&mut self) {
        for lease in &self.reservations {
            let _ = self.shared.sql.release_lease(lease);
        }
        let _ = self
            .shared
            .sql
            .release_project_gate(&self.project, &self.owner);
    }
}
