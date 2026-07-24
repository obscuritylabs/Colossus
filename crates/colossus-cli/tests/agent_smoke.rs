//! Credential-free end-to-end agent CLI smoke test.

use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SIGNING_KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
        .current_dir(config.parent().expect("config directory"))
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_AGENT_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_AGENT_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn offline_agent_run_uses_active_tools_and_persists_typed_events() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    fs::write(
        directory.path().join("restart-evidence.md"),
        "Rust reconstruction records survive a redb reopen.\n",
    )
    .expect("research evidence");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_AGENT_TEST_JOURNAL_KEY
    journal_key_id: agent-test-journal-v1
    signing_variable: COLOSSUS_AGENT_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo, filesystem.list, filesystem.read, filesystem.search]
    exclude: []
  actions:
    allow: [filesystem.read, filesystem.search, task.create, task.update, decision.create, decision.update, decision.archive, decision.supersede, plan.create, goal.create, goal.show, goal.update, goal.iteration.record, subagent.create, subagent.read, subagent.list, subagent.start, subagent.complete, subagent.fail, subagent.cancel, subagent.interrupt, subagent.requeue, memory.create, memory.archive, memory.supersede, memory.read, memory.list, memory.search, memory.index.status, memory.index.sync, memory.index.rebuild, research.run, context.show, context.compact, context.snapshots, context.restore]
    requireApproval: []
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
subagents:
  maxConcurrent: 2
sandbox:
  backend: native
  profile: agent-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {workspace}
      mode: read
  executables: []
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

    let tools = run(binary, &config, &["tools", "list"]);
    assert!(
        tools.status.success(),
        "{}",
        String::from_utf8_lossy(&tools.stderr)
    );
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tool JSON");
    assert_eq!(tools.as_array().map(Vec::len), Some(4));
    assert_eq!(tools[0]["name"], "echo");
    assert_eq!(tools[0]["effect_action"], Value::Null);
    assert_eq!(tools[0]["profile"], "pinned");
    assert_eq!(tools[0]["source"], "core");
    assert_eq!(tools[0]["family"], "utility");
    assert_eq!(tools[1]["name"], "filesystem.list");
    assert_eq!(tools[1]["action_class"], "read");
    assert_eq!(tools[1]["decision"], "deny");
    assert_eq!(tools[2]["name"], "filesystem.read");
    assert_eq!(tools[3]["name"], "filesystem.search");

    let output = run(
        binary,
        &config,
        &["run", "offline agent", "--max-turns", "4", "--stream"],
    );
    let streamed = String::from_utf8_lossy(&output.stderr);
    assert!(streamed.contains("[activity] preparing"));
    assert!(streamed.contains("[activity] waiting_for_model echo"));
    assert!(streamed.contains("[activity] responding"));
    assert!(streamed.contains("offline agent"));
    assert!(streamed.contains("[activity] completed"));
    assert!(!streamed.contains("\x1b[2K"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "offline agent");
    assert_eq!(result["profile"], "echo");
    assert_eq!(result["event_count"], 6);
    let first_run_id = result["run_id"].as_str().expect("run id").to_owned();
    let session_id = result["session_id"]
        .as_str()
        .expect("session id")
        .to_owned();

    let resumed = run(
        binary,
        &config,
        &["run", "second turn", "--session", &session_id],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed: Value = serde_json::from_slice(&resumed.stdout).expect("resumed JSON");
    assert_eq!(resumed["session_id"], session_id);
    assert_eq!(resumed["output"], "second turn");

    let latest = run(binary, &config, &["run", "third turn", "--resume"]);
    assert!(
        latest.status.success(),
        "{}",
        String::from_utf8_lossy(&latest.stderr)
    );
    let latest: Value = serde_json::from_slice(&latest.stdout).expect("latest JSON");
    assert_eq!(latest["session_id"], session_id);

    let sessions = run(binary, &config, &["sessions", "list"]);
    assert!(sessions.status.success());
    let sessions: Value = serde_json::from_slice(&sessions.stdout).expect("sessions JSON");
    assert_eq!(sessions[0]["id"], session_id);
    assert_eq!(sessions[0]["message_count"], 6);
    assert_eq!(sessions[0]["last_user_preview"], "third turn");

    let messages = run(binary, &config, &["sessions", "messages", &session_id]);
    assert!(messages.status.success());
    let messages: Value = serde_json::from_slice(&messages.stdout).expect("messages JSON");
    assert_eq!(messages.as_array().map(Vec::len), Some(6));
    assert_eq!(messages[0]["message"]["content"], "offline agent");
    assert_eq!(messages[5]["message"]["content"], "third turn");

    let telemetry = run(
        binary,
        &config,
        &["telemetry", "runs", "--session", &session_id],
    );
    assert!(telemetry.status.success());
    let telemetry: Value = serde_json::from_slice(&telemetry.stdout).expect("telemetry JSON");
    assert_eq!(telemetry.as_array().map(Vec::len), Some(3));
    assert!(
        telemetry
            .as_array()
            .is_some_and(|runs| runs.iter().any(|run| run["run_id"] == first_run_id))
    );

    let prefix = &first_run_id[..20];
    let detail = run(
        binary,
        &config,
        &["telemetry", "show", prefix, "--limit", "3"],
    );
    assert!(
        detail.status.success(),
        "{}",
        String::from_utf8_lossy(&detail.stderr)
    );
    assert!(!String::from_utf8_lossy(&detail.stdout).contains("offline agent"));
    let detail: Value = serde_json::from_slice(&detail.stdout).expect("telemetry detail JSON");
    assert_eq!(detail["summary"]["run_id"], first_run_id);
    assert_eq!(detail["summary"]["final_outputs"], 1);
    assert!(
        detail["summary"]["model_output_chars"]
            .as_u64()
            .is_some_and(|value| value > 0)
    );
    assert_eq!(detail["records"].as_array().map(Vec::len), Some(3));
    assert_eq!(detail["truncated"], true);

    let metrics = run(
        binary,
        &config,
        &["telemetry", "metrics", "--session", &session_id],
    );
    assert!(metrics.status.success());
    let metrics: Value = serde_json::from_slice(&metrics.stdout).expect("telemetry metrics JSON");
    assert_eq!(metrics["run_count"], 3);
    assert_eq!(metrics["final_outputs"], 3);

    let status = run(binary, &config, &["context", "status", &session_id]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).expect("context status JSON");
    assert_eq!(status["message_count"], 6);
    assert_eq!(status["active_snapshot_id"], Value::Null);

    let compacted = run(
        binary,
        &config,
        &[
            "--approval-mode",
            "full-access",
            "context",
            "compact",
            &session_id,
        ],
    );
    assert!(
        compacted.status.success(),
        "{}",
        String::from_utf8_lossy(&compacted.stderr)
    );
    let compacted: Value =
        serde_json::from_slice(&compacted.stdout).expect("compacted context JSON");
    let snapshot_id = compacted["snapshot_id"]
        .as_str()
        .expect("snapshot id")
        .to_owned();
    assert_eq!(compacted["strategy"], "deterministic");

    let snapshots = run(binary, &config, &["context", "list", &session_id]);
    assert!(snapshots.status.success());
    let snapshots: Value =
        serde_json::from_slice(&snapshots.stdout).expect("context snapshots JSON");
    assert_eq!(snapshots.as_array().map(Vec::len), Some(1));
    assert_eq!(snapshots[0]["id"], snapshot_id);

    let restored = run(
        binary,
        &config,
        &[
            "--approval-mode",
            "full-access",
            "context",
            "restore",
            &session_id,
            &snapshot_id,
        ],
    );
    assert!(restored.status.success());
    let messages_after = run(binary, &config, &["sessions", "messages", &session_id]);
    let messages_after: Value =
        serde_json::from_slice(&messages_after.stdout).expect("messages after compact JSON");
    assert_eq!(messages_after.as_array().map(Vec::len), Some(6));

    let task = run(
        binary,
        &config,
        &[
            "tasks",
            "create",
            &session_id,
            "Verify Rust parity",
            "--description",
            "Run the full workspace gates",
        ],
    );
    assert!(
        task.status.success(),
        "{}",
        String::from_utf8_lossy(&task.stderr)
    );
    let task: Value = serde_json::from_slice(&task.stdout).expect("task JSON");
    let task_id = task["id"].as_str().expect("task id").to_owned();
    let updated_task = run(
        binary,
        &config,
        &["tasks", "update", &task_id, "--status", "completed"],
    );
    assert!(updated_task.status.success());
    let updated_task: Value =
        serde_json::from_slice(&updated_task.stdout).expect("updated task JSON");
    assert_eq!(updated_task["status"], "completed");
    let tasks = run(
        binary,
        &config,
        &[
            "tasks",
            "list",
            "--session",
            &session_id,
            "--status",
            "completed",
        ],
    );
    let tasks: Value = serde_json::from_slice(&tasks.stdout).expect("tasks JSON");
    assert_eq!(tasks[0]["id"], task_id);

    let denied_config = directory.path().join("denied-config.yaml");
    fs::write(
        &denied_config,
        fs::read_to_string(&config)
            .expect("read config")
            .replace(
                "allow: [filesystem.read, filesystem.search, task.create, task.update, decision.create, decision.update, decision.archive, decision.supersede, plan.create, goal.create, goal.show, goal.update, goal.iteration.record, subagent.create, subagent.read, subagent.list, subagent.start, subagent.complete, subagent.fail, subagent.cancel, subagent.interrupt, subagent.requeue, memory.create, memory.archive, memory.supersede, memory.read, memory.list, memory.search, memory.index.status, memory.index.sync, memory.index.rebuild, research.run, context.show, context.compact, context.snapshots, context.restore]",
                "allow: []",
            ),
    )
    .expect("denied config");
    let denied_task = run(
        binary,
        &denied_config,
        &["tasks", "create", &session_id, "Must not persist"],
    );
    assert!(!denied_task.status.success());
    let tasks_after_denial = run(
        binary,
        &config,
        &["tasks", "list", "--session", &session_id],
    );
    let tasks_after_denial: Value =
        serde_json::from_slice(&tasks_after_denial.stdout).expect("tasks after denial JSON");
    assert_eq!(tasks_after_denial.as_array().map(Vec::len), Some(1));

    let decision = run(
        binary,
        &config,
        &[
            "decisions",
            "create",
            &session_id,
            "Audit boundary",
            "Every durable mutation appends an immutable event.",
            "--priority",
            "critical",
            "--intent",
            "Preserve evidence",
            "--applies-when",
            "Changing canonical state",
        ],
    );
    assert!(
        decision.status.success(),
        "{}",
        String::from_utf8_lossy(&decision.stderr)
    );
    let decision: Value = serde_json::from_slice(&decision.stdout).expect("decision JSON");
    let decision_id = decision["id"].as_str().expect("decision id").to_owned();
    let with_decision = run(
        binary,
        &config,
        &["run", "decision-aware turn", "--session", &session_id],
    );
    assert!(with_decision.status.success());
    let with_decision: Value =
        serde_json::from_slice(&with_decision.stdout).expect("decision run JSON");
    assert_eq!(with_decision["output"], "decision-aware turn");

    let superseded = run(
        binary,
        &config,
        &[
            "decisions",
            "supersede",
            &decision_id,
            "Audit and policy boundary",
            "Every durable mutation and external effect uses its canonical boundary.",
            "--priority",
            "critical",
        ],
    );
    assert!(superseded.status.success());
    let superseded: Value = serde_json::from_slice(&superseded.stdout).expect("superseded JSON");
    assert_eq!(superseded[0]["status"], "superseded");
    assert_eq!(superseded[1]["supersedes"], decision_id);
    let replacement_id = superseded[1]["id"]
        .as_str()
        .expect("replacement id")
        .to_owned();
    let active = run(
        binary,
        &config,
        &[
            "decisions",
            "list",
            "--session",
            &session_id,
            "--status",
            "active",
        ],
    );
    let active: Value = serde_json::from_slice(&active.stdout).expect("active decisions JSON");
    assert_eq!(active.as_array().map(Vec::len), Some(1));
    assert_eq!(active[0]["id"], replacement_id);

    let plan = run(
        binary,
        &config,
        &[
            "plans",
            "create",
            &session_id,
            "Reconstruct durable state after restart",
            "--content",
            "Verify every canonical record through a fresh CLI process.",
            "--step",
            "Create records",
            "--step",
            "Reopen redb",
        ],
    );
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: Value = serde_json::from_slice(&plan.stdout).expect("plan JSON");
    let plan_id = plan["id"].as_str().expect("plan id").to_owned();
    assert_eq!(plan["status"], "draft");
    assert_eq!(plan["steps"].as_array().map(Vec::len), Some(2));
    let shown_plan = run(binary, &config, &["plans", "show", &plan_id]);
    assert!(shown_plan.status.success());
    let shown_plan: Value = serde_json::from_slice(&shown_plan.stdout).expect("shown plan JSON");
    assert_eq!(shown_plan["id"], plan_id);
    let plans = run(
        binary,
        &config,
        &["plans", "list", "--session", &session_id],
    );
    let plans: Value = serde_json::from_slice(&plans.stdout).expect("plans JSON");
    assert_eq!(plans.as_array().map(Vec::len), Some(1));
    assert_eq!(plans[0]["id"], plan_id);

    let memory = run(
        binary,
        &config,
        &[
            "memories",
            "create",
            "Run cargo clippy before declaring Rust work complete.",
            "--scope",
            "session",
            "--scope-id",
            &session_id,
            "--kind",
            "preference",
        ],
    );
    assert!(
        memory.status.success(),
        "{}",
        String::from_utf8_lossy(&memory.stderr)
    );
    let memory: Value = serde_json::from_slice(&memory.stdout).expect("memory JSON");
    let memory_id = memory["id"].as_str().expect("memory id").to_owned();
    assert_eq!(memory["scope"]["kind"], "session");
    assert_eq!(memory["scope"]["id"], session_id);

    let searched = run(
        binary,
        &config,
        &[
            "memories",
            "search",
            "cargo clippy",
            "--session",
            &session_id,
        ],
    );
    assert!(searched.status.success());
    let searched: Value = serde_json::from_slice(&searched.stdout).expect("memory search JSON");
    assert_eq!(searched.as_array().map(Vec::len), Some(1));
    assert_eq!(searched[0]["id"], memory_id);

    let memory_turn = run(
        binary,
        &config,
        &["run", "cargo clippy reminder", "--session", &session_id],
    );
    assert!(memory_turn.status.success());
    let memory_turn: Value = serde_json::from_slice(&memory_turn.stdout).expect("memory turn JSON");
    assert_eq!(memory_turn["output"], "cargo clippy reminder");

    let superseded_memory = run(
        binary,
        &config,
        &[
            "memories",
            "supersede",
            &memory_id,
            "Run formatting, Clippy, and workspace tests before completion.",
        ],
    );
    assert!(superseded_memory.status.success());
    let superseded_memory: Value =
        serde_json::from_slice(&superseded_memory.stdout).expect("superseded memory JSON");
    assert_eq!(superseded_memory[0]["status"], "superseded");
    assert_eq!(
        superseded_memory[0]["superseded_by"],
        superseded_memory[1]["id"]
    );
    let replacement_memory_id = superseded_memory[1]["id"]
        .as_str()
        .expect("replacement memory id")
        .to_owned();

    let index_status = run(binary, &config, &["memories", "index", "status"]);
    assert!(index_status.status.success());
    let index_status: Value =
        serde_json::from_slice(&index_status.stdout).expect("index status JSON");
    assert_eq!(index_status["ready"], true);
    assert_eq!(index_status["lag"], 0);

    let denied_memory = run(
        binary,
        &denied_config,
        &["memories", "create", "Must not persist"],
    );
    assert!(!denied_memory.status.success());
    let memories = run(binary, &config, &["memories", "list", "--status", "all"]);
    let memories: Value = serde_json::from_slice(&memories.stdout).expect("memories JSON");
    assert_eq!(memories.as_array().map(Vec::len), Some(2));

    let goal = run(
        binary,
        &config,
        &[
            "goals",
            "run",
            "Complete a bounded offline check",
            "--session",
            &session_id,
            "--max-iterations",
            "2",
        ],
    );
    assert!(
        goal.status.success(),
        "{}",
        String::from_utf8_lossy(&goal.stderr)
    );
    let goal: Value = serde_json::from_slice(&goal.stdout).expect("goal JSON");
    assert_eq!(goal["goal"]["status"], "active");
    assert_eq!(goal["goal"]["iterations_completed"], 2);
    assert_eq!(goal["iterations"].as_array().map(Vec::len), Some(2));
    assert_eq!(goal["iteration_budget_exhausted"], true);
    let goal_id = goal["goal"]["id"].as_str().expect("goal id");
    let shown_goal = run(binary, &config, &["goals", "show", goal_id]);
    assert!(shown_goal.status.success());
    let shown_goal: Value = serde_json::from_slice(&shown_goal.stdout).expect("shown goal JSON");
    assert_eq!(shown_goal["id"], goal_id);

    let mut agent_ids = Vec::new();
    for index in 0..4 {
        let task = format!("child complete {index}");
        let queued_agent = run(binary, &config, &["agents", "queue", &session_id, &task]);
        assert!(queued_agent.status.success());
        let queued_agent: Value =
            serde_json::from_slice(&queued_agent.stdout).expect("queued agent JSON");
        agent_ids.push(queued_agent["id"].as_str().expect("agent id").to_owned());
        assert_eq!(queued_agent["status"], "queued");
    }
    let queued_status = run(binary, &config, &["agents", "status"]);
    let queued_status: Value =
        serde_json::from_slice(&queued_status.stdout).expect("queue status JSON");
    assert_eq!(queued_status["queued"], 4);
    assert_eq!(queued_status["max_concurrent"], 2);
    let work = run(binary, &config, &["work", "--session", &session_id]);
    assert!(
        work.status.success(),
        "{}",
        String::from_utf8_lossy(&work.stderr)
    );
    let work: Value = serde_json::from_slice(&work.stdout).expect("work state JSON");
    assert_eq!(work["session_id"], session_id);
    assert_eq!(work["tasks"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["open_task_count"], 0);
    assert_eq!(work["active_decisions"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["actionable_plans"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["current_goals"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["current_subagents"].as_array().map(Vec::len), Some(4));
    let drained_agents = run(binary, &config, &["agents", "drain"]);
    assert!(
        drained_agents.status.success(),
        "{}",
        String::from_utf8_lossy(&drained_agents.stderr)
    );
    let drained_agents: Value =
        serde_json::from_slice(&drained_agents.stdout).expect("drained agents JSON");
    assert_eq!(drained_agents["completed"], 4);
    let child = run(binary, &config, &["agents", "show", &agent_ids[0]]);
    let child: Value = serde_json::from_slice(&child.stdout).expect("child JSON");
    assert_eq!(child["status"], "completed");
    assert_eq!(child["final_output"], "child complete 0");
    assert!(child["child_run_id"].as_str().is_some());
    let refreshed = run(binary, &config, &["work", "--session", &session_id]);
    let refreshed: Value =
        serde_json::from_slice(&refreshed.stdout).expect("refreshed work state JSON");
    assert!(
        refreshed["current_subagents"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let research = run(
        binary,
        &config,
        &[
            "research",
            "run",
            "Rust reconstruction",
            "--session",
            &session_id,
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
    assert_eq!(research["status"], "completed");
    let research_id = research["id"].as_str().expect("research id").to_owned();
    let shown_research = run(binary, &config, &["research", "show", &research_id]);
    assert!(shown_research.status.success());
    let shown_research: Value =
        serde_json::from_slice(&shown_research.stdout).expect("shown research JSON");
    assert_eq!(shown_research["id"], research_id);
    let research_runs = run(
        binary,
        &config,
        &["research", "list", "--session", &session_id],
    );
    let research_runs: Value =
        serde_json::from_slice(&research_runs.stdout).expect("research list JSON");
    assert_eq!(research_runs.as_array().map(Vec::len), Some(1));
    assert_eq!(research_runs[0]["id"], research_id);
    let research_sources = run(binary, &config, &["research", "sources", &research_id]);
    let research_sources: Value =
        serde_json::from_slice(&research_sources.stdout).expect("research sources JSON");
    assert!(research_sources.as_array().is_some_and(|sources| {
        sources
            .iter()
            .any(|source| source["uri"] == "restart-evidence.md")
    }));

    // Every command below opens a fresh runtime and reconstructs from the same redb journal.
    let reconstructed_session = run(binary, &config, &["sessions", "show", &session_id]);
    let reconstructed_session: Value =
        serde_json::from_slice(&reconstructed_session.stdout).expect("reconstructed session JSON");
    assert_eq!(reconstructed_session["id"], session_id);
    let reconstructed_messages = run(binary, &config, &["sessions", "messages", &session_id]);
    let reconstructed_messages: Value = serde_json::from_slice(&reconstructed_messages.stdout)
        .expect("reconstructed messages JSON");
    assert!(
        reconstructed_messages
            .as_array()
            .is_some_and(|messages| !messages.is_empty())
    );
    for (arguments, expected_id) in [
        (vec!["tasks", "show", task_id.as_str()], task_id.as_str()),
        (
            vec!["decisions", "show", replacement_id.as_str()],
            replacement_id.as_str(),
        ),
        (vec!["plans", "show", plan_id.as_str()], plan_id.as_str()),
        (
            vec!["memories", "show", replacement_memory_id.as_str()],
            replacement_memory_id.as_str(),
        ),
        (vec!["goals", "show", goal_id], goal_id),
        (
            vec!["agents", "show", agent_ids[0].as_str()],
            agent_ids[0].as_str(),
        ),
        (
            vec!["research", "show", research_id.as_str()],
            research_id.as_str(),
        ),
    ] {
        let reconstructed = run(binary, &config, &arguments);
        assert!(
            reconstructed.status.success(),
            "{}",
            String::from_utf8_lossy(&reconstructed.stderr)
        );
        let reconstructed: Value =
            serde_json::from_slice(&reconstructed.stdout).expect("reconstructed record JSON");
        assert_eq!(reconstructed["id"], expected_id);
    }

    let audit = run(binary, &config, &["audit", "show", "--limit", "1000"]);
    assert!(audit.status.success());
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let event_types = events
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"model.request.prepared.v1"));
    assert!(event_types.contains(&"context.prepared.v1"));
    assert!(event_types.contains(&"effect.requested.v1"));
    assert!(event_types.contains(&"final.output.v1"));
    assert!(event_types.contains(&"task.created.v1"));
    assert!(event_types.contains(&"task.updated.v1"));
    assert!(event_types.contains(&"decision.created.v1"));
    assert!(event_types.contains(&"decision.superseded.v1"));
    assert!(event_types.contains(&"plan.created.v1"));
    assert!(event_types.contains(&"memory.created.v1"));
    assert!(event_types.contains(&"memory.superseded.v1"));
    assert!(event_types.contains(&"goal.created.v1"));
    assert!(event_types.contains(&"goal.updated.v1"));
    assert!(event_types.contains(&"subagent.queued.v1"));
    assert!(event_types.contains(&"subagent.status_changed.v1"));
    assert!(event_types.contains(&"research.run_created.v1"));
    assert!(event_types.contains(&"research.source_added.v1"));
}
