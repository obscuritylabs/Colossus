//! An installed portable stdio MCP declaration must execute with host-bound plugin paths.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;

use colossus_contracts::{Actor, ActorType};
use colossus_home::ColossusHome;
use colossus_plugins::PluginStore;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const JOURNAL_KEY: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const SIGNING_KEY: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

#[test]
fn installed_plugin_stdio_discovers_and_calls_with_host_bound_paths() {
    let directory = process_support::tempdir().expect("temporary workspace");
    let workspace = directory.path().canonicalize().expect("workspace");
    let isolated = process_support::isolated_user_home(&workspace);
    let home = ColossusHome::ensure_at(isolated.colossus_home()).expect("private home");
    let source = workspace.join("fixture-plugin");
    fs::create_dir_all(source.join("bin")).expect("plugin source");
    fs::write(source.join("plugin.json"), r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"fixture-plugin","version":"1.0.0"}"#).expect("plugin manifest");
    let server = Path::new(env!("CARGO_BIN_EXE_colossus-mcp-test-server"));
    let executable_name = if cfg!(windows) {
        "server.exe"
    } else {
        "server"
    };
    fs::copy(server, source.join("bin").join(executable_name)).expect("portable MCP binary");
    fs::write(source.join("mcp.json"), format!(r#"{{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{{"mail":{{"type":"stdio","command":"./bin/{executable_name}","cwd":"${{PLUGIN_ROOT}}"}}}}}}"#)).expect("portable MCP declaration");
    let store = PluginStore::new(home.root()).expect("plugin store");
    let actor = Actor {
        actor_type: ActorType::User,
        id: "plugin-mcp-smoke".into(),
    };
    let installed = store
        .install_directory(&source, actor.clone())
        .expect("install fixture");
    store
        .enable("fixture-plugin", &installed.digest, true, actor)
        .expect("enable fixture");
    let snapshot = store.snapshot(&[], &[]).expect("installed plugin snapshot");
    assert_eq!(snapshot.len(), 1, "{snapshot:?}");
    assert_eq!(snapshot[0].mcp_servers.len(), 1, "{snapshot:?}");
    let data = store.data_path("fixture-plugin").expect("plugin data");

    let workflows = workspace.join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let config = workspace.join("config.yaml");
    let backend = if cfg!(windows) {
        "windows_job"
    } else {
        "native"
    };
    let timeout_ms = if cfg!(windows) { 10_000 } else { 5_000 };
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 3
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PLUGIN_TEST_JOURNAL_KEY
    journal_key_id: plugin-test-journal-v1
    signing_variable: COLOSSUS_PLUGIN_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: false
workflows:
  repository: {workflows}
  user: {workflows}
plugins:
  mcpServers:
    fixture-plugin/mail:
      enabled: true
      allowedTools: [echo, plugin_paths]
sandbox:
  backend: {backend}
  profile: plugin-mcp-smoke-v1
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
  timeoutMs: {timeout_ms}
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 134217728
  maxConcurrency: 1
"#,
            state = serde_json::to_string(&workspace.join("state.redb")).unwrap(),
            anchor = serde_json::to_string(&workspace.join("anchor.json")).unwrap(),
            workflows = serde_json::to_string(&workflows).unwrap(),
            workspace = serde_json::to_string(&workspace).unwrap(),
        ),
    )
    .expect("runtime config");

    let run = |arguments: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
        command
            .current_dir(&workspace)
            .arg("--config")
            .arg(&config)
            .args(arguments)
            .env("COLOSSUS_HOME", home.root())
            .env("HOME", isolated.path())
            .env("COLOSSUS_PLUGIN_TEST_JOURNAL_KEY", JOURNAL_KEY)
            .env("COLOSSUS_PLUGIN_TEST_SIGNING_KEY", SIGNING_KEY);
        #[cfg(windows)]
        command
            .env("USERPROFILE", isolated.path())
            .env("LOCALAPPDATA", isolated.local_app_data())
            .env("TEMP", isolated.temporary_directory())
            .env("TMP", isolated.temporary_directory());
        command.output().expect("run CLI")
    };
    let tools = run(&[
        "--approval-mode",
        "full-access",
        "mcp",
        "tools",
        "--server",
        "fixture-plugin/mail",
    ]);
    assert!(
        tools.status.success(),
        "{}",
        String::from_utf8_lossy(&tools.stderr)
    );
    let discovered: Value = serde_json::from_slice(&tools.stdout).expect("tool discovery");
    let names = discovered
        .as_array()
        .expect("tool list")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["echo", "plugin_paths"]);

    let called = run(&[
        "--approval-mode",
        "full-access",
        "mcp",
        "call",
        "fixture-plugin/mail",
        "plugin_paths",
        "{}",
    ]);
    assert!(
        called.status.success(),
        "{}",
        String::from_utf8_lossy(&called.stderr)
    );
    let output: Value = serde_json::from_slice(&called.stdout).expect("tool result");
    let paths = &output["result"]["structuredContent"];
    let root = paths["root"].as_str().expect("host plugin root");
    let child_data = paths["data"].as_str().expect("host plugin data");
    assert_eq!(
        fs::canonicalize(root).expect("returned plugin root"),
        fs::canonicalize(&installed.root).expect("installed root")
    );
    assert_eq!(
        fs::canonicalize(child_data).expect("returned plugin data"),
        fs::canonicalize(data).expect("plugin data")
    );
}
