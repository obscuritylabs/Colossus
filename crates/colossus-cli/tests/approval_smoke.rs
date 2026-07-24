//! Credential-free terminal approval-mode acceptance.

use serde_json::{Value, json};
use std::{
    fs,
    io::{ErrorKind, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

fn command(binary: &Path, config: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_APPROVAL_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_APPROVAL_TEST_SIGNING_KEY", SIGNING_KEY);
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
    relative_path: &str,
    expected_requests: usize,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let address = listener.local_addr().expect("provider address");
    let arguments = json!({
        "path": relative_path,
        "content": "approval-mode-write",
        "mode": "create"
    })
    .to_string();
    let tool_call = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "approval-tool",
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "approval-call",
                        "type": "function",
                        "function": {
                            "name": "filesystem.write",
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        })
    );
    let final_answer = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "approval-final",
            "choices": [{
                "index": 0,
                "delta": {"content": "tool-finished"},
                "finish_reason": "stop"
            }]
        })
    );
    let task = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut requests = Vec::new();
        while requests.len() < expected_requests && Instant::now() < deadline {
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
            let body = if requests.is_empty() {
                &tool_call
            } else {
                &final_answer
            };
            respond_sse(&mut stream, body);
            requests.push(request);
        }
        assert_eq!(requests.len(), expected_requests, "provider request count");
        requests
    });
    (format!("http://{address}"), task)
}

fn write_tool_config(
    directory: &Path,
    origin: &str,
    allow_write: bool,
    approve_write: bool,
) -> std::path::PathBuf {
    let workflows = directory.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let mut allow_actions = vec![json!("provider.openai.chat")];
    if allow_write {
        allow_actions.push(json!("filesystem.write"));
    }
    let approval_actions = approve_write
        .then_some(vec![json!("filesystem.write")])
        .unwrap_or_default();
    let config = directory.join("config.json");
    let document = json!({
        "schemaVersion": 2,
        "storage": {
            "path": directory.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_APPROVAL_TEST_JOURNAL_KEY",
                "journal_key_id": "approval-tool-journal-v1",
                "signing_variable": "COLOSSUS_APPROVAL_TEST_SIGNING_KEY",
                "anchor_path": directory.join("anchor.json")
            }
        },
        "access": {
            "profile": "pinned",
            "tools": {"include": ["filesystem.write"], "exclude": []},
            "actions": {
                "allow": allow_actions,
                "requireApproval": approval_actions,
                "deny": []
            }
        },
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {
            "profiles": {
                "loopback": {
                    "kind": "open_ai_compatible",
                    "baseUrl": format!("{origin}/v1"),
                    "credentialReference": null,
                    "timeoutMs": 5000
                }
            }
        },
        "models": {
            "profiles": {
                "loopback": {
                    "providerProfile": "loopback",
                    "model": "approval-tool-model",
                    "contextWindowTokens": 32768,
                    "maxOutputTokens": 4096,
                    "capabilities": {"toolCalls": true, "streaming": true}
                }
            },
            "roles": {"primary": "loopback"}
        },
        "agent": {"maxTurns": 4},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "approval-tool-v1",
            "allowBrokerFallback": false,
            "helperPath": null,
            "ociRuntime": null,
            "ociImage": null,
            "ociProxyImage": null,
            "filesystem": [{"root": directory, "mode": "write"}],
            "executables": [],
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

fn run_tool_scenario(
    binary: &Path,
    directory: &Path,
    config: &Path,
    mode: &str,
    approve_prompt: bool,
) -> std::process::Output {
    let mut process = command(binary, config);
    process
        .current_dir(directory)
        .args([
            "--approval-mode",
            mode,
            "run",
            "Write the requested file.",
            "--max-turns",
            "4",
        ])
        .stdin(if approve_prompt {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.spawn().expect("agent run");
    if approve_prompt {
        child
            .stdin
            .take()
            .expect("approval stdin")
            .write_all(b"yes\n")
            .expect("approve tool");
    }
    child.wait_with_output().expect("agent output")
}

#[test]
fn terminal_modes_deny_prompt_or_auto_prove_the_same_policy_obligation() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let workflows = directory.path().join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let config = directory.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_APPROVAL_TEST_JOURNAL_KEY
    journal_key_id: approval-test-journal-v1
    signing_variable: COLOSSUS_APPROVAL_TEST_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: []
    requireApproval: [provider.echo]
    deny: []
policy:
  kind: built_in
  require_post_effect: true
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
agent:
  maxTurns: 4
sandbox:
  backend: native
  profile: approval-test-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
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
        ),
    )
    .expect("config");

    let denied = command(binary, &config)
        .args(["echo", "denied"])
        .output()
        .expect("deny run");
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("operator declined"));

    let mut prompted = command(binary, &config);
    prompted
        .args(["--approval-mode", "ask", "echo", "prompted"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut prompted = prompted.spawn().expect("prompt run");
    prompted
        .stdin
        .take()
        .expect("prompt stdin")
        .write_all(b"yes\n")
        .expect("approve");
    let prompted = prompted.wait_with_output().expect("prompt result");
    assert!(prompted.status.success());
    assert_eq!(String::from_utf8_lossy(&prompted.stdout), "prompted\n");
    assert!(String::from_utf8_lossy(&prompted.stderr).contains("approval required"));

    let automatic = command(binary, &config)
        .args(["--approval-mode", "full-access", "echo", "automatic"])
        .output()
        .expect("automatic run");
    assert!(automatic.status.success());
    assert_eq!(String::from_utf8_lossy(&automatic.stdout), "automatic\n");
    assert!(!String::from_utf8_lossy(&automatic.stderr).contains("approval required"));

    let audit = command(binary, &config)
        .args(["audit", "show", "--limit", "100"])
        .output()
        .expect("audit");
    assert!(audit.status.success());
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let names = events
        .iter()
        .filter_map(|event| event.get("event_type").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(names.contains(&"approval.denied.v1"));
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "approval.granted.v1")
            .count(),
        2
    );
}

#[test]
fn every_terminal_mode_preserves_allowed_denied_and_approval_required_tool_semantics() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let root = tempdir().expect("root directory");
    let modes = ["deny", "ask", "risk-auto", "full-access"];

    for mode in modes {
        let directory = root.path().join(format!("allowed-{mode}"));
        fs::create_dir(&directory).expect("allowed directory");
        let relative_path = format!("allowed-{mode}.txt");
        let (origin, server) = tool_server(&relative_path, 2);
        let config = write_tool_config(&directory, &origin, true, false);
        let output = run_tool_scenario(binary, &directory, &config, mode, false);
        assert!(
            output.status.success(),
            "allowed/{mode}: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(directory.join(&relative_path)).expect("allowed write"),
            "approval-mode-write"
        );
        let requests = server.join().expect("allowed provider");
        assert!(requests[0].contains(r#""name":"filesystem.write""#));
        assert!(requests[1].contains(r#""tool_call_id":"approval-call""#));
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("approval required"),
            "allowed action prompted under {mode}"
        );
    }

    for mode in modes {
        let directory = root.path().join(format!("denied-{mode}"));
        fs::create_dir(&directory).expect("denied directory");
        let relative_path = format!("denied-{mode}.txt");
        let (origin, server) = tool_server(&relative_path, 1);
        let config = write_tool_config(&directory, &origin, false, false);
        let output = run_tool_scenario(binary, &directory, &config, mode, false);
        assert!(
            !output.status.success(),
            "deterministic deny was bypassed by {mode}"
        );
        assert!(
            !directory.join(&relative_path).exists(),
            "denied write executed under {mode}"
        );
        let requests = server.join().expect("denied provider");
        assert_eq!(requests.len(), 1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("denied by built-in default"),
            "{mode}: {stderr}"
        );
        assert!(
            !stderr.contains("approval required"),
            "deterministic deny prompted under {mode}: {stderr}"
        );
    }

    for mode in modes {
        let directory = root.path().join(format!("approval-{mode}"));
        fs::create_dir(&directory).expect("approval directory");
        let relative_path = format!("approval-{mode}.txt");
        let should_approve = mode != "deny";
        let (origin, server) = tool_server(&relative_path, usize::from(should_approve) + 1);
        let config = write_tool_config(&directory, &origin, false, true);
        let output = run_tool_scenario(
            binary,
            &directory,
            &config,
            mode,
            matches!(mode, "ask" | "risk-auto"),
        );
        assert_eq!(
            output.status.success(),
            should_approve,
            "approval/{mode}: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            directory.join(&relative_path).exists(),
            should_approve,
            "approval-required write state under {mode}"
        );
        let requests = server.join().expect("approval provider");
        assert_eq!(requests.len(), usize::from(should_approve) + 1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        match mode {
            "deny" => assert!(stderr.contains("operator declined"), "{stderr}"),
            "ask" => assert!(stderr.contains("approval required"), "{stderr}"),
            "risk-auto" => {
                assert!(stderr.contains("approval required"), "{stderr}");
                assert!(!stderr.contains("risk review:"), "{stderr}");
            }
            "full-access" => assert!(!stderr.contains("approval required"), "{stderr}"),
            _ => unreachable!("fixed approval mode"),
        }
    }
}
