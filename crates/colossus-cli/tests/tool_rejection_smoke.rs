//! Loopback-live proof that invalid or denied model tools never reach effect execution.

use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    io::{ErrorKind, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "4545454545454545454545454545454545454545454545454545454545454545";
const SIGNING_KEY: &str = "5656565656565656565656565656565656565656565656565656565656565656";

fn command(binary: &Path, config: &Path, directory: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(directory)
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_TOOL_REJECTION_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_TOOL_REJECTION_SIGNING_KEY", SIGNING_KEY);
    command
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4_096];
    loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert_ne!(count, 0, "client closed an incomplete request");
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or_default();
        if request.len() >= header_end + 4 + content_length {
            return String::from_utf8(request).expect("UTF-8 request");
        }
    }
}

fn respond_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("SSE response");
    stream.flush().expect("flush response");
}

fn tool_server(
    tool: &str,
    arguments: Value,
    final_answer: bool,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let address = listener.local_addr().expect("provider address");
    let tool_call = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "rejection-tool",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "rejection-call",
                        "type": "function",
                        "function": {
                            "name": tool,
                            "arguments": arguments.to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
    );
    let completed = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "rejection-recovered",
            "choices": [{
                "index": 0,
                "delta": {"content": "recovered"},
                "finish_reason": "stop"
            }]
        })
    );
    let expected = usize::from(final_answer) + 1;
    let task = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut requests = Vec::new();
        while requests.len() < expected && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("provider accept: {error}"),
            };
            stream
                .set_nonblocking(false)
                .expect("blocking provider stream");
            let request = read_request(&mut stream);
            respond_sse(
                &mut stream,
                if requests.is_empty() {
                    &tool_call
                } else {
                    &completed
                },
            );
            requests.push(request);
        }
        assert_eq!(requests.len(), expected, "provider request count");
        requests
    });
    (format!("http://{address}"), task)
}

fn write_config(
    directory: &Path,
    origin: &str,
    tool: &str,
    allow_tool_effect: bool,
) -> std::path::PathBuf {
    let workflows = directory.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let mut allow_actions = vec![json!("provider.openai.chat")];
    if allow_tool_effect {
        allow_actions.push(json!(if tool == "shell.run" {
            "shell.run"
        } else {
            "filesystem.write"
        }));
    }
    let config = directory.join("config.json");
    let executable = std::env::current_exe().expect("current test executable");
    let document = json!({
        "schemaVersion": 1,
        "storage": {
            "path": directory.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_TOOL_REJECTION_JOURNAL_KEY",
                "journal_key_id": "tool-rejection-journal-v1",
                "signing_variable": "COLOSSUS_TOOL_REJECTION_SIGNING_KEY",
                "anchor_path": directory.join("anchor.json")
            }
        },
        "policy": {
            "kind": "built_in",
            "allow_actions": allow_actions,
            "approval_actions": [],
            "require_post_effect": true
        },
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {
            "profiles": {
                "rejection": {
                    "kind": "open_ai_compatible",
                    "model": "tool-rejection-model",
                    "baseUrl": format!("{origin}/v1"),
                    "credentialReference": null,
                    "timeoutMs": 5000
                }
            },
            "roles": {"primary": "rejection"}
        },
        "agent": {"maxTurns": 3, "tools": [tool]},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "tool-rejection-v1",
            "allowBrokerFallback": false,
            "helperPath": null,
            "ociRuntime": null,
            "ociImage": null,
            "ociProxyImage": null,
            "filesystem": [{"root": directory, "mode": "write"}],
            "executables": [executable],
            "environment": [],
            "networkDestinations": [origin],
            "timeoutMs": 5000,
            "maxOutputBytes": 1048576,
            "maxProcesses": 2,
            "maxMemoryBytes": 67108864,
            "maxConcurrency": 1
        }
    });
    fs::write(
        &config,
        serde_json::to_vec_pretty(&document).expect("config JSON"),
    )
    .expect("write config");
    config
}

fn run_tool(
    binary: &Path,
    directory: &Path,
    config: &Path,
    full_access: bool,
) -> std::process::Output {
    let mut command = command(binary, config, directory);
    if full_access {
        command.args(["--approval-mode", "full-access"]);
    }
    command
        .args([
            "run",
            "Execute the requested tool exactly once.",
            "--stream",
            "--max-turns",
            "3",
        ])
        .output()
        .expect("agent run")
}

fn audit(binary: &Path, directory: &Path, config: &Path) -> Vec<Value> {
    let output = command(binary, config, directory)
        .args(["audit", "show", "--limit", "200"])
        .output()
        .expect("audit show");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("audit JSON")
}

fn tool_effect_stream_count(events: &[Value]) -> usize {
    events
        .iter()
        .filter_map(|event| {
            let stream_id = event["stream_id"].as_str()?;
            (stream_id.starts_with("effect:")
                && event["actor"]["id"].as_str() == Some("tool-call:rejection-call"))
            .then_some(stream_id)
        })
        .collect::<BTreeSet<_>>()
        .len()
}

fn denied_effect_events(events: &[Value]) -> Vec<&Value> {
    let stream_id = events
        .iter()
        .find(|event| {
            event["event_type"] == "effect.denied.v1"
                && event["actor"]["id"] == "tool-call:rejection-call"
        })
        .and_then(|event| event["stream_id"].as_str());
    stream_id.map_or_else(Vec::new, |stream_id| {
        events
            .iter()
            .filter(|event| event["stream_id"] == stream_id)
            .collect()
    })
}

#[test]
fn invalid_and_denied_tools_stop_before_effect_execution() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let root = tempdir().expect("root directory");

    let traversal_directory = root.path().join("traversal");
    fs::create_dir(&traversal_directory).expect("traversal directory");
    let escaped_path = root.path().join("escaped-must-not-exist.txt");
    let (origin, server) = tool_server(
        "filesystem.write",
        json!({
            "path": "../escaped-must-not-exist.txt",
            "content": "escape",
            "mode": "create"
        }),
        false,
    );
    let config = write_config(&traversal_directory, &origin, "filesystem.write", true);
    let output = run_tool(binary, &traversal_directory, &config, false);
    assert!(!output.status.success(), "traversal succeeded");
    assert!(!escaped_path.exists(), "traversal created an outside file");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("model filesystem paths must be workspace-relative"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.join().expect("traversal provider").len(), 1);
    let events = audit(binary, &traversal_directory, &config);
    assert_eq!(tool_effect_stream_count(&events), 0);
    assert!(denied_effect_events(&events).is_empty());

    let shell_directory = root.path().join("shell-wrapper");
    fs::create_dir(&shell_directory).expect("shell directory");
    let shell_marker = shell_directory.join("shell-must-not-exist.txt");
    let (origin, server) = tool_server(
        "shell.run",
        json!({
            "argv": ["sh", "-c", "echo escaped > shell-must-not-exist.txt"],
            "cwd": ".",
            "env": {}
        }),
        false,
    );
    let config = write_config(&shell_directory, &origin, "shell.run", true);
    let output = run_tool(binary, &shell_directory, &config, false);
    assert!(!output.status.success(), "shell wrapper succeeded");
    assert!(!shell_marker.exists(), "shell wrapper created a marker");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("shell wrapper execution is denied: sh"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.join().expect("shell provider").len(), 1);
    let events = audit(binary, &shell_directory, &config);
    assert_eq!(tool_effect_stream_count(&events), 0);
    assert!(denied_effect_events(&events).is_empty());

    let arguments_directory = root.path().join("unknown-arguments");
    fs::create_dir(&arguments_directory).expect("arguments directory");
    let arguments_marker = arguments_directory.join("arguments-must-not-exist.txt");
    let (origin, server) = tool_server(
        "filesystem.write",
        json!({
            "path": "arguments-must-not-exist.txt",
            "content": "invalid",
            "mode": "create",
            "unexpected": true
        }),
        true,
    );
    let config = write_config(&arguments_directory, &origin, "filesystem.write", true);
    let output = run_tool(binary, &arguments_directory, &config, false);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !arguments_marker.exists(),
        "unknown argument reached the adapter"
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "recovered");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("[file] start"));
    let requests = server.join().expect("arguments provider");
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("invalid_arguments"));
    assert!(requests[1].contains(r#""tool_call_id":"rejection-call""#));
    let events = audit(binary, &arguments_directory, &config);
    assert_eq!(tool_effect_stream_count(&events), 0);
    assert!(denied_effect_events(&events).is_empty());

    let policy_directory = root.path().join("policy-deny");
    fs::create_dir(&policy_directory).expect("policy directory");
    let policy_marker = policy_directory.join("policy-must-not-exist.txt");
    let (origin, server) = tool_server(
        "filesystem.write",
        json!({
            "path": "policy-must-not-exist.txt",
            "content": "denied",
            "mode": "create"
        }),
        false,
    );
    let config = write_config(&policy_directory, &origin, "filesystem.write", false);
    let output = run_tool(binary, &policy_directory, &config, true);
    assert!(
        !output.status.success(),
        "deterministic policy deny succeeded"
    );
    assert!(
        !policy_marker.exists(),
        "policy-denied write reached the adapter"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("denied by built-in default"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.join().expect("policy provider").len(), 1);
    let events = audit(binary, &policy_directory, &config);
    assert_eq!(tool_effect_stream_count(&events), 1);
    let effect_events = denied_effect_events(&events);
    assert!(!effect_events.is_empty(), "policy request was not audited");
    assert!(
        effect_events
            .iter()
            .any(|event| event["event_type"] == "effect.denied.v1")
    );
    assert!(
        effect_events
            .iter()
            .all(|event| event["event_type"] != "effect.started.v1")
    );
}
