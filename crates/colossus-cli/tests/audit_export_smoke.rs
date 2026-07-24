//! End-to-end configured directory audit-export acceptance.

use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> Output {
    Command::new(binary)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_AUDIT_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_AUDIT_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn configured_audit_export_is_queued_policy_bound_redacted_and_replayable() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    let exports = directory.path().join("audit-exports");
    fs::create_dir_all(&workflows).expect("workflows");
    fs::create_dir_all(&exports).expect("exports");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_AUDIT_TEST_JOURNAL_KEY
    journal_key_id: audit-test-journal-v1
    signing_variable: COLOSSUS_AUDIT_TEST_SIGNING_KEY
    anchor_path: {anchor}
audit:
  exporter:
    kind: directory
    path: {exports}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [audit.export.write]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: false
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
sandbox:
  backend: broker
  profile: audit-test-v1
  allowBrokerFallback: true
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {exports}
      mode: write
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
            exports = exports.display(),
        ),
    )
    .expect("config");

    let source = run(binary, &config, &["run", "audit-export-source"]);
    assert!(
        source.status.success(),
        "{}",
        String::from_utf8_lossy(&source.stderr)
    );
    let drained = run(binary, &config, &["audit", "exporter-drain"]);
    assert!(
        drained.status.success(),
        "{}",
        String::from_utf8_lossy(&drained.stderr)
    );
    let report: Value = serde_json::from_slice(&drained.stdout).expect("drain JSON");
    assert_eq!(report["status"]["configured"], true);
    assert_eq!(report["status"]["ready"], true);
    assert!(report["exported"].as_u64().is_some_and(|count| count > 0));
    assert!(report["skipped"].as_u64().is_some_and(|count| count > 0));

    let mut records = fs::read_dir(&exports)
        .expect("export directory")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    records.sort();
    assert!(!records.is_empty());
    for record in records {
        let value: Value = serde_json::from_slice(&fs::read(record).expect("evidence bytes"))
            .expect("evidence JSON");
        assert!(value.get("ciphertext").is_none());
        assert!(value.get("nonce").is_none());
        assert!(value["payload_plaintext_hash"].is_string());
        assert_ne!(value["actor"]["id"], "audit-exporter");
    }

    let replay = run(binary, &config, &["audit", "exporter-reset"]);
    assert!(replay.status.success());
    let replayed = run(binary, &config, &["audit", "exporter-drain"]);
    assert!(replayed.status.success());
    let replayed: Value = serde_json::from_slice(&replayed.stdout).expect("replay JSON");
    assert_eq!(replayed["status"]["ready"], true);
}
