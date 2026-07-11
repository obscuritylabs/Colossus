//! Windows process isolation stays fail-closed until an accepted backend is available.
#![cfg(windows)]

use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_WINDOWS_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_WINDOWS_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

fn yaml_path(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).expect("YAML path")
}

#[test]
fn unavailable_windows_process_backend_never_downgrades_or_executes() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("cmd.exe");
    let marker = directory.path().join("unsafe-downgrade.txt");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_WINDOWS_TEST_JOURNAL_KEY
    journal_key_id: windows-test-journal-v1
    signing_variable: COLOSSUS_WINDOWS_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [process.spawn]
  approval_actions: []
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
sandbox:
  backend: windows_job
  profile: windows-fail-closed-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {workspace}
      mode: write
  executables: [{executable}]
  environment: []
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = yaml_path(&state),
            anchor = yaml_path(&anchor),
            workflows = yaml_path(&workflows),
            workspace = yaml_path(directory.path()),
            executable = yaml_path(&executable),
        ),
    )
    .expect("config");

    let doctor = run(binary, &config, &["sandbox", "doctor"]);
    assert!(doctor.status.success());
    let doctor: Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    assert_eq!(doctor["platform"], "windows");
    assert_eq!(doctor["native_supported"], false);

    let executable = executable.to_string_lossy();
    let workspace = directory.path().to_string_lossy();
    let marker_command = format!("echo unsafe > \"{}\"", marker.display());
    let attempt = run(
        binary,
        &config,
        &[
            "process",
            "run",
            executable.as_ref(),
            "--cwd",
            workspace.as_ref(),
            "--",
            "/D",
            "/S",
            "/C",
            &marker_command,
        ],
    );
    assert!(
        !attempt.status.success(),
        "reserved Windows backend unexpectedly executed"
    );
    let error = String::from_utf8_lossy(&attempt.stderr);
    assert!(
        error.contains("reserved and currently fail-closed")
            || error.contains("not available in this build"),
        "unexpected Windows backend failure: {error}"
    );
    assert!(
        !marker.exists(),
        "Windows process execution silently downgraded outside isolation"
    );
}
