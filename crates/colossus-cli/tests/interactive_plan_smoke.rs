//! Cross-process acceptance for the interactive Plan workflow in both runtime hosts.

#[path = "support/process.rs"]
mod process_support;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use process_support::tempdir;
use serde_json::{Value, json};
use std::{
    fs,
    io::{ErrorKind, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const JOURNAL_KEY: &str = "bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc";
const SIGNING_KEY: &str = "dededededededededededededededededededededededededededededededede";
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(20);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
const SCREEN_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug)]
enum RuntimeHost {
    Embedded,
    Worker,
}

impl RuntimeHost {
    const ALL: [Self; 2] = [Self::Embedded, Self::Worker];

    const fn label(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Worker => "worker",
        }
    }
}

struct WorkerGuard {
    child: Child,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct Fixture {
    _directory: TempDir,
    config: PathBuf,
}

fn command(binary: &Path, config: &Path) -> process_support::IsolatedCommand {
    let mut command = Command::new(binary);
    let isolated_home =
        process_support::isolate_user_home(&mut command, config.parent().expect("config parent"));
    command
        .current_dir(config.parent().expect("config parent"))
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_INTERACTIVE_PLAN_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_INTERACTIVE_PLAN_SIGNING_KEY", SIGNING_KEY)
        .env(
            "COLOSSUS_THEME_DIR",
            config.parent().expect("config parent").join("themes"),
        );
    process_support::IsolatedCommand::new(command, isolated_home)
}

fn interactive_command(
    binary: &Path,
    config: &Path,
    host: RuntimeHost,
) -> process_support::IsolatedCommand {
    let mut command = command(binary, config);
    if matches!(host, RuntimeHost::Embedded) {
        command.arg("--approval-mode").arg("full-access");
    }
    command
}

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> Output {
    command(binary, config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

fn run_with_input(
    binary: &Path,
    config: &Path,
    host: RuntimeHost,
    arguments: &[&str],
    input: &str,
) -> Output {
    let mut child = interactive_command(binary, config, host)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Colossus");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write Colossus input");
    child.wait_with_output().expect("wait for Colossus")
}

fn parse_success(output: &Output, label: &str) -> Value {
    assert!(
        output.status.success(),
        "{label}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label} was not JSON: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn write_config(origin: &str) -> Fixture {
    let directory = tempdir().expect("fixture directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.path().join("config.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 2,
            "storage": {
                "path": directory.path().join("state.redb"),
                "keys": {
                    "kind": "environment",
                    "journal_variable": "COLOSSUS_INTERACTIVE_PLAN_JOURNAL_KEY",
                    "journal_key_id": "interactive-plan-journal-v1",
                    "signing_variable": "COLOSSUS_INTERACTIVE_PLAN_SIGNING_KEY",
                    "anchor_path": directory.path().join("anchor.json")
                }
            },
            "access": {
                "profile": "pinned",
                "tools": {
                    "include": ["plan.create", "plan.update"],
                    "exclude": []
                },
                "actions": {
                    "allow": [
                        "provider.openai.chat",
                        "plan.create",
                        "plan.update",
                        "plan.discard",
                        "plan.execute",
                        "context.show"
                    ],
                    "requireApproval": ["plan.approve_request"],
                    "deny": []
                }
            },
            "policy": {"kind": "built_in", "require_post_effect": true},
            "workflows": {"repository": workflows, "user": workflows},
            "providers": {
                "profiles": {
                    "interactive": {
                        "kind": "open_ai_compatible",
                        "baseUrl": format!("{origin}/v1"),
                        "credentialReference": null,
                        "timeoutMs": 5000
                    }
                }
            },
            "models": {
                "profiles": {
                    "interactive": {
                        "providerProfile": "interactive",
                        "model": "interactive-plan-fixture",
                        "contextWindowTokens": 32768,
                        "maxOutputTokens": 4096,
                        "capabilities": {"toolCalls": true, "streaming": true}
                    }
                },
                "roles": {"primary": "interactive"}
            },
            "agent": {"maxTurns": 4},
            "subagents": {"maxConcurrent": 1},
            "sandbox": {
                "backend": "native",
                "profile": "interactive-plan-test-v1",
                "allowBrokerFallback": false,
                "helperPath": null,
                "ociRuntime": null,
                "ociImage": null,
                "ociProxyImage": null,
                "filesystem": [{"root": directory.path(), "mode": "write"}],
                "executables": [],
                "environment": [],
                "networkDestinations": [origin],
                "timeoutMs": 5000,
                "maxOutputBytes": 1048576,
                "maxProcesses": 2,
                "maxMemoryBytes": 67108864,
                "maxConcurrency": 1
            }
        }))
        .expect("config JSON"),
    )
    .expect("write config");
    Fixture {
        _directory: directory,
        config,
    }
}

fn start_worker(binary: &Path, config: &Path) -> WorkerGuard {
    let child = command(binary, config)
        // Keep this acceptance on Tokio's production default worker-thread stack so a
        // developer-level override cannot mask oversized async state regressions.
        .env_remove("RUST_MIN_STACK")
        .arg("--approval-mode")
        .arg("full-access")
        .arg("worker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start worker");
    let mut worker = WorkerGuard { child };
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        if let Some(status) = worker.child.try_wait().expect("worker status") {
            panic!("worker exited before it became ready: {status}");
        }
        let status = run(binary, config, &["worker", "--status"]);
        if status.status.success() {
            return worker;
        }
        last_error = String::from_utf8_lossy(&status.stderr).into_owned();
        thread::sleep(Duration::from_millis(25));
    }
    panic!("worker did not become ready: {last_error}");
}

fn stop_worker(binary: &Path, config: &Path, worker: &mut WorkerGuard) {
    let shutdown = run(binary, config, &["worker", "--shutdown"]);
    assert!(
        shutdown.status.success(),
        "worker shutdown failed: {}",
        String::from_utf8_lossy(&shutdown.stderr)
    );
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while Instant::now() < deadline {
        if worker.child.try_wait().expect("worker status").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("worker did not stop");
}

fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("read provider request: {error}"))?;
        if count == 0 {
            return Err("client closed an incomplete provider request".into());
        }
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or_default();
        if request.len() >= header_end + 4 + content_length {
            return String::from_utf8(request).map_err(|error| error.to_string());
        }
    }
}

fn serve(responses: Vec<String>) -> (String, thread::JoinHandle<Result<Vec<String>, String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking provider");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        let mut requests = Vec::with_capacity(responses.len());
        for body in responses {
            let deadline = Instant::now() + PROVIDER_TIMEOUT;
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        return Err("timed out waiting for provider request".into());
                    }
                    Err(error) => return Err(format!("accept provider request: {error}")),
                }
            };
            stream
                .set_nonblocking(false)
                .map_err(|error| format!("make provider stream blocking: {error}"))?;
            let request = read_request(&mut stream)?;
            let headers = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(headers.as_bytes())
                .map_err(|error| error.to_string())?;
            stream
                .write_all(body.as_bytes())
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            requests.push(request);
        }
        Ok(requests)
    });
    (format!("http://{address}"), task)
}

fn stream_tool(id: &str, call_id: &str, name: &str, arguments: Value) -> String {
    let event = json!({
        "id": id,
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&arguments).expect("tool arguments")
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    format!("data: {event}\n\ndata: [DONE]\n\n")
}

fn stream_text(id: &str, content: &str) -> String {
    let event = json!({
        "id": id,
        "choices": [{
            "index": 0,
            "delta": {"content": content},
            "finish_reason": "stop"
        }]
    });
    format!("data: {event}\n\ndata: [DONE]\n\n")
}

fn workflow_responses() -> Vec<String> {
    vec![
        stream_tool(
            "plan-create",
            "create-call",
            "plan.create",
            json!({
                "prompt": "Plan the interactive rollout",
                "content": "# Initial rollout",
                "steps": [{
                    "title": "Implement",
                    "detail": "Implement the terminal workflow",
                    "requires_mutation": true
                }]
            }),
        ),
        stream_text("plan-create-finished", "draft-created"),
        stream_tool(
            "plan-update",
            "update-call",
            "plan.update",
            json!({
                "content": "# Refined rollout",
                "steps": [
                    {
                        "title": "Implement",
                        "detail": "Implement the terminal workflow",
                        "requires_mutation": true
                    },
                    {
                        "title": "Verify",
                        "detail": "Exercise embedded and worker-backed terminals",
                        "requires_mutation": false
                    }
                ]
            }),
        ),
        stream_text("plan-update-finished", "draft-refined"),
        stream_text("plan-direct-finished", "direct-executed"),
    ]
}

fn request_body(request: &str) -> Value {
    serde_json::from_str(
        request
            .split("\r\n\r\n")
            .nth(1)
            .expect("provider request body"),
    )
    .expect("provider request JSON")
}

fn tool_names(request: &Value) -> Vec<&str> {
    request["tools"]
        .as_array()
        .expect("tool catalog")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect()
}

fn exercise_scripted_workflow(binary: &Path, host: RuntimeHost) {
    let (origin, provider) = serve(workflow_responses());
    let fixture = write_config(&origin);
    let mut worker = match host {
        RuntimeHost::Embedded => None,
        RuntimeHost::Worker => Some(start_worker(binary, &fixture.config)),
    };
    let session = parse_success(
        &run(
            binary,
            &fixture.config,
            &["sessions", "new", &format!("{} line plan", host.label())],
        ),
        "create session",
    );
    let session_id = session["id"].as_str().expect("session id");
    let terminal = run_with_input(
        binary,
        &fixture.config,
        host,
        &["--output", "json", "tui", "--session", session_id],
        concat!(
            "/plan new\n",
            "Plan the interactive rollout\n",
            "Refine the plan with explicit verification\n",
            "/plan status\n",
            "/plan approve\n",
            "/plan execute direct\n",
            "/plan status\n",
            "/exit\n"
        ),
    );
    assert!(
        terminal.status.success(),
        "{} line workflow failed: stdout={}\nstderr={}",
        host.label(),
        String::from_utf8_lossy(&terminal.stdout),
        String::from_utf8_lossy(&terminal.stderr)
    );
    let stdout = String::from_utf8_lossy(&terminal.stdout);
    assert!(
        stdout.contains("draft-created"),
        "stdout={stdout}\nstderr={}",
        String::from_utf8_lossy(&terminal.stderr)
    );
    assert!(stdout.contains("draft-refined"), "{stdout}");
    assert!(stdout.contains("direct-executed"), "{stdout}");
    assert!(stdout.contains("mode=execute; plan=none"), "{stdout}");

    let plans = parse_success(
        &run(
            binary,
            &fixture.config,
            &["plans", "list", "--session", session_id],
        ),
        "list plans",
    );
    let plans = plans.as_array().expect("plan list");
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(plan["prompt"], "Plan the interactive rollout");
    assert_eq!(plan["content"], "# Refined rollout");
    assert_eq!(plan["steps"].as_array().map(Vec::len), Some(2));
    assert_eq!(plan["status"], "executed");
    assert_eq!(plan["revision"], 4);
    assert!(plan["executed_run_id"].as_str().is_some());

    if let Some(worker) = worker.as_mut() {
        stop_worker(binary, &fixture.config, worker);
    }
    let requests = provider
        .join()
        .expect("provider thread")
        .expect("provider fixture");
    assert_eq!(requests.len(), 5);
    let create = request_body(&requests[0]);
    let create_tools = tool_names(&create);
    assert!(create_tools.contains(&"plan_create"), "{create_tools:?}");
    assert!(!create_tools.contains(&"plan_update"), "{create_tools:?}");
    let update = request_body(&requests[2]);
    let update_tools = tool_names(&update);
    assert!(update_tools.contains(&"plan_update"), "{update_tools:?}");
    assert!(!update_tools.contains(&"plan_create"), "{update_tools:?}");
    let update_schema = update["tools"]
        .as_array()
        .expect("update tools")
        .iter()
        .find(|tool| tool["function"]["name"] == "plan_update")
        .expect("plan.update schema");
    assert!(
        update_schema["function"]["parameters"]["properties"]
            .get("id")
            .is_none(),
        "{update_schema}"
    );
}

#[test]
fn scripted_line_mode_completes_the_plan_workflow_in_both_hosts() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    for host in RuntimeHost::ALL {
        exercise_scripted_workflow(binary, host);
    }
}

#[test]
fn worker_run_plan_uses_the_default_worker_thread_stack() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let (origin, provider) = serve(vec![
        stream_tool(
            "worker-plan-create",
            "worker-plan-create-call",
            "plan.create",
            json!({
                "prompt": "Plan the worker route",
                "content": "# Worker route",
                "steps": [{
                    "title": "Verify",
                    "detail": "Exercise WorkerOperation::RunPlan",
                    "requires_mutation": false
                }]
            }),
        ),
        stream_text("worker-plan-finished", "worker-plan-created"),
    ]);
    let fixture = write_config(&origin);
    let mut worker = start_worker(binary, &fixture.config);
    let session = parse_success(
        &run(
            binary,
            &fixture.config,
            &["sessions", "new", "worker RunPlan"],
        ),
        "create worker RunPlan session",
    );
    let session_id = session["id"].as_str().expect("session id");
    let planned = parse_success(
        &run(
            binary,
            &fixture.config,
            &[
                "run",
                "Plan the worker route",
                "--plan",
                "--session",
                session_id,
            ],
        ),
        "worker RunPlan",
    );
    assert_eq!(planned["output"], "worker-plan-created");
    assert_eq!(planned["plan"]["status"], "draft");
    stop_worker(binary, &fixture.config, &mut worker);

    let requests = provider
        .join()
        .expect("provider thread")
        .expect("provider fixture");
    assert_eq!(requests.len(), 2);
    let request = request_body(&requests[0]);
    let tools = tool_names(&request);
    assert!(tools.contains(&"plan_create"), "{tools:?}");
}

fn unused_loopback_origin() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused provider address");
    format!("http://{}", listener.local_addr().expect("unused address"))
}

fn wait_for_screen(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16, needle: &str) -> bool {
    let deadline = Instant::now() + SCREEN_TIMEOUT;
    while Instant::now() < deadline {
        if screen_contents(output, rows, cols).contains(needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

fn screen_contents(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16) -> String {
    let bytes = output.lock().expect("PTY output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(&bytes);
    parser.screen().contents()
}

fn run_full_screen_lifecycle(
    binary: &Path,
    config: &Path,
    session_id: &str,
    plan_id: &str,
    host: RuntimeHost,
) {
    const ROWS: u16 = 32;
    const COLS: u16 = 140;
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open PTY");
    let mut process = CommandBuilder::new(binary);
    let workspace = config.parent().expect("config parent");
    let isolated_home = process_support::isolated_user_home(workspace);
    let user_home = isolated_home.path();
    process.cwd(workspace);
    process.arg("--config");
    process.arg(config);
    if matches!(host, RuntimeHost::Embedded) {
        process.arg("--approval-mode");
        process.arg("full-access");
    }
    process.arg("--alt-screen");
    process.arg("tui");
    process.arg("--session");
    process.arg(session_id);
    process.env("COLOSSUS_INTERACTIVE_PLAN_JOURNAL_KEY", JOURNAL_KEY);
    process.env("COLOSSUS_INTERACTIVE_PLAN_SIGNING_KEY", SIGNING_KEY);
    process.env("HOME", user_home);
    process.env("COLOSSUS_HOME", isolated_home.colossus_home());
    #[cfg(windows)]
    {
        process.env("USERPROFILE", user_home);
        process.env("TEMP", isolated_home.temporary_directory());
        process.env("TMP", isolated_home.temporary_directory());
    }
    process.env("COLOSSUS_THEME_DIR", workspace.join("themes"));
    process.env("TERM", "xterm-256color");
    let mut child = pair
        .slave
        .spawn_command(process)
        .expect("spawn full-screen TUI");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8_192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_output
                    .lock()
                    .expect("PTY output")
                    .extend_from_slice(&buffer[..read]),
            }
        }
    });
    let mut writer = pair.master.take_writer().expect("PTY writer");
    #[cfg(windows)]
    {
        writer
            .write_all(b"\x1b[1;1R")
            .expect("write cursor position response");
        writer.flush().expect("flush cursor position response");
    }

    let booted = wait_for_screen(&output, ROWS, COLS, "mode=execute");
    if booted {
        let commands = format!("/plan on\r/plan use {plan_id}\r/plan approve\r");
        writer
            .write_all(commands.as_bytes())
            .expect("write Plan lifecycle commands");
        writer.flush().expect("flush Plan lifecycle commands");
    }
    let short_id = plan_id.chars().take(8).collect::<String>();
    let approved_marker = format!("plan={short_id}:r2:approved");
    let approved = booted && wait_for_screen(&output, ROWS, COLS, &approved_marker);
    let execution_choice_visible =
        approved && wait_for_screen(&output, ROWS, COLS, "choose strategy");
    let approved_screen = screen_contents(&output, ROWS, COLS);
    // A startup or command failure can close the slave before cleanup. Preserve
    // the captured screen as the primary diagnostic instead of masking it with
    // the PTY's expected EIO after the child has already exited.
    if execution_choice_visible {
        let _ = writer.write_all(b"\x1b");
        let _ = writer.flush();
        let _ = wait_for_screen(&output, ROWS, COLS, "use /plan execute");
    }
    let _ = writer.write_all(b"/exit\r");
    let _ = writer.flush();

    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("TUI status") {
            Some(status) => break Some(status),
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            None => {
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("PTY reader thread");
    let raw_output = String::from_utf8_lossy(&output.lock().expect("PTY output")).into_owned();

    assert!(
        booted,
        "{} TUI did not boot: status={:?}; screen={}; raw={:?}",
        host.label(),
        status,
        approved_screen,
        raw_output
    );
    assert!(
        approved,
        "{} TUI did not apply queued selection before approval: {}",
        host.label(),
        approved_screen
    );
    assert!(
        approved_screen.contains("Approved plan"),
        "{}",
        approved_screen
    );
    assert!(
        execution_choice_visible,
        "{} TUI did not open the execution strategy dock after approval: {}",
        host.label(),
        approved_screen
    );
    assert!(
        status.as_ref().is_some_and(|status| status.success()),
        "{} TUI did not exit successfully: {status:?}",
        host.label()
    );
}

fn exercise_full_screen_lifecycle(binary: &Path, host: RuntimeHost) {
    let fixture = write_config(&unused_loopback_origin());
    let mut worker = match host {
        RuntimeHost::Embedded => None,
        RuntimeHost::Worker => Some(start_worker(binary, &fixture.config)),
    };
    let session = parse_success(
        &run(
            binary,
            &fixture.config,
            &["sessions", "new", &format!("{} TUI plan", host.label())],
        ),
        "create TUI session",
    );
    let session_id = session["id"].as_str().expect("session id");
    let plan = parse_success(
        &run(
            binary,
            &fixture.config,
            &[
                "plans",
                "create",
                session_id,
                "Approve through the full-screen TUI",
                "--step",
                "Review the queued lifecycle",
            ],
        ),
        "create TUI plan",
    );
    let plan_id = plan["id"].as_str().expect("plan id");

    run_full_screen_lifecycle(binary, &fixture.config, session_id, plan_id, host);

    let approved = parse_success(
        &run(binary, &fixture.config, &["plans", "show", plan_id]),
        "show approved TUI plan",
    );
    assert_eq!(approved["status"], "approved");
    assert_eq!(approved["revision"], 2);
    if let Some(worker) = worker.as_mut() {
        stop_worker(binary, &fixture.config, worker);
    }
}

#[test]
fn full_screen_tui_applies_queued_plan_lifecycle_in_both_hosts() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    for host in RuntimeHost::ALL {
        exercise_full_screen_lifecycle(binary, host);
    }
}
