//! Cross-process single-writer worker and authenticated local IPC acceptance.

#[path = "support/audit.rs"]
mod audit_support;
#[path = "support/process.rs"]
mod process_support;

use colossus_worker_protocol::{
    PROTOCOL_VERSION, WorkerApprovalMode, WorkerControlClient, worker_ipc_endpoint,
};
use hmac::{Hmac, Mac as _};
use process_support::tempdir;
use serde_json::Value;
use sha2::Sha256;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const JOURNAL_KEY: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const SIGNING_KEY: &str = "6666666666666666666666666666666666666666666666666666666666666666";
const WEBHOOK_SECRET: &str = "worker-webhook-secret-with-at-least-thirty-two-bytes";
#[cfg(not(windows))]
const WORKER_AGENT_DRAIN_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(windows)]
const WORKER_AGENT_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(windows))]
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(windows)]
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn command(binary: &Path, config: &Path) -> process_support::IsolatedCommand {
    let mut command = Command::new(binary);
    let isolated_home = process_support::isolate_user_home(
        &mut command,
        config.parent().expect("config directory"),
    );
    command
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_WORKER_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_WORKER_TEST_SIGNING_KEY", SIGNING_KEY)
        .env("COLOSSUS_WORKER_TEST_WEBHOOK_SECRET", WEBHOOK_SECRET)
        .env(
            "COLOSSUS_THEME_DIR",
            config.parent().expect("config parent").join("themes"),
        );
    process_support::IsolatedCommand::new(command, isolated_home)
}

fn webhook_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("webhook timestamp")
}

fn webhook_signature(timestamp: &str, delivery_id: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).expect("HMAC key");
    mac.update(timestamp.as_bytes());
    mac.update(b"\n");
    mac.update(delivery_id.as_bytes());
    mac.update(b"\n");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> Output {
    command(binary, config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

fn run_with_input(binary: &Path, config: &Path, arguments: &[&str], input: &str) -> Output {
    let mut child = command(binary, config)
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

fn wait_for_worker(binary: &Path, config: &Path, worker: &mut ChildGuard, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut last_error = String::new();
    while Instant::now() < deadline {
        if let Some(status) = worker.0.try_wait().expect("worker status") {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(mut pipe) = worker.0.stdout.take() {
                pipe.read_to_string(&mut stdout).expect("worker stdout");
            }
            if let Some(mut pipe) = worker.0.stderr.take() {
                pipe.read_to_string(&mut stderr).expect("worker stderr");
            }
            panic!(
                "worker exited before becoming ready ({status}); stdout: {stdout}; stderr: {stderr}"
            );
        }
        let status = run(binary, config, &["worker", "--status"]);
        if status.status.success() {
            return;
        }
        last_error = String::from_utf8_lossy(&status.stderr).into_owned();
        thread::sleep(Duration::from_millis(20));
    }
    panic!("worker endpoint did not become ready: {last_error}");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("worker status") {
            return status;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "worker process {} did not exit within {timeout:?}",
        child.id()
    );
}

fn worker_exit_diagnostics(worker: &mut ChildGuard) -> String {
    let Some(status) = worker.0.try_wait().expect("worker status") else {
        return "worker process remained alive".into();
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = worker.0.stdout.take() {
        pipe.read_to_string(&mut stdout).expect("worker stdout");
    }
    if let Some(mut pipe) = worker.0.stderr.take() {
        pipe.read_to_string(&mut stderr).expect("worker stderr");
    }
    format!("worker exited with {status}; stdout: {stdout}; stderr: {stderr}")
}

#[test]
fn worker_owns_lease_routes_streams_rejects_wrong_key_and_shuts_down_cleanly() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let mut worker_auth_path = state.as_os_str().to_os_string();
    worker_auth_path.push(".worker-auth");
    let worker_auth_path = std::path::PathBuf::from(worker_auth_path);
    #[cfg(unix)]
    let socket = std::path::PathBuf::from(format!("{}.worker.sock", state.display()));
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let themes = directory.path().join("themes");
    fs::create_dir_all(&themes).expect("themes");
    fs::write(
        themes.join("ocean.json"),
        r##"{
          "schemaVersion": 1,
          "name": "ocean",
          "base": "default",
          "title": "Ocean",
          "caret": ">",
          "continuation": "|",
          "prompt": {"left": "#00ffff", "indicator": "#00d7ff"},
          "styles": {"assistant": {"foreground": "#d7ffff"}},
          "spinner": "line"
        }"##,
    )
    .expect("ocean theme");
    #[cfg(unix)]
    let process_executable = Path::new("/bin/echo").to_path_buf();
    #[cfg(windows)]
    let process_executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("cmd.exe");
    let state_yaml = serde_json::to_string(&state.to_string_lossy()).expect("state YAML path");
    let anchor_yaml = serde_json::to_string(&anchor.to_string_lossy()).expect("anchor YAML path");
    let workflows_yaml =
        serde_json::to_string(&workflows.to_string_lossy()).expect("workflow YAML path");
    let workspace_yaml =
        serde_json::to_string(&directory.path().to_string_lossy()).expect("workspace YAML path");
    let process_executable_yaml =
        serde_json::to_string(&process_executable.to_string_lossy()).expect("executable YAML path");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_WORKER_TEST_JOURNAL_KEY
    journal_key_id: worker-test-journal-v1
    signing_variable: COLOSSUS_WORKER_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [filesystem.read, process.spawn, research.run, task.create, task.update, decision.create, decision.update, decision.archive, decision.supersede, plan.create, goal.create, goal.show, goal.update, goal.iteration.record, subagent.create, subagent.read, subagent.list, subagent.start, subagent.complete, subagent.fail, subagent.cancel, subagent.interrupt, subagent.requeue, memory.create, memory.update, memory.archive, memory.supersede, memory.read, memory.list, memory.search, memory.index.status, memory.index.sync, memory.index.rebuild, workflow.webhook.ingest, presentation.preferences.update, presentation.history.append, context.show, context.snapshots]
    requireApproval: [plan.approve_request]
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    echo:
      kind: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 5000
models:
  profiles:
    echo:
      providerProfile: echo
      model: echo
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: echo
agent:
  maxTurns: 4
sandbox:
  backend: broker
  profile: worker-test-v1
  allowBrokerFallback: true
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {workspace}
      mode: read
  executables: [{process_executable}]
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state_yaml,
            anchor = anchor_yaml,
            workflows = workflows_yaml,
            workspace = workspace_yaml,
            process_executable = process_executable_yaml,
        ),
    )
    .expect("config");

    let child = command(binary, &config)
        .args(["--approval-mode", "full-access", "worker"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start worker");
    let mut worker = ChildGuard(child);
    wait_for_worker(binary, &config, &mut worker, Duration::from_secs(10));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&socket)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let status = run(binary, &config, &["worker", "--status"]);
    assert!(
        status.status.success(),
        "worker status failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout).expect("worker status JSON");
    assert_eq!(status["ready"], true);
    assert_eq!(status["protocol_version"], PROTOCOL_VERSION);

    let mut encoded_authentication =
        fs::read_to_string(&worker_auth_path).expect("read worker control authentication");
    let encoded_key = encoded_authentication
        .strip_prefix("colossus-worker-auth-v1:")
        .expect("worker authentication prefix");
    let decoded_key = hex::decode(encoded_key).expect("worker authentication hex");
    let authentication: [u8; 32] = decoded_key
        .try_into()
        .expect("worker authentication length");
    zeroize::Zeroize::zeroize(&mut encoded_authentication);
    let control = WorkerControlClient::new(
        worker_ipc_endpoint(&state).expect("worker control endpoint"),
        zeroize::Zeroizing::new(authentication),
    )
    .expect("worker control client");
    tokio::runtime::Runtime::new()
        .expect("control runtime")
        .block_on(async {
            assert_eq!(
                control.approval_mode().await.expect("read approval mode"),
                Some(WorkerApprovalMode::FullAccess)
            );
            assert_eq!(
                control
                    .set_approval_mode(WorkerApprovalMode::Deny)
                    .await
                    .expect("set deny mode"),
                WorkerApprovalMode::Deny
            );
            assert_eq!(
                control
                    .set_approval_mode(WorkerApprovalMode::FullAccess)
                    .await
                    .expect("restore full access mode"),
                WorkerApprovalMode::FullAccess
            );
        });

    let route = run(binary, &config, &["models", "route", "primary"]);
    assert!(
        route.status.success(),
        "{}",
        String::from_utf8_lossy(&route.stderr)
    );
    let route: Value = serde_json::from_slice(&route.stdout).expect("provider route JSON");
    assert_eq!(route["role"], "primary");
    assert_eq!(route["profile"], "echo");
    assert_eq!(route["provider"], "echo");
    assert_eq!(route["model"], "echo");

    thread::scope(|scope| {
        let config = &config;
        let handles = (0..8)
            .map(|index| {
                scope.spawn(move || {
                    let message = format!("parallel-{index}");
                    let output = run(binary, config, &["echo", &message]);
                    assert!(
                        output.status.success(),
                        "parallel client {index} failed; stderr: {}; stdout: {}",
                        String::from_utf8_lossy(&output.stderr),
                        String::from_utf8_lossy(&output.stdout)
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&output.stdout),
                        format!("{message}\n")
                    );
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("parallel client");
        }
    });

    let streamed = run(binary, &config, &["run", "worker-stream", "--stream"]);
    assert!(
        streamed.status.success(),
        "{}",
        String::from_utf8_lossy(&streamed.stderr)
    );
    let worker_stream = String::from_utf8_lossy(&streamed.stderr);
    assert!(worker_stream.contains("[activity] preparing"));
    assert!(worker_stream.contains("[activity] waiting_for_model echo"));
    assert!(worker_stream.contains("[activity] responding"));
    assert!(worker_stream.contains("worker-stream"));
    assert!(worker_stream.contains("[activity] completed"));
    assert!(!worker_stream.contains("\x1b[2K"));
    let result: Value = serde_json::from_slice(&streamed.stdout).expect("run JSON");
    assert_eq!(result["output"], "worker-stream");
    assert_eq!(result["profile"], "echo");
    let exported = run(
        binary,
        &config,
        &["audit", "export", "--from", "2", "--limit", "2"],
    );
    audit_support::assert_audit_evidence_jsonl(&exported, "worker-test-journal-v1", 2, 2);
    let shown = run(
        binary,
        &config,
        &["audit", "show", "--from", "2", "--limit", "2"],
    );
    audit_support::assert_audit_evidence_array(&shown, "worker-test-journal-v1", 2, 2);

    let state_status = run(binary, &config, &["state", "doctor"]);
    assert!(state_status.status.success());
    let state_status: Value =
        serde_json::from_slice(&state_status.stdout).expect("state doctor JSON");
    assert_eq!(state_status["writer_lease"]["held"], true);
    let providers = run(binary, &config, &["provider", "profiles"]);
    assert!(providers.status.success());
    let providers: Value = serde_json::from_slice(&providers.stdout).expect("providers JSON");
    assert_eq!(providers[0]["profile"], "echo");
    let telemetry = run(binary, &config, &["telemetry", "metrics"]);
    assert!(telemetry.status.success());
    let telemetry: Value = serde_json::from_slice(&telemetry.stdout).expect("telemetry JSON");
    assert_eq!(telemetry["run_count"], 1);
    let verified = run(binary, &config, &["audit", "verify"]);
    assert!(verified.status.success());
    let export_status = run(binary, &config, &["audit", "exporter-status"]);
    assert!(export_status.status.success());
    let export_status: Value =
        serde_json::from_slice(&export_status.stdout).expect("audit exporter status JSON");
    assert_eq!(export_status["configured"], false);
    assert_eq!(export_status["ready"], true);
    let export_drain = run(binary, &config, &["audit", "exporter-drain"]);
    assert!(export_drain.status.success());
    let export_reset = run(binary, &config, &["audit", "exporter-reset"]);
    assert!(export_reset.status.success());

    let session_id = result["session_id"].as_str().expect("session id");
    let task = run(
        binary,
        &config,
        &[
            "tasks",
            "create",
            session_id,
            "Worker task",
            "--description",
            "Created over IPC",
        ],
    );
    assert!(
        task.status.success(),
        "{}",
        String::from_utf8_lossy(&task.stderr)
    );
    let task: Value = serde_json::from_slice(&task.stdout).expect("task JSON");
    let task_id = task["id"].as_str().expect("task id");
    let updated_task = run(
        binary,
        &config,
        &["tasks", "update", task_id, "--status", "completed"],
    );
    assert!(updated_task.status.success());
    let updated_task: Value =
        serde_json::from_slice(&updated_task.stdout).expect("updated task JSON");
    assert_eq!(updated_task["status"], "completed");

    let decision = run(
        binary,
        &config,
        &[
            "decisions",
            "create",
            session_id,
            "Use worker",
            "Keep the worker as single writer",
            "--priority",
            "high",
        ],
    );
    assert!(decision.status.success());
    let decision: Value = serde_json::from_slice(&decision.stdout).expect("decision JSON");
    let decision_id = decision["id"].as_str().expect("decision id");
    let archived = run(binary, &config, &["decisions", "archive", decision_id]);
    assert!(archived.status.success());
    let archived: Value = serde_json::from_slice(&archived.stdout).expect("archived JSON");
    assert_eq!(archived["status"], "archived");

    let plan = run(
        binary,
        &config,
        &[
            "plans",
            "create",
            session_id,
            "Finish cutover",
            "--step",
            "Verify worker parity",
        ],
    );
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: Value = serde_json::from_slice(&plan.stdout).expect("plan JSON");
    let plan_id = plan["id"].as_str().expect("plan id");
    let approved = run(binary, &config, &["plans", "approve", plan_id]);
    assert!(
        approved.status.success(),
        "{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    let approved: Value = serde_json::from_slice(&approved.stdout).expect("approved plan JSON");
    assert_eq!(approved["status"], "approved");

    let goal = run(
        binary,
        &config,
        &[
            "goals",
            "run",
            "Exercise worker goal routing",
            "--session",
            session_id,
            "--max-iterations",
            "1",
        ],
    );
    if !goal.status.success() {
        panic!(
            "goal command failed: {}; {}",
            String::from_utf8_lossy(&goal.stderr),
            worker_exit_diagnostics(&mut worker),
        );
    }
    let goal: Value = serde_json::from_slice(&goal.stdout).expect("goal JSON");
    assert_eq!(goal["goal"]["iteration_budget"], 1);
    let work = run(binary, &config, &["work", "--session", session_id]);
    assert!(work.status.success());
    let work: Value = serde_json::from_slice(&work.stdout).expect("worker work state JSON");
    assert_eq!(work["session_id"], session_id);
    assert_eq!(work["open_task_count"], 0);
    assert_eq!(work["actionable_plans"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["current_goals"].as_array().map(Vec::len), Some(1));

    let agent = run(
        binary,
        &config,
        &[
            "agents",
            "queue",
            session_id,
            "Return a bounded worker result",
            "--role",
            "primary",
        ],
    );
    assert!(
        agent.status.success(),
        "{}",
        String::from_utf8_lossy(&agent.stderr)
    );
    let agent: Value = serde_json::from_slice(&agent.stdout).expect("agent JSON");
    let job_id = agent["id"].as_str().expect("job id");
    let deadline = Instant::now() + WORKER_AGENT_DRAIN_TIMEOUT;
    loop {
        let shown = run(binary, &config, &["agents", "show", job_id]);
        assert!(shown.status.success());
        let shown: Value = serde_json::from_slice(&shown.stdout).expect("shown agent JSON");
        if shown["status"] == "completed" {
            break;
        }
        assert!(
            matches!(shown["status"].as_str(), Some("queued" | "running")),
            "worker agent entered an unexpected state: {}",
            shown["status"]
        );
        assert!(
            Instant::now() < deadline,
            "worker did not drain queued agent; last status={}",
            shown["status"]
        );
        thread::sleep(Duration::from_millis(50));
    }

    let memory = run(
        binary,
        &config,
        &["memories", "create", "worker IPC memory"],
    );
    assert!(
        memory.status.success(),
        "{}",
        String::from_utf8_lossy(&memory.stderr)
    );
    let memory: Value = serde_json::from_slice(&memory.stdout).expect("memory JSON");
    let memory_id = memory["id"].as_str().expect("memory id");
    let searched = run(
        binary,
        &config,
        &["memories", "search", "worker IPC", "--limit", "4"],
    );
    assert!(searched.status.success());
    let searched: Value = serde_json::from_slice(&searched.stdout).expect("search JSON");
    assert!(
        searched
            .as_array()
            .is_some_and(|records| { records.iter().any(|record| record["id"] == memory_id) })
    );
    let superseded = run(
        binary,
        &config,
        &["memories", "supersede", memory_id, "worker IPC replacement"],
    );
    assert!(superseded.status.success());
    let superseded: Value =
        serde_json::from_slice(&superseded.stdout).expect("superseded memory JSON");
    assert_eq!(superseded[1]["text"], "worker IPC replacement");

    let research = run(
        binary,
        &config,
        &[
            "research",
            "run",
            "Summarize worker IPC",
            "--session",
            session_id,
            "--depth",
            "quick",
            "--source",
            "repo",
        ],
    );
    assert!(
        research.status.success(),
        "{}",
        String::from_utf8_lossy(&research.stderr)
    );
    let research: Value = serde_json::from_slice(&research.stdout).expect("research JSON");
    assert_eq!(research["session_id"], session_id);

    let process_executable = process_executable.to_string_lossy();
    let workspace = directory.path().to_string_lossy();
    let mut process_arguments = vec![
        "process",
        "run",
        process_executable.as_ref(),
        "--cwd",
        workspace.as_ref(),
        "--",
    ];
    #[cfg(unix)]
    process_arguments.push("worker-process");
    #[cfg(windows)]
    process_arguments.extend(["/D", "/S", "/C", "echo worker-process"]);
    let process = run(binary, &config, &process_arguments);
    assert!(
        process.status.success(),
        "{}",
        String::from_utf8_lossy(&process.stderr)
    );
    let process: Value = serde_json::from_slice(&process.stdout).expect("process JSON");
    assert_eq!(process["exit_code"], 0);

    let mcp = run(binary, &config, &["mcp", "servers"]);
    assert!(mcp.status.success());
    let mcp: Value = serde_json::from_slice(&mcp.stdout).expect("MCP JSON");
    assert_eq!(mcp.as_array().map(Vec::len), Some(0));

    let skills = run(binary, &config, &["skills", "list"]);
    assert!(skills.status.success());
    let skills: Value = serde_json::from_slice(&skills.stdout).expect("skills JSON");
    assert!(skills.is_array());
    let packs = run(binary, &config, &["packs", "list"]);
    assert!(packs.status.success());
    let packs: Value = serde_json::from_slice(&packs.stdout).expect("packs JSON");
    assert_eq!(packs.as_array().map(Vec::len), Some(0));
    let integrations = run(binary, &config, &["integrations", "list"]);
    assert!(integrations.status.success());
    let integrations: Value =
        serde_json::from_slice(&integrations.stdout).expect("integrations JSON");
    assert_eq!(integrations.as_array().map(Vec::len), Some(0));

    let terminal = run_with_input(
        binary,
        &config,
        &["tui", "--session", session_id],
        "/theme mono\n/theme carrot\n/theme hacker\n/theme high-contrast\n/theme preview ocean\n/theme validate\n/theme scaffold midnight\n/theme ocean\n/theme\np 5\n\n/events off\n/transcript compact\n/stream off\n/reasoning off\n/multiline on\n/stream invalid\n/tui prefs\n/sessions\n/work\ntasks-through-worker\n/tasks\n/decisions\n/plans\n/goals\n/agents\n/agents drain\n/memories\n/memory search worker\n/research list\n/telemetry\n/telemetry metrics\n/skills\n/packs list\n/packs trust list\n/integrations\n/mcp servers\n/mcp tools\n/context status\n/context list\n/workflow list\n/audit verify\n/projection status\n/tools\n/session show\n/resume 5\n/session\n/session resume\n1\n/session show\n/exit\n",
    );
    assert!(
        terminal.status.success(),
        "stderr={}\nstdout={}",
        String::from_utf8_lossy(&terminal.stderr),
        String::from_utf8_lossy(&terminal.stdout)
    );
    let terminal_stdout = String::from_utf8_lossy(&terminal.stdout);
    let terminal_stderr = String::from_utf8_lossy(&terminal.stderr);
    assert!(terminal_stdout.contains("Colossus Rust line runner via authenticated worker"));
    assert!(terminal_stdout.contains(session_id));
    assert!(terminal_stdout.contains("[work] session="));
    assert!(terminal_stdout.contains("[context] session="));
    assert!(terminal_stdout.contains("tasks-through-worker"));
    assert!(terminal_stdout.contains("Stream mode"));
    assert!(terminal_stdout.contains("off"));
    assert!(terminal_stdout.contains("Name"));
    assert!(terminal_stdout.contains("ocean"));
    assert!(terminal_stdout.contains("Source hash"));
    assert!(terminal_stdout.contains("Active theme: ocean"));
    assert!(terminal_stdout.contains("Custom theme search locations"));
    assert!(terminal_stdout.contains("Hacker theme preview"));
    assert!(terminal_stdout.contains("Theme library valid"));
    assert!(terminal_stdout.contains("Custom theme scaffold: midnight"));
    assert!(terminal_stdout.contains("does not write this file"));
    assert!(!terminal_stdout.contains("{\"names\""));
    assert!(terminal_stdout.contains("recoverable: invalid presentation command"));
    assert!(terminal_stdout.contains("global_sequence"));
    assert!(!terminal_stdout.contains("not yet available through worker IPC"));
    assert!(!terminal_stdout.contains("unknown terminal command"));
    assert!(!terminal_stdout.contains("\x1b[2K"));
    assert!(!terminal_stderr.contains("\x1b[2K"));

    let history = run(
        binary,
        &config,
        &["preferences", "history", "--limit", "1000"],
    );
    assert!(
        history.status.success(),
        "{}",
        String::from_utf8_lossy(&history.stderr)
    );
    let history: Vec<String> =
        serde_json::from_slice(&history.stdout).expect("terminal history JSON");
    assert!(history.iter().any(|entry| entry == "tasks-through-worker"));
    assert!(history.iter().any(|entry| entry == "/theme hacker"));
    assert!(history.iter().any(|entry| entry == "/theme ocean"));
    assert!(history.iter().any(|entry| entry == "/context status"));
    assert_eq!(history.last().map(String::as_str), Some("/exit"));

    let preferences = run(binary, &config, &["preferences", "show"]);
    assert!(preferences.status.success());
    let preferences: Value = serde_json::from_slice(&preferences.stdout).expect("preferences JSON");
    assert_eq!(preferences["theme"], "default");
    assert_eq!(preferences["custom_theme"]["name"], "ocean");
    assert_eq!(preferences["custom_theme"]["base"], "default");
    assert_eq!(
        preferences["custom_theme"]["sourceHash"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    assert_eq!(preferences["multiline"], true);
    assert_eq!(preferences["stream_mode"], "off");
    assert_eq!(preferences["events_mode"], "off");
    assert_eq!(preferences["show_reasoning"], false);
    assert_eq!(preferences["transcript_density"], "compact");

    let workflow = directory.path().join("smoke.yaml");
    fs::write(
        &workflow,
        r#"apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: smoke
  version: 1.0.0
  description: Worker IPC smoke
inputs:
  type: object
  properties:
    message: { type: string }
outputs:
  type: object
capabilities: []
maxConcurrency: 1
stepBudget: 2
steps:
  - type: emit
    id: result
    value: { ok: true }
"#,
    )
    .expect("workflow");
    let registered = run(
        binary,
        &config,
        &["workflow", "register", &workflow.to_string_lossy()],
    );
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    let registered: Value = serde_json::from_slice(&registered.stdout).expect("registered JSON");
    assert_eq!(registered["registered"], true);

    let worker_webhook = run(
        binary,
        &config,
        &[
            "workflow",
            "webhook",
            "create",
            "worker-smoke",
            "smoke",
            "1.0.0",
            "--secret-reference",
            "env:COLOSSUS_WORKER_TEST_WEBHOOK_SECRET",
        ],
    );
    assert!(
        worker_webhook.status.success(),
        "{}",
        String::from_utf8_lossy(&worker_webhook.stderr)
    );
    let worker_webhook: Value =
        serde_json::from_slice(&worker_webhook.stdout).expect("worker webhook JSON");
    assert_eq!(worker_webhook["webhook_id"], "worker-smoke");
    assert_eq!(worker_webhook["enabled"], true);

    let disabled = run(
        binary,
        &config,
        &["workflow", "webhook", "disable", "worker-smoke"],
    );
    assert!(disabled.status.success());
    let disabled: Value = serde_json::from_slice(&disabled.stdout).expect("disabled webhook JSON");
    assert_eq!(disabled["enabled"], false);
    let enabled = run(
        binary,
        &config,
        &["workflow", "webhook", "enable", "worker-smoke"],
    );
    assert!(enabled.status.success());

    let worker_timestamp = webhook_timestamp();
    let worker_delivery_id = "worker-delivery-1";
    let worker_body = r#"{"message":"worker webhook"}"#;
    let worker_signature = webhook_signature(&worker_timestamp, worker_delivery_id, worker_body);
    let worker_delivery = run(
        binary,
        &config,
        &[
            "workflow",
            "webhook",
            "ingest",
            "worker-smoke",
            "--delivery-id",
            worker_delivery_id,
            "--timestamp",
            &worker_timestamp,
            "--signature",
            &worker_signature,
            "--header",
            "x-source=worker-smoke",
            "--body",
            worker_body,
        ],
    );
    assert!(
        worker_delivery.status.success(),
        "{}",
        String::from_utf8_lossy(&worker_delivery.stderr)
    );
    let worker_delivery: Value =
        serde_json::from_slice(&worker_delivery.stdout).expect("worker delivery JSON");
    assert_eq!(
        worker_delivery["delivery"]["delivery_id"],
        worker_delivery_id
    );
    assert_eq!(worker_delivery["run"]["status"], "queued");
    let replay = run(
        binary,
        &config,
        &[
            "workflow",
            "webhook",
            "ingest",
            "worker-smoke",
            "--delivery-id",
            worker_delivery_id,
            "--timestamp",
            &worker_timestamp,
            "--signature",
            &worker_signature,
            "--body",
            worker_body,
        ],
    );
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("already accepted"));

    let workflow_run = run(
        binary,
        &config,
        &[
            "workflow",
            "run",
            "smoke",
            "1.0.0",
            "--inputs",
            r#"{"message":"ipc"}"#,
        ],
    );
    assert!(
        workflow_run.status.success(),
        "{}",
        String::from_utf8_lossy(&workflow_run.stderr)
    );
    let workflow_run: Value =
        serde_json::from_slice(&workflow_run.stdout).expect("workflow run JSON");
    assert_eq!(workflow_run["status"], "completed");

    let lease = run(binary, &config, &["worker", "--once"]);
    assert!(!lease.status.success());
    let lease_stderr = String::from_utf8_lossy(&lease.stderr);
    assert!(
        lease_stderr.contains("writer lease is already held"),
        "{lease_stderr}"
    );

    let worker_authentication = fs::read(&worker_auth_path).expect("worker authentication key");
    fs::write(
        &worker_auth_path,
        b"colossus-worker-auth-v1:7777777777777777777777777777777777777777777777777777777777777777",
    )
    .expect("replace worker authentication key for negative test");
    let wrong_key = command(binary, &config)
        .args(["worker", "--shutdown"])
        .output()
        .expect("wrong-key shutdown");
    assert!(!wrong_key.status.success());
    assert!(worker.0.try_wait().expect("worker status").is_none());

    let wrong_key_echo = command(binary, &config)
        .args(["echo", "must-not-send"])
        .output()
        .expect("wrong-key echo");
    assert!(!wrong_key_echo.status.success());
    let wrong_key_error = String::from_utf8_lossy(&wrong_key_echo.stderr);
    assert!(wrong_key_error.contains("authentication tag mismatch"));
    assert!(!wrong_key_error.contains("writer lease"));
    fs::write(&worker_auth_path, worker_authentication).expect("restore worker authentication key");

    let echo = run(binary, &config, &["echo", "still-alive"]);
    assert!(echo.status.success());
    assert_eq!(String::from_utf8_lossy(&echo.stdout), "still-alive\n");

    let shutdown = run(binary, &config, &["worker", "--shutdown"]);
    assert!(
        shutdown.status.success(),
        "{}",
        String::from_utf8_lossy(&shutdown.stderr)
    );
    let shutdown: Value = serde_json::from_slice(&shutdown.stdout).expect("shutdown JSON");
    assert_eq!(shutdown["stopping"], true);
    let worker_status = wait_for_exit(&mut worker.0, WORKER_SHUTDOWN_TIMEOUT);
    assert!(
        worker_status.success(),
        "worker exited unsuccessfully after shutdown: {worker_status}"
    );
    #[cfg(unix)]
    assert!(!socket.exists());

    let audit_output = run(binary, &config, &["audit", "show", "--limit", "10000"]);
    assert!(audit_output.status.success());
    let audit_text = String::from_utf8_lossy(&audit_output.stdout);
    let audit: Value = serde_json::from_slice(&audit_output.stdout).expect("audit JSON");
    assert!(audit.as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["event_type"] == "worker.ipc.accepted.v1")
    }));
    assert!(audit.as_array().is_some_and(|events| {
        events
            .iter()
            .any(|event| event["event_type"] == "worker.ipc.rejected.v1")
    }));
    assert!(audit.as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["event_type"] == "presentation.preferences.updated.v1"
                && event["stream_id"] == "presentation:repl"
        })
    }));
    assert!(audit.as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["event_type"] == "presentation.history.appended.v1"
                && event["stream_id"] == "presentation:history"
        })
    }));
    assert!(!audit_text.contains("tasks-through-worker"));
    assert!(!audit_text.contains(WEBHOOK_SECRET));
    assert!(!audit_text.contains(&worker_signature));

    let embedded = run(binary, &config, &["run", "embedded-fallback"]);
    assert!(embedded.status.success());
    let embedded: Value = serde_json::from_slice(&embedded.stdout).expect("fallback JSON");
    assert_eq!(embedded["output"], "embedded-fallback");

    let embedded_webhook = run(
        binary,
        &config,
        &[
            "workflow",
            "webhook",
            "create",
            "embedded-smoke",
            "smoke",
            "1.0.0",
            "--secret-reference",
            "env:COLOSSUS_WORKER_TEST_WEBHOOK_SECRET",
        ],
    );
    assert!(
        embedded_webhook.status.success(),
        "{}",
        String::from_utf8_lossy(&embedded_webhook.stderr)
    );
    let embedded_timestamp = webhook_timestamp();
    let embedded_delivery_id = "embedded-delivery-1";
    let embedded_body = r#"{"message":"embedded webhook"}"#;
    let embedded_signature =
        webhook_signature(&embedded_timestamp, embedded_delivery_id, embedded_body);
    let embedded_delivery = run(
        binary,
        &config,
        &[
            "workflow",
            "webhook",
            "ingest",
            "embedded-smoke",
            "--delivery-id",
            embedded_delivery_id,
            "--timestamp",
            &embedded_timestamp,
            "--signature",
            &embedded_signature,
            "--body",
            embedded_body,
        ],
    );
    assert!(
        embedded_delivery.status.success(),
        "{}",
        String::from_utf8_lossy(&embedded_delivery.stderr)
    );
    let embedded_delivery: Value =
        serde_json::from_slice(&embedded_delivery.stdout).expect("embedded delivery JSON");
    assert_eq!(
        embedded_delivery["delivery"]["delivery_id"],
        embedded_delivery_id
    );
    assert_eq!(embedded_delivery["run"]["status"], "queued");

    let embedded_terminal = run_with_input(
        binary,
        &config,
        &["tui", "--session", session_id],
        "/tui reset\n/session show\n/session resume\n/session\n/session bogus\n/work\n/exit\n",
    );
    assert!(
        embedded_terminal.status.success(),
        "{}",
        String::from_utf8_lossy(&embedded_terminal.stderr)
    );
    let embedded_stdout = String::from_utf8_lossy(&embedded_terminal.stdout);
    assert!(embedded_stdout.contains(session_id));
    assert!(embedded_stdout.contains("Current work"));
    assert!(embedded_stdout.contains("unknown terminal command: /session bogus"));

    let embedded_history = run(
        binary,
        &config,
        &["preferences", "history", "--limit", "1000"],
    );
    assert!(embedded_history.status.success());
    let embedded_history: Vec<String> =
        serde_json::from_slice(&embedded_history.stdout).expect("embedded history JSON");
    assert!(embedded_history.iter().any(|entry| entry == "/tui reset"));
    assert_eq!(embedded_history.last().map(String::as_str), Some("/exit"));

    let reset_preferences = run(binary, &config, &["preferences", "show"]);
    let reset_preferences: Value =
        serde_json::from_slice(&reset_preferences.stdout).expect("reset preferences JSON");
    assert_eq!(reset_preferences["theme"], "default");
    assert_eq!(reset_preferences["multiline"], false);
    assert_eq!(reset_preferences["stream_mode"], "on");
}
