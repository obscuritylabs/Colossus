use super::*;
use colossus_connector::{DeploymentKind, EnrollmentStore, RuntimeInventory};
use colossus_contracts::VaultRecord;
use colossus_ports::{CredentialError, CredentialKey, CredentialVault};
use std::{
    future::ready,
    sync::{Mutex as StdMutex, atomic::AtomicUsize},
};

fn config(enabled: bool) -> ConnectionConfig {
    ConnectionConfig {
        endpoint: "https://control-plane.example".into(),
        project_id: "synthetic-project".into(),
        node_id: "synthetic-node".into(),
        instance_id: "synthetic-instance".into(),
        certificate_pem: "synthetic-certificate".into(),
        ca_pem: "synthetic-ca".into(),
        capabilities: Vec::new(),
        revoked: false,
        inventory: Some(RuntimeInventory {
            policy: None,
            host_id: "synthetic-host".into(),
            host_label: "Synthetic host".into(),
            platform: "macos".into(),
            deployment_kind: DeploymentKind::Desktop,
            workspace_id: "synthetic-workspace".into(),
            workspace_label: "Synthetic workspace".into(),
            sharing: if enabled {
                WorkspaceSharing::SharedVisibleSessions
            } else {
                WorkspaceSharing::CloudOwned
            },
        }),
        shared_continuation: enabled,
    }
}

struct Vault {
    record: StdMutex<VaultRecord>,
    fail_write: bool,
    commit_before_failure: bool,
    writes: AtomicUsize,
}
impl Vault {
    fn new(config: &ConnectionConfig, fail_write: bool, commit_before_failure: bool) -> Arc<Self> {
        let record = serde_json::json!({ "config": config, "pending": { "client_key_pem": "synthetic-private-key", "csr_pem": "synthetic-csr", "token": "", "enrollment_url": "https://control-plane.example/api/enroll", "instance_id": config.instance_id }, "rotation": null });
        Arc::new(Self {
            record: StdMutex::new(VaultRecord::new(serde_json::to_vec(&record).unwrap()).unwrap()),
            fail_write,
            commit_before_failure,
            writes: AtomicUsize::new(0),
        })
    }
}
impl CredentialVault for Vault {
    fn read(&self, _: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
        Ok(Some(
            VaultRecord::new(self.record.lock().unwrap().expose().to_vec()).unwrap(),
        ))
    }
    fn write(&self, _: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if !self.fail_write || self.commit_before_failure {
            *self.record.lock().unwrap() = VaultRecord::new(record.expose().to_vec()).unwrap();
        }
        if self.fail_write {
            Err(CredentialError::Io)
        } else {
            Ok(())
        }
    }
    fn delete(&self, _: &CredentialKey) -> Result<(), CredentialError> {
        panic!("sharing must not delete enrollment")
    }
}

struct TaskLifetime(Arc<AtomicBool>);
impl Drop for TaskLifetime {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn active(config: &ConnectionConfig) -> (Connections, Arc<AtomicBool>, Arc<AtomicBool>) {
    active_with_stop(config, true).await
}

async fn active_with_stop(
    config: &ConnectionConfig,
    cooperative: bool,
) -> (Connections, Arc<AtomicBool>, Arc<AtomicBool>) {
    let (shutdown, mut receiver) = watch::channel(false);
    let (_, status) = watch::channel(ConnectorStatus::Connected);
    let alive = Arc::new(AtomicBool::new(true));
    let dropped = Arc::new(AtomicBool::new(false));
    let lifetime = TaskLifetime(Arc::clone(&dropped));
    let (started, started_receiver) = tokio::sync::oneshot::channel();
    let task = tauri::async_runtime::spawn(async move {
        let _lifetime = lifetime;
        started.send(()).unwrap();
        if cooperative {
            let _ = receiver.changed().await;
        } else {
            std::future::pending::<()>().await;
        }
    });
    started_receiver.await.unwrap();
    let connections = Mutex::new(HashMap::from([(
        "target".into(),
        CloudSession {
            config: config.clone(),
            status,
            shutdown,
            task: Some(task),
            alive: Arc::clone(&alive),
            sharing_recovery_required: false,
            shutdown_confirmed: false,
            sharing_restart_required: false,
        },
    )]));
    (connections, alive, dropped)
}

fn state(request: SetWorkspaceSharingRequest) -> WorkspaceSharingState {
    WorkspaceSharingState {
        recipient_application_id: request.recipient_application_id,
        enabled: request.enabled,
        allow_continuation: request.allow_continuation,
    }
}

fn assert_paused(connections: &Connections, dropped: &AtomicBool) {
    assert!(
        dropped.load(Ordering::Acquire),
        "connector destructor completed before any permission change"
    );
    let sessions = connections.try_lock().unwrap();
    let session = &sessions["target"];
    assert!(session.task.is_none());
    assert!(!session.alive.load(Ordering::Acquire));
    assert!(session.sharing_recovery_required);
}

#[tokio::test]
async fn successful_update_quiesces_before_grant_and_real_enrollment_write() {
    let original = config(false);
    let (connections, alive, dropped) = active(&original).await;
    let vault = Vault::new(&original, false, false);
    let store = EnrollmentStore::new(vault.clone(), "target").unwrap();
    let (saved, previous) = update(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            ready(Ok(state(request)))
        },
        || store.set_sharing(true, true),
    )
    .await
    .unwrap();
    assert_eq!(previous, ConnectorStatus::Connected);
    assert!(!alive.load(Ordering::Acquire));
    assert!(saved_request(&saved).allow_continuation);
    assert!(saved_request(&store.load().unwrap().unwrap().0).enabled);
    assert_eq!(vault.writes.load(Ordering::SeqCst), 1);
    let summary = summary(&connections, "target").await.unwrap();
    assert_eq!(summary.status, ConnectorStatus::Disconnected);
    assert!(!summary.sharing_recovery_required.is_required());
}

#[tokio::test]
async fn failed_enable_cannot_export_new_sessions_and_compensates_without_restart() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let vault = Vault::new(&original, true, false);
    let store = EnrollmentStore::new(vault, "target").unwrap();
    let mut calls = Vec::new();
    let mut runtime_enabled = false;
    let mut escaped = 0;
    let error = update(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            if request.enabled && !dropped.load(Ordering::Acquire) {
                escaped += 1;
            }
            assert_paused(&connections, &dropped);
            runtime_enabled = request.enabled;
            calls.push((request.enabled, request.allow_continuation));
            ready(Ok(state(request)))
        },
        || store.set_sharing(true, true),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, [(true, true), (false, false)]);
    assert_eq!(escaped, 0);
    assert!(!runtime_enabled);
    assert!(!error.outcome_unknown);
    assert!(!saved_request(&store.load().unwrap().unwrap().0).enabled);
    let summary = summary(&connections, "target").await.unwrap();
    assert_eq!(summary.status, ConnectorStatus::Disconnected);
    assert!(summary.sharing_recovery_required.is_required());
}

#[tokio::test]
async fn failed_disable_never_restores_an_old_grant_and_explicit_reconnect_reconciles_saved_choice()
{
    let original = config(true);
    let (connections, _, dropped) = active(&original).await;
    let vault = Vault::new(&original, true, false);
    let store = EnrollmentStore::new(vault, "target").unwrap();
    let mut calls = Vec::new();
    let mut runtime_enabled = true;
    update(
        &connections,
        "target",
        &original,
        request(false, true),
        |request| {
            assert_paused(&connections, &dropped);
            calls.push((request.enabled, request.allow_continuation));
            runtime_enabled = request.enabled;
            ready(Ok(state(request)))
        },
        || store.set_sharing(false, true),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, [(false, false), (false, false)]);
    assert!(!runtime_enabled);
    let retained = store.load().unwrap().unwrap().0;
    assert!(saved_request(&retained).enabled);
    let summary_before = summary(&connections, "target").await.unwrap();
    assert!(
        summary_before.shared_sessions,
        "retained choice, not an actual permission claim"
    );
    assert!(summary_before.sharing_recovery_required.is_required());
    assert_eq!(
        saved_choice(&retained),
        "share Desktop history and allow continuation"
    );
    update(
        &connections,
        "target",
        &retained,
        saved_request(&retained),
        |request| {
            assert_paused(&connections, &dropped);
            runtime_enabled = request.enabled;
            ready(Ok(state(request)))
        },
        || ready(Ok(retained.clone())),
    )
    .await
    .unwrap();
    assert!(
        runtime_enabled,
        "only the subsequent explicitly confirmed saved-choice reconciliation restores sharing"
    );
    assert!(
        !summary(&connections, "target")
            .await
            .unwrap()
            .sharing_recovery_required
            .is_required()
    );
}

#[tokio::test]
async fn failed_write_acknowledgement_and_compensation_remain_unknown() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let vault = Vault::new(&original, true, true);
    let store = EnrollmentStore::new(vault, "target").unwrap();
    let mut calls = 0;
    let error = update(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            calls += 1;
            if calls == 1 {
                ready(Ok(state(request)))
            } else {
                ready(Err(colossus_sdk::ApiError::failed_precondition(
                    colossus_sdk::ApiErrorReason::InternalInvariant,
                    "synthetic compensation failure",
                )))
            }
        },
        || store.set_sharing(true, true),
    )
    .await
    .err()
    .unwrap();
    assert!(
        saved_request(&store.load().unwrap().unwrap().0).enabled,
        "write committed before acknowledgement failed"
    );
    assert!(error.outcome_unknown);
    assert!(!error.retryable);
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_recovery_required
            .is_required()
    );
}

#[tokio::test]
async fn runtime_failure_does_not_persist_and_always_attempts_disable() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let vault = Vault::new(&original, false, false);
    let store = EnrollmentStore::new(vault.clone(), "target").unwrap();
    let mut calls = Vec::new();
    let error = update(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            calls.push(request.enabled);
            if calls.len() == 1 {
                ready(Err(colossus_sdk::ApiError::failed_precondition(
                    colossus_sdk::ApiErrorReason::InternalInvariant,
                    "synthetic known runtime rejection",
                )))
            } else {
                ready(Ok(state(request)))
            }
        },
        || store.set_sharing(true, true),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, [true, false]);
    assert_eq!(vault.writes.load(Ordering::SeqCst), 0);
    assert!(!error.outcome_unknown);
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_recovery_required
            .is_required()
    );
}

#[tokio::test]
async fn fresh_private_enrollment_clears_prior_sharing_before_connector_start() {
    let fresh = config(false);
    let connections = Mutex::new(HashMap::new());
    let mut runtime_enabled = true;
    update(
        &connections,
        "target",
        &fresh,
        saved_request(&fresh),
        |request| {
            assert_eq!(
                request.recipient_application_id,
                "app:colossus-desktop-cloud"
            );
            runtime_enabled = request.enabled;
            assert!(!request.allow_continuation);
            ready(Ok(state(request)))
        },
        || ready(Ok(fresh.clone())),
    )
    .await
    .unwrap();
    assert!(!runtime_enabled);
    assert!(connections.lock().await["target"].task.is_none());
    assert!(
        !summary(&connections, "target")
            .await
            .unwrap()
            .sharing_recovery_required
            .is_required()
    );
}

#[tokio::test]
async fn stop_timeout_blocks_permission_changes_even_after_forced_parent_abort() {
    let original = config(false);
    let (connections, _, _) = active_with_stop(&original, false).await;
    let error = quiesce(&connections, "target", &original, Duration::from_millis(10))
        .await
        .err()
        .unwrap();
    assert!(error.outcome_unknown);
    let mut commits = 0;
    for _ in 0..2 {
        update(
            &connections,
            "target",
            &original,
            request(true, true),
            |request| {
                commits += 1;
                ready(Ok(state(request)))
            },
            || ready(Ok(original.clone())),
        )
        .await
        .err()
        .unwrap();
    }
    assert_eq!(
        commits, 0,
        "parent abort is not proof that its child watches were joined"
    );
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_recovery_required
            .is_required()
    );
}

#[test]
fn disconnected_saved_metadata_does_not_claim_current_runtime_sharing() {
    let shared = config(true);
    let summary = super::super::disconnected_summary(
        "target".into(),
        Some(&shared),
        ConnectorStatus::Disconnected,
    );
    assert!(
        summary.shared_sessions,
        "retained saved intent remains available for explicit confirmation"
    );
    assert!(summary.sharing_recovery_required.is_required());
}

#[test]
fn reconnect_confirmation_reconciles_only_managed_local_authority() {
    let shared = config(true);
    let managed = reconnect_message(&shared, true);
    assert!(managed.contains("share Desktop history and allow continuation"));
    assert!(managed.contains("Reconnecting reapplies"));
    let external = reconnect_message(&shared, false);
    assert_eq!(
        external,
        "Reconnect runtime synthetic-instance to https://control-plane.example?"
    );
}

fn short_deadlines() -> Deadlines {
    Deadlines {
        runtime: Duration::from_millis(10),
        persistence: Duration::from_millis(10),
    }
}

#[tokio::test]
async fn timed_out_runtime_commit_stays_unknown_after_disable_ack_and_late_server_commit() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let runtime_enabled = Arc::new(AtomicBool::new(false));
    let delayed_state = Arc::clone(&runtime_enabled);
    let (release, delayed) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        delayed.await.unwrap();
        delayed_state.store(true, Ordering::Release);
    });
    let mut calls = 0;
    let error = update_with_deadlines(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            calls += 1;
            let first = calls == 1;
            let runtime_enabled = Arc::clone(&runtime_enabled);
            async move {
                if first {
                    std::future::pending::<ApiResult<WorkspaceSharingState>>().await
                } else {
                    runtime_enabled.store(false, Ordering::Release);
                    Ok(state(request))
                }
            }
        },
        || ready(Ok(original.clone())),
        short_deadlines(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, 2);
    assert!(
        error.outcome_unknown,
        "a disable ACK cannot order an older timed-out RPC"
    );
    assert!(!runtime_enabled.load(Ordering::Acquire));
    release.send(()).unwrap();
    server.await.unwrap();
    assert!(
        runtime_enabled.load(Ordering::Acquire),
        "the old server request may complete late"
    );
    let snapshot = summary(&connections, "target").await.unwrap();
    assert!(
        snapshot.sharing_restart_required.is_required()
            && snapshot.sharing_recovery_required.is_required()
    );
    assert!(
        ensure_recoverable(&connections, "target").await.is_err(),
        "disconnect/forget/revoke/enroll share the same restart fence"
    );
    assert!(check_start(&connections, "target").await.is_err());
    let mut retry_calls = 0;
    update(
        &connections,
        "target",
        &original,
        request(false, false),
        |request| {
            retry_calls += 1;
            ready(Ok(state(request)))
        },
        || ready(Ok(original.clone())),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(retry_calls, 0);
}

#[tokio::test]
async fn persistence_timeout_remains_unconfirmed_even_when_disable_succeeds() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let mut requests = Vec::new();
    let error = update_with_deadlines(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            requests.push(request.enabled);
            ready(Ok(state(request)))
        },
        std::future::pending::<Result<ConnectionConfig, &'static str>>,
        short_deadlines(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(requests, [true, false]);
    assert!(error.outcome_unknown);
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_restart_required
            .is_required()
    );
    assert!(ensure_recoverable(&connections, "target").await.is_err());
}

#[tokio::test]
async fn pending_compensation_is_bounded_and_requires_restart_without_retry() {
    let original = config(false);
    let (connections, _, dropped) = active(&original).await;
    let mut calls = 0;
    let error = update_with_deadlines(
        &connections,
        "target",
        &original,
        request(true, true),
        |request| {
            assert_paused(&connections, &dropped);
            calls += 1;
            let pending = calls == 2;
            async move {
                if pending {
                    std::future::pending::<ApiResult<WorkspaceSharingState>>().await
                } else {
                    Ok(state(request))
                }
            }
        },
        || ready(Err("synthetic persistence failure")),
        short_deadlines(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, 2);
    assert!(error.outcome_unknown && !error.retryable);
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_restart_required
            .is_required()
    );
}

#[tokio::test]
async fn lost_transport_acknowledgement_is_not_a_definitive_disable_even_after_compensation() {
    let original = config(false);
    let (connections, _, _) = active(&original).await;
    let mut calls = 0;
    let error = update(
        &connections,
        "target",
        &original,
        request(true, false),
        |request| {
            calls += 1;
            if calls == 1 {
                ready(Err(colossus_sdk::ApiError {
                    code: ApiErrorCode::Unavailable,
                    ..colossus_sdk::ApiError::failed_precondition(
                        colossus_sdk::ApiErrorReason::InternalInvariant,
                        "synthetic lost transport acknowledgement",
                    )
                }))
            } else {
                ready(Ok(state(request)))
            }
        },
        || ready(Ok(original.clone())),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(calls, 2);
    assert!(error.outcome_unknown);
    assert!(
        summary(&connections, "target")
            .await
            .unwrap()
            .sharing_restart_required
            .is_required()
    );
}
