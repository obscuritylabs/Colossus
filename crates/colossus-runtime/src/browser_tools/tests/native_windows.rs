//! Real standard-user Windows CEF execution through Core's effect gateway.
//! Chromium sandbox remains enabled; outer OS egress containment is explicitly absent.
use super::*;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_browser_presentation::{Configure, FrameCodec, Input, PresentationError};
use colossus_ports::{
    BrowserArtifactError, BrowserArtifactPublication, BrowserArtifactPublisher, EventJournal,
};
use colossus_sandbox::{
    BrowserEgressLimits, WindowsBrowserConfig, WindowsBrowserShutdownReceipt,
    WindowsBrowserSupervisor,
};
use sha2::Digest as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
mod pki;
mod rich_interactions;

const PAGE: &str = r#"<!doctype html><html><head><title>Core Windows fixture</title></head>
<body style="background:rgb(231,241,255)"><h1>Core Windows fixture</h1>
<button onclick="document.querySelector('output').textContent='Core click received'">Core click</button>
<output>Waiting for Core</output><script>
function addMetadataFrame(){
  if(!window.startedFrame){
    window.startedFrame=true;
    const frame=document.createElement('iframe');
    frame.hidden=true;frame.src='/metadata-frame';document.body.append(frame);
  }
  const status=document.getElementById('child-frame-result');
  if(status)status.textContent='Background frame added';
}
addEventListener('resize',()=>{if(innerWidth<1000)addMetadataFrame();});
function addInspectionFrame(){
  const status=document.getElementById('child-frame-result');
  status.textContent='Child frame loading';
  const frame=document.createElement('iframe');
  frame.hidden=true;
  frame.onload=()=>{status.textContent='Child frame added';};
  frame.src='about:blank';
  document.body.append(frame);
}
</script>
<button onmouseenter="document.getElementById('hover-result').textContent='Hover received'">Hover target</button><output id="hover-result">Hover waiting</output>
<button ondblclick="document.getElementById('double-result').textContent='Double received'">Double target</button><output id="double-result">Double waiting</output>
<button oncontextmenu="event.preventDefault();document.getElementById('right-result').textContent='Right received'">Context target</button><output id="right-result">Right waiting</output>
<button id="coordinate-button" style="position:absolute;left:100px;top:250px;width:100px;height:40px" onclick="document.getElementById('pixel-result').textContent='Coordinate received'">Coordinate target</button><output id="pixel-result">Coordinate waiting</output>
<div role="button" aria-label="Pointer drag surface" style="position:absolute;left:100px;top:310px;width:150px;height:60px;background:lightblue" onpointerdown="this.setPointerCapture(event.pointerId)" onpointerup="document.getElementById('pointer-result').textContent='Pointer drag received'">Drag canvas</div><output id="pointer-result">Pointer waiting</output>
<div draggable="true" role="button" aria-label="HTML drag source" style="position:absolute;left:300px;top:310px;width:100px;height:60px;background:lightgreen" ondragstart="event.dataTransfer.setData('text/plain','owned-drag')">Drag source</div>
<div role="button" aria-label="HTML drop target" style="position:absolute;left:450px;top:310px;width:120px;height:60px;background:pink" ondragover="event.preventDefault()" ondrop="event.preventDefault();document.getElementById('html-result').textContent=event.dataTransfer.getData('text/plain')==='owned-drag'?'HTML drop received':'HTML drop failed'">Drop target</div><output id="html-result">HTML waiting</output>
<button onclick="addInspectionFrame()">Add child frame</button><output id="child-frame-result">Child frame waiting</output>
<button onclick="document.querySelectorAll('iframe').forEach(frame=>frame.remove())">Remove child frame</button>
</body></html>"#;

const INSPECTION_TIMER: &str = r#"<!doctype html><title>Inspection timer</title>
<h1>Inspection timer</h1><input id="race-field" type="text" value="public" hidden>
<h2 id="race-ticks">Race tick 0</h2>
<output id="race-mode">Race running</output>
<button onclick="clearInterval(window.raceTimer);ordinary();document.getElementById('race-mode').textContent='Frozen ordinary'">Freeze ordinary</button>
<button onclick="clearInterval(window.raceTimer);protectedField();document.getElementById('race-mode').textContent='Frozen protected'">Freeze protected</button>
<button onclick="window.raceTimer=setInterval(tick,5);document.getElementById('race-mode').textContent='Race running'">Resume race</button>
<script>
window.raceTicks=0;
let raceType='text',raceValue='public';
function ordinary(){const field=document.getElementById('race-field');raceValue='public';field.value='public';raceType='text';field.type='text';}
function protectedField(){const field=document.getElementById('race-field');field.type='password';raceType='password';field.value='owned-race-secret';raceValue='owned-race-secret';}
function tick(){
  ++window.raceTicks;
  if(window.raceTicks%2) protectedField(); else ordinary();
  document.getElementById('race-ticks').textContent='Race tick '+window.raceTicks;
}
window.raceTimer=setInterval(tick,5);
</script>"#;
const INSPECTION_SHADOW: &str = r#"<!doctype html><title>Inspection shadow</title>
<h1>Inspection shadow</h1><div id="closed-host"></div><script>
const root=document.getElementById('closed-host').attachShadow({mode:'closed'});
window.closedCredential=document.createElement('input');
window.closedCredential.type='password';window.closedCredential.value='owned-shadow-secret';
root.append(window.closedCredential);
</script>"#;
const INSPECTION_DORMANT: &str =
    "<!doctype html><title>Inspection dormant</title><h1>Inspection dormant</h1>";
const INSPECTION_NAVIGATION: &str = r#"<!doctype html><title>Inspection navigation</title>
<h1>Inspection navigation ready</h1>
<script>setTimeout(()=>location.href='/inspection-dormant',250);</script>"#;

struct CapturedHost {
    supervisor: Arc<WindowsBrowserSupervisor>,
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
        assert_eq!(request.run_id, "native-browser-windows");
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
    let mut timer_state = Vec::new();
    loop {
        let result = tokio::time::timeout_at(
            deadline,
            call(executor, context, "browser.snapshot", target.clone()),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "real Windows AX observation deadline for {text:?}; timer markers: {timer_state:?}"
            )
        });
        let value = match result {
            Ok(value) => value,
            // Only a known no-effect stale result from this read-only snapshot
            // may be retried while a child document settles. Mutations and
            // outcome-unknown failures never pass through this fixture helper.
            Err(ToolError::Failed(message))
                if message == "effect failed: browser document or element is stale" =>
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
                continue;
            }
            Err(error) => panic!("real Windows AX observation of {text:?} failed: {error}"),
        };
        let observation: BrowserObservation = serde_json::from_value(value).unwrap();
        timer_state = observation
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.nodes)
            .filter(|node| {
                node.name == "Race running"
                    || node.name.starts_with("Frozen ")
                    || node.name.starts_with("Race tick ")
                    || node.name.starts_with("Child frame ")
            })
            .take(8)
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
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
            "real Windows page never exposed {text:?}; timer markers: {timer_state:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn fixture() -> (String, tokio::task::JoinHandle<()>, Arc<AtomicBool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let frame_requested = Arc::new(AtomicBool::new(false));
    let pending_frame = Arc::clone(&frame_requested);
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
            let page = if header.starts_with(b"GET /metadata-frame") {
                pending_frame.store(true, Ordering::Release);
                tokio::time::sleep(Duration::from_millis(250)).await;
                "<!doctype html><title>Background frame</title>".into()
            } else if header.starts_with(b"GET /protected") {
                PAGE.replace("</body>","<input type='password' name='password' value='owned-fixture-secret' hidden></body>")
            } else if header.starts_with(b"GET /inspection-timer") {
                INSPECTION_TIMER.into()
            } else if header.starts_with(b"GET /inspection-shadow") {
                INSPECTION_SHADOW.into()
            } else if header.starts_with(b"GET /inspection-dormant") {
                INSPECTION_DORMANT.into()
            } else if header.starts_with(b"GET /inspection-navigation") {
                INSPECTION_NAVIGATION.into()
            } else if header.starts_with(b"GET /second") {
                PAGE.replace("Core Windows fixture", "Core Windows navigation")
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
    (origin, task, frame_requested)
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
            "real Windows CEF page never painted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Operator command stages the exact verified Debug component first.
/// This exercises Core and real CEF, not production PKI or parent-crash acceptance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the explicit ordinary-user Windows CEF stage; operator command in native/browser/README.md"]
async fn real_windows_chromium_executes_core_tools_and_paints_authenticated_frames() {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .expect("explicit verified Windows development component"),
    );
    let parent = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
            .expect("explicit private Windows state parent"),
    );
    let manifest = std::fs::read(component.join("browser-component.json")).unwrap();
    let metadata: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(metadata["modes"]["desktop"], false);
    assert_eq!(metadata["modes"]["headless"], false);
    let mut state = colossus_windows_native::PrivateDirectoryCreation::create(
        &parent.join(format!("windows-core-{}", Uuid::now_v7().simple())),
    )
    .unwrap();
    let allocation = state.path().to_owned();
    let workspace = private_tempdir();
    let (origin, fixture_task, frame_requested) = fixture().await;
    let limits = BrowserLimits {
        max_sessions: 1,
        max_tabs: 1,
        ..BrowserLimits::default()
    };
    let supervisor = Arc::new(
        WindowsBrowserSupervisor::install_for_development(WindowsBrowserConfig {
            component_root: component,
            state_root: allocation.clone(),
            manifest_sha256: sha2::Sha256::digest(&manifest).into(),
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some(metadata["cef_version"].as_str().unwrap().into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Navigate,
                    BrowserActionKind::Click,
                    BrowserActionKind::Screenshot,
                    BrowserActionKind::Evaluate,
                    BrowserActionKind::Cdp,
                    BrowserActionKind::Hover,
                    BrowserActionKind::DoubleClick,
                    BrowserActionKind::RightClick,
                    BrowserActionKind::Drag,
                    BrowserActionKind::ClickAt,
                    BrowserActionKind::DragAt,
                    BrowserActionKind::Annotate,
                    BrowserActionKind::AnnotationClear,
                    BrowserActionKind::AnnotatedScreenshot,
                ],
                limits,
                private_ca_trust: false,
                client_identities: false,
                restrictive_egress: false,
            },
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
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
    assert!(!supervisor.capabilities().restrictive_egress);
    let captured = Arc::new(CapturedHost {
        supervisor: Arc::clone(&supervisor),
        request: StdMutex::new(None),
    });
    let host = RuntimeBrowserHost::from_supervised_factory(captured.clone())
        .unwrap()
        .with_presenter(captured.clone());
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
        let context = begin(&runtime, "native-browser-windows");
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !frame_requested.load(Ordering::Acquire) {
            presenter.renew(lease, 1500).await.unwrap();
            assert!(
                tokio::time::Instant::now() < deadline,
                "native resize did not start the background frame"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let state = presenter.observe(lease).await.unwrap();
        assert_eq!(state.target.document_id, observed.tab.document_id);
        assert!(
            call(
                &executor,
                &context,
                "browser.screenshot",
                observed_target(&opened, &observed)
            )
            .await
            .is_err(),
            "read-only observation refreshed the agent's stale action revision"
        );
        let refreshed = observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core click received",
        )
        .await;
        assert_ne!(refreshed.tab.document_id, observed.tab.document_id);
        let observed = refreshed;
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
        let mut viewport_generation = 3;
        let observed = rich_interactions::inspect_adversarial(
            &executor,
            &context,
            &opened,
            observed,
            &origin,
            &presenter,
            &request,
            digest,
            &mut viewport_generation,
        )
        .await;
        let mut navigate = observed_target(&opened, &observed);
        navigate["url"] = json!(format!("{origin}/"));
        let observed: BrowserObservation = serde_json::from_value(
            call(&executor, &context, "browser.navigate", navigate)
                .await
                .unwrap(),
        )
        .unwrap();
        let observed = observe(
            &executor,
            &context,
            observed_target(&opened, &observed),
            "Core Windows fixture",
        )
        .await;
        rich_interactions::current_fixture_view(
            &presenter,
            &request,
            &opened,
            &observed,
            digest,
            &mut viewport_generation,
        )
        .await;
        let observed = rich_interactions::run(
            &executor,
            &context,
            &opened,
            observed,
            &presenter,
            digest,
            &request,
            &task_publisher,
            &mut viewport_generation,
        )
        .await;
        presenter.disconnect();
        let mut navigate = observed_target(&opened, &observed);
        navigate["url"] = json!(format!("{origin}/second"));
        let navigation = call(&executor, &context, "browser.navigate", navigate)
            .await
            .unwrap();
        let navigated: BrowserObservation = serde_json::from_value(navigation).unwrap();
        let observed = observe(
            &executor,
            &context,
            observed_target(&opened, &navigated),
            "Core Windows navigation",
        )
        .await;
        let mut protected = observed_target(&opened, &observed);
        protected["url"] = json!(format!("{origin}/protected"));
        let protected: BrowserObservation = serde_json::from_value(
            call(&executor, &context, "browser.navigate", protected)
                .await
                .unwrap(),
        )
        .unwrap();
        let protected = observe(
            &executor,
            &context,
            observed_target(&opened, &protected),
            "Core Windows fixture",
        )
        .await;
        let mut inspect = observed_target(&opened, &protected);
        inspect["snapshot_id"] = json!(protected.snapshot.as_ref().unwrap().snapshot_id);
        inspect["expression"] = json!("document.querySelector('input[type=password]').value");
        let error = call(&executor, &context, "browser.evaluate", inspect.clone())
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("owned-fixture-secret"));
        inspect.as_object_mut().unwrap().remove("expression");
        inspect["method"] = json!("DOM.getDocument");
        inspect["params"] = json!({});
        assert!(
            call(&executor, &context, "browser.cdp", inspect)
                .await
                .is_err()
        );
        rich_interactions::inspect_navigation_restoration(
            &executor, &context, &opened, protected, &origin,
        )
        .await;
        runtime
            .browser
            .as_ref()
            .unwrap()
            .finish_run("native-browser-windows")
            .await;
        assert!(!runtime.browser_active_work());
    });
    let result = tokio::time::timeout(Duration::from_secs(90), &mut check).await;
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
    drop(captured);
    drop(supervisor);
    state
        .remove_empty()
        .expect("complete physical Windows Core state cleanup");
    assert!(!allocation.exists());
    match result {
        Ok(Ok(())) => {
            assert_eq!(receipts.len(), 1);
            assert_eq!(
                receipts[0].1,
                Some(WindowsBrowserShutdownReceipt::GracefulCefShutdown)
            );
            if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_SCREENSHOT") {
                std::fs::write(
                    path,
                    publisher.0.lock().unwrap().as_ref().unwrap().as_slice(),
                )
                .unwrap();
            }
            eprintln!(
                "Core Windows development: real page snapshot, native click, screenshot, authenticated Retina frame, viewer detach, navigation and full Job and physical state shutdown passed; production acceptance false"
            );
        }
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        other => panic!("Core Windows development did not finish: {other:?}"),
    }
}
