//! Credential-free end-to-end agent CLI smoke test.

use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SIGNING_KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
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
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_AGENT_TEST_JOURNAL_KEY
    journal_key_id: agent-test-journal-v1
    signing_variable: COLOSSUS_AGENT_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: []
  approval_actions: []
  require_post_effect: false
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
  backend: native
  profile: agent-test-v1
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
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
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
    assert_eq!(tools[0]["name"], "echo");
    assert_eq!(tools[0]["effect_action"], Value::Null);

    let output = run(
        binary,
        &config,
        &["run", "offline agent", "--max-turns", "4"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "offline agent");
    assert_eq!(result["profile"], "echo");
    assert_eq!(result["event_count"], 3);
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

    let audit = run(binary, &config, &["audit", "show", "--limit", "20"]);
    assert!(audit.status.success());
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let event_types = events
        .iter()
        .filter_map(|event| event["event_type"].as_str())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&"model.request.prepared.v1"));
    assert!(event_types.contains(&"effect.requested.v1"));
    assert!(event_types.contains(&"final.output.v1"));
}
