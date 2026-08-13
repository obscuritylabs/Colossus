//! End-to-end configured MCP discovery, invocation, redaction, and research tests.
#![cfg(any(target_os = "linux", target_os = "macos"))]

#[path = "support/process.rs"]
mod process_support;

use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const SIGNING_KEY: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const MCP_SECRET: &str = "fixture-mcp-secret-value";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_MCP_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_MCP_TEST_SIGNING_KEY", SIGNING_KEY)
        .env("MCP_TEST_SECRET", MCP_SECRET)
        .output()
        .expect("run Colossus")
}

#[test]
fn configured_mcp_is_allowlisted_permit_bound_redacted_and_research_capable() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let server = Path::new(env!("CARGO_BIN_EXE_colossus-mcp-test-server"));
    let directory = tempdir().expect("directory");
    let workspace = directory.path().canonicalize().expect("workspace");
    let workflows = workspace.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let state = workspace.join("state.redb");
    let anchor = workspace.join("anchor.json");
    let config = workspace.join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_MCP_TEST_JOURNAL_KEY
    journal_key_id: mcp-test-journal-v1
    signing_variable: COLOSSUS_MCP_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: [research.run]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
skills:
  enabled: true
  allowUserOverrides: false
  bundled: {missing}
  repository: {missing}
  user: {missing}
  disabled: []
mcp:
  servers:
    fixture:
      command: {server}
      args: []
      workingDirectory: {workspace}
      environment:
        MCP_TEST_SECRET: env:MCP_TEST_SECRET
      allowedTools: [echo, secret]
      researchTools:
        - tool: echo
          title: Fixture MCP source
          arguments:
            text: "{{query}}"
      timeoutMs: 5000
      maxOutputBytes: 1048576
sandbox:
  backend: native
  profile: mcp-native-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {workspace}
      mode: read
  executables:
    - {server}
  environment: [MCP_TEST_SECRET]
  networkDestinations: []
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 134217728
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            missing = workspace.join("missing").display(),
            server = server.display(),
            workspace = workspace.display(),
        ),
    )
    .expect("config");

    let servers = run(binary, &config, &workspace, &["mcp", "servers"]);
    assert!(
        servers.status.success(),
        "{}",
        String::from_utf8_lossy(&servers.stderr)
    );
    let servers_text = String::from_utf8_lossy(&servers.stdout);
    assert!(servers_text.contains("fixture"));
    assert!(servers_text.contains("echo"));
    assert!(!servers_text.contains(server.to_string_lossy().as_ref()));
    assert!(!servers_text.contains(MCP_SECRET));

    let tools = run(binary, &config, &workspace, &["mcp", "tools"]);
    assert!(
        tools.status.success(),
        "{}",
        String::from_utf8_lossy(&tools.stderr)
    );
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    assert_eq!(tools.as_array().expect("array").len(), 2);
    assert_eq!(tools[0]["name"], "echo");
    assert_eq!(tools[1]["name"], "secret");
    assert!(
        !serde_json::to_string(&tools)
            .expect("JSON")
            .contains("blocked")
    );

    let denied = run(
        binary,
        &config,
        &workspace,
        &["mcp", "call", "fixture", "echo", r#"{"text":"hello"}"#],
    );
    assert!(!denied.status.success());
    assert!(!String::from_utf8_lossy(&denied.stderr).contains(MCP_SECRET));

    let invalid = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "call",
            "fixture",
            "echo",
            r#"{"text":42}"#,
        ],
    );
    assert!(!invalid.status.success());
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("InvalidArguments"),
        "{}",
        String::from_utf8_lossy(&invalid.stderr)
    );

    let called = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "call",
            "fixture",
            "echo",
            r#"{"text":"hello"}"#,
        ],
    );
    assert!(
        called.status.success(),
        "{}",
        String::from_utf8_lossy(&called.stderr)
    );
    let called_text = String::from_utf8_lossy(&called.stdout);
    assert!(called_text.contains("hello"));
    assert!(called_text.contains("<redacted>"));
    assert!(!called_text.contains(MCP_SECRET));

    let research = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "research",
            "run",
            "fixture query",
            "--depth",
            "quick",
            "--source",
            "mcp",
        ],
    );
    assert!(
        research.status.success(),
        "{}",
        String::from_utf8_lossy(&research.stderr)
    );
    let research: Value = serde_json::from_slice(&research.stdout).expect("research JSON");
    assert_eq!(research["status"], "completed");
    let run_id = research["id"].as_str().expect("run id");
    let sources = run(
        binary,
        &config,
        &workspace,
        &["research", "sources", run_id],
    );
    assert!(sources.status.success());
    let sources_text = String::from_utf8_lossy(&sources.stdout);
    assert!(sources_text.contains("mcp://fixture/echo"));
    assert!(sources_text.contains("fixture query"));
    assert!(!sources_text.contains(MCP_SECRET));

    let audit = run(binary, &config, &workspace, &["audit", "verify"]);
    assert!(audit.status.success());
    let audit: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    assert!(
        audit["event_count"]
            .as_u64()
            .is_some_and(|count| count > 20)
    );
}
