//! Credential-free end-to-end durable research CLI smoke test.

use serde_json::Value;
use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const SIGNING_KEY: &str = "6666666666666666666666666666666666666666666666666666666666666666";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_RESEARCH_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_RESEARCH_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn repository_research_crosses_gateway_and_reconstructs_citations() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    fs::write(
        directory.path().join("evidence.md"),
        "Audit records are chained and encrypted.\n",
    )
    .expect("evidence");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    let listener = TcpListener::bind("127.0.0.1:0").expect("search listener");
    listener.set_nonblocking(true).expect("nonblocking search");
    let search_address = listener.local_addr().expect("search address");
    let search_server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "search request timed out");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("search connection: {error}"),
            }
        };
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).expect("search request");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.contains("GET /search?q=audit&format=json"));
        let body = r#"{"results":[{"url":"https://example.test/audit","title":"External audit","content":"External evidence is policy released.","engine":"test"}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("search response");
    });
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_RESEARCH_TEST_JOURNAL_KEY
    journal_key_id: research-test-journal-v1
    signing_variable: COLOSSUS_RESEARCH_TEST_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [research.run, network.http]
  approval_actions: []
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
research:
  maxSources: 5
  maxWorkers: 2
  search:
    kind: searxng
    endpoint: http://{search_address}/search
    userAgent: colossus-test
sandbox:
  backend: native
  profile: research-test-v1
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
  networkDestinations: [http://{search_address}]
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
            search_address = search_address,
        ),
    )
    .expect("config");

    let output = run(
        binary,
        &config,
        directory.path(),
        &[
            "research", "run", "audit", "--depth", "quick", "--source", "repo,web",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    search_server.join().expect("search server");
    let research: Value = serde_json::from_slice(&output.stdout).expect("research JSON");
    assert_eq!(research["status"], "completed");
    assert_eq!(research["lanes"][0]["status"], "completed");
    assert_eq!(research["lanes"][0]["source_count"], 1);
    assert_eq!(research["lanes"][1]["status"], "completed");
    assert_eq!(research["lanes"][1]["source_count"], 1);
    assert!(
        research["report"]
            .as_str()
            .is_some_and(|report| report.contains("[R1]"))
    );
    let progress = research["progress"].as_array().expect("progress");
    assert!(
        progress
            .iter()
            .any(|item| { item["phase"] == "planning" && item["status"] == "fallback" })
    );
    assert!(
        progress
            .iter()
            .any(|item| { item["phase"] == "workers" && item["status"] == "fallback" })
    );
    assert!(
        progress
            .iter()
            .any(|item| { item["phase"] == "synthesis" && item["status"] == "fallback" })
    );
    let run_id = research["id"].as_str().expect("run id");
    let session_id = research["session_id"].as_str().expect("session id");

    let sources = run(
        binary,
        &config,
        directory.path(),
        &["research", "sources", run_id],
    );
    assert!(sources.status.success());
    let sources: Value = serde_json::from_slice(&sources.stdout).expect("sources JSON");
    assert_eq!(sources[0]["label"], "R1");
    assert_eq!(sources[0]["uri"], "evidence.md");
    assert_eq!(sources[1]["label"], "R2");
    assert_eq!(sources[1]["kind"], "web");
    assert_eq!(sources[1]["uri"], "https://example.test/audit");

    let telemetry = run(
        binary,
        &config,
        directory.path(),
        &["telemetry", "runs", "--session", session_id],
    );
    assert!(telemetry.status.success());
    let telemetry: Value = serde_json::from_slice(&telemetry.stdout).expect("telemetry JSON");
    assert_eq!(telemetry[0]["run_id"], run_id);
    assert!(
        telemetry[0]["research_events"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        !serde_json::to_string(&telemetry)
            .expect("telemetry serialization")
            .contains("Audit records are chained")
    );

    let verify = run(binary, &config, directory.path(), &["audit", "verify"]);
    assert!(verify.status.success());
    let verify: Value = serde_json::from_slice(&verify.stdout).expect("verify JSON");
    assert!(
        verify["event_count"]
            .as_u64()
            .is_some_and(|count| count >= 12)
    );
}
