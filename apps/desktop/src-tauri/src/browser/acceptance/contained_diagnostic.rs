//! Debug-only, real Core -> managed sidecar -> native Desktop acceptance.
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
#[cfg(target_os = "macos")]
use colossus_native_browser::presentation::AcceptanceFocusState;
use colossus_native_browser::{
    BrowserError, BrowserView,
    remote::{RemoteAcceptanceProbe, RemoteAcceptedFrame, RemoteSurface},
};
use colossus_sdk::{
    ApiMajor, AppPrivateInstanceDir, CancelRunRequest, Colossus, CreateRunRequest, GetRunRequest,
    IdempotencyKey, InputContentPart, InstanceId, ManagedAccessProfile, ManagedExecutionBoundary,
    ManagedFieldOverride, ManagedProviderKind, ManagedRuntimeConfig, NativeSidecarLifecycle,
    RunMode, RunStatus, RunTerminal, SidecarApplicationGrant, SidecarBootstrapConfig,
    SidecarOptions,
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
#[cfg(windows)]
#[path = "windows_remote.rs"]
mod windows;
#[cfg(windows)]
#[path = "windows_frames.rs"]
mod windows_frames;
#[cfg(windows)]
use windows_frames::{accepted_frame, focus};

mod native_input;
mod owned_tabs;
mod viewport_heartbeat;
#[cfg(windows)]
mod wait_continuity;
use viewport_heartbeat::ViewportHeartbeat;

const SPACE: &str = if cfg!(windows) {
    "windows-owned-acceptance"
} else {
    "mac-contained-acceptance"
};
const TOOLS: [&str; 12] = [
    "browser.open",
    "browser.status",
    "browser.snapshot",
    "browser.click",
    "browser.wait",
    "browser.navigate",
    "browser.screenshot",
    "browser.close",
    "browser.tabs",
    "browser.tab.open",
    "browser.tab.select",
    "browser.tab.close",
];
const MODEL_TIMEOUT: Duration = Duration::from_secs(20);
const VISUAL_MODEL_TIMEOUT: Duration = Duration::from_secs(90);
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);
// The existing agent writer lease is sixty seconds. Reserve twenty seconds
// for tool latency, viewer detachment, Core close and acknowledged shutdown.
const MAX_VISUAL_HOLD_MS: u64 = 40_000;

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
    native_input: bool,
    native_tabs: bool,
    tabs_begin: AtomicBool,
    tabs_ack: AtomicUsize,
    tab_event: Mutex<Option<(usize, BrowserTarget)>>,
    second_tab: Mutex<Option<BrowserTarget>>,
    model_timeout: Duration,
    phase: AtomicUsize,
    model_seen: AtomicBool,
    handoff_done: AtomicBool,
    post_click_snapshot: AtomicBool,
    click_observed: AtomicBool,
    navigation_observed: AtomicBool,
    screenshot_seen: AtomicBool,
    viewer_detached: AtomicBool,
    changed: Notify,
    target: Mutex<Option<Value>>,
    navigated_target: Mutex<Option<BrowserTarget>>,
    run_id: Mutex<Option<String>>,
    failure: Mutex<Option<String>>,
    #[cfg(windows)]
    wait_continuity: wait_continuity::State,
}

fn close_after_completion() -> bool {
    cfg!(windows)
        && std::env::var("COLOSSUS_BROWSER_ACCEPTANCE_CLOSE_AFTER_COMPLETION").as_deref() == Ok("1")
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
        let native_tabs = owned_tabs::enabled()?;
        ensure!(
            !native_tabs || visual_hold <= Duration::from_secs(20),
            "owned-tab acceptance reserves the existing writer lease by limiting the visual hold to 20000ms"
        );
        let state = Arc::new(FixtureState {
            native_input: native_input::enabled()?,
            native_tabs,
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
            let page: &[u8] = if cfg!(windows) {
                include_bytes!("contained_diagnostic/wait_continuity.html")
            } else if state.native_input {
                include_bytes!("native_input.html")
            } else {
                br#"<!doctype html><html><head><title>Human fixture</title></head><body style="background:#e7f1ff"><h1>Human fixture</h1><button onclick="document.title='Agent click received';document.querySelector('output').textContent='Agent clicked'">Agent click</button><output>Waiting for Core</output></body></html>"#
            };
            respond(&mut stream, 200, "text/html; charset=utf-8", page).await?;
        }
        ("GET", "/agent") => {
            let page = br#"<!doctype html><html><head><title>Agent controlled page</title></head><body style="background:#d9f7d8"><h1>Agent controlled page</h1><p>Core navigated this page after native handoff.</p></body></html>"#;
            respond(&mut stream, 200, "text/html; charset=utf-8", page).await?;
        }
        ("GET", "/tab-second") => {
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                owned_tabs::PAGE,
            )
            .await?;
        }
        ("GET", "/favicon.ico") => {
            respond(&mut stream, 200, "image/x-icon", &[]).await?;
        }
        #[cfg(windows)]
        ("GET", "/wait-release") => {
            wait_continuity::release_response(&mut stream, state).await?;
        }
        ("POST", "/v1/chat/completions") => {
            let body: Value = serde_json::from_slice(&request.body)?;
            ensure!(
                body["stream"] == false,
                "fixture requires non-streaming model"
            );
            let turn = state.phase.fetch_add(1, Ordering::AcqRel);
            ensure!(
                turn <= (if state.native_tabs { 11 } else { 7 }) + usize::from(cfg!(windows)),
                "unexpected extra model turn {turn}"
            );
            #[cfg(windows)]
            let phase = wait_continuity::logical_phase(turn);
            #[cfg(not(windows))]
            let phase = turn;
            state.signal(&state.model_seen);
            if phase == 0 {
                state
                    .wait(&state.handoff_done, "native human-to-agent handoff")
                    .await?;
            }
            if phase == 4 {
                let observed = tool_result(&body, "fixture-3")?;
                ensure!(
                    observed["snapshot"]["snapshot_id"].is_string(),
                    "missing post-click snapshot"
                );
                state.signal(&state.post_click_snapshot);
                state
                    .wait(&state.click_observed, "visible Core click")
                    .await?;
            }
            if phase == 5 {
                let result = tool_result(&body, "fixture-4")?;
                let target = browser_target(&result["tab"])?;
                *state.navigated_target.lock().unwrap() = Some(target);
                state.changed.notify_waiters();
                state
                    .wait(&state.navigation_observed, "visible Core navigation")
                    .await?;
            }
            #[cfg(windows)]
            let response = if turn == 4 {
                wait_continuity::model_wait(&body, state)?
            } else {
                if turn == 5 {
                    wait_continuity::verify_result(&body, state)?;
                }
                model_response(phase, &body, origin, state).await?
            };
            #[cfg(not(windows))]
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
    let result: Value = serde_json::from_str(content).context("tool evidence is not JSON")?;
    if result.get("error").is_some() {
        let code = categorical_tool_failure(&result);
        eprintln!(
            "EVIDENCE core-tool-failure code={code} browser_error={}",
            categorical_browser_failure(&result)
        );
    }
    Ok(result)
}

fn categorical_tool_failure(result: &Value) -> &'static str {
    match result["error"]["code"]
        .as_str()
        .or_else(|| result["error"]["type"].as_str())
    {
        Some("tool.denied" | "denied") => "tool.denied",
        Some("tool.outcome_unknown" | "outcome_unknown") => "tool.outcome_unknown",
        Some("unknown_tool") => "tool.unknown",
        Some("invalid_arguments") => "tool.invalid_arguments",
        Some("execution_error") => "tool.execution_error",
        Some("validation_error") => "tool.validation_error",
        _ => "tool.unclassified",
    }
}

fn categorical_browser_failure(result: &Value) -> &'static str {
    match result["error"]["message"].as_str() {
        Some("browser requires a fresh snapshot") => "snapshot_required",
        Some("browser document or element is stale" | "browser target is stale") => {
            "stale_document"
        }
        Some("browser control lease is stale") => "stale_control",
        Some("browser destination denied" | "browser native boundary denied operation") => "denied",
        Some("browser native host failed") => "native_failed",
        Some(
            "browser outcome unknown" | "browser outcome unknown; automatic retry is prohibited",
        ) => "outcome_unknown",
        _ => "unclassified",
    }
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
            args["max_nodes"] = json!(128);
            Ok(model_tool(phase, &alias(body, "browser.snapshot")?, &args))
        }
        4 => {
            let observed = tool_result(body, "fixture-3")?;
            ensure!(
                observed["snapshot"]["snapshot_id"].is_string(),
                "Core fresh snapshot after click is missing"
            );
            let mut args = session.context("missing owned target")?;
            args["tab_id"] = observed["tab"]["tab_id"].clone();
            args["document_id"] = observed["tab"]["document_id"].clone();
            args["url"] = json!(format!("{origin}/agent"));
            Ok(model_tool(phase, &alias(body, "browser.navigate")?, &args))
        }
        5 => {
            let navigated = tool_result(body, "fixture-4")?;
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
        6 => {
            let screenshot = tool_result(body, "fixture-5")?;
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
            ensure!(
                body["messages"]
                    .as_array()
                    .is_some_and(|messages| messages.iter().any(|message| message["content"]
                        .as_array()
                        .is_some_and(|parts| parts.iter().any(|part| part["type"]
                            == "image_url"
                            && part["image_url"]["url"].as_str().is_some_and(
                                |url| url.starts_with("data:image/png;base64,iVBOR")
                            ))))),
                "Screenshot metadata reached the model but its PNG image did not"
            );
            state.signal(&state.screenshot_seen);
            if state.native_tabs {
                return owned_tabs::response(phase, body, origin, state).await;
            }
            state
                .wait(&state.viewer_detached, "native viewer detachment")
                .await?;
            if close_after_completion() {
                return Ok(
                    json!({"id":"fixture-natural-complete","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Owned browser natural completion acceptance."},"finish_reason":"stop"}]}),
                );
            }
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
        7..=11 if state.native_tabs => owned_tabs::response(phase, body, origin, state).await,
        7 => {
            let closed = tool_result(body, "fixture-6")?;
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
                    "message": {"role": "assistant", "content": "Owned browser automation acceptance complete."},
                    "finish_reason": "stop"
                }]
            }))
        }
        _ => anyhow::bail!("unexpected fixture phase"),
    }
}

fn private_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(windows)]
    colossus_windows_native::create_private_directory(path)?;
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
    // Unknown model names deliberately use conservative image token estimates.
    // Leave room for the real screenshot and bounded tool transcript so this
    // native acceptance reaches the model without bypassing context accounting.
    runtime.models[0].context_window_tokens = 131_072;
    runtime.models[0].capabilities.streaming = false.into();
    runtime.models[0].capabilities.image_inputs = true.into();
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
                "browser.wait",
                "browser.navigate",
                "browser.screenshot",
                "browser.close",
                "browser.tabs",
                "browser.tab.open",
                "browser.tab.select",
                "browser.tab.close"
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
    // Exercise the interactive app's authenticated tool ceiling rather than
    // supplying a fixture-only grant that can hide Desktop composition gaps.
    Ok(crate::managed_runtime::application_grant(
        crate::desktop_settings::AccessProfileSetting::AllowAll,
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
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    super::super::commands::require_controller(&controller)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let generation = {
        let registry = state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?;
        ensure!(
            !registry.draining
                && registry.scope.as_deref() == Some(SPACE)
                && registry
                    .selected
                    .get(SPACE)
                    .is_some_and(|selected| selected == id)
                && registry
                    .tabs
                    .iter()
                    .any(|tab| tab.scope == SPACE && tab.dto.id == id),
            "fixture viewport lost its exact selected native tab"
        );
        registry.generation
    };
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

async fn title(view: &RemoteSurface, expected: &str) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    let mut last = String::from("no native title receipt");
    loop {
        if let Ok(page) = view.inspect().await {
            last.clone_from(&page.title);
            if page.title == expected && !page.loading {
                return Ok(());
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "native RemoteSurface did not show title {expected:?}; last title {last:?}"
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
    after: Option<RemoteAcceptedFrame>,
}

fn same_document_frame_after(frame: RemoteAcceptedFrame, prior: RemoteAcceptedFrame) -> bool {
    frame.lease.tab == prior.lease.tab
        && frame.lease.session_generation == prior.lease.session_generation
        && frame.lease.control_generation == prior.lease.control_generation
        && frame.lease.document_generation == prior.lease.document_generation
        && frame.lease.pixel_width == prior.lease.pixel_width
        && frame.lease.pixel_height == prior.lease.pixel_height
        && frame.sequence > 0
        && frame.count > 0
        && (frame.lease.viewport_generation > prior.lease.viewport_generation
            || (frame.lease.viewport_generation == prior.lease.viewport_generation
                && frame.sequence > prior.sequence
                && frame.count > prior.count))
}

#[cfg(target_os = "macos")]
async fn frame_timeout_evidence(
    app: &tauri::AppHandle,
    id: &str,
    rect: BrowserRect,
    expected: &ExpectedFrame<'_>,
    probe: Option<&RemoteAcceptanceProbe>,
    last_probe_error: &str,
    activation_requests: u8,
) {
    let frame = probe.and_then(|observed| observed.frame);
    let color_match = frame.is_some_and(|accepted| accepted.center_bgra == expected.center_bgra);
    let target_match = probe.is_some_and(|observed| {
        expected
            .core_target
            .is_none_or(|target| observed.page.target == *target)
            && observed.page.title == expected.title
            && observed.page.url == expected.url
    });
    let appkit = if let Some(window) = app.get_window("main") {
        colossus_native_browser::presentation::acceptance_focus_state(&window, false)
            .await
            .ok()
    } else {
        None
    };
    let controller_active = if let Some(controller) = app.get_webview("main") {
        colossus_native_browser::is_active(&controller).await.ok()
    } else {
        None
    };
    let valid_bounds = app
        .get_window("main")
        .and_then(|window| {
            let scale = window.scale_factor().ok()?;
            let size = window.inner_size().ok()?.to_logical::<f64>(scale);
            Some(super::super::viewport::valid_rect(
                rect,
                size.width,
                size.height,
            ))
        })
        .unwrap_or(false);
    let state = app.state::<AppState>();
    let registry = state.browser.lock().ok();
    let registry_observed = registry.is_some();
    let registry_draining = registry.as_ref().is_some_and(|registry| registry.draining);
    let registry_scope_match = registry
        .as_ref()
        .is_some_and(|registry| registry.scope.as_deref() == Some(SPACE));
    let registry_selected = registry.as_ref().is_some_and(|registry| {
        registry
            .selected
            .get(SPACE)
            .is_some_and(|selected| selected == id)
    });
    let tab = registry
        .as_ref()
        .and_then(|registry| registry.tabs.iter().find(|tab| tab.dto.id == id));
    let registry_tab_present = tab.is_some();
    let registry_tab_scope_match = tab.is_some_and(|tab| tab.scope == SPACE);
    let registry_tab_error = tab.is_some_and(|tab| tab.dto.error.is_some());
    eprintln!(
        "EVIDENCE mac-remote-frame timeout probe_present={} last_probe_error={last_probe_error} frame_present={} count={} color_match={color_match} target_match={target_match} visible={} width={} height={} focus_observed={} app_active={} window_key={} app_key_matches={} controller_observed={} controller_active={} valid_bounds={valid_bounds} registry_observed={registry_observed} registry_draining={registry_draining} registry_scope_match={registry_scope_match} registry_selected={registry_selected} registry_tab_present={registry_tab_present} registry_tab_scope_match={registry_tab_scope_match} registry_tab_error={registry_tab_error} activation_requests={activation_requests}",
        probe.is_some(),
        frame.is_some(),
        frame.map_or(0, |accepted| accepted.count.min(1_000_000)),
        probe.is_some_and(|observed| observed.visible),
        probe.map_or(0, |observed| observed.width.min(16_384)),
        probe.map_or(0, |observed| observed.height.min(16_384)),
        appkit.is_some(),
        appkit.is_some_and(AcceptanceFocusState::app_active),
        appkit.is_some_and(AcceptanceFocusState::window_key),
        appkit.is_some_and(AcceptanceFocusState::app_key_matches),
        controller_active.is_some(),
        controller_active.unwrap_or(false),
    );
}

#[cfg(target_os = "macos")]
async fn accepted_frame(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    expected: ExpectedFrame<'_>,
) -> anyhow::Result<RemoteAcceptanceProbe> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    let mut next_focus_sample = tokio::time::Instant::now();
    let mut next_activation = tokio::time::Instant::now();
    let mut activation_requests = 0_u8;
    loop {
        if tokio::time::Instant::now() >= next_focus_sample {
            let controller_active = colossus_native_browser::is_active(&controller)
                .await
                .unwrap_or(false);
            let appkit =
                colossus_native_browser::presentation::acceptance_focus_state(&window, false)
                    .await
                    .ok();
            if !(controller_active && appkit.is_some_and(AcceptanceFocusState::foreground))
                && tokio::time::Instant::now() >= next_activation
            {
                // Only the diagnostic fixture's own window is activated. The
                // frame still needs independently observed foreground state.
                let _ = window.set_focus();
                let _ =
                    colossus_native_browser::presentation::acceptance_focus_state(&window, true)
                        .await;
                activation_requests = activation_requests.saturating_add(1);
                next_activation = tokio::time::Instant::now() + Duration::from_secs(2);
            }
            next_focus_sample = tokio::time::Instant::now() + Duration::from_millis(500);
        }
        let (probe, last_probe_error) = match view.acceptance_probe().await {
            Ok(probe) => (Some(probe), "none"),
            Err(BrowserError::Stale) => (None, "stale"),
            Err(BrowserError::Unavailable) => (None, "unavailable"),
            Err(error) => return Err(error.into()),
        };
        if let Some(probe) = probe.as_ref() {
            let target_matches = expected
                .core_target
                .is_none_or(|target| probe.page.target == *target)
                && probe.page.title == expected.title
                && probe.page.url == expected.url;
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
                    && expected
                        .after
                        .is_none_or(|prior| same_document_frame_after(frame, prior))
                    && frame.sequence > 0
                    && frame.count > 0
                    && frame.center_bgra == expected.center_bgra
                {
                    let controller_active = colossus_native_browser::is_active(&controller)
                        .await
                        .unwrap_or(false);
                    let appkit = colossus_native_browser::presentation::acceptance_focus_state(
                        &window, false,
                    )
                    .await
                    .ok();
                    if controller_active && appkit.is_some_and(AcceptanceFocusState::foreground) {
                        return Ok(probe.clone());
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            frame_timeout_evidence(
                app,
                id,
                rect,
                &expected,
                probe.as_ref(),
                last_probe_error,
                activation_requests,
            )
            .await;
            anyhow::bail!(
                "native NSView did not accept an authenticated frame for the expected current document and fixture color"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "macos")]
async fn next_viewport_frame(
    app: &tauri::AppHandle,
    view: &RemoteSurface,
    previous: &RemoteAcceptanceProbe,
) -> anyhow::Result<RemoteAcceptanceProbe> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let scale = window.scale_factor()?;
    ensure!(
        scale.is_finite() && (2.0..=4.0).contains(&scale),
        "Mac Retina acceptance requires an active 2x-or-greater display"
    );
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scale_milli = (scale * 1000.0).round() as u32;
    let previous_frame = previous.frame.context("previous native frame missing")?;
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        let probe = match view.acceptance_probe().await {
            Ok(probe) => Some(probe),
            Err(BrowserError::Stale | BrowserError::Unavailable) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(probe) = probe
            && let Some(frame) = probe.frame
        {
            let width = (u64::from(probe.width) * u64::from(scale_milli)).div_ceil(1000);
            let height = (u64::from(probe.height) * u64::from(scale_milli)).div_ceil(1000);
            let width = u32::try_from(width).context("Retina width exceeds frame bound")?;
            let height = u32::try_from(height).context("Retina height exceeds frame bound")?;
            if probe.visible
                && !probe.page.loading
                && probe.page.target == previous.page.target
                && probe.page.url == previous.page.url
                && probe.page.title == previous.page.title
                && frame.lease.document_generation == previous_frame.lease.document_generation
                && frame.lease.control_generation == previous_frame.lease.control_generation
                && frame.lease.viewport_generation > previous_frame.lease.viewport_generation
                && frame.lease.pixel_width == width
                && frame.lease.pixel_height == height
                && frame.sequence > 0
                && frame.count > 0
                && frame.center_bgra == previous_frame.center_bgra
            {
                return Ok(probe);
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Retina resize did not submit a new authenticated native viewport frame"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "macos")]
async fn resize_and_occlude(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    previous: &RemoteAcceptanceProbe,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<(BrowserRect, RemoteAcceptanceProbe)> {
    heartbeat.pause().await?;
    let window = app.get_window("main").context("Desktop window missing")?;
    window.set_size(tauri::LogicalSize::new(1050.0, 760.0))?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    focus(app).await?;
    let rect = bounds(app)?;
    heartbeat.resume(id, rect).await?;
    let resized = next_viewport_frame(app, view, previous).await?;
    ensure!(
        resized.width != previous.width || resized.height != previous.height,
        "Desktop resize did not change the authenticated native viewport"
    );
    let resized_frame = resized.frame.context("resized native frame missing")?;
    println!(
        "PASS authenticated Retina Desktop resize: logical={}x{} pixels={}x{} viewport_generation={}",
        resized.width,
        resized.height,
        resized_frame.lease.pixel_width,
        resized_frame.lease.pixel_height,
        resized_frame.lease.viewport_generation
    );

    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    heartbeat.pause().await?;
    controller.eval("const overlay = document.createElement('div'); overlay.id = 'contained-acceptance-overlay'; overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:var(--surface-primary,#fff)'; document.body.appendChild(overlay);")?;
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
                tab_id: None,
                rect: None,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        ensure!(
            colossus_native_browser::is_active(&controller).await?
                && colossus_native_browser::presentation::acceptance_focus_state(&window, false)
                    .await?
                    .foreground(),
            "overlay acceptance lost the foreground Desktop window"
        );
        if view.acceptance_hidden_after(resized_frame).await? {
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "overlay did not hide the exact native view and clear its pixels"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("PASS Desktop overlay revoked the authenticated native view and cleared pixels");
    controller.eval("document.getElementById('contained-acceptance-overlay')?.remove();")?;
    heartbeat.resume(id, rect).await?;
    let restored = next_viewport_frame(app, view, &resized).await?;
    println!("PASS authenticated page returned after Desktop overlay removal");
    Ok((rect, restored))
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
    view: &RemoteSurface,
    core_target: &BrowserTarget,
    agent_url: &str,
    hold: Duration,
) -> anyhow::Result<()> {
    if hold.is_zero() {
        return Ok(());
    }
    println!(
        "VISUAL_CHECKPOINT authenticated green agent page submitted to Desktop native presenter; hold_ms={}",
        hold.as_millis()
    );
    std::io::stdout().flush()?;
    let deadline = tokio::time::Instant::now() + hold;
    loop {
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

#[cfg(target_os = "macos")]
async fn focus(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    window.show()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut attempt = 0_u32;
    let mut activation_requests = 0_u8;
    loop {
        let controller_key = colossus_native_browser::is_active(&controller).await?;
        let appkit =
            colossus_native_browser::presentation::acceptance_focus_state(&window, false).await?;
        if controller_key && appkit.foreground() {
            return Ok(());
        }
        if attempt.is_multiple_of(20) {
            window.set_focus()?;
            // AppKit may register a LaunchServices window without activating it.
            // This request targets only this fixture's own controller window;
            // the loop still waits for independently observed foreground state.
            let _ = colossus_native_browser::presentation::acceptance_focus_state(&window, true)
                .await?;
            activation_requests = activation_requests.saturating_add(1);
        }
        if tokio::time::Instant::now() >= deadline {
            eprintln!(
                "EVIDENCE mac-foreground-focus controller_key={controller_key} parent_attached={} own_window={} window_visible={} app_active={} window_key={} app_key_matches={} activation_requests={activation_requests}",
                appkit.parent_attached(),
                appkit.own_window(),
                appkit.window_visible(),
                appkit.app_active(),
                appkit.window_key(),
                appkit.app_key_matches(),
            );
            anyhow::bail!("contained browser acceptance requires a foreground Desktop window");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        attempt = attempt.saturating_add(1);
    }
}

fn request(conversation_id: String) -> anyhow::Result<CreateRunRequest> {
    Ok(CreateRunRequest {
        model_profile: None,
        reasoning_effort: None,
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
        max_turns: 12 + u32::from(cfg!(windows)),
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
    home: &Path,
    heartbeat: &mut ViewportHeartbeat,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state.select_target(Some(SPACE.into())).await;
    let blank = action(
        app,
        BrowserAction::New {
            conversation_id: None,
            url: String::new(),
        },
    )
    .await?;
    let blank_id = blank.selected_tab_id.context("blank tab missing")?;
    let blank_record = blank
        .tabs
        .iter()
        .find(|tab| tab.id == blank_id)
        .context("blank record missing")?;
    ensure!(
        blank_record.session_id.is_empty()
            && !blank_record.page.loading
            && blank_record.error.is_none(),
        "address-entry tab allocated a native session or reported loading"
    );
    ensure!(
        app.webviews().len() == 1,
        "blank tab allocated a guest webview"
    );
    let snapshot = action(
        app,
        BrowserAction::Navigate {
            tab_id: blank_id,
            url: format!("{}/first", fixture.origin),
        },
    )
    .await
    .context("stage=desktop-new-native-admission")?;
    ensure!(
        snapshot.tabs.len() == 1,
        "first navigation retained its empty tab"
    );
    println!(
        "PASS empty new tab allocates no guest and first navigation replaces it through native admission"
    );
    ensure!(
        snapshot.engine.kind
            == if cfg!(windows) {
                "owned_chromium"
            } else {
                "contained_chromium"
            }
            && snapshot.engine.ready
            && snapshot.engine.preview != cfg!(windows),
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
    let view = remote(app, &tab).context("stage=desktop-remote-surface-lookup")?;
    ensure!(
        view.session_id() == session && view.conversation_id() == conversation,
        "Desktop RemoteSurface did not retain Core's exact native admission"
    );
    ensure!(
        app.webviews().len() == 1,
        "contained page created a Tauri WebView"
    );
    focus(app).await.context("stage=desktop-foreground-focus")?;
    let rect = bounds(app).context("stage=desktop-viewport-bounds")?;
    heartbeat.resume(&tab, rect).await?;
    title(&view, "Human fixture")
        .await
        .context("stage=desktop-human-page-title-and-viewport")?;
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
            after: None,
        },
    )
    .await
    .context("stage=desktop-human-frame-acceptance")?;
    ensure!(
        human
            .frame
            .is_some_and(|frame| frame.lease.control_generation == 0),
        "human page was not submitted under its native human lease"
    );
    println!("PASS authenticated Core human page submitted to Desktop RemoteSurface");
    #[cfg(feature = "windows-browser-development")]
    {
        let loaded = state
            .browser
            .snapshot()
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        ensure!(
            loaded
                .tabs
                .iter()
                .find(|entry| entry.id == tab)
                .is_some_and(|entry| !entry.page.loading),
            "a loaded supervised page kept the legacy Windows loading spinner"
        );
        println!(
            "PASS supervised Windows loading metadata clears the spinner after real navigation"
        );
    }
    #[cfg(windows)]
    windows::human(app, &tab, &view, rect, home, heartbeat).await?;
    #[cfg(not(windows))]
    let _ = home;
    #[cfg(target_os = "macos")]
    let (rect, human) = resize_and_occlude(app, &tab, &view, &human, heartbeat)
        .await
        .context("stage=desktop-retina-resize-overlay-occlusion")?;

    if !visual_hold.is_zero() {
        println!(
            "HUMAN_VISUAL_CHECKPOINT authenticated human page submitted to Desktop NSView; hold_ms=10000"
        );
        std::io::stdout().flush()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let probe = view.acceptance_probe().await?;
            let frame = probe
                .frame
                .context("human visual checkpoint lost native frame")?;
            ensure!(
                probe.visible
                    && probe.page.title == "Human fixture"
                    && probe.page.url == format!("{}/first", fixture.origin)
                    && frame.lease.control_generation == 0
                    && frame.center_bgra == [255, 241, 231, 255],
                "human visual checkpoint lost its authenticated page"
            );
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
    }

    if fixture.state.native_input {
        native_input::checkpoint(app, &tab, &view, rect, &fixture.origin).await?;
    }
    let run = client.create_run(request(conversation.clone())?).await?.run;
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
    #[cfg(windows)]
    windows::agent_ready(app, &tab, &view, rect).await?;
    fixture.state.signal(&fixture.state.handoff_done);
    println!("PASS authenticated native human-to-agent handoff");

    title(&view, "Agent click received").await?;
    fixture
        .state
        .wait(
            &fixture.state.post_click_snapshot,
            "Core snapshot after click",
        )
        .await?;
    #[cfg(windows)]
    windows::agent_click_indicator(app, &tab, &view, rect, home).await?;
    fixture.state.signal(&fixture.state.click_observed);
    println!("PASS Core browser.snapshot and browser.click changed the visible page");
    #[cfg(windows)]
    wait_continuity::checkpoint(client, &run.run_id, &view, fixture, heartbeat).await?;
    title(&view, "Agent controlled page").await?;
    #[cfg(windows)]
    println!("EVIDENCE Core navigation title acknowledged; awaiting matching native frame");
    // Core may begin its next finite observation after the title receipt. The
    // authenticated frame loop below tolerates transient Stale receipts and
    // independently requires the exact URL, Core target and current page pixels.
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
            after: None,
        },
    )
    .await?;
    let agent_frame = probe.frame.context("accepted agent frame missing")?;
    println!(
        "PASS Core browser.navigate submitted authenticated agent-page BGRA to Desktop {}: document_generation={} sequence={} center_bgra={:?}",
        if cfg!(windows) { "HWND" } else { "NSView" },
        agent_frame.lease.document_generation,
        agent_frame.sequence,
        agent_frame.center_bgra,
    );
    fixture.state.signal(&fixture.state.navigation_observed);
    #[cfg(windows)]
    windows::agent(app, &tab, &view, rect, home).await?;
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        heartbeat.check()?;
        fixture.state.check()?;
        if fixture.state.screenshot_seen.load(Ordering::Acquire) {
            break;
        }
        let observed_run = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_run(GetRunRequest {
                run_id: run.run_id.clone(),
            }),
        )
        .await
        .context("Core screenshot continuation run status timed out")??
        .run;
        if fixture.state.screenshot_seen.load(Ordering::Acquire) {
            break;
        }
        if matches!(
            observed_run.status,
            RunStatus::Completed
                | RunStatus::Failed
                | RunStatus::Cancelled
                | RunStatus::Interrupted
                | RunStatus::OutcomeUnknown
        ) {
            let reason = match observed_run.terminal.as_ref() {
                Some(RunTerminal::Failure(failure)) => failure.reason.as_str(),
                _ => "no released failure",
            };
            anyhow::bail!(
                "Core browser run ended with {:?} before screenshot reached the model: {reason}",
                observed_run.status
            );
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Core browser.screenshot did not reach the model with verified PNG metadata and image bytes"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("PASS Core browser.screenshot published verified PNG artifact metadata");
    // Screenshot capture intentionally retires the presentation lease. Its
    // published PNG is not evidence that a new frame has reached the NSView.
    // Keep the independent trusted heartbeat active and require a subsequent
    // exact-document native acknowledgement before exposing the visual marker.
    let recovered = accepted_frame(
        app,
        &tab,
        &view,
        rect,
        ExpectedFrame {
            title: "Agent controlled page",
            url: &format!("{}/agent", fixture.origin),
            center_bgra: [216, 247, 217, 255],
            core_target: Some(&core_target),
            previous: None,
            after: Some(agent_frame),
        },
    )
    .await
    .context("stage=agent-frame-recovery-after-screenshot")?;
    let recovered_frame = recovered.frame.context("recovered agent frame missing")?;
    println!(
        "PASS authenticated agent frame recovered after Core screenshot: document_generation={} viewport_generation={} sequence={}",
        recovered_frame.lease.document_generation,
        recovered_frame.lease.viewport_generation,
        recovered_frame.sequence
    );
    hold_visual_checkpoint(
        &view,
        &core_target,
        &format!("{}/agent", fixture.origin),
        visual_hold,
    )
    .await?;
    if fixture.state.native_tabs {
        owned_tabs::checkpoint(app, &tab, &view, rect, fixture).await?;
    }
    heartbeat.pause().await?;
    if close_after_completion() {
        ensure!(
            !fixture.state.native_tabs,
            "natural-completion fixture uses its original single tab"
        );
        fixture.state.signal(&fixture.state.viewer_detached);
        run_complete(client, &run.run_id).await?;
        #[cfg(feature = "windows-browser-development")]
        state
            .browser
            .watch_agent(app, SPACE, &run.run_id, &conversation)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        #[cfg(feature = "windows-browser-development")]
        {
            let completed_view = {
                let registry = state
                    .browser
                    .lock()
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                ensure!(
                    registry.tabs.len() == 1 && registry.selected.get(SPACE) == Some(&tab),
                    "a completed run removed or replaced its selected Desktop tab"
                );
                let completed = registry
                    .tabs
                    .iter()
                    .find(|entry| entry.dto.id == tab)
                    .context("completed Desktop tab is missing")?;
                ensure!(
                    completed.dto.control == "completed"
                        && completed.dto.completed_snapshot
                        && completed.dto.page.url == format!("{}/agent", fixture.origin)
                        && completed.dto.page.title == "Agent controlled page"
                        && !completed.dto.page.loading
                        && !completed.dto.page.can_go_back
                        && !completed.dto.page.can_go_forward
                        && completed.dto.error.is_none()
                        && completed.dto.popup_url.is_none(),
                    "completed tab lost its retained page or kept live navigation state"
                );
                ensure!(
                    matches!(&completed.view, BrowserView::Snapshot(_))
                        && !completed.view.human_control_available()
                        && completed.view.contained_session_id().is_none(),
                    "completed page retained native browser or human input authority"
                );
                completed.view.clone()
            };
            ensure!(
                completed_view.inspect().await.is_err()
                    && completed_view
                        .control(colossus_native_browser::NavigationAction::Reload)
                        .await
                        .is_err()
                    && completed_view
                        .navigate(tauri::Url::parse(&format!("{}/first", fixture.origin))?)
                        .is_err()
                    && completed_view.handoff(&run.run_id).await.is_err(),
                "completed snapshot admitted native observation, navigation, control or handoff"
            );
            state.select_target(None).await;
            state.select_target(Some(SPACE.into())).await;
            state
                .browser
                .watch_agent(app, SPACE, &run.run_id, &conversation)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let selected = action(
                app,
                BrowserAction::Select {
                    tab_id: tab.clone(),
                },
            )
            .await?;
            ensure!(
                selected.tabs.len() == 1
                    && selected.selected_tab_id.as_deref() == Some(tab.as_str())
                    && selected.tabs[0].id == tab
                    && selected.tabs[0].control == "completed"
                    && selected.tabs[0].completed_snapshot
                    && selected.tabs[0].error.is_none(),
                "selection or context reconciliation removed the completed snapshot tab"
            );
            println!(
                "PASS completed Core run retained its selected read-only page snapshot across context reconciliation without native browser authority"
            );
            action(
                app,
                BrowserAction::Close {
                    tab_id: tab.clone(),
                },
            )
            .await?;
        }
    }
    if !close_after_completion() {
        #[cfg(windows)]
        view.acceptance_disconnect_presentation();
        #[cfg(feature = "windows-browser-development")]
        {
            // The normal context reconciliation must replace a failed viewer
            // without opening a browser or granting another agent writer.
            let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
            loop {
                let failed = state
                    .browser
                    .lock()
                    .map_err(|error| anyhow::anyhow!(error.message))?
                    .tabs
                    .iter()
                    .any(|entry| {
                        entry.dto.id == tab
                            && entry.dto.error.as_deref()
                                == Some(super::super::registry::CRASHED_MESSAGE)
                    });
                if failed {
                    break;
                }
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "lost viewer channel did not signal its failure"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            state
                .browser
                .watch_agent(app, SPACE, &run.run_id, &conversation)
                .await
                .map_err(|error| anyhow::anyhow!(error.message))?;
            let replacement = state
                .browser
                .lock()
                .map_err(|error| anyhow::anyhow!(error.message))?
                .selected
                .get(SPACE)
                .cloned()
                .context("recovered viewer is missing")?;
            ensure!(
                replacement != tab,
                "failed viewer was reused instead of freshly authenticated"
            );
            let recovered = remote(app, &replacement)?;
            ensure!(
                recovered.session_id() == session && !recovered.human_control_available(),
                "viewer recovery changed the browser or granted human control"
            );
            heartbeat.resume(&replacement, rect).await?;
            let color = if fixture.state.native_tabs {
                [255, 255, 255, 255]
            } else {
                [216, 247, 217, 255]
            };
            windows::selected_tab(app, &replacement, &recovered, rect, color).await?;
            heartbeat.stop().await?;
            println!(
                "PASS normal context recovery freshly authenticated the same Core browser and displayed native pixels after viewer channel loss"
            );
            action(
                app,
                BrowserAction::Close {
                    tab_id: replacement,
                },
            )
            .await?;
        }
        #[cfg(not(feature = "windows-browser-development"))]
        action(app, BrowserAction::Close { tab_id: tab }).await?;
    }
    #[cfg(not(feature = "windows-browser-development"))]
    if close_after_completion() {
        action(app, BrowserAction::Close { tab_id: tab }).await?;
    }
    heartbeat.stop().await?;
    #[cfg(windows)]
    println!("PASS lost viewer stream reconciled through separate authenticated native detach");
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
    if close_after_completion() {
        #[cfg(feature = "windows-browser-development")]
        println!(
            "PASS explicit close removed the completed page snapshot after acknowledged native detach"
        );
        #[cfg(not(feature = "windows-browser-development"))]
        println!(
            "PASS completed Core run retired its viewer; later native close reconciled the positive cleanup receipt"
        );
    }
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
        "EVIDENCE owned {} diagnostic origin {}",
        if cfg!(windows) { "Windows" } else { "macOS" },
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
    let mut heartbeat = ViewportHeartbeat::start(app.clone());
    let exercise = exercise_owned(app, &client, &fixture, visual_hold, home, &mut heartbeat).await;
    // Settle every trusted pulse before cancellation, native hide or viewer drain,
    // including when an assertion returns early. No background show can revive it.
    let heartbeat_exit = heartbeat.stop().await;
    let exercise = exercise.and(heartbeat_exit);
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
        #[cfg(windows)]
        if let Err(error) = &exercise {
            eprintln!(
                "EVIDENCE original Windows acceptance assertion before fixture cleanup: {error:#}"
            );
        }
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
    exercise?;
    fixture.state.check()?;
    drain.map_err(|error| anyhow::anyhow!(error.message))?;
    close?;
    println!(
        "PASS verified sidecar exit required GracefulDirectChildExit; Desktop native viewer shut down"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        FixtureState, categorical_browser_failure, categorical_tool_failure, grant,
        managed_runtime, model_response, same_document_frame_after,
    };
    use colossus_sdk::SidecarBootstrapConfig;
    use std::time::Duration;

    #[test]
    fn tool_failure_trace_releases_only_a_fixed_category() {
        assert_eq!(
            categorical_tool_failure(
                &serde_json::json!({"error":{"code":"tool.outcome_unknown","message":"private-page-and-secret"}})
            ),
            "tool.outcome_unknown"
        );
        assert_eq!(
            categorical_tool_failure(
                &serde_json::json!({"error":{"type":"execution_error","message":"private-page-and-secret"}})
            ),
            "tool.execution_error"
        );
        assert_eq!(
            categorical_tool_failure(
                &serde_json::json!({"error":{"code":"private-page-and-secret","type":"private-page-and-secret"}})
            ),
            "tool.unclassified"
        );
        assert_eq!(
            categorical_browser_failure(
                &serde_json::json!({"error":{"message":"browser requires a fresh snapshot"}})
            ),
            "snapshot_required"
        );
        assert_eq!(
            categorical_browser_failure(
                &serde_json::json!({"error":{"message":"private-page-and-secret"}})
            ),
            "unclassified"
        );
    }

    #[test]
    fn screenshot_recovery_requires_new_ack_for_the_same_document_and_controller() {
        use colossus_native_browser::remote::RemoteAcceptedFrame;
        let prior = RemoteAcceptedFrame {
            lease: serde_json::from_value(serde_json::json!({"tab":3,"session_generation":1,"control_generation":2,"viewport_generation":8,"document_generation":5,"pixel_width":2004,"pixel_height":1280})).unwrap(),
            sequence: 16,
            center_bgra: [216,247,217,255],
            count: 19,
        };
        assert!(!same_document_frame_after(prior, prior));
        let fresh = RemoteAcceptedFrame {
            sequence: 17,
            count: 20,
            ..prior
        };
        assert!(same_document_frame_after(fresh, prior));
        let mut changed = fresh;
        changed.lease.viewport_generation += 1;
        changed.sequence = 1;
        changed.count = 1;
        assert!(same_document_frame_after(changed, prior));
        for field in 0..5 {
            let mut foreign = changed;
            match field {
                0 => foreign.lease.tab += 1,
                1 => foreign.lease.session_generation += 1,
                2 => foreign.lease.control_generation += 1,
                3 => foreign.lease.document_generation += 1,
                _ => foreign.lease.viewport_generation = 7,
            }
            assert!(!same_document_frame_after(foreign, prior));
        }
    }

    #[tokio::test]
    async fn fixture_observes_fresh_document_after_click_before_navigation() {
        use serde_json::{Value, json};
        let state = FixtureState::default();
        *state.target.lock().unwrap() = Some(
            json!({"session_id":"session-owned", "control_generation":7, "tab_id":"tab-old", "document_id":"doc-old"}),
        );
        let body = |call_id: &str, observation: Value| {
            json!({
                "tools":[{"function":{"name":"browser_snapshot"}}, {"function":{"name":"browser_navigate"}}],
                "messages":[{"role":"tool", "tool_call_id":call_id, "content":observation.to_string()}]
            })
        };
        let clicked = body(
            "fixture-2",
            json!({"tab":{"tab_id":"tab-owned", "document_id":"doc-clicked"}}),
        );
        let observed = model_response(3, &clicked, "http://127.0.0.1:12345", &state)
            .await
            .unwrap();
        let function = &observed["choices"][0]["message"]["tool_calls"][0]["function"];
        assert_eq!(function["name"], "browser_snapshot");
        let args: Value = serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["tab_id"], "tab-owned");
        assert_eq!(args["document_id"], "doc-clicked");
        assert_eq!(args["max_nodes"], 128);
        assert!(args.get("url").is_none());
        let fresh = body(
            "fixture-3",
            json!({"tab":{"tab_id":"tab-owned", "document_id":"doc-fresh"}, "snapshot":{"snapshot_id":"snapshot-fresh"}}),
        );
        let navigate = model_response(4, &fresh, "http://127.0.0.1:12345", &state)
            .await
            .unwrap();
        let function = &navigate["choices"][0]["message"]["tool_calls"][0]["function"];
        assert_eq!(function["name"], "browser_navigate");
        let args: Value = serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["session_id"], "session-owned");
        assert_eq!(args["control_generation"], 7);
        assert_eq!(args["tab_id"], "tab-owned");
        assert_eq!(args["document_id"], "doc-fresh");
        assert_eq!(args["url"], "http://127.0.0.1:12345/agent");
    }

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
