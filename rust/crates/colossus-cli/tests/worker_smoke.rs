//! Cross-process single-writer worker and authenticated local IPC acceptance.

#![cfg(unix)]

use serde_json::Value;
use std::{
    fs,
    io::Write as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const SIGNING_KEY: &str = "6666666666666666666666666666666666666666666666666666666666666666";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn command(binary: &Path, config: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_WORKER_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_WORKER_TEST_SIGNING_KEY", SIGNING_KEY);
    command
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

fn wait_for(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        path.exists(),
        "worker endpoint was not created: {}",
        path.display()
    );
}

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while child.try_wait().expect("worker status").is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(child.try_wait().expect("worker status").is_some());
}

#[test]
fn worker_owns_lease_routes_streams_rejects_wrong_key_and_shuts_down_cleanly() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let socket = PathBuf::from(format!("{}.worker.sock", state.display()));
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_WORKER_TEST_JOURNAL_KEY
    journal_key_id: worker-test-journal-v1
    signing_variable: COLOSSUS_WORKER_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [filesystem.read, process.spawn, research.run, task.create, task.update, decision.create, decision.update, decision.archive, decision.supersede, plan.create, goal.create, goal.show, goal.update, goal.iteration.record, subagent.create, subagent.read, subagent.list, subagent.start, subagent.complete, subagent.fail, subagent.cancel, subagent.interrupt, subagent.requeue, memory.create, memory.update, memory.archive, memory.supersede, memory.read, memory.list, memory.search, memory.index.status, memory.index.sync, memory.index.rebuild]
  approval_actions: [plan.approve_request]
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    echo:
      kind: echo
      model: echo
      baseUrl: null
      credentialReference: null
      timeoutMs: 5000
  roles:
    primary: echo
agent:
  maxTurns: 4
  tools: [echo]
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
  executables: [/bin/echo]
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            workspace = directory.path().display(),
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
    wait_for(&socket, Duration::from_secs(5));
    assert_eq!(
        fs::metadata(&socket)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let status = run(binary, &config, &["worker", "--status"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).expect("worker status JSON");
    assert_eq!(status["ready"], true);
    assert_eq!(status["protocol_version"], 1);

    thread::scope(|scope| {
        let config = &config;
        let handles = (0..8)
            .map(|index| {
                scope.spawn(move || {
                    let message = format!("parallel-{index}");
                    let output = run(binary, config, &["echo", &message]);
                    assert!(output.status.success());
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
    assert_eq!(String::from_utf8_lossy(&streamed.stderr), "worker-stream\n");
    let result: Value = serde_json::from_slice(&streamed.stdout).expect("run JSON");
    assert_eq!(result["output"], "worker-stream");
    assert_eq!(result["profile"], "echo");

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
    assert!(
        goal.status.success(),
        "{}",
        String::from_utf8_lossy(&goal.stderr)
    );
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
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let shown = run(binary, &config, &["agents", "show", job_id]);
        assert!(shown.status.success());
        let shown: Value = serde_json::from_slice(&shown.stdout).expect("shown agent JSON");
        if shown["status"] == "completed" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not drain queued agent"
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

    let process = run(
        binary,
        &config,
        &[
            "process",
            "run",
            "/bin/echo",
            "--cwd",
            directory.path().to_str().expect("workspace path"),
            "--",
            "worker-process",
        ],
    );
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

    let repl = run_with_input(
        binary,
        &config,
        &["repl", "--session", session_id],
        "/theme high_contrast\n/events off\n/transcript compact\n/stream off\n/reasoning off\n/multiline on\n/stream invalid\n/repl prefs\n/sessions\n/work\ntasks-through-worker\n/tasks\n/decisions\n/plans\n/goals\n/agents\n/agents drain\n/memories\n/memory search worker\n/research list\n/telemetry\n/telemetry metrics\n/skills\n/packs list\n/packs trust list\n/integrations\n/mcp servers\n/mcp tools\n/context status\n/context list\n/workflow list\n/audit verify\n/projection status\n/tools\n/session show\n/resume 5\n1\n/session show\n/exit\n",
    );
    assert!(
        repl.status.success(),
        "{}",
        String::from_utf8_lossy(&repl.stderr)
    );
    let repl_stdout = String::from_utf8_lossy(&repl.stdout);
    assert!(repl_stdout.contains("Colossus Rust REPL via authenticated worker"));
    assert!(repl_stdout.contains(session_id));
    assert!(repl_stdout.contains("worker IPC replacement"));
    assert!(repl_stdout.contains("WORK: session="));
    assert!(repl_stdout.contains("CONTEXT: session="));
    assert!(repl_stdout.contains("tasks-through-worker"));
    assert!(repl_stdout.contains(r#""stream_mode": "off""#));
    assert!(repl_stdout.contains("recoverable: invalid presentation command"));
    assert!(repl_stdout.contains("global_sequence"));
    assert!(!repl_stdout.contains("not yet available through worker IPC"));
    assert!(!repl_stdout.contains("unknown REPL command"));

    let preferences = run(binary, &config, &["preferences", "show"]);
    assert!(preferences.status.success());
    let preferences: Value = serde_json::from_slice(&preferences.stdout).expect("preferences JSON");
    assert_eq!(preferences["theme"], "high_contrast");
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
  required: [message]
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
    assert!(String::from_utf8_lossy(&lease.stderr).contains("writer lease is already held"));

    let wrong_key = command(binary, &config)
        .args(["worker", "--shutdown"])
        .env(
            "COLOSSUS_WORKER_TEST_SIGNING_KEY",
            "7777777777777777777777777777777777777777777777777777777777777777",
        )
        .output()
        .expect("wrong-key shutdown");
    assert!(!wrong_key.status.success());
    assert!(worker.0.try_wait().expect("worker status").is_none());

    let wrong_key_echo = command(binary, &config)
        .args(["echo", "must-not-send"])
        .env(
            "COLOSSUS_WORKER_TEST_SIGNING_KEY",
            "7777777777777777777777777777777777777777777777777777777777777777",
        )
        .output()
        .expect("wrong-key echo");
    assert!(!wrong_key_echo.status.success());
    let wrong_key_error = String::from_utf8_lossy(&wrong_key_echo.stderr);
    assert!(wrong_key_error.contains("authentication tag mismatch"));
    assert!(!wrong_key_error.contains("writer lease"));

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
    wait_for_exit(&mut worker.0, Duration::from_secs(5));
    assert!(!socket.exists());

    let audit = run(binary, &config, &["audit", "show", "--limit", "1000"]);
    assert!(audit.status.success());
    let audit: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
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

    let embedded = run(binary, &config, &["run", "embedded-fallback"]);
    assert!(embedded.status.success());
    let embedded: Value = serde_json::from_slice(&embedded.stdout).expect("fallback JSON");
    assert_eq!(embedded["output"], "embedded-fallback");

    let embedded_repl = run_with_input(
        binary,
        &config,
        &["repl", "--session", session_id],
        "/repl reset\n/session show\n/work\n/exit\n",
    );
    assert!(
        embedded_repl.status.success(),
        "{}",
        String::from_utf8_lossy(&embedded_repl.stderr)
    );
    let embedded_stdout = String::from_utf8_lossy(&embedded_repl.stdout);
    assert!(embedded_stdout.contains(session_id));
    assert!(embedded_stdout.contains("[work] session="));

    let reset_preferences = run(binary, &config, &["preferences", "show"]);
    let reset_preferences: Value =
        serde_json::from_slice(&reset_preferences.stdout).expect("reset preferences JSON");
    assert_eq!(reset_preferences["theme"], "default");
    assert_eq!(reset_preferences["multiline"], false);
    assert_eq!(reset_preferences["stream_mode"], "on");
}
