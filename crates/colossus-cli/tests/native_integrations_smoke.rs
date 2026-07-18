//! End-to-end native GitHub, SearXNG, and OpenSearch connector smoke tests.

use serde_json::Value;
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::Duration,
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SIGNING_KEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(binary)
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_NATIVE_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_NATIVE_TEST_SIGNING_KEY", SIGNING_KEY)
        .env("COLOSSUS_NATIVE_GITHUB_TOKEN", "github-secret")
        .env("COLOSSUS_NATIVE_OPENSEARCH_USER", "search-user")
        .env("COLOSSUS_NATIVE_OPENSEARCH_PASSWORD", "search-password")
        .output()
        .expect("run Colossus")
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let count = stream.read(&mut buffer).expect("request read");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let header_end = header_end + 4;
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + content_length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("UTF-8 request")
}

fn respond(stream: &mut TcpStream, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("headers");
    stream.write_all(body).expect("body");
}

fn approved_connect(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> Value {
    let mut command = vec!["--approval-mode", "full-access", "integrations", "connect"];
    command.extend_from_slice(arguments);
    let output = run(binary, config, workspace, &command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("connection JSON")
}

#[test]
fn native_connectors_are_hidden_typed_credential_brokered_and_post_gated() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workspace = directory.path().canonicalize().expect("workspace");
    let workflows = workspace.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let github = TcpListener::bind("127.0.0.1:0").expect("GitHub listener");
    let searxng = TcpListener::bind("127.0.0.1:0").expect("SearXNG listener");
    let opensearch = TcpListener::bind("127.0.0.1:0").expect("OpenSearch listener");
    let github_origin = format!("http://{}", github.local_addr().expect("GitHub address"));
    let searxng_origin = format!("http://{}", searxng.local_addr().expect("SearXNG address"));
    let opensearch_origin = format!(
        "http://{}",
        opensearch.local_addr().expect("OpenSearch address")
    );
    let state = workspace.join("state.redb");
    let anchor = workspace.join("anchor.json");
    let config = workspace.join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_NATIVE_TEST_JOURNAL_KEY
    journal_key_id: native-test-journal-v1
    signing_variable: COLOSSUS_NATIVE_TEST_SIGNING_KEY
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
skills:
  enabled: true
  allowUserOverrides: false
  bundled: {missing}
  repository: {missing}
  user: {missing}
  disabled: []
agent:
  maxTurns: 4
sandbox:
  backend: native
  profile: native-integration-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: ["{github_origin}", "{searxng_origin}", "{opensearch_origin}"]
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            missing = workspace.join("missing").display(),
        ),
    )
    .expect("config");

    let pending = approved_connect(
        binary,
        &config,
        &workspace,
        &["github", "--base-url", &github_origin],
    );
    assert_eq!(pending["status"], "pending_auth");
    let tools = run(binary, &config, &workspace, &["tools", "list"]);
    assert!(!String::from_utf8_lossy(&tools.stdout).contains("github.repos"));

    let connected = approved_connect(
        binary,
        &config,
        &workspace,
        &[
            "github",
            "--base-url",
            &github_origin,
            "--credential-reference",
            "env:COLOSSUS_NATIVE_GITHUB_TOKEN",
        ],
    );
    assert_eq!(connected["status"], "connected");
    assert_eq!(connected["scopes"], serde_json::json!(["repo", "workflow"]));
    assert!(
        !serde_json::to_string(&connected)
            .expect("JSON")
            .contains("github-secret")
    );

    let searx_connection = approved_connect(
        binary,
        &config,
        &workspace,
        &[
            "searxng",
            "--base-url",
            &searxng_origin,
            "--auth-type",
            "none",
        ],
    );
    assert_eq!(searx_connection["status"], "connected");
    let search_connection = approved_connect(
        binary,
        &config,
        &workspace,
        &[
            "opensearch",
            "--base-url",
            &opensearch_origin,
            "--auth-type",
            "basic",
            "--username-reference",
            "env:COLOSSUS_NATIVE_OPENSEARCH_USER",
            "--password-reference",
            "env:COLOSSUS_NATIVE_OPENSEARCH_PASSWORD",
        ],
    );
    assert_eq!(search_connection["status"], "connected");
    assert_eq!(
        search_connection["credential_references"]["username"],
        "env:COLOSSUS_NATIVE_OPENSEARCH_USER"
    );
    let serialized = serde_json::to_string(&search_connection).expect("JSON");
    assert!(!serialized.contains("search-user"));
    assert!(!serialized.contains("search-password"));

    let tools = run(binary, &config, &workspace, &["tools", "list"]);
    let tools = String::from_utf8_lossy(&tools.stdout);
    for name in [
        "github.repos",
        "github.issues",
        "searxng.search",
        "opensearch.search",
        "opensearch.update_document",
    ] {
        assert!(tools.contains(name), "missing dynamic tool {name}");
    }

    let denied = run(
        binary,
        &config,
        &workspace,
        &[
            "integrations",
            "call",
            "github.repos",
            r#"{"visibility":"private","max_results":5}"#,
        ],
    );
    assert!(!denied.status.success());
    github.set_nonblocking(true).expect("nonblocking");
    assert!(
        github.accept().is_err(),
        "denied native call opened a socket"
    );
    github.set_nonblocking(false).expect("blocking");

    let github_server = thread::spawn(move || {
        let (mut stream, _) = github.accept().expect("GitHub accept");
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /user/repos?visibility=private&per_page=5 HTTP/1.1"));
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer github-secret"));
        assert!(lower.contains("x-github-api-version: 2022-11-28"));
        respond(&mut stream, br#"[{"name":"private-repo"}]"#);
    });
    let github_call = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "call",
            "github.repos",
            r#"{"visibility":"private","max_results":5}"#,
        ],
    );
    github_server.join().expect("GitHub server");
    assert!(
        github_call.status.success(),
        "{}",
        String::from_utf8_lossy(&github_call.stderr)
    );
    assert!(!String::from_utf8_lossy(&github_call.stdout).contains("github-secret"));

    let searx_server = thread::spawn(move || {
        let (mut stream, _) = searxng.accept().expect("SearXNG accept");
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /search?q=rust+agent&format=json HTTP/1.1"));
        respond(
            &mut stream,
            br#"{"results":[{"title":"One","url":"https://one.test","content":"First","engine":"demo"},{"title":"Two","url":"https://two.test","content":"Second"}]}"#,
        );
    });
    let searx_call = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "call",
            "searxng.search",
            r#"{"query":"rust agent","max_results":1}"#,
        ],
    );
    searx_server.join().expect("SearXNG server");
    assert!(
        searx_call.status.success(),
        "{}",
        String::from_utf8_lossy(&searx_call.stderr)
    );
    let searx_call: Value = serde_json::from_slice(&searx_call.stdout).expect("SearXNG JSON");
    assert_eq!(searx_call["result"]["count"], 1);
    assert_eq!(
        searx_call["result"]["results"][0]["metadata"]["engine"],
        "demo"
    );

    let search_server = thread::spawn(move || {
        let (mut stream, _) = opensearch.accept().expect("OpenSearch accept");
        let request = read_request(&mut stream);
        assert!(request.starts_with("POST /notes/_update/doc-1?refresh=wait_for HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: basic c2vhcmnolxvzzxi6c2vhcmnolxbhc3n3b3jk")
        );
        let body = request.split_once("\r\n\r\n").expect("body").1;
        let body: Value = serde_json::from_str(body).expect("body JSON");
        assert_eq!(body["doc"]["status"], "done");
        assert_eq!(body["doc_as_upsert"], true);
        respond(&mut stream, br#"{"result":"updated"}"#);
    });
    let search_call = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "call",
            "opensearch.update_document",
            r#"{"index":"notes","id":"doc-1","doc":{"status":"done"},"doc_as_upsert":true,"refresh":"wait_for"}"#,
        ],
    );
    search_server.join().expect("OpenSearch server");
    assert!(
        search_call.status.success(),
        "{}",
        String::from_utf8_lossy(&search_call.stderr)
    );
    let search_call: Value = serde_json::from_slice(&search_call.stdout).expect("OpenSearch JSON");
    assert_eq!(search_call["result"]["result"], "updated");
}
