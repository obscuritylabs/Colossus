//! Explicit developer acceptance of the real contained host through runtime policy.
use super::*;
use colossus_browser_bridge::{BrowserHostFactory, BrowserHostPool};
use colossus_browser_presentation::{Configure, FrameCodec, Input, PresentationError};
use colossus_sandbox::{
    OciBrowserConfig, OciBrowserLimits, OciBrowserShutdownReceipt, OciBrowserSupervisor,
};
use sha2::{Digest as _, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

struct CapturedFactory {
    factory: Arc<OciBrowserSupervisor>,
    request: StdMutex<Option<BrowserDriverOpenRequest>>,
}
#[async_trait::async_trait]
impl BrowserHostFactory for CapturedFactory {
    fn capabilities(&self) -> BrowserCapabilities {
        self.factory.capabilities()
    }
    async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        *self.request.lock().unwrap() = Some(request.clone());
        self.factory.launch(request, control).await
    }
    async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        self.factory.reap_failed_launch(request).await
    }
}

/// Run on a native Linux host with an administrator-selected local Docker daemon.
/// The component is an explicit fresh inventoried debug stage; no system browser fallback.
#[tokio::test]
#[ignore = "requires real CEF component, immutable OCI image and local native Docker"]
async fn contained_readonly_frames_share_runtime_page_without_human_input_authority() {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_OCI_ACCEPTANCE_COMPONENT")
            .expect("select the explicit inventoried native host stage"),
    );
    let image = std::env::var("COLOSSUS_BROWSER_OCI_ACCEPTANCE_IMAGE_ID")
        .expect("select the locally installed immutable image sha256");
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    assert!(manifest.len() <= 8 * 1024 * 1024);
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    let workspace = private_tempdir();
    // Retain the private state if startup or teardown is uncertain. Automatic
    // TempDir cleanup must never erase an unresolved container's ownership record.
    let state = private_tempdir().keep();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&requests);
    let fixture = tokio::spawn(async move {
        const PAGE: &str = include_str!("../../../../../native/browser/tests/host_fixture.html");
        let page = PAGE.replace("<body>", "<body style=\"background:rgb(231,241,255)\">");
        assert_ne!(page, PAGE, "the real fixture must contain its color marker");
        loop {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            let mut buffer = [0; 1024];
            while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                let count =
                    tokio::time::timeout(Duration::from_secs(2), connection.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                if count == 0 {
                    break;
                }
                header.extend_from_slice(&buffer[..count]);
                assert!(header.len() <= 8192);
            }
            observed.fetch_add(1, Ordering::SeqCst);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            connection.write_all(response.as_bytes()).await.unwrap();
            connection.shutdown().await.unwrap();
        }
    });
    let capabilities = BrowserCapabilities {
        available: true,
        engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
        modes: vec![BrowserMode::Headless],
        actions: vec![
            BrowserActionKind::Snapshot,
            BrowserActionKind::Click,
            BrowserActionKind::Fill,
        ],
        limits: BrowserLimits::default(),
        private_ca_trust: false,
        client_identities: false,
        restrictive_egress: true,
    };
    let supervisor = OciBrowserSupervisor::install_for_acceptance(OciBrowserConfig {
        docker: std::env::var_os("COLOSSUS_BROWSER_OCI_ACCEPTANCE_DOCKER")
            .map_or_else(|| PathBuf::from("/usr/bin/docker"), PathBuf::from),
        image,
        component_root: component,
        manifest_sha256: Sha256::digest(&manifest).into(),
        state_root: state.clone(),
        limits: OciBrowserLimits::default(),
        capabilities,
        pki: None,
        pki_enrollment: None,
        profile_store: None,
        presentation: true,
    })
    .await
    .unwrap();
    let supervisor = Arc::new(supervisor);
    let captured = Arc::new(CapturedFactory {
        factory: Arc::clone(&supervisor),
        request: StdMutex::new(None),
    });
    let pool = Arc::new(BrowserHostPool::new(captured.clone()).unwrap());
    let presenter_owner = Arc::clone(&supervisor);
    let mut check = tokio::spawn(async move {
        let host = RuntimeBrowserHost::new(pool.clone());
        let mut config = config(workspace.path());
        config.sandbox.network_destinations = vec![origin.clone()];
        let runtime = Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(workspace.path())
                .unwrap()
                .with_browser_host(host),
        )
        .unwrap();
        let executor = executor(&runtime);
        let context = begin(&runtime, "native-browser-oci");
        assert!(
            call(
                &executor,
                &context,
                "browser.open",
                json!({
                    "mode":"headless", "allowed_origins":["https://unreviewed.invalid"],
                    "initial_url":"https://unreviewed.invalid/"
                })
            )
            .await
            .is_err()
        );
        assert_eq!(pool.owned_session_count().unwrap(), 0);
        let opened = call(
            &executor,
            &context,
            "browser.open",
            json!({
                "mode":"headless", "allowed_origins":[origin],
                "initial_url":format!("{origin}/fixture.html")
            }),
        )
        .await
        .unwrap();
        assert!(runtime.browser_active_work());
        let snapshot =
            super::native::loaded_fixture_snapshot(&executor, &context, &opened, "Ordinary query")
                .await;
        let observation = serde_json::to_string(&snapshot).unwrap();
        assert!(observation.contains("Ordinary query"));
        assert!(!observation.contains("synthetic-protected-value"));
        assert!(requests.load(Ordering::SeqCst) > 0);
        assert_eq!(pool.owned_session_count().unwrap(), 1);
        let request = captured
            .request
            .lock()
            .unwrap()
            .clone()
            .expect("exact admitted private enrollment");
        let mut foreign = request.clone();
        foreign.run_id = Some("foreign-run".into());
        assert!(matches!(
            presenter_owner.presentation(&foreign).await,
            Err(BrowserDriverError::Denied)
        ));
        let (presenter, digest) = presenter_owner.presentation(&request).await.unwrap();
        let observed: BrowserObservation = serde_json::from_value(snapshot).unwrap();
        let generation = opened["control_generation"].as_u64().unwrap();
        assert!(generation > 0, "viewer observes a real agent writer");
        let configure = Configure {
            session: request.session_id.clone(),
            target: BrowserTarget {
                tab_id: observed.tab.tab_id,
                document_id: observed.tab.document_id,
            },
            control_generation: generation,
            viewport_generation: 1,
            width: 800,
            height: 600,
            scale_milli: 1001,
            lease_ms: 1500,
        };
        let current_target = configure.target.clone();
        let lease = presenter.configure(configure.clone()).await.unwrap();
        assert_eq!((lease.pixel_width, lease.pixel_height), (801, 601));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            presenter.renew(lease, 1500).await.unwrap();
            if let Some(bytes) = presenter.next_frame(lease).await.unwrap() {
                let frame = FrameCodec::new(presenter.surface_key(), digest, lease)
                    .unwrap()
                    .decode(bytes)
                    .unwrap();
                // Resize may first publish a blank but authenticated frame. The
                // positive proof is this fixture's actual BGRA background color.
                if frame
                    .pixels
                    .chunks_exact(4)
                    .any(|pixel| pixel == [255, 241, 231, 255])
                {
                    break;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "native page did not paint"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            presenter
                .input(lease, Input::Character { text: "X".into() }, 0)
                .await,
            Err(PresentationError::Hidden)
        );
        assert_eq!(
            presenter.focus(lease, true).await,
            Err(PresentationError::Hidden)
        );
        let mut invented = configure;
        invented.control_generation = 0;
        invented.viewport_generation = 2;
        assert_eq!(
            presenter.configure(invented).await,
            Err(PresentationError::Stale)
        );
        presenter.disconnect();
        // Disconnecting this independent read-only viewer cannot revoke the agent.
        let mut current_snapshot = snapshot_target(&opened);
        current_snapshot["tab_id"] = json!(current_target.tab_id);
        current_snapshot["document_id"] = json!(current_target.document_id);
        call(&executor, &context, "browser.snapshot", current_snapshot)
            .await
            .unwrap();

        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("native-browser-oci")
            .await;
        assert!(!runtime.browser_active_work());
        assert_eq!(pool.owned_session_count().unwrap(), 0);
    });
    let result = tokio::time::timeout(Duration::from_secs(90), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    // A test panic or timeout does not bypass physical cleanup. The supervisor
    // survives the policy task and independently owns every launch obligation.
    let cleanup = tokio::time::timeout(Duration::from_secs(90), supervisor.shutdown()).await;
    let receipts = supervisor.acceptance_launch_stages().await;
    eprintln!("native readonly presentation launch and cleanup: {receipts:?}");
    fixture.abort();
    let fixture_result = fixture.await;
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "native teardown remains unknown; private state retained"
    );
    drop(supervisor);
    std::fs::remove_dir_all(state).unwrap();
    assert!(fixture_result.is_err_and(|error| error.is_cancelled()));
    match result {
        Ok(Ok(())) => {
            assert_eq!(receipts.len(), 1, "one exactly owned native host");
            assert!(
                receipts[0].1,
                "owned process and egress cleanup acknowledged"
            );
            assert_eq!(
                receipts[0].2,
                OciBrowserShutdownReceipt::GracefulCefShutdown
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("native runtime acceptance did not finish: {other:?}"),
    }
}
