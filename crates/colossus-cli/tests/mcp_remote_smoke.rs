//! Remote MCP diagnostics and opt-in credential-free public server acceptance.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

#[path = "support/process.rs"]
mod process_support;

use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn run(config: &Path, arguments: &[&str]) -> std::process::Output {
    let workspace = config.parent().expect("workspace");
    let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
    let _home = process_support::isolate_user_home(&mut command, workspace);
    command
        .current_dir(workspace)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

#[test]
#[ignore = "requires public HTTPS access to docs.mcp.cloudflare.com"]
fn cloudflare_docs_discovery_and_call_require_stateless_opt_in() {
    let directory = process_support::tempdir().expect("private workspace");
    let workspace = directory.path().canonicalize().expect("workspace");
    let config = workspace.join("config.yaml");
    let mut settings = json!({
        "schemaVersion": 3,
        "storage": { "path": workspace.join("state.redb"), "keys": {"kind": "none"} },
        "access": {"profile": "development"},
        "mcp": {"servers": {"cloudflare-docs": {
            "transport": "streamable_http",
            "url": "https://docs.mcp.cloudflare.com/mcp",
            "allowedTools": ["search_cloudflare_documentation"],
            "allowStateless": false,
            "timeoutMs": 30000,
            "maxOutputBytes": 1048576
        }}},
        "sandbox": {
            "backend": if cfg!(windows) { "windows_job" } else { "native" },
            "networkDestinations": ["https://docs.mcp.cloudflare.com"],
            "timeoutMs": 30000,
            "maxOutputBytes": 1048576
        }
    });
    fs::write(&config, settings.to_string()).expect("configuration");
    let rejected = run(&config, &["mcp", "tools"]);
    assert!(
        !rejected.status.success(),
        "stateless server requires opt-in"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("initialization failed"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    settings["mcp"]["servers"]["cloudflare-docs"]["allowStateless"] = json!(true);
    fs::write(&config, settings.to_string()).expect("stateless configuration");
    let discovered = run(&config, &["mcp", "tools"]);
    assert!(
        discovered.status.success(),
        "{}",
        String::from_utf8_lossy(&discovered.stderr)
    );
    let tools: Value = serde_json::from_slice(&discovered.stdout).expect("discovered tools");
    assert_eq!(tools[0]["name"], "search_cloudflare_documentation");

    let called = run(
        &config,
        &[
            "--approval-mode",
            "full-access",
            "mcp",
            "call",
            "cloudflare-docs",
            "search_cloudflare_documentation",
            r#"{"query":"What are Cloudflare Workers?"}"#,
        ],
    );
    assert!(
        called.status.success(),
        "{}",
        String::from_utf8_lossy(&called.stderr)
    );
    let result: Value = serde_json::from_slice(&called.stdout).expect("tool result");
    assert_eq!(result["server"], "cloudflare-docs");
    assert_eq!(result["tool"], "search_cloudflare_documentation");
    assert_ne!(result["result"]["isError"], json!(true), "tool succeeded");
    assert!(
        result["result"]["content"]
            .as_array()
            .expect("content blocks")
            .iter()
            .any(|block| block["text"]
                .as_str()
                .is_some_and(|text| text.contains("Workers"))),
        "expected documentation result"
    );

    let audit = run(&config, &["audit", "verify"]);
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
}

#[test]
fn mcp_doctor_preserves_http_failure_and_policy_denial_without_remote_payloads() {
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        time::Duration,
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("local server");
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < Duration::from_secs(30) =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("health check did not reach fixture: {error}"),
            }
        };
        // Winsock can carry the listener's nonblocking mode onto accepted streams.
        // Read requests with the bounded blocking timeout below.
        stream
            .set_nonblocking(false)
            .expect("blocking fixture stream");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
        }
        let header_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = std::str::from_utf8(&request[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        assert!(content_length < 64 * 1024);
        while request.len() < header_end + content_length {
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
        }
        stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer secret-challenge\r\nContent-Length: 11\r\nConnection: close\r\n\r\nsecret-body").unwrap();
    });
    let directory = process_support::tempdir().unwrap();
    let workspace = directory.path().canonicalize().unwrap();
    let config = workspace.join("config.yaml");
    let origin = format!("http://{address}");
    let mut settings = json!({
        "schemaVersion": 3,
        "storage": {"path": workspace.join("state.redb"), "keys": {"kind": "none"}},
        "access": {"profile": "development"},
        "policy": {"kind": "built_in", "require_post_effect": false},
        "mcp": {"servers": {"fixture": {
            "transport": "streamable_http", "url": format!("{origin}/mcp"),
            "allowedTools": ["*"], "timeoutMs": 10000, "maxOutputBytes": 1048576
        }}},
        "sandbox": {
            "backend": if cfg!(windows) {"windows_job"} else {"native"},
            "networkDestinations": [origin], "timeoutMs": 10000, "maxOutputBytes": 1048576
        }
    });
    fs::write(&config, settings.to_string()).unwrap();
    let output = run(&config, &["mcp", "doctor", "fixture"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().unwrap();
    let check: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(check["report"]["healthy"], false);
    assert_eq!(check["report"]["stage"], "initialize");
    assert_eq!(check["report"]["failure"]["httpStatus"], 401);
    assert_eq!(check["report"]["configuration"]["directHttp"], true);
    assert_eq!(check["tools"], json!([]));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&address.to_string()));

    settings["access"]["actions"] = json!({"deny": ["mcp.tools"]});
    fs::write(&config, settings.to_string()).unwrap();
    let output = run(&config, &["mcp", "doctor", "fixture"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let check: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(check["report"]["failure"]["code"], "policy");
    assert!(check["report"]["failure"]["httpStatus"].is_null());
}
