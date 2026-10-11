//! Real Mac CEF execution through Core's effect gateway, for synthetic HTTP only.
use super::*;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_browser_presentation::{Configure, FrameCodec, Input, PresentationError};
use colossus_ports::{
    BrowserArtifactError, BrowserArtifactPublication, BrowserArtifactPublisher, EventJournal,
};
use colossus_sandbox::{
    BrowserEgressLimits, MacosBrowserAcceptanceConfig, MacosBrowserShutdownReceipt,
    MacosBrowserSupervisor,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const PAGE: &str = r#"<!doctype html><html><head><title>Core Mac fixture</title></head>
<body style="background:rgb(231,241,255)"><h1>Core Mac fixture</h1>
<button onclick="document.querySelector('output').textContent='Core click received'">Core click</button>
<output>Waiting for Core</output></body></html>"#;

struct CapturedHost {
    supervisor: Arc<MacosBrowserSupervisor>,
    request: StdMutex<Option<BrowserDriverOpenRequest>>,
}

#[async_trait]
impl BrowserHostFactory for CapturedHost {
    fn capabilities(&self) -> BrowserCapabilities {
        self.supervisor.capabilities()
    }
    async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        *self.request.lock().unwrap() = Some(request.clone());
        self.supervisor.launch(request, control).await
    }
    async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        self.supervisor.reap_failed_launch(request).await
    }
}

#[async_trait]
impl RuntimeBrowserPresenter for CapturedHost {
    async fn acquire(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(colossus_browser_presentation::PresentationClient, [u8; 32]), BrowserDriverError>
    {
        self.supervisor.presentation(request).await
    }
}

#[derive(Default)]
struct CapturePublisher(StdMutex<Option<zeroize::Zeroizing<Vec<u8>>>>);

impl std::fmt::Debug for CapturePublisher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CapturePublisher { private_image: <redacted> }")
    }
}

#[async_trait]
impl BrowserArtifactPublisher for CapturePublisher {
    async fn publish(
        &self,
        _: Arc<dyn EventJournal>,
        request: BrowserArtifactPublication,
    ) -> Result<ModelImageReference, BrowserArtifactError> {
        assert!(request.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(request.run_id, "native-browser-macos");
        assert!(request.descriptor.width > 0 && request.descriptor.height > 0);
        let image = ModelImageReference {
            artifact_id: format!("artifact-{}", request.descriptor.sha256),
            file_name: "mac-core-browser.png".into(),
            media_type: "image/png".into(),
            size_bytes: request.bytes.len() as u64,
            sha256: request.descriptor.sha256,
            width_pixels: request.descriptor.width,
            height_pixels: request.descriptor.height,
            detail: ModelImageDetail::Auto,
        };
        *self.0.lock().unwrap() = Some(request.bytes);
        Ok(image)
    }
}

fn digest<const N: usize>(name: &str) -> [u8; N] {
    hex::decode(std::env::var(name).expect("native launcher must supply verified stage identities"))
        .unwrap()
        .try_into()
        .unwrap()
}

fn observed_target(opened: &Value, observation: &BrowserObservation) -> Value {
    json!({"session_id":observation.session_id, "control_generation":opened["control_generation"],
        "tab_id":observation.tab.tab_id, "document_id":observation.tab.document_id})
}

async fn observe(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    mut target: Value,
    text: &str,
) -> BrowserObservation {
    target["max_nodes"] = json!(128);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let value = tokio::time::timeout_at(
            deadline,
            call(executor, context, "browser.snapshot", target.clone()),
        )
        .await
        .expect("real Mac AX observation deadline")
        .unwrap();
        let observation: BrowserObservation = serde_json::from_value(value).unwrap();
        if observation
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.nodes.iter().any(|node| node.name.contains(text)))
        {
            return observation;
        }
        target["tab_id"] = json!(observation.tab.tab_id);
        target["document_id"] = json!(observation.tab.document_id);
        assert!(
            tokio::time::Instant::now() < deadline,
            "real Mac page never exposed the fixture text"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn fixture() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            let mut block = [0; 1024];
            while !header.windows(4).any(|part| part == b"\r\n\r\n") {
                let count =
                    tokio::time::timeout(Duration::from_secs(2), connection.read(&mut block))
                        .await
                        .unwrap()
                        .unwrap();
                if count == 0 {
                    break;
                }
                header.extend_from_slice(&block[..count]);
                assert!(header.len() <= 8192);
            }
            let page = if header.starts_with(b"GET /second") {
                PAGE.replace("Core Mac fixture", "Core Mac navigation")
            } else {
                PAGE.into()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            connection.write_all(response.as_bytes()).await.unwrap();
            connection.shutdown().await.unwrap();
        }
    });
    (origin, task)
}

async fn painted(
    presenter: &colossus_browser_presentation::PresentationClient,
    digest: [u8; 32],
    lease: colossus_browser_presentation::Lease,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut codec = FrameCodec::new(presenter.surface_key(), digest, lease).unwrap();
    loop {
        presenter.renew(lease, 1500).await.unwrap();
        if let Some(bytes) = presenter.next_frame(lease).await.unwrap() {
            let frame = codec.decode(bytes).unwrap();
            if frame
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 241, 231, 255])
            {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "real Mac CEF page never painted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Operator command stages the exact signed Debug main/helper envelope first.
/// This exercises Core and real CEF, not production PKI or parent-crash acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the explicit signed synthetic-profile Mac CEF stage; operator command in native/browser/README.md"]
async fn real_mac_chromium_executes_core_tools_and_paints_authenticated_frames() {
    let app = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_MACOS_ACCEPTANCE_APP")
            .expect("explicit signed diagnostic app"),
    );
    let allocation = app.parent().unwrap().to_path_buf();
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(app.with_file_name(format!(
            "{}.browser-component.json",
            app.file_name().unwrap().to_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    let workspace = private_tempdir();
    let (origin, fixture_task) = fixture().await;
    let mut limits = BrowserLimits::default();
    limits.max_sessions = 1;
    limits.max_tabs = 1;
    let supervisor = Arc::new(
        MacosBrowserSupervisor::install_for_acceptance(MacosBrowserAcceptanceConfig {
            allocation_root: allocation.clone(),
            app_bundle: app,
            manifest_sha256: digest("COLOSSUS_BROWSER_MACOS_MANIFEST_SHA256"),
            front_cdhash: digest("COLOSSUS_BROWSER_MACOS_FRONT_CDHASH"),
            body_cdhash: digest("COLOSSUS_BROWSER_MACOS_BODY_CDHASH"),
            policy_sha256: digest("COLOSSUS_BROWSER_MACOS_POLICY_SHA256"),
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Navigate,
                    BrowserActionKind::Click,
                    BrowserActionKind::Screenshot,
                ],
                limits,
                private_ca_trust: false,
                client_identities: false,
                restrictive_egress: true,
            },
            egress: BrowserEgressLimits {
                max_connections: 4,
                lifetime: Duration::from_secs(60),
                connection_timeout: Duration::from_secs(10),
                max_connection_bytes: 1024 * 1024,
            },
        })
        .await
        .unwrap(),
    );
    assert!(
        !supervisor.readiness().is_available(),
        "diagnostic launch cannot promote production readiness"
    );
    let captured = Arc::new(CapturedHost {
        supervisor: Arc::clone(&supervisor),
        request: StdMutex::new(None),
    });
    let (host, retained) =
        RuntimeBrowserHost::macos_supervised_for_acceptance(Arc::clone(&captured)).unwrap();
    let host = host.with_native_profile_protection(vec![
        colossus_home::ConfinedRoot::bind(&allocation).unwrap(),
    ]);
    let mut configuration = config(workspace.path());
    configuration.sandbox.network_destinations = vec![origin.clone()];
    let runtime = Runtime::open_with_options(
        &configuration,
        Arc::new(DenyApproval),
        None,
        RuntimeOpenOptions::for_workspace(workspace.path())
            .unwrap()
            .with_browser_host(host),
    )
    .unwrap();
    let publisher = Arc::new(CapturePublisher::default());
    runtime.install_browser_artifact_publisher(publisher.clone());
    let task_supervisor = Arc::clone(&supervisor);
    let task_captured = Arc::clone(&captured);
    let task_publisher = Arc::clone(&publisher);
    let mut check = tokio::spawn(async move {
        let executor = executor(&runtime);
        let context = begin(&runtime, "native-browser-macos");
        assert!(call(&executor, &context, "browser.open", json!({"mode":"embedded",
            "allowed_origins":["https://unreviewed.invalid"], "initial_url":"https://unreviewed.invalid/"})).await.is_err());
        assert!(task_captured.request.lock().unwrap().is_none());
        let opened = call(
            &executor,
            &context,
            "browser.open",
            json!({"mode":"embedded",
            "allowed_origins":[origin], "initial_url":format!("{origin}/fixture")}),
        )
        .await
        .unwrap();
        let observed = observe(&executor, &context, snapshot_target(&opened), "Core click").await;
        let node = observed
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.name == "Core click")
            .unwrap();
        let mut click = observed_target(&opened, &observed);
        click["snapshot_id"] = json!(node.element.snapshot_id);
        click["element_id"] = json!(node.element.element_id);
        call(&executor, &context, "browser.click", click)
            .await
            .unwrap();
        let observed = observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core click received",
        )
        .await;
        let request = task_captured.request.lock().unwrap().clone().unwrap();
        let (presenter, digest) = task_supervisor.presentation(&request).await.unwrap();
        let lease = presenter
            .configure(Configure {
                session: request.session_id.clone(),
                target: BrowserTarget {
                    tab_id: observed.tab.tab_id.clone(),
                    document_id: observed.tab.document_id.clone(),
                },
                control_generation: opened["control_generation"].as_u64().unwrap(),
                viewport_generation: 1,
                width: 800,
                height: 600,
                scale_milli: 1001,
                lease_ms: 1500,
            })
            .await
            .unwrap();
        assert_eq!((lease.pixel_width, lease.pixel_height), (801, 601));
        painted(&presenter, digest, lease).await;
        let retina_lease = presenter
            .configure(Configure {
                session: request.session_id.clone(),
                target: BrowserTarget {
                    tab_id: observed.tab.tab_id.clone(),
                    document_id: observed.tab.document_id.clone(),
                },
                control_generation: opened["control_generation"].as_u64().unwrap(),
                viewport_generation: 2,
                width: 640,
                height: 480,
                scale_milli: 2000,
                lease_ms: 1500,
            })
            .await
            .unwrap();
        assert_eq!(
            (retina_lease.pixel_width, retina_lease.pixel_height),
            (1280, 960)
        );
        painted(&presenter, digest, retina_lease).await;
        assert_eq!(
            presenter
                .input(retina_lease, Input::Character { text: "X".into() }, 0)
                .await,
            Err(PresentationError::Hidden)
        );
        call(
            &executor,
            &context,
            "browser.screenshot",
            observed_target(&opened, &observed),
        )
        .await
        .unwrap();
        {
            let screenshot = task_publisher.0.lock().unwrap();
            let image = image::load_from_memory_with_format(
                screenshot.as_ref().unwrap().as_slice(),
                image::ImageFormat::Png,
            )
            .unwrap()
            .to_rgba8();
            assert_eq!(image.dimensions(), (1280, 960));
            assert_eq!(image.get_pixel(640, 480).0, [231, 241, 255, 255]);
        }
        presenter.disconnect();
        let mut navigate = observed_target(&opened, &observed);
        navigate["url"] = json!(format!("{origin}/second"));
        let navigation = call(&executor, &context, "browser.navigate", navigate)
            .await
            .unwrap();
        let navigated: BrowserObservation = serde_json::from_value(navigation).unwrap();
        observe(
            &executor,
            &context,
            observed_target(&opened, &navigated),
            "Core Mac navigation",
        )
        .await;
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("native-browser-macos")
            .await;
        assert!(!runtime.browser_active_work());
    });
    let result = tokio::time::timeout(Duration::from_secs(60), &mut check).await;
    if result.is_err() {
        check.abort();
        let _ = check.await;
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(20), supervisor.shutdown()).await;
    let receipts = supervisor.shutdown_receipts().await;
    fixture_task.abort();
    assert!(
        matches!(cleanup, Ok(Ok(()))),
        "diagnostic native cleanup unknown; allocation retained"
    );
    drop(retained);
    match result {
        Ok(Ok(())) => {
            assert_eq!(receipts.len(), 1);
            assert_eq!(
                receipts[0].1,
                Some(MacosBrowserShutdownReceipt::GracefulDirectChildExit)
            );
            if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_MACOS_ACCEPTANCE_SCREENSHOT") {
                std::fs::write(
                    path,
                    publisher.0.lock().unwrap().as_ref().unwrap().as_slice(),
                )
                .unwrap();
            }
            eprintln!(
                "Core Mac diagnostic: real page snapshot, native click, screenshot, authenticated Retina frame, viewer detach, navigation and direct-child shutdown passed; production acceptance false"
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("Core Mac diagnostic did not finish: {other:?}"),
    }
}
