#![cfg(unix)]

//! Native managed-sidecar bootstrap and pinned-gRPC lifecycle acceptance test.

use colossus_api::{ApiScope, IdempotencyKey, scopes};
use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, BackendKind, Colossus, CreateRunRequest, InputContentPart,
    InstanceId, ManagedAccessProfile, ManagedRuntimeConfig, NativeSidecarLifecycle,
    NativeSidecarStatus, RunMode, Secret, Sha256Digest, SidecarApplicationGrant,
    SidecarApprovalBrokerGrant, SidecarBootstrapConfig, SidecarOptions, VerifiedExecutable,
};
use sha2::{Digest as _, Sha256};
use std::{
    fs::{File, Permissions},
    io::Read as _,
    os::unix::fs::PermissionsExt as _,
    path::Path,
};
use uuid::Uuid;

const KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.managed-runtime";

fn current_workspace_identity(metadata: &std::fs::Metadata) -> colossus_sdk::WorkspaceIdentity {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;

        colossus_sdk::WorkspaceIdentity::from_macos_parts(
            metadata.st_dev(),
            metadata.st_ino(),
            metadata.st_birthtime(),
            metadata.st_birthtime_nsec(),
        )
        .expect("current workspace identity")
    }
    #[cfg(not(target_os = "macos"))]
    {
        use std::os::unix::fs::MetadataExt as _;

        colossus_sdk::WorkspaceIdentity::from_unix_parts(metadata.dev(), metadata.ino())
    }
}

struct KeyringCleanup {
    instance_id: Uuid,
}

impl Drop for KeyringCleanup {
    fn drop(&mut self) {
        for account in [
            format!("journal-key:journal-{}", self.instance_id),
            format!("signing-key:checkpoint-{}", self.instance_id),
            format!("journal-anchor:journal-{}", self.instance_id),
        ] {
            delete_test_credential(&account);
        }
    }
}

#[cfg(target_os = "macos")]
fn delete_test_credential(account: &str) {
    let _ = std::process::Command::new("/usr/bin/security")
        .env_clear()
        .args([
            "delete-generic-password",
            "-s",
            KEYRING_SERVICE,
            "-a",
            account,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(target_os = "macos"))]
fn delete_test_credential(account: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, account) {
        let _ = entry.delete_credential();
    }
}

fn executable_digest(path: &Path) -> [u8; 32] {
    let mut file = File::open(path).expect("open sidecar executable");
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).expect("read executable");
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    digest.finalize().into()
}

/// Live acceptance requires a usable platform credential store and loopback sockets.
#[tokio::test]
#[ignore = "requires a native credential store and loopback networking"]
async fn verified_sidecar_bootstraps_pinned_grpc_and_closes_by_guardian_eof() {
    let workspace = tempfile::tempdir().expect("workspace");
    let instance_root = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o700))
        .tempdir()
        .expect("instance root");
    let instance = instance_root
        .path()
        .join("managed-local")
        .join("d".repeat(64));
    std::fs::create_dir_all(&instance).expect("desktop-shaped instance");
    std::fs::set_permissions(&instance, Permissions::from_mode(0o700))
        .expect("instance permissions");
    let workspace = std::fs::canonicalize(workspace.path()).expect("canonical workspace");
    let workspace_metadata = std::fs::symlink_metadata(&workspace).expect("workspace metadata");
    let instance_dir = std::fs::canonicalize(&instance).expect("canonical instance");
    let executable_bundle = tempfile::tempdir().expect("executable bundle");
    let executable = executable_bundle.path().join("colossus-sidecar");
    std::fs::copy(env!("CARGO_BIN_EXE_colossus-sidecar"), &executable)
        .expect("copy stable sidecar executable");
    std::fs::set_permissions(&executable, Permissions::from_mode(0o500))
        .expect("sidecar executable permissions");
    let instance_id = Uuid::now_v7();
    let _cleanup = KeyringCleanup { instance_id };
    let grant = SidecarApplicationGrant::new(
        "app:native-sidecar-acceptance",
        [
            ApiScope::new(scopes::RUNS_EXECUTE).expect("execute scope"),
            ApiScope::new(scopes::RUNS_READ).expect("read scope"),
        ],
        ["primary".into()],
        Vec::<String>::new(),
    )
    .expect("grant");
    let bootstrap = SidecarBootstrapConfig::new(
        &workspace,
        ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal),
        grant,
    )
    .expect("bootstrap")
    .with_expected_workspace_identity(current_workspace_identity(&workspace_metadata))
    .expect("expected workspace identity");
    let bootstrap = bootstrap
        .with_approval_broker_grant(
            SidecarApprovalBrokerGrant::new("app:native-sidecar-acceptance", ["primary".into()])
                .expect("approval broker grant"),
        )
        .expect("approval broker bootstrap")
        .with_worker_ipc_authentication(
            Secret::new(vec![0x5a; 32]).expect("bounded worker authentication"),
        )
        .expect("worker authentication bootstrap");
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    assert_eq!(lifecycle.status(), NativeSidecarStatus::Starting);
    let options = SidecarOptions::new(
        InstanceId::from_uuid(instance_id),
        AppPrivateInstanceDir::new(&instance_dir).expect("instance directory"),
        VerifiedExecutable::new(
            &executable,
            Sha256Digest::from_bytes(executable_digest(&executable)),
        )
        .expect("verified executable"),
        ApiMajor::new(1).expect("API major"),
    )
    .expect("sidecar options");
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .expect("start sidecar");
    assert_eq!(lifecycle.status(), NativeSidecarStatus::Ready);
    assert_eq!(client.backend_kind(), BackendKind::Sidecar);
    let created = client
        .create_run(CreateRunRequest {
            input: vec![InputContentPart::Text("managed sidecar self-test".into())],
            session_id: None,
            role: "primary".into(),
            mode: RunMode::Plan,
            selected_skills: Vec::new(),
            plan_action: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new(Uuid::now_v7().to_string())
                .expect("idempotency key"),
        })
        .await
        .expect("create run");
    assert!(!created.run.run_id.is_empty());
    client.close().await.expect("graceful close");
    assert_eq!(lifecycle.status(), NativeSidecarStatus::Stopping);
}
