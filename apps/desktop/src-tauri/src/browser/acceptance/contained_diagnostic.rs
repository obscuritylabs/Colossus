//! Debug-only, real Core -> managed sidecar -> contained Mac Desktop acceptance.
//! The fixture is bound to one compiler-sealed loopback origin and grants only
//! the browser actions exercised here. A failed assertion never reports a pass.

use std::{
    io::Write as _,
    net::{Ipv4Addr, SocketAddrV4},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    bundle::VerifiedBundle, desktop_settings::validate_workspace, native_browser_authentication,
    state::AppState,
};
use anyhow::{Context as _, ensure};
use colossus_contracts::{BrowserDocumentId, BrowserTabId, BrowserTarget};
use colossus_native_browser::{
    BrowserError, BrowserView,
    remote::{RemoteAcceptanceProbe, RemoteSurface},
};
use colossus_sdk::{
    ApiMajor, ApiScope, AppPrivateInstanceDir, CancelRunRequest, Colossus, CreateRunRequest,
    GetRunRequest, IdempotencyKey, InputContentPart, InstanceId, ManagedAccessProfile,
    ManagedExecutionBoundary, ManagedFieldOverride, ManagedProviderKind, ManagedRuntimeConfig,
    NativeSidecarLifecycle, RunMode, RunStatus, SidecarApplicationGrant, SidecarBootstrapConfig,
    SidecarOptions, scopes,
};
use serde_json::{Value, json};
use tauri::Manager as _;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::JoinHandle,
};

use super::{
    super::dto::{BrowserAction, BrowserRect, BrowserViewportRequest},
    action,
};

const SPACE: &str = "mac-contained-acceptance";
const TOOLS: [&str; 7] = [
    "browser.open",
    "browser.status",
    "browser.snapshot",
    "browser.click",
    "browser.navigate",
    "browser.screenshot",
    "browser.close",
];
const MODEL_TIMEOUT: Duration = Duration::from_secs(20);
const VISUAL_MODEL_TIMEOUT: Duration = Duration::from_secs(90);
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_VISUAL_HOLD_MS: u64 = 60_000;

struct Fixture {
    origin: String,
    task: JoinHandle<()>,
    state: Arc<FixtureState>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct FixtureState {
    model_timeout: Duration,
    phase: AtomicUsize,
    model_seen: AtomicBool,
    handoff_done: AtomicBool,
    click_observed: AtomicBool,
    navigation_observed: AtomicBool,
    screenshot_seen: AtomicBool,
    viewer_detached: AtomicBool,
    changed: Notify,
    target: Mutex<Option<Value>>,
    navigated_target: Mutex<Option<BrowserTarget>>,
    run_id: Mutex<Option<String>>,
    failure: Mutex<Option<String>>,
}

impl FixtureState {
    async fn wait(&self, flag: &AtomicBool, what: &str) -> anyhow::Result<()> {
        tokio::time::timeout(self.model_timeout, async {
            loop {
                let notified = self.changed.notified();
                self.check()?;
                if flag.load(Ordering::Acquire) {
                    return Ok::<(), anyhow::Error>(());
                }
                notified.await;
            }
        })
        .await
        .with_context(|| format!("timed out waiting for {what}"))??;
        Ok(())
    }

    fn signal(&self, flag: &AtomicBool) {
        flag.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }

    fn fail(&self, message: String) {
        if let Ok(mut failure) = self.failure.lock() {
            failure.get_or_insert(message);
        }
        self.changed.notify_waiters();
    }

    fn check(&self) -> anyhow::Result<()> {
        if let Some(failure) = self.failure.lock().unwrap().as_deref() {
            anyhow::bail!("deterministic provider fixture failed: {failure}");
        }
        Ok(())
    }

    async fn navigated_target(&self) -> anyhow::Result<BrowserTarget> {
        tokio::time::timeout(self.model_timeout, async {
            loop {
                let notified = self.changed.notified();
                self.check()?;
                if let Some(target) = self.navigated_target.lock().unwrap().clone() {
                    return Ok::<BrowserTarget, anyhow::Error>(target);
                }
                notified.await;
            }
        })
        .await
        .context("timed out waiting for Core navigation target")?
    }
}

impl Fixture {
    async fn bind(address: &str, visual_hold: Duration) -> anyhow::Result<Self> {
        let url = tauri::Url::parse(address)?;
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "diagnostic fixture requires an exact loopback origin"
        );
        let port = url
            .port()
            .context("diagnostic fixture requires a fixed port")?;
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
        let origin = url.origin().ascii_serialization();
        let state = Arc::new(FixtureState {
            model_timeout: if visual_hold.is_zero() {
                MODEL_TIMEOUT
            } else {
                VISUAL_MODEL_TIMEOUT
            },
            ..FixtureState::default()
        });
        let server_state = Arc::clone(&state);
        let server_origin = origin.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    break;
                };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let state = Arc::clone(&server_state);
                let origin = server_origin.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve(stream, &origin, &state).await {
                        state.fail(format!("{error:#}"));
                    }
                });
            }
        });
        Ok(Self {
            origin,
            task,
            state,
        })
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> anyhow::Result<Option<Request>> {
    let mut bytes = Vec::new();
    let mut block = [0_u8; 4096];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        ensure!(bytes.len() < 16 * 1024, "fixture headers exceeded limit");
        let count =
            match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut block)).await {
                Ok(result) => result?,
                Err(_) if bytes.is_empty() => return Ok(None),
                Err(error) => return Err(error.into()),
            };
        // Chromium may open an idle speculative connection and close it without
        // sending a request. That is not a fixture or browser failure.
        if count == 0 && bytes.is_empty() {
            return Ok(None);
        }
        ensure!(count > 0, "fixture connection ended inside headers");
        bytes.extend_from_slice(&block[..count]);
    };
    let header = std::str::from_utf8(&bytes[..header_end])?;
    let mut lines = header.split("\r\n");
    let line = lines.next().context("missing fixture request line")?;
    let mut first = line.split_ascii_whitespace();
    let method = first.next().context("missing fixture method")?.to_owned();
    let path = first.next().context("missing fixture path")?.to_owned();
    ensure!(
        first.next() == Some("HTTP/1.1"),
        "unexpected fixture HTTP version"
    );
    let mut content_length = 0_usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse()?;
        }
    }
    ensure!(content_length <= 1024 * 1024, "fixture body exceeded limit");
    let total = header_end + content_length;
    while bytes.len() < total {
        let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut block)).await??;
        ensure!(count > 0, "fixture connection ended inside body");
        bytes.extend_from_slice(&block[..count]);
        ensure!(
            bytes.len() <= total + 4096,
            "fixture request exceeded length"
        );
    }
    Ok(Some(Request {
        method,
        path,
        body: bytes[header_end..total].to_vec(),
    }))
}

async fn respond(stream: &mut TcpStream, code: u16, kind: &str, body: &[u8]) -> anyhow::Result<()> {
    let header = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        if code == 200 { "OK" } else { "Error" },
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn serve(
    mut stream: TcpStream,
    origin: &str,
    state: &Arc<FixtureState>,
) -> anyhow::Result<()> {
    let Some(request) = read_request(&mut stream).await? else {
        return Ok(());
    };
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/first") => {
            let page = br#"<!doctype html><html><head><title>Human fixture</title></head><body style="background:#e7f1ff"><h1>Human fixture</h1><button onclick="document.title='Agent click received';document.querySelector('output').textContent='Agent clicked'">Agent click</button><output>Waiting for Core</output></body></html>"#;
            respond(&mut stream, 200, "text/html; charset=utf-8", page).await?;
        }
        ("GET", "/agent") => {
            let page = br#"<!doctype html><html><head><title>Agent controlled page</title></head><body style="background:#d9f7d8"><h1>Agent controlled page</h1><p>Core navigated this page after native handoff.</p></body></html>"#;
            respond(&mut stream, 200, "text/html; charset=utf-8", page).await?;
        }
        ("GET", "/favicon.ico") => {
            respond(&mut stream, 200, "image/x-icon", &[]).await?;
        }
        ("POST", "/v1/chat/completions") => {
            let body: Value = serde_json::from_slice(&request.body)?;
            ensure!(
                body["stream"] == false,
                "fixture requires non-streaming model"
            );
            let phase = state.phase.fetch_add(1, Ordering::AcqRel);
            ensure!(phase <= 6, "unexpected extra model turn {phase}");
            state.signal(&state.model_seen);
            if phase == 0 {
                state
                    .wait(&state.handoff_done, "native human-to-agent handoff")
                    .await?;
            }
            if phase == 3 {
                state
                    .wait(&state.click_observed, "visible Core click")
                    .await?;
            }
            if phase == 4 {
                let result = tool_result(&body, "fixture-3")?;
                let target = browser_target(&result["tab"])?;
                *state.navigated_target.lock().unwrap() = Some(target);
                state.changed.notify_waiters();
                state
                    .wait(&state.navigation_observed, "visible Core navigation")
                    .await?;
            }
            let response = model_response(phase, &body, origin, state).await?;
            let encoded = serde_json::to_vec(&response)?;
            respond(&mut stream, 200, "application/json", &encoded).await?;
        }
        _ => {
            respond(&mut stream, 404, "text/plain", b"fixture route unavailable").await?;
        }
    }
    Ok(())
}

fn tool_result(body: &Value, call_id: &str) -> anyhow::Result<Value> {
    let messages = body["messages"]
        .as_array()
        .context("model messages missing")?;
    let message = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
        .with_context(|| format!("Core did not return {call_id} tool evidence"))?;
    let content = message["content"]
        .as_str()
        .context("tool evidence is not scalar text")?;
    serde_json::from_str(content).context("tool evidence is not JSON")
}

fn browser_target(tab: &Value) -> anyhow::Result<BrowserTarget> {
    Ok(BrowserTarget {
        tab_id: BrowserTabId::parse(
            tab["tab_id"]
                .as_str()
                .context("Core tab identity missing")?,
        )?,
        document_id: BrowserDocumentId::parse(
            tab["document_id"]
                .as_str()
                .context("Core document identity missing")?,
        )?,
    })
}

fn alias(body: &Value, canonical: &str) -> anyhow::Result<String> {
    let tools = body["tools"].as_array().context("model tools missing")?;
    let expected = canonical.replace('.', "_");
    tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .find(|name| *name == canonical || *name == expected)
        .map(str::to_owned)
        .with_context(|| format!("Core did not offer {canonical}"))
}

fn model_tool(phase: usize, name: &str, arguments: &Value) -> Value {
    json!({
        "id": format!("fixture-call-{phase}"),
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("fixture-{phase}"),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })
}

// Keep the deterministic Core tool transcript in one ordered source fixture.
#[allow(clippy::too_many_lines)]
async fn model_response(
    phase: usize,
    body: &Value,
    origin: &str,
    state: &FixtureState,
) -> anyhow::Result<Value> {
    let session = state.target.lock().unwrap().clone();
    match phase {
        0 => {
            ensure!(
                alias(body, "browser.open").is_ok(),
                "registered Core run did not offer browser.open for native handoff"
            );
            let id = session
                .as_ref()
                .and_then(|target| target["session_id"].as_str())
                .context("native session was not installed before model release")?;
            Ok(model_tool(
                phase,
                &alias(body, "browser.status")?,
                &json!({"session_id": id}),
            ))
        }
        1 => {
            let status = tool_result(body, "fixture-0")?;
            let tab = status["tabs"]
                .as_array()
                .and_then(|tabs| tabs.first())
                .context("Core browser.status lacked the owned tab")?;
            let target = json!({
                "session_id": status["session_id"],
                "control_generation": status["control_generation"],
                "tab_id": tab["tab_id"],
                "document_id": tab["document_id"]
            });
            let expected_session = session
                .as_ref()
                .and_then(|target| target["session_id"].as_str())
                .context("missing owned session at Core status")?;
            ensure!(
                target["session_id"] == expected_session
                    && target["control_generation"]
                        .as_u64()
                        .is_some_and(|value| value > 0),
                "Core status did not describe the handed-off agent page"
            );
            *state.target.lock().unwrap() = Some(target.clone());
            let mut args = target;
            args["max_nodes"] = json!(128);
            Ok(model_tool(phase, &alias(body, "browser.snapshot")?, &args))
        }
        2 => {
            let observation = tool_result(body, "fixture-1")?;
            let node = observation["snapshot"]["nodes"]
                .as_array()
                .and_then(|nodes| nodes.iter().find(|node| node["name"] == "Agent click"))
                .context("Core AX snapshot did not expose the click target")?;
            let mut args = session.context("missing owned target")?;
            args["tab_id"] = observation["tab"]["tab_id"].clone();
            args["document_id"] = observation["tab"]["document_id"].clone();
            args["snapshot_id"] = node["element"]["snapshot_id"].clone();
            args["element_id"] = node["element"]["element_id"].clone();
            Ok(model_tool(phase, &alias(body, "browser.click")?, &args))
        }
        3 => {
            let clicked = tool_result(body, "fixture-2")?;
            ensure!(
                clicked["tab"]["tab_id"].is_string(),
                "Core click did not return a browser observation"
            );
            let mut args = session.context("missing owned target")?;
            args["tab_id"] = clicked["tab"]["tab_id"].clone();
            args["document_id"] = clicked["tab"]["document_id"].clone();
            args["url"] = json!(format!("{origin}/agent"));
            Ok(model_tool(phase, &alias(body, "browser.navigate")?, &args))
        }
        4 => {
            let navigated = tool_result(body, "fixture-3")?;
            ensure!(
                navigated["tab"]["origin"].is_object() || navigated["tab"]["origin"].is_string(),
                "Core navigate did not return an owned browser observation"
            );
            let mut args = session.context("missing owned target")?;
            args["tab_id"] = navigated["tab"]["tab_id"].clone();
            args["document_id"] = navigated["tab"]["document_id"].clone();
            Ok(model_tool(
                phase,
                &alias(body, "browser.screenshot")?,
                &args,
            ))
        }
        5 => {
            let screenshot = tool_result(body, "fixture-4")?;
            ensure!(
                screenshot["artifact"]["media_type"] == "image/png"
                    && screenshot["artifact"]["width_pixels"]
                        .as_u64()
                        .is_some_and(|width| width > 0)
                    && screenshot["artifact"]["height_pixels"]
                        .as_u64()
                        .is_some_and(|height| height > 0)
                    && screenshot["artifact"]["sha256"]
                        .as_str()
                        .is_some_and(|hash| hash.len() == 64),
                "Core screenshot did not publish verified PNG artifact metadata"
            );
            state.signal(&state.screenshot_seen);
            state
                .wait(&state.viewer_detached, "native viewer detachment")
                .await?;
            let target = session.context("missing owned target for Core close")?;
            Ok(model_tool(
                phase,
                &alias(body, "browser.close")?,
                &json!({
                    "session_id": target["session_id"],
                    "control_generation": target["control_generation"]
                }),
            ))
        }
        6 => {
            let closed = tool_result(body, "fixture-5")?;
            let expected_session = session
                .as_ref()
                .and_then(|target| target["session_id"].as_str())
                .context("missing owned session at Core close confirmation")?;
            ensure!(
                closed["closed"] == true && closed["session_id"] == expected_session,
                "Core browser.close did not confirm the owned session"
            );
            Ok(json!({
                "id": "fixture-complete",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Mac contained browser diagnostic complete."},
                    "finish_reason": "stop"
                }]
            }))
        }
        _ => anyhow::bail!("unexpected fixture phase"),
    }
}

fn private_directory(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

fn managed_runtime(origin: &str, visual_hold: Duration) -> ManagedRuntimeConfig {
    let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Pinned)
        .with_execution_boundary(ManagedExecutionBoundary::OfflineIsolated);
    runtime.providers[0].kind = ManagedProviderKind::OpenAiCompatible;
    runtime.providers[0].base_url = Some(format!("{origin}/v1"));
    runtime.providers[0].timeout_ms = if visual_hold.is_zero() {
        30_000
    } else {
        90_000
    };
    runtime.models[0].model = "mac-browser-fixture".into();
    runtime.models[0].capabilities.streaming = false.into();
    runtime.field_overrides = vec![
        ManagedFieldOverride {
            field_id: "access.tools.include".into(),
            value: json!(TOOLS),
        },
        ManagedFieldOverride {
            field_id: "access.actions.allow".into(),
            value: json!([
                "provider.call",
                "provider.openai.chat",
                "browser.open",
                "browser.status",
                "browser.snapshot",
                "browser.click",
                "browser.navigate",
                "browser.screenshot",
                "browser.close"
            ]),
        },
        ManagedFieldOverride {
            field_id: "sandbox.networkDestinations".into(),
            value: json!([origin]),
        },
    ];
    runtime
}

fn grant() -> anyhow::Result<SidecarApplicationGrant> {
    let scopes = [scopes::RUNS_EXECUTE, scopes::RUNS_READ]
        .into_iter()
        .map(ApiScope::new)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SidecarApplicationGrant::new(
        "app:mac-contained-browser-diagnostic",
        scopes,
        ["primary".into()],
        TOOLS.into_iter().map(str::to_owned),
    )?)
}

async fn start_sidecar(
    home: &Path,
    origin: &str,
    visual_hold: Duration,
) -> anyhow::Result<(Colossus, colossus_sdk::NativeBrowserClient)> {
    let workspace_path = home.join("workspace");
    let instance_path = home.join("instance");
    let runtime_home = home.join("colossus-home");
    private_directory(&workspace_path)?;
    private_directory(&instance_path)?;
    private_directory(&runtime_home)?;
    let workspace =
        validate_workspace(&workspace_path).map_err(|error| anyhow::anyhow!(error.message))?;
    let bundle = VerifiedBundle::load().map_err(|error| anyhow::anyhow!(error.message))?;
    let options = SidecarOptions::new(
        InstanceId::from_uuid(uuid::Uuid::now_v7()),
        AppPrivateInstanceDir::new(instance_path)?,
        bundle.sidecar,
        ApiMajor::new(1)?,
    )?;
    let authentication = native_browser_authentication::generate()?;
    let bootstrap = SidecarBootstrapConfig::new(
        workspace.path,
        managed_runtime(origin, visual_hold),
        grant()?,
    )?
    .with_expected_workspace_identity(
        workspace
            .identity
            .context("private workspace identity missing")?,
    )?
    .with_colossus_home(runtime_home)?
    .with_plaintext_journal_for_development()
    .without_automatic_agent_instructions_for_diagnostics()
    .with_native_browser_authentication(authentication)?;
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let native = lifecycle
        .native_browser_client(&options)?
        .context("diagnostic sidecar did not enroll native browser client")?;
    let client = Colossus::start_sidecar(&lifecycle, options).await?;
    let availability = native.available().await;
    if !matches!(availability, Ok(true)) {
        // Once the authenticated child has started, an admission failure must
        // await its shutdown. A successful SDK close proves sidecar exit here;
        // no browser session exists from which to infer a direct-child receipt.
        let close = client.close().await;
        eprintln!(
            "EVIDENCE contained diagnostic bootstrap cleanup SDKClose={}",
            if close.is_ok() {
                "verifiedSidecarExit"
            } else {
                "unknown"
            }
        );
        ensure!(availability?, "Core did not admit Embedded browser");
    }
    Ok((client, native))
}

fn bounds(app: &tauri::AppHandle) -> anyhow::Result<BrowserRect> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    ensure!(
        size.width >= 300.0 && size.height >= 300.0,
        "Desktop window too small"
    );
    Ok(BrowserRect {
        x: 24.0,
        y: 96.0,
        width: size.width - 48.0,
        height: size.height - 120.0,
    })
}

async fn present(app: &tauri::AppHandle, id: &str, rect: BrowserRect) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let generation = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .generation;
    state
        .browser
        .viewport(
            app,
            &BrowserViewportRequest {
                generation,
                tab_id: Some(id.to_owned()),
                rect: Some(rect),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

fn remote(app: &tauri::AppHandle, id: &str) -> anyhow::Result<RemoteSurface> {
    let (view, _) = app
        .state::<AppState>()
        .browser
        .tab(id, SPACE)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    match view {
        BrowserView::Remote(surface) => Ok(surface),
        _ => anyhow::bail!("Desktop diagnostic did not attach a RemoteSurface"),
    }
}

async fn title(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    expected: &str,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        present(app, id, rect).await?;
        if let Ok(page) = view.inspect().await
            && page.title == expected
            && !page.loading
        {
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "native RemoteSurface did not show title {expected:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ExpectedFrame<'a> {
    title: &'a str,
    url: &'a str,
    center_bgra: [u8; 4],
    core_target: Option<&'a BrowserTarget>,
    previous: Option<&'a RemoteAcceptanceProbe>,
}

async fn accepted_frame(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    expected: ExpectedFrame<'_>,
) -> anyhow::Result<RemoteAcceptanceProbe> {
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        present(app, id, rect).await?;
        let probe = match view.acceptance_probe().await {
            Ok(probe) => Some(probe),
            Err(BrowserError::Stale | BrowserError::Unavailable) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(probe) = probe {
            let target_matches = expected
                .core_target
                .is_none_or(|target| probe.page.target == *target);
            if let Some(frame) = probe.frame {
                let new_document = expected.previous.is_none_or(|previous| {
                    previous.frame.is_some_and(|prior| {
                        probe.page.target.document_id != previous.page.target.document_id
                            && frame.lease.document_generation > prior.lease.document_generation
                            && frame.lease.viewport_generation > prior.lease.viewport_generation
                            && frame.lease.control_generation > prior.lease.control_generation
                    })
                });
                if probe.visible
                    && probe.width > 0
                    && probe.height > 0
                    && !probe.page.loading
                    && probe.page.title == expected.title
                    && probe.page.url == expected.url
                    && target_matches
                    && new_document
                    && frame.sequence > 0
                    && frame.count > 0
                    && frame.center_bgra == expected.center_bgra
                {
                    return Ok(probe);
                }
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "native NSView did not accept an authenticated frame for the expected current document and fixture color"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn visual_hold_duration() -> anyhow::Result<Duration> {
    const NAME: &str = "COLOSSUS_BROWSER_ACCEPTANCE_VISUAL_HOLD_MS";
    let Some(raw) = std::env::var_os(NAME) else {
        return Ok(Duration::ZERO);
    };
    let text = raw
        .to_str()
        .context("visual hold must be an ASCII decimal millisecond count")?;
    ensure!(
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()),
        "{NAME} must be decimal milliseconds from 0 through {MAX_VISUAL_HOLD_MS}"
    );
    let milliseconds: u64 = text
        .parse()
        .context("visual hold milliseconds overflowed")?;
    ensure!(
        milliseconds <= MAX_VISUAL_HOLD_MS,
        "{NAME} must be at most {MAX_VISUAL_HOLD_MS}"
    );
    Ok(Duration::from_millis(milliseconds))
}

async fn hold_visual_checkpoint(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    core_target: &BrowserTarget,
    agent_url: &str,
    hold: Duration,
) -> anyhow::Result<()> {
    if hold.is_zero() {
        return Ok(());
    }
    println!(
        "VISUAL_CHECKPOINT authenticated green agent page submitted to Desktop NSView; hold_ms={}",
        hold.as_millis()
    );
    std::io::stdout().flush()?;
    let deadline = tokio::time::Instant::now() + hold;
    loop {
        // The native view's heartbeat is intentionally short. Continue the
        // ordinary viewport path while the Mac foreground screenshot is taken.
        present(app, tab, rect).await?;
        let probe = view.acceptance_probe().await?;
        let frame = probe.frame.context("visual checkpoint lost native frame")?;
        ensure!(
            probe.visible
                && probe.page.target == *core_target
                && probe.page.url == agent_url
                && probe.page.title == "Agent controlled page"
                && !probe.page.loading
                && frame.sequence > 0
                && frame.center_bgra == [216, 247, 217, 255],
            "visual checkpoint lost the authenticated green agent page"
        );
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(250))).await;
    }
    println!("PASS bounded visual checkpoint retained authenticated agent page");
    Ok(())
}

async fn focus(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    window.show()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if colossus_native_browser::is_active(&controller).await? {
            return Ok(());
        }
        window.set_focus()?;
        ensure!(
            tokio::time::Instant::now() < deadline,
            "contained browser acceptance requires a foreground Desktop window"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn request(conversation_id: String) -> anyhow::Result<CreateRunRequest> {
    Ok(CreateRunRequest {
        plugin_skill_ids: Vec::new(),
        input: vec![InputContentPart::Text(
            "Use the existing handed-off browser page. Observe its button, click it, then navigate to /agent.".into(),
        )],
        session_id: Some(conversation_id),
        end_user_id: None,
        role: "primary".into(),
        mode: RunMode::Execute,
        goal_max_iterations: 0,
        research_depth: None,
        research_sources: Vec::new(),
        plan_action: None,
        branch: None,
        max_turns: 8,
        idempotency_key: IdempotencyKey::new(format!(
            "mac-contained-browser-{}",
            uuid::Uuid::now_v7()
        ))?,
    })
}

async fn run_complete(client: &Colossus, run_id: &str) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let run = client
                .get_run(GetRunRequest {
                    run_id: run_id.to_owned(),
                })
                .await?
                .run;
            match run.status {
                RunStatus::Completed => return Ok(()),
                RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::Interrupted
                | RunStatus::OutcomeUnknown => {
                    anyhow::bail!("Core browser run ended with {:?}", run.status)
                }
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await
    .context("Core browser run did not complete")?
}

// Each assertion must preserve its relationship to the live native lease.
#[allow(clippy::too_many_lines)]
async fn exercise_owned(
    app: &tauri::AppHandle,
    client: &Colossus,
    fixture: &Fixture,
    visual_hold: Duration,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state.select_target(Some(SPACE.into())).await;
    let snapshot = action(
        app,
        BrowserAction::New {
            conversation_id: None,
            url: format!("{}/first", fixture.origin),
        },
    )
    .await?;
    ensure!(
        snapshot.engine.kind == "contained_chromium"
            && snapshot.engine.ready
            && snapshot.engine.preview,
        "Desktop did not report the Debug contained Chromium diagnostic"
    );
    let tab = snapshot.selected_tab_id.context("contained tab missing")?;
    let record = snapshot
        .tabs
        .iter()
        .find(|entry| entry.id == tab)
        .context("contained tab record missing")?;
    let conversation = record
        .conversation_id
        .clone()
        .context("Core did not bind a canonical conversation")?;
    let session = record.session_id.clone();
    let view = remote(app, &tab)?;
    ensure!(
        view.session_id() == session && view.conversation_id() == conversation,
        "Desktop RemoteSurface did not retain Core's exact native admission"
    );
    ensure!(
        app.webviews().len() == 1,
        "contained page created a Tauri WebView"
    );
    focus(app).await?;
    let rect = bounds(app)?;
    title(app, &tab, &view, rect, "Human fixture").await?;
    let human = accepted_frame(
        app,
        &tab,
        &view,
        rect,
        ExpectedFrame {
            title: "Human fixture",
            url: &format!("{}/first", fixture.origin),
            center_bgra: [255, 241, 231, 255],
            core_target: None,
            previous: None,
        },
    )
    .await?;
    ensure!(
        human
            .frame
            .is_some_and(|frame| frame.lease.control_generation == 0),
        "human page was not submitted under its native human lease"
    );
    println!("PASS authenticated Core human page submitted to Desktop RemoteSurface");

    let run = client.create_run(request(conversation)?).await?.run;
    *fixture.state.run_id.lock().unwrap() = Some(run.run_id.clone());
    fixture
        .state
        .wait(&fixture.state.model_seen, "Core model request")
        .await?;
    fixture.state.check()?;
    *fixture.state.target.lock().unwrap() = Some(json!({"session_id": session}));
    action(
        app,
        BrowserAction::Handoff {
            tab_id: tab.clone(),
            run_id: run.run_id.clone(),
        },
    )
    .await?;
    ensure!(
        !view.human_control_available(),
        "handoff retained human input"
    );
    fixture.state.signal(&fixture.state.handoff_done);
    println!("PASS authenticated native human-to-agent handoff");

    title(app, &tab, &view, rect, "Agent click received").await?;
    fixture.state.signal(&fixture.state.click_observed);
    println!("PASS Core browser.snapshot and browser.click changed the visible page");
    title(app, &tab, &view, rect, "Agent controlled page").await?;
    let page = view.inspect().await?;
    ensure!(
        page.url == format!("{}/agent", fixture.origin),
        "Core navigation reached the wrong page: {}",
        page.url
    );
    let core_target = fixture.state.navigated_target().await?;
    let probe = accepted_frame(
        app,
        &tab,
        &view,
        rect,
        ExpectedFrame {
            title: "Agent controlled page",
            url: &format!("{}/agent", fixture.origin),
            center_bgra: [216, 247, 217, 255],
            core_target: Some(&core_target),
            previous: Some(&human),
        },
    )
    .await?;
    let agent_frame = probe.frame.context("accepted agent frame missing")?;
    println!(
        "PASS Core browser.navigate submitted authenticated agent-page BGRA to Desktop NSView: document_generation={} sequence={} center_bgra={:?}",
        agent_frame.lease.document_generation, agent_frame.sequence, agent_frame.center_bgra,
    );
    fixture.state.signal(&fixture.state.navigation_observed);
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        present(app, &tab, rect).await?;
        fixture.state.check()?;
        if fixture.state.screenshot_seen.load(Ordering::Acquire) {
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Core browser.screenshot did not publish its verified PNG"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("PASS Core browser.screenshot published verified PNG artifact metadata");
    hold_visual_checkpoint(
        app,
        &tab,
        &view,
        rect,
        &core_target,
        &format!("{}/agent", fixture.origin),
        visual_hold,
    )
    .await?;
    action(app, BrowserAction::Close { tab_id: tab }).await?;
    ensure!(
        state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .is_empty(),
        "viewer detach left a Desktop tab"
    );
    ensure!(
        app.webviews().len() == 1,
        "viewer detach left an unmanaged WebView"
    );
    fixture.state.signal(&fixture.state.viewer_detached);
    run_complete(client, &run.run_id).await?;
    fixture.state.check()?;
    println!("PASS viewer detachment, Core run completion, and native browser close");
    Ok(())
}

pub(super) async fn exercise(
    app: &tauri::AppHandle,
    address: &str,
    home: &Path,
) -> anyhow::Result<()> {
    let visual_hold = visual_hold_duration()?;
    let fixture = Fixture::bind(address, visual_hold).await?;
    println!(
        "EVIDENCE contained Mac diagnostic origin {}",
        fixture.origin
    );
    let (client, native) = start_sidecar(home, &fixture.origin, visual_hold).await?;
    let state = app.state::<AppState>();
    state
        .configure_managed_browser_for(SPACE, Some(native))
        .await;
    state
        .replace_target(
            SPACE,
            client.clone(),
            crate::state::TargetConsentContext::ManagedLocal,
        )
        .await;
    let exercise = exercise_owned(app, &client, &fixture, visual_hold).await;
    let failed_run_id = if exercise.is_err() {
        fixture.state.run_id.lock().unwrap().take()
    } else {
        None
    };
    if let Some(run_id) = failed_run_id {
        let _ = client
            .cancel_run(CancelRunRequest {
                run_id,
                idempotency_key: IdempotencyKey::new(format!(
                    "mac-contained-cancel-{}",
                    uuid::Uuid::now_v7()
                ))?,
            })
            .await;
    }
    // These operations run even after a failed assertion. The sidecar owns the
    // direct-child shutdown receipt; no helper-tree cleanup is inferred here.
    let drain = state.browser.close_all_settled().await;
    let close = client.close().await;
    state.configure_managed_browser_for(SPACE, None).await;
    state.remove_target(SPACE).await;
    state.select_target(None).await;
    if exercise.is_err() {
        eprintln!(
            "EVIDENCE failed contained diagnostic cleanup ViewerDrain={} SDKClose={}",
            if drain.is_ok() { "ok" } else { "unknown" },
            if close.is_ok() {
                "verifiedGracefulDirectChild"
            } else {
                "unknown"
            }
        );
    }
    fixture.state.check()?;
    exercise?;
    drain.map_err(|error| anyhow::anyhow!(error.message))?;
    close?;
    println!(
        "PASS verified sidecar exit required GracefulDirectChildExit; Desktop native viewer shut down"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{grant, managed_runtime};
    use colossus_sdk::SidecarBootstrapConfig;
    use std::time::Duration;

    #[test]
    fn contained_fixture_bootstrap_passes_protocol_validation() {
        let origin = "http://127.0.0.1:51813";
        let runtime = managed_runtime(origin, Duration::ZERO);
        runtime.validate().expect("exact fixture runtime protocol");
        SidecarBootstrapConfig::new(
            std::env::temp_dir(),
            runtime,
            grant().expect("fixture grant"),
        )
        .expect("SDK must admit the fixed fixture configuration");

        let mut malformed = managed_runtime(origin, Duration::ZERO);
        malformed.providers[0].base_url = Some("http://fixture.example/v1".into());
        assert!(malformed.validate().is_err());
    }
}
