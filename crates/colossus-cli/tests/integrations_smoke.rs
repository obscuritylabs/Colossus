//! Credential-free end-to-end OpenAPI connection and dynamic-tool smoke test.

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

const JOURNAL_KEY: &str = "9999999999999999999999999999999999999999999999999999999999999999";
const SIGNING_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn run(binary: &Path, config: &Path, workspace: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    let _isolated_home = process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .env("COLOSSUS_INTEGRATION_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_INTEGRATION_TEST_SIGNING_KEY", SIGNING_KEY)
        .output()
        .expect("run Colossus")
}

#[test]
fn openapi_connections_are_durable_hidden_until_connected_and_gateway_bound() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workspace = directory.path().canonicalize().expect("workspace");
    let workflows = workspace.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let origin = format!("http://{address}");
    let spec = workspace.join("openapi.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "openapi": "3.1.0",
            "info": {"title": "Local Demo"},
            "servers": [{"url": format!("{origin}/v1/")}],
            "paths": {
                "/status/{id}": {
                    "get": {
                        "operationId": "getStatus",
                        "parameters": [
                            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                            {"name": "verbose", "in": "query", "schema": {"type": "boolean"}}
                        ]
                    }
                }
            }
        }))
        .expect("spec JSON"),
    )
    .expect("spec");
    let state = workspace.join("state.redb");
    let anchor = workspace.join("anchor.json");
    let config = workspace.join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 3
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_INTEGRATION_TEST_JOURNAL_KEY
    journal_key_id: integration-test-journal-v1
    signing_variable: COLOSSUS_INTEGRATION_TEST_SIGNING_KEY
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
agent:
  maxTurns: 4
sandbox:
  backend: native
  profile: integration-test-v1
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
  networkDestinations: [{origin}]
  timeoutMs: 5000
  maxOutputBytes: 1048576
  maxProcesses: 4
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            workspace = workspace.display(),
        ),
    )
    .expect("config");

    let before = run(binary, &config, &workspace, &["tools", "list"]);
    assert!(before.status.success());
    assert!(!String::from_utf8_lossy(&before.stdout).contains("openapi.demo.getstatus"));

    let imported = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "import-openapi",
            "demo",
            "openapi.json",
            "--auth-type",
            "none",
        ],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let imported: Value = serde_json::from_slice(&imported.stdout).expect("import JSON");
    assert_eq!(imported["status"], "connected");
    assert_eq!(
        imported["operations"][0]["tool"]["name"],
        "openapi.demo.getstatus"
    );

    let pending = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "import-openapi",
            "pending",
            "openapi.json",
            "--credential-reference",
            "env:COLOSSUS_MISSING_INTEGRATION_TOKEN",
        ],
    );
    assert!(pending.status.success());
    let pending: Value = serde_json::from_slice(&pending.stdout).expect("pending JSON");
    assert_eq!(pending["status"], "pending_auth");
    assert_eq!(
        pending["credential_reference"],
        "env:COLOSSUS_MISSING_INTEGRATION_TOKEN"
    );

    let tools = run(binary, &config, &workspace, &["tools", "list"]);
    assert!(tools.status.success());
    assert!(String::from_utf8_lossy(&tools.stdout).contains("openapi.demo.getstatus"));
    assert!(!String::from_utf8_lossy(&tools.stdout).contains("openapi.pending.getstatus"));
    let listed = run(binary, &config, &workspace, &["integrations", "list"]);
    assert!(listed.status.success());
    let listed: Value = serde_json::from_slice(&listed.stdout).expect("list JSON");
    assert_eq!(listed[0]["credential_reference"], Value::Null);

    let denied = run(
        binary,
        &config,
        &workspace,
        &[
            "integrations",
            "call",
            "openapi.demo.getstatus",
            r#"{"id":"a b","verbose":true}"#,
        ],
    );
    assert!(!denied.status.success());
    listener.set_nonblocking(true).expect("nonblocking");
    assert!(
        listener.accept().is_err(),
        "denied call reached the network adapter"
    );
    listener.set_nonblocking(false).expect("blocking");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = vec![0_u8; 8_192];
        let read = stream.read(&mut request).expect("read request");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("GET /v1/status/a%20b?verbose=true HTTP/1.1"));
        let body = br#"{"connected":true}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .expect("headers");
        stream.write_all(body).expect("body");
    });
    let called = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "call",
            "openapi.demo.getstatus",
            r#"{"id":"a b","verbose":true}"#,
        ],
    );
    server.join().expect("server");
    assert!(
        called.status.success(),
        "{}",
        String::from_utf8_lossy(&called.stderr)
    );
    let called: Value = serde_json::from_slice(&called.stdout).expect("call JSON");
    assert_eq!(called["status_code"], 200);
    assert_eq!(called["result"]["connected"], true);

    let disconnected = run(
        binary,
        &config,
        &workspace,
        &[
            "--approval-mode",
            "full-access",
            "integrations",
            "disconnect",
            "demo",
        ],
    );
    assert!(disconnected.status.success());
    let tools = run(binary, &config, &workspace, &["tools", "list"]);
    assert!(tools.status.success());
    assert!(!String::from_utf8_lossy(&tools.stdout).contains("openapi.demo.getstatus"));
}
