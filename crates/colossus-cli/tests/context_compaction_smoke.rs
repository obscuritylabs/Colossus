//! Cross-process automatic compaction, visibility, and raw-history acceptance.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const JOURNAL_KEY: &str = "6767676767676767676767676767676767676767676767676767676767676767";
const SIGNING_KEY: &str = "7878787878787878787878787878787878787878787878787878787878787878";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    let root = config.parent().expect("config directory");
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, root);
    command
        .current_dir(root)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_CONTEXT_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_CONTEXT_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

fn parse(output: &std::process::Output, label: &str) -> Value {
    assert!(
        output.status.success(),
        "{label}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect(label)
}

#[test]
fn automatic_compaction_is_visible_deterministic_and_preserves_raw_history() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_CONTEXT_TEST_JOURNAL_KEY
    journal_key_id: context-test-journal-v1
    signing_variable: COLOSSUS_CONTEXT_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [context.show, context.snapshots]
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
      contextWindowTokens: 2048
      maxOutputTokens: 256
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: echo
agent:
  maxTurns: 4
subagents:
  maxConcurrent: 1
context:
  autoCompaction: true
  compactAtPercent: 50
  targetPercent: 30
  preserveRecentMessages: 2
  modelAssisted: true
sandbox:
  backend: native
  profile: context-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = directory.path().join("state.redb").display(),
            anchor = directory.path().join("anchor.json").display(),
            workflows = workflows.display(),
        ),
    )
    .expect("config");

    let prompts = [
        format!("turn-one durable requirement {}", "a".repeat(700)),
        format!("turn-two implementation detail {}", "b".repeat(700)),
        format!("turn-three verification request {}", "c".repeat(700)),
    ];
    let first = parse(
        &run(binary, &config, &["run", &prompts[0]]),
        "first run JSON",
    );
    let session_id = first["session_id"].as_str().expect("session id").to_owned();
    assert_eq!(first["output"], prompts[0]);

    let second = parse(
        &run(
            binary,
            &config,
            &["run", &prompts[1], "--session", &session_id],
        ),
        "second run JSON",
    );
    assert_eq!(second["output"], prompts[1]);
    let before = parse(
        &run(binary, &config, &["context", "status", &session_id]),
        "status before compaction JSON",
    );
    assert_eq!(before["message_count"], 4);
    assert_eq!(before["compacted"], false);
    assert_eq!(before["active_snapshot_id"], Value::Null);

    let third = parse(
        &run(
            binary,
            &config,
            &["run", &prompts[2], "--session", &session_id, "--stream"],
        ),
        "third run JSON",
    );
    assert_eq!(third["output"], prompts[2]);

    let status = parse(
        &run(binary, &config, &["context", "status", &session_id]),
        "compacted status JSON",
    );
    assert_eq!(status["message_count"], 6);
    assert_eq!(status["auto_compaction"], true);
    assert_eq!(status["compacted"], true);
    assert_eq!(status["context_window_tokens"], 2048);
    assert_eq!(status["max_output_tokens"], 256);
    assert_eq!(status["safety_margin_tokens"], 512);
    assert_eq!(status["input_budget_tokens"], 1280);
    assert_eq!(status["threshold_tokens"], 640);
    assert_eq!(status["target_tokens"], 384);
    assert!(status["active_snapshot_id"].as_str().is_some());
    assert!(
        status["raw_token_estimate"]
            .as_u64()
            .zip(status["token_estimate"].as_u64())
            .is_some_and(|(raw, prepared)| raw > prepared)
    );

    let snapshots = parse(
        &run(binary, &config, &["context", "list", &session_id]),
        "context snapshots JSON",
    );
    assert_eq!(snapshots.as_array().map(Vec::len), Some(1));
    assert_eq!(snapshots[0]["strategy"], "deterministic");
    assert_eq!(snapshots[0]["source_start_sequence"], 1);
    assert_eq!(snapshots[0]["source_end_sequence"], 2);
    assert_eq!(snapshots[0]["id"], status["active_snapshot_id"]);

    let messages = parse(
        &run(binary, &config, &["sessions", "messages", &session_id]),
        "raw session messages JSON",
    );
    let messages = messages.as_array().expect("messages array");
    assert_eq!(messages.len(), 6);
    for (index, prompt) in prompts.iter().enumerate() {
        assert_eq!(messages[index * 2]["message"]["content"], *prompt);
        assert_eq!(messages[index * 2 + 1]["message"]["content"], *prompt);
    }

    let audit = parse(
        &run(binary, &config, &["audit", "show", "--limit", "200"]),
        "audit JSON",
    );
    let event_types = audit
        .as_array()
        .expect("audit array")
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == "context.snapshot.created.v1")
            .count(),
        1
    );
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == "context.snapshot.activated.v1")
            .count(),
        1
    );
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == "context.prepared.v1")
            .count(),
        3
    );
}
