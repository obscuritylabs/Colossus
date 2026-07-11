//! Credential-free terminal approval-mode acceptance.

use serde_json::Value;
use std::{
    fs,
    io::Write as _,
    path::Path,
    process::{Command, Stdio},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn command(binary: &Path, config: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_APPROVAL_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_APPROVAL_TEST_SIGNING_KEY", SIGNING_KEY);
    command
}

#[test]
fn terminal_modes_deny_prompt_or_auto_prove_the_same_policy_obligation() {
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
    journal_variable: COLOSSUS_APPROVAL_TEST_JOURNAL_KEY
    journal_key_id: approval-test-journal-v1
    signing_variable: COLOSSUS_APPROVAL_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: []
  approval_actions: [provider.echo]
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
  backend: native
  profile: approval-test-v1
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

    let denied = command(binary, &config)
        .args(["echo", "denied"])
        .output()
        .expect("deny run");
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("operator declined"));

    let mut prompted = command(binary, &config);
    prompted
        .args(["--approval-mode", "ask", "echo", "prompted"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut prompted = prompted.spawn().expect("prompt run");
    prompted
        .stdin
        .take()
        .expect("prompt stdin")
        .write_all(b"yes\n")
        .expect("approve");
    let prompted = prompted.wait_with_output().expect("prompt result");
    assert!(prompted.status.success());
    assert_eq!(String::from_utf8_lossy(&prompted.stdout), "prompted\n");
    assert!(String::from_utf8_lossy(&prompted.stderr).contains("approval required"));

    let automatic = command(binary, &config)
        .args(["--approval-mode", "full-access", "echo", "automatic"])
        .output()
        .expect("automatic run");
    assert!(automatic.status.success());
    assert_eq!(String::from_utf8_lossy(&automatic.stdout), "automatic\n");
    assert!(!String::from_utf8_lossy(&automatic.stderr).contains("approval required"));

    let audit = command(binary, &config)
        .args(["audit", "show", "--limit", "100"])
        .output()
        .expect("audit");
    assert!(audit.status.success());
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let names = events
        .iter()
        .filter_map(|event| event.get("event_type").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"approval.denied.v1"));
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "approval.granted.v1")
            .count(),
        2
    );
}
