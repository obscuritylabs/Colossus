//! Windows native managed-sidecar lifecycle acceptance.

#![cfg(windows)]

#[path = "windows_lifecycle/mcp.rs"]
mod mcp;

use colossus_api::{ApiScope, IdempotencyKey, scopes};
use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, BackendKind, Colossus, CreateRunRequest, GetRunRequest,
    InputContentPart, InstanceId, ManagedAccessProfile, ManagedExecutionBoundary,
    ManagedRuntimeConfig, NativeSidecarLifecycle, NativeSidecarStatus, RunMode, RunStatus,
    Sha256Digest, SidecarApplicationGrant, SidecarApprovalBrokerGrant, SidecarBootstrapConfig,
    SidecarOptions, VerifiedExecutable, WorkspaceIdentity,
};
use colossus_windows_native::{BoundPath, create_private_directory};
use colossus_worker_protocol::{WorkerApprovalMode, WorkerControlClient, worker_ipc_endpoint};
use sha2::{Digest as _, Sha256};
use std::{fs::File, io::Read as _, path::Path, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";

struct KeyringCleanup {
    instance_id: Uuid,
}

struct PrivateDirectory {
    path: std::path::PathBuf,
}

impl PrivateDirectory {
    fn in_user_profile() -> Self {
        let user_profile = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute())
            .expect("absolute Windows user profile");
        let path = user_profile.join(format!(".colossus-windows-test-{}", Uuid::now_v7()));
        create_private_directory(&path).expect("private instance directory");
        Self { path }
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let user_profile = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .expect("Windows user profile");
        assert_eq!(self.path.parent(), Some(user_profile.as_path()));
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl Drop for KeyringCleanup {
    fn drop(&mut self) {
        for account in [
            format!("journal-key:journal-{}", self.instance_id),
            format!("signing-key:checkpoint-{}", self.instance_id),
            format!("journal-anchor:journal-{}", self.instance_id),
        ] {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &account) {
                let _ = entry.delete_credential();
            }
        }
    }
}

fn executable_digest(path: &Path) -> [u8; 32] {
    let mut file = File::open(path).expect("open sidecar executable");
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).expect("read sidecar executable");
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    digest.finalize().into()
}

fn workspace_identity(path: &Path) -> WorkspaceIdentity {
    let binding = BoundPath::open_directory(path).expect("bind workspace");
    let identity = binding.identity();
    WorkspaceIdentity::from_windows_parts(identity.volume_serial_number, identity.file_id)
        .expect("workspace identity")
}

async fn wait_for_terminal_run(client: &Colossus, run_id: &str) -> RunStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let run = client
                .get_run(GetRunRequest {
                    run_id: run_id.to_owned(),
                })
                .await
                .expect("get run")
                .run;
            if matches!(
                run.status,
                RunStatus::Completed
                    | RunStatus::Failed
                    | RunStatus::Cancelled
                    | RunStatus::Interrupted
                    | RunStatus::OutcomeUnknown
            ) {
                return run.status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("run terminal timeout")
}

/// Live acceptance requires Windows Credential Manager and loopback sockets.
#[tokio::test]
#[ignore = "requires Windows Credential Manager and loopback networking"]
async fn verified_sidecar_bootstraps_pinned_grpc_runs_echo_and_closes() {
    let workspace = tempfile::tempdir().expect("workspace");
    let instance = PrivateDirectory::in_user_profile();
    let workspace = std::fs::canonicalize(workspace.path()).expect("canonical workspace");
    let instance_id = Uuid::now_v7();
    let _cleanup = KeyringCleanup { instance_id };
    let executable = std::fs::canonicalize(env!("CARGO_BIN_EXE_colossus-sidecar"))
        .expect("canonical sidecar executable");
    let grant = SidecarApplicationGrant::new(
        "app:native-sidecar-windows-acceptance",
        [
            ApiScope::new(scopes::RUNS_EXECUTE).expect("execute scope"),
            ApiScope::new(scopes::RUNS_READ).expect("read scope"),
        ],
        ["primary".into()],
        Vec::<String>::new(),
    )
    .expect("application grant");
    let bootstrap = SidecarBootstrapConfig::new(
        &workspace,
        ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal)
            .with_execution_boundary(ManagedExecutionBoundary::OfflineIsolated),
        grant,
    )
    .expect("bootstrap")
    .with_expected_workspace_identity(workspace_identity(&workspace))
    .expect("expected workspace identity")
    .with_approval_broker_grant(
        SidecarApprovalBrokerGrant::new(
            "app:native-sidecar-windows-acceptance",
            ["primary".into()],
        )
        .unwrap(),
    )
    .expect("approval broker")
    .with_connector_grant(
        SidecarApplicationGrant::new(
            "app:cloud-windows-acceptance",
            [
                ApiScope::new(scopes::RUNS_EXECUTE).unwrap(),
                ApiScope::new(scopes::RUNS_READ).unwrap(),
            ],
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap(),
    )
    .expect("independent cloud grant")
    .with_risk_auto_approvals()
    .with_worker_ipc_authentication(colossus_sdk::Secret::new(vec![0x5a; 32]).expect("worker key"))
    .expect("worker authentication");
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let options = SidecarOptions::new(
        InstanceId::from_uuid(instance_id),
        AppPrivateInstanceDir::new(&instance.path).expect("instance directory"),
        VerifiedExecutable::new(
            &executable,
            Sha256Digest::from_bytes(executable_digest(&executable)),
        )
        .expect("verified executable"),
        ApiMajor::new(1).expect("API major"),
    )
    .expect("sidecar options");

    let started = std::time::Instant::now();
    let client = match Colossus::start_sidecar(&lifecycle, options.clone()).await {
        Ok(client) => client,
        Err(error) => {
            let entries = std::fs::read_dir(&instance.path)
                .expect("inspect failed instance")
                .map(|entry| entry.expect("instance entry").file_name())
                .collect::<Vec<_>>();
            panic!(
                "start sidecar after {:?}: {error:?}; instance entries: {entries:?}",
                started.elapsed()
            );
        }
    };
    assert_eq!(lifecycle.status(), NativeSidecarStatus::Ready);
    assert_eq!(client.backend_kind(), BackendKind::Sidecar);
    let state_path = instance
        .path
        .join("state.redb")
        .canonicalize()
        .expect("state");
    let worker = WorkerControlClient::new(
        worker_ipc_endpoint(&state_path).expect("worker endpoint"),
        Zeroizing::new([0x5a; 32]),
    )
    .expect("authenticated worker");
    assert_eq!(
        worker.approval_mode().await.expect("initial approval mode"),
        Some(WorkerApprovalMode::RiskAuto)
    );
    assert_eq!(
        worker
            .set_approval_mode(WorkerApprovalMode::Ask)
            .await
            .expect("manual Ask"),
        WorkerApprovalMode::Ask
    );
    assert_eq!(
        worker
            .approval_mode()
            .await
            .expect("selected approval mode"),
        Some(WorkerApprovalMode::Ask)
    );
    let created = client
        .create_run(CreateRunRequest {
            plugin_skill_ids: Vec::new(),
            input: vec![InputContentPart::Text("managed sidecar self-test".into())],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            goal_max_iterations: 0,
            research_depth: None,
            research_sources: Vec::new(),
            plan_action: None,
            branch: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new(Uuid::now_v7().to_string())
                .expect("idempotency key"),
        })
        .await
        .expect("create run");
    let run_id = created.run.run_id;
    assert!(!run_id.is_empty());
    let connector = client
        .connector_runs()
        .expect("dedicated Windows cloud client");
    assert!(client.instance_id().is_some());
    assert!(
        connector
            .get_run(GetRunRequest {
                run_id: run_id.clone()
            })
            .await
            .is_err(),
        "cloud cannot inherit Desktop runs"
    );
    let cloud_run = connector
        .create_run(CreateRunRequest {
            plugin_skill_ids: vec![],
            input: vec![InputContentPart::Text(
                "Windows cloud grant self-test".into(),
            )],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            goal_max_iterations: 0,
            research_depth: None,
            research_sources: vec![],
            plan_action: None,
            branch: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new("windows-cloud-once").unwrap(),
        })
        .await
        .expect("cloud grant executes");
    assert!(
        client
            .get_run(GetRunRequest {
                run_id: cloud_run.run.run_id.clone()
            })
            .await
            .is_err(),
        "Desktop cannot inherit cloud runs"
    );
    connector
        .get_run(GetRunRequest {
            run_id: cloud_run.run.run_id,
        })
        .await
        .expect("cloud reads its own run");
    assert_eq!(
        wait_for_terminal_run(&client, &run_id).await,
        RunStatus::Completed
    );
    client.close().await.expect("graceful close");
    let restarted = Colossus::start_sidecar(&lifecycle, options)
        .await
        .expect("restart sidecar");
    let restarted_mode = worker.approval_mode().await;
    restarted.close().await.expect("close restarted sidecar");
    assert_eq!(
        restarted_mode.expect("restarted approval mode"),
        Some(WorkerApprovalMode::RiskAuto)
    );
}
