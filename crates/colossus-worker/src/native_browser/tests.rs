use super::*;
use colossus_api::{ApiScope, ApplicationKind};
use colossus_browser_presentation::native_admission::client_handshake;

struct Fixture {
    _directory: tempfile::TempDir,
    prepared: PreparedNativeBrowser,
    enrollment: NativeBrowserEnrollment,
    credentials: Arc<PublicApiCredentialManager>,
    credential_id: String,
}
async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let mut config = RuntimeConfig::offline_template(root.join("state.redb"));
    config.workflows.repository = root.join("repository");
    config.workflows.user = root.join("user");
    std::fs::create_dir(&config.workflows.repository).unwrap();
    std::fs::create_dir(&config.workflows.user).unwrap();
    let server = WorkerServer::open_at_workspace(
        &config,
        Arc::new(AllowApproval {
            approved_by: "native-browser-unit".into(),
        }),
        RuntimeOpenOptions::for_workspace(&root).unwrap(),
    )
    .unwrap();
    let credentials =
        Arc::new(server.public_api_credential_manager(PublicApiAuthenticationKey::new([3; 32])));
    let grant = ApplicationGrant::new(
        "app:native-unit",
        ApplicationKind::Sidecar,
        [ApiScope::new("sessions:read").unwrap()],
        Vec::<String>::new(),
        Vec::<String>::new(),
    )
    .unwrap();
    let issued = credentials.issue_pending(&grant).unwrap();
    let credential_id = issued.credential_id().to_owned();
    let enrollment = NativeBrowserEnrollment {
        generation: *Uuid::now_v7().as_bytes(),
        instance: *Uuid::now_v7().as_bytes(),
        application_id: "app:native-unit".into(),
        workspace_version: 1,
        workspace_digest: [5; 32],
        // This in-process transport test exercises actual kernel peer checks, not parent
        // bootstrap attestation; the production constructor rejects this PID as parent.
        parent_process_id: std::process::id(),
    };
    let mut server = server
        .with_native_browser_api(NativeBrowserServerConfig {
            instance: root,
            enrollment: enrollment.clone(),
            authentication: Zeroizing::new([7; 32]),
            primary_credential_id: credential_id.clone(),
            credentials: Arc::clone(&credentials),
        })
        .await
        .unwrap();
    let prepared = server.native_browser.take().unwrap();
    Fixture {
        _directory: directory,
        prepared,
        enrollment,
        credentials,
        credential_id,
    }
}
async fn request(
    enrollment: &NativeBrowserEnrollment,
    key: [u8; 32],
    digest: [u8; 32],
    request: NativeBrowserRequest,
) -> Result<NativeBrowserReply, colossus_browser_presentation::PresentationError> {
    let path = enrollment
        .unix_endpoint(rustix::process::geteuid().as_raw())
        .unwrap();
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|_| colossus_browser_presentation::PresentationError::OutcomeUnknown)?;
    client_handshake(stream, &Zeroizing::new(key), digest)
        .await?
        .request(&request)
        .await
}
#[tokio::test]
async fn native_endpoint_rechecks_primary_activation_and_never_promotes_headless_or_absent_hosts() {
    let fixture = fixture().await;
    let digest = fixture.enrollment.digest().unwrap();
    let service = Arc::clone(&fixture.prepared.service);
    let mut running = fixture.prepared.start();
    assert!(matches!(
        request(
            &fixture.enrollment,
            [7; 32],
            digest,
            NativeBrowserRequest::Probe
        )
        .await
        .unwrap(),
        NativeBrowserReply::Denied
    ));
    fixture
        .credentials
        .activate(&fixture.credential_id)
        .unwrap();
    assert!(matches!(
        request(
            &fixture.enrollment,
            [7; 32],
            digest,
            NativeBrowserRequest::Probe
        )
        .await
        .unwrap(),
        NativeBrowserReply::Unavailable
    ));
    let open = colossus_browser_presentation::native_admission::NativeBrowserOpen {
        conversation_id: None,
        url: colossus_contracts::BrowserUrl::parse("https://example.test").unwrap(),
        width: 800,
        height: 600,
        scale_milli: 1000,
        viewport_generation: 1,
        lease_ms: 1000,
    };
    assert!(matches!(
        request(
            &fixture.enrollment,
            [7; 32],
            digest,
            NativeBrowserRequest::Open(open)
        )
        .await
        .unwrap(),
        NativeBrowserReply::Unavailable
    ));
    assert!(service.sessions.lock().unwrap().is_empty());
    assert_eq!(service.admitted.load(Ordering::Acquire), 0);
    fixture.credentials.revoke(&fixture.credential_id).unwrap();
    assert!(!service.authorized());
    running.shutdown().await.unwrap();
    assert!(
        !fixture
            .enrollment
            .unix_endpoint(rustix::process::geteuid().as_raw())
            .unwrap()
            .exists()
    );
}
#[tokio::test]
async fn foreign_worker_key_or_workspace_cannot_use_the_native_endpoint() {
    let fixture = fixture().await;
    fixture
        .credentials
        .activate(&fixture.credential_id)
        .unwrap();
    let digest = fixture.enrollment.digest().unwrap();
    let mut running = fixture.prepared.start();
    assert!(
        request(
            &fixture.enrollment,
            [8; 32],
            digest,
            NativeBrowserRequest::Probe
        )
        .await
        .is_err()
    );
    let mut foreign = digest;
    foreign[0] ^= 1;
    assert!(
        request(
            &fixture.enrollment,
            [7; 32],
            foreign,
            NativeBrowserRequest::Probe
        )
        .await
        .is_err()
    );
    assert!(matches!(
        request(
            &fixture.enrollment,
            [7; 32],
            digest,
            NativeBrowserRequest::Probe
        )
        .await
        .unwrap(),
        NativeBrowserReply::Unavailable
    ));
    running.shutdown().await.unwrap();
}
#[tokio::test]
async fn dropping_the_native_server_requests_owned_socket_cleanup() {
    let fixture = fixture().await;
    let path = fixture
        .enrollment
        .unix_endpoint(rustix::process::geteuid().as_raw())
        .unwrap();
    let running = fixture.prepared.start();
    assert!(path.exists());
    drop(running);
    tokio::time::timeout(Duration::from_secs(3), async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
