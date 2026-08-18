//! Provider-neutral search profile, tool-visibility, and query CLI smoke test.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::Value;
use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
};

const JOURNAL_KEY: &str = "7171717171717171717171717171717171717171717171717171717171717171";
const SIGNING_KEY: &str = "8181818181818181818181818181818181818181818181818181818181818181";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_SEARCH_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_SEARCH_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn configured_search_is_inspectable_visible_and_queryable() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("search request");
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).expect("read request");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.contains("GET /search?q=provider+neutral&format=json"));
        let body = r#"{"results":[{"url":"https://example.test/result","title":"Normalized result","content":"Provider-neutral snippet","engine":"unit"}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("response");
    });
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_SEARCH_TEST_JOURNAL_KEY
    journal_key_id: search-test-journal-v1
    signing_variable: COLOSSUS_SEARCH_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo, web.search]
    exclude: []
  actions:
    allow: [web.search]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
search:
  profiles:
    local:
      kind: searxng
      endpoint: http://{address}/search
      credentialReference: null
      timeoutMs: 5000
  roles:
    agent: local
    research: local
agent:
  maxTurns: 4
sandbox:
  backend: native
  profile: search-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: [http://{address}]
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

    let profiles = run(binary, &config, directory.path(), &["search", "profiles"]);
    assert!(
        profiles.status.success(),
        "{}",
        String::from_utf8_lossy(&profiles.stderr)
    );
    let profiles: Value = serde_json::from_slice(&profiles.stdout).expect("profiles JSON");
    assert_eq!(profiles[0]["profile"], "local");
    assert_eq!(profiles[0]["provider"], "searxng");

    let tools = run(binary, &config, directory.path(), &["tools", "list"]);
    assert!(tools.status.success());
    let tools: Value = serde_json::from_slice(&tools.stdout).expect("tools JSON");
    assert!(
        tools
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "web.search"))
    );

    let query = run(
        binary,
        &config,
        directory.path(),
        &[
            "search",
            "query",
            "provider neutral",
            "--role",
            "agent",
            "--limit",
            "1",
        ],
    );
    assert!(
        query.status.success(),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    server.join().expect("server");
    let response: Value = serde_json::from_slice(&query.stdout).expect("search JSON");
    assert_eq!(response["count"], 1);
    assert_eq!(response["results"][0]["title"], "Normalized result");
    assert_eq!(response["results"][0]["rank"], 1);
}
