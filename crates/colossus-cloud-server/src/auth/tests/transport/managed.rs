//! Browser-to-native-sidecar acceptance and application-exit connection semantics.
use super::*;
use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, Colossus, GetRunRequest, InstanceId, ManagedAccessProfile,
    ManagedRuntimeConfig, NativeSidecarLifecycle, Sha256Digest, SidecarApplicationGrant,
    SidecarApprovalBrokerGrant, SidecarBootstrapConfig, SidecarOptions, VerifiedExecutable,
};
use std::{fs::File, io::Read as _};

struct NativeKeys(uuid::Uuid);
impl Drop for NativeKeys {
    fn drop(&mut self) {
        for account in [
            format!("journal-key:journal-{}", self.0),
            format!("signing-key:checkpoint-{}", self.0),
            format!("journal-anchor:journal-{}", self.0),
        ] {
            delete_native_test_key(&account);
        }
    }
}

#[cfg(target_os = "macos")]
fn delete_native_test_key(account: &str) {
    // The keyring delete implementation first reads the secret, which can open
    // an access prompt for keys created by the sidecar. Delete only this test's
    // UUID-scoped metadata, matching native_lifecycle's macOS cleanup.
    let _ = std::process::Command::new("/usr/bin/security")
        .env_clear()
        .args([
            "delete-generic-password",
            "-s",
            "com.obscuritylabs.colossus.managed-runtime",
            "-a",
            account,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(target_os = "macos"))]
fn delete_native_test_key(account: &str) {
    if let Ok(entry) = keyring::Entry::new("com.obscuritylabs.colossus.managed-runtime", account) {
        let _ = entry.delete_credential();
    }
}

fn digest(path: &Path) -> [u8; 32] {
    let mut file = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65_536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    hash.finalize().into()
}

#[tokio::test]
#[ignore = "native credential store, loopback and COLOSSUS_ACCEPTANCE_SIDECAR required"]
async fn managed_sidecar_cloud_run_isolated_and_application_exit_disconnects() {
    let sidecar = std::fs::canonicalize(
        std::env::var_os("COLOSSUS_ACCEPTANCE_SIDECAR").expect("prepared verified native sidecar"),
    )
    .unwrap();
    let (auth, issuer, oidc) = fixture().await;
    #[cfg(windows)]
    let directory = tempfile::tempdir_in(std::env::var_os("USERPROFILE").unwrap()).unwrap();
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let home = ColossusHome::ensure_at(root.join("native")).unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let instance_home =
        ColossusHome::ensure_at(root.join("managed-local").join("a".repeat(64))).unwrap();
    let instance = uuid::Uuid::now_v7();
    let _keys = NativeKeys(instance);
    let grant = |application, scopes: &[&str]| {
        SidecarApplicationGrant::new(
            application,
            scopes.iter().map(|scope| ApiScope::new(*scope).unwrap()),
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap()
    };
    let bootstrap = SidecarBootstrapConfig::new(
        &workspace,
        ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal),
        grant(
            "app:desktop-cloud-acceptance",
            &[
                scopes::RUNS_EXECUTE,
                scopes::RUNS_READ,
                scopes::RUNS_CONTROL,
                scopes::PROMPTS_RESPOND,
            ],
        ),
    )
    .unwrap()
    .with_colossus_home(home.root())
    .unwrap()
    .with_approval_broker_grant(
        SidecarApprovalBrokerGrant::new("app:desktop-cloud-acceptance", ["primary".into()])
            .unwrap(),
    )
    .unwrap()
    .with_connector_grant(grant(
        "app:independent-cloud-acceptance",
        &[
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
            scopes::PROMPTS_RESPOND,
            scopes::APPROVALS_RESPOND,
        ],
    ))
    .unwrap();
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let desktop = Colossus::start_sidecar(
        &lifecycle,
        SidecarOptions::new(
            InstanceId::from_uuid(instance),
            AppPrivateInstanceDir::new(instance_home.root()).unwrap(),
            VerifiedExecutable::new(&sidecar, Sha256Digest::from_bytes(digest(&sidecar))).unwrap(),
            ApiMajor::new(1).unwrap(),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let runs = desktop
        .connector_runs()
        .expect("separate native cloud grant");
    let browser = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = auth.config.clone();
    config.http_bind = browser.local_addr().unwrap();
    config.grpc_bind = grpc.local_addr().unwrap();
    config.public_origin = format!("http://{}", config.http_bind);
    config.grpc_endpoint = format!("https://localhost:{}", config.grpc_bind.port());
    config.memberships[0]
        .permissions
        .insert(CloudPermission::Administer);
    tls_files(&root, &mut config);
    drop(auth);
    let origin = config.public_origin.clone();
    let server = crate::server::CloudServer::with_store(
        config,
        Arc::new(colossus_cloud::storage::MemoryCloudStore::default()),
    )
    .await
    .unwrap();
    let (stop, shutdown) = watch::channel(false);
    let host = tokio::spawn(server.serve_listeners(browser, grpc, shutdown));
    let browser_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let headers = sign_in(&browser_client, &origin, &issuer).await;
    let invitation = post(
        &browser_client,
        &origin,
        &headers,
        "/api/projects/project-a/invitations",
        json!({"label":"Managed Local", "roles":["primary"]}),
    )
    .await;
    let vault = Arc::new(
        PlatformCredentialVault::with_key_store(
            home.confined_root().clone(),
            "managed-cloud-acceptance",
            Arc::new(TestKeys::default()),
        )
        .unwrap(),
    );
    let enrollment = EnrollmentStore::new(vault, "managed-local").unwrap();
    let config = enrollment
        .enroll(
            invitation["enrollment_url"].as_str().unwrap().into(),
            Zeroizing::new(invitation["token"].as_str().unwrap().into()),
            desktop.instance_id().unwrap().to_string(),
            vec![],
            true,
        )
        .await
        .unwrap();
    let (_, key) = enrollment.load().unwrap().unwrap();
    let (_disconnect, close) = watch::channel(false);
    let (status, mut online) = watch::channel(ConnectorStatus::Connecting);
    let connector = tokio::spawn(
        RuntimeConnector::new(config.clone(), key, runs)
            .unwrap()
            .with_enrollment_store(enrollment.clone())
            .run(close, status),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while *online.borrow() != ConnectorStatus::Connected {
            online.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let allocated = post(&browser_client, &origin, &headers, "/api/projects/project-a/tasks",
        json!({"node_id":config.node_id,"request":{"plugin_skill_ids":[],"input":[{"text":"Native Desktop cloud acceptance"}],"session_id":null,"end_user_id":null,"role":"primary","mode":"execute","research_depth":null,"research_sources":[],"plan_action":null,"branch":null,"max_turns":1,"idempotency_key":"native-desktop-cloud-task"}})).await;
    let task_id = allocated["task"]["task_id"].as_str().unwrap();
    let completed = wait_task(&browser_client, &origin, &headers, task_id, |task| {
        task.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.run.status == RunStatus::Completed
                && task.last_sequence == snapshot.run.last_sequence
        })
    })
    .await;
    let run_id = completed.run_id.unwrap();
    assert!(
        completed.last_sequence > 0 && completed.released_bytes > 0,
        "released native output reaches the browser"
    );
    let mut events = browser_client
        .get(format!(
            "{origin}/api/projects/project-a/tasks/{task_id}/events?after=0"
        ))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    let chunk = tokio::time::timeout(Duration::from_secs(3), events.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&chunk).contains("id: 1"));
    drop(events);
    assert!(
        desktop
            .get_run(GetRunRequest {
                run_id: run_id.clone()
            })
            .await
            .is_err(),
        "Desktop primary cannot inherit cloud application runs"
    );
    // Closing the application-owned SDK lifetime must take its cloud connection
    // offline, even while the connector task and enrollment remain alive.
    desktop.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), connector)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(*online.borrow(), ConnectorStatus::Disconnected);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let fleet: Value = browser_client
                .get(format!("{origin}/api/projects/project-a/nodes"))
                .headers(headers.clone())
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if fleet["nodes"][0]["presence"].is_null() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("application exit removes cloud readiness");
    let retained = wait_task(&browser_client, &origin, &headers, task_id, |_| true).await;
    assert_eq!(retained.run_id.as_deref(), Some(run_id.as_str()));
    enrollment.revoke().await.unwrap();
    enrollment.forget().unwrap();
    stop.send_replace(true);
    host.await.unwrap().unwrap();
    oidc.abort();
}
