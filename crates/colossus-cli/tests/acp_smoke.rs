//! Exercise the public ACP stdio boundary using an independent JSON-RPC client.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Read as _, Write},
    net::TcpListener,
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const JOURNAL_KEY: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const SIGNING_KEY: &str = "8888888888888888888888888888888888888888888888888888888888888888";

struct AgentProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    frames: Receiver<Result<Value, String>>,
    _home: process_support::IsolatedUserHome,
}

impl AgentProcess {
    fn launch(binary: &Path, workspace: &Path, config: &Path) -> Self {
        let mut command = Command::new(binary);
        let home = process_support::isolate_user_home(&mut command, workspace);
        let mut child = command
            .env("COLOSSUS_ACP_TEST_JOURNAL_KEY", JOURNAL_KEY)
            .env("COLOSSUS_ACP_TEST_SIGNING_KEY", SIGNING_KEY)
            .current_dir(workspace)
            .arg("--workspace")
            .arg(workspace)
            .arg("--config")
            .arg(config)
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("launch ACP agent");
        let stdin = child.stdin.take().expect("ACP stdin");
        let stdout = child.stdout.take().expect("ACP stdout");
        let (sender, frames) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let frame = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(frame).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            frames,
            _home: home,
        }
    }

    fn send(&mut self, frame: Value) {
        self.send_raw(&frame.to_string());
    }

    fn send_raw(&mut self, frame: &str) {
        let stdin = self.stdin.as_mut().expect("ACP stdin is open");
        writeln!(stdin, "{frame}").expect("write ACP frame");
        stdin.flush().expect("flush ACP frame");
    }

    fn next(&self) -> Value {
        self.frames
            .recv_timeout(Duration::from_secs(20))
            .expect("ACP response before timeout")
            .expect("stdout contains JSON-RPC frames only")
    }

    fn response(&self, id: u64) -> Value {
        loop {
            let frame = self.next();
            if frame["id"] == id {
                return frame;
            }
        }
    }

    fn finish(mut self) {
        self.stdin.take();
        for _ in 0..100 {
            if let Some(status) = self.child.try_wait().expect("wait for ACP agent") {
                assert!(status.success(), "ACP agent exited unsuccessfully");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("ACP agent did not exit after stdin closed");
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn acp_v1_stdio_streams_a_durable_echo_turn_and_rejects_unowned_roots() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("workspace");
    let workspace = directory.path();
    let config = workspace.join("config.yaml");
    let mut init = Command::new(binary);
    let _home = process_support::isolate_user_home(&mut init, workspace);
    let init = init
        .current_dir(workspace)
        .arg("--workspace")
        .arg(workspace)
        .arg("--config")
        .arg(&config)
        .args([
            "config",
            "init",
            "--development",
            "--storage-keys",
            "none",
            "--access-profile",
            "pinned",
            "--sandbox-profile",
            "offline-default",
        ])
        .output()
        .expect("create offline development config");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let mut agent = AgentProcess::launch(binary, workspace, &config);
    agent.send_raw("{");
    assert_eq!(agent.next()["error"]["code"], -32700);
    agent.send(json!({"jsonrpc":"2.0","id":8,"method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}));
    assert_eq!(agent.response(8)["error"]["code"], -32000);
    agent
        .send(json!({"jsonrpc":"2.0","id":9,"method":"initialize","params":{"protocolVersion":2}}));
    assert_eq!(agent.response(9)["result"]["protocolVersion"], 1);
    agent.send(json!({"jsonrpc":"2.0","id":10,"method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}));
    assert_eq!(agent.response(10)["error"]["code"], -32000);
    agent
        .send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}));
    let initialized = agent.response(1);
    assert_eq!(initialized["result"]["protocolVersion"], 1);
    assert_eq!(
        initialized["result"]["agentCapabilities"]["loadSession"],
        false
    );

    let outside = workspace.parent().expect("parent");
    agent.send(json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":outside,"mcpServers":[]}}));
    assert_eq!(agent.response(2)["error"]["code"], -32602);

    agent.send(json!({"jsonrpc":"2.0","id":3,"method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}));
    let session = agent.response(3)["result"]["sessionId"]
        .as_str()
        .expect("durable session id")
        .to_owned();

    agent.send(json!({"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"hello ACP"}]}}));
    let mut chunks = String::new();
    loop {
        let frame = agent.next();
        if frame["id"] == 4 {
            assert_eq!(frame["result"]["stopReason"], "end_turn", "{frame}");
            break;
        }
        assert_eq!(frame["method"], "session/update", "{frame}");
        if let Some(text) = frame["params"]["update"]["content"]["text"].as_str() {
            chunks.push_str(text);
        }
    }
    assert!(chunks.contains("hello ACP"), "chunks={chunks:?}");

    agent.send(json!({"jsonrpc":"2.0","id":7,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"cancel this turn"}]}}));
    agent.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}));
    assert_eq!(agent.response(7)["result"]["stopReason"], "cancelled");

    agent.send(json!({"jsonrpc":"2.0","id":5,"method":"session/load","params":{"sessionId":session,"cwd":workspace,"mcpServers":[]}}));
    assert_eq!(agent.response(5)["error"]["code"], -32601);

    agent.send(json!({"jsonrpc":"2.0","id":6,"method":"session/new","params":{"cwd":workspace,"additionalDirectories":[outside],"mcpServers":[]}}));
    assert_eq!(agent.response(6)["error"]["code"], -32602);
    agent.finish();

    let mut inspect = Command::new(binary);
    let _home = process_support::isolate_user_home(&mut inspect, workspace);
    let inspect = inspect
        .current_dir(workspace)
        .arg("--workspace")
        .arg(workspace)
        .arg("--config")
        .arg(&config)
        .args(["sessions", "show", &session])
        .output()
        .expect("inspect persisted ACP session");
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let record: Value = serde_json::from_slice(&inspect.stdout).expect("session JSON");
    assert_eq!(record["id"], session);

    let mut restarted = AgentProcess::launch(binary, workspace, &config);
    restarted
        .send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}));
    assert_eq!(restarted.response(1)["result"]["protocolVersion"], 1);
    restarted.send(json!({"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"sessionId":session,"prompt":[{"type":"text","text":"old session"}]}}));
    assert_eq!(restarted.response(2)["error"]["code"], -32602);
    restarted.finish();
}

fn read_http_request(stream: &mut std::net::TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("provider read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).expect("provider request");
        assert_ne!(count, 0, "provider request ended early");
        bytes.extend_from_slice(&buffer[..count]);
        let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or_default();
        if bytes.len() >= header_end + 4 + length {
            return;
        }
    }
}

fn provider_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let origin = format!(
        "http://{}",
        listener.local_addr().expect("provider address")
    );
    let tool_call = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "acp-tool",
            "choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": 0,
                "id": "acp-write",
                "type": "function",
                "function": {"name": "filesystem_write", "arguments": json!({
                    "path": "approved.txt", "content": "written after approval", "mode": "create"
                }).to_string()}
            }]}, "finish_reason": "tool_calls"}]
        })
    );
    let final_answer = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "acp-final",
            "choices": [{"index": 0, "delta": {"content": "finished"}, "finish_reason": "stop"}]
        })
    );
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        for index in 0..expected_requests {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("provider accept {index}: {error}"),
                }
            };
            read_http_request(&mut stream);
            let body = if index == 0 {
                &tool_call
            } else {
                &final_answer
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("provider response");
            stream.flush().expect("provider flush");
        }
    });
    (origin, handle)
}

fn write_approval_config(workspace: &Path, origin: &str) -> std::path::PathBuf {
    let config = workspace.join("config.json");
    let workflows = workspace.join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let document = json!({
        "schemaVersion": 3,
        "storage": {"path": workspace.join("state.redb"), "keys": {
            "kind": "environment",
            "journal_variable": "COLOSSUS_ACP_TEST_JOURNAL_KEY",
            "journal_key_id": "acp-test-journal-v1",
            "signing_variable": "COLOSSUS_ACP_TEST_SIGNING_KEY",
            "anchor_path": workspace.join("anchor.json")
        }},
        "access": {"profile": "pinned", "tools": {"include": ["filesystem.write"], "exclude": []},
            "actions": {"allow": ["provider.openai.chat"], "requireApproval": ["filesystem.write"], "deny": []}},
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {"profiles": {"loopback": {
            "kind": "open_ai_compatible", "baseUrl": format!("{origin}/v1"),
            "credentialReference": null, "timeoutMs": 5000
        }}},
        "models": {"profiles": {"loopback": {
            "providerProfile": "loopback", "model": "acp-test-model",
            "contextWindowTokens": 32768, "maxOutputTokens": 4096,
            "capabilities": {"toolCalls": true, "streaming": true}
        }}, "roles": {"primary": "loopback"}},
        "agent": {"maxTurns": 4},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native", "profile": "acp-test-v1", "allowBrokerFallback": false,
            "helperPath": null, "ociRuntime": null, "ociImage": null, "ociProxyImage": null,
            "filesystem": [{"root": workspace, "mode": "write"}], "executables": [],
            "environment": [], "networkDestinations": [origin], "timeoutMs": 5000,
            "maxOutputBytes": 1048576, "maxProcesses": 2, "maxMemoryBytes": 67108864,
            "maxConcurrency": 1
        }
    });
    fs::write(&config, serde_json::to_vec(&document).expect("config JSON")).expect("config");
    config
}

#[test]
fn acp_permission_response_is_bound_to_colossus_policy_and_tool_execution() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    for (option, should_write, cancel) in [
        ("reject-once", false, false),
        ("allow-once", true, false),
        ("cancelled", false, true),
    ] {
        let directory = tempdir().expect("workspace");
        let workspace = directory.path();
        let (origin, provider) = provider_server(if should_write { 2 } else { 1 });
        let config = write_approval_config(workspace, &origin);
        let mut agent = AgentProcess::launch(binary, workspace, &config);
        agent.send(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}),
        );
        assert_eq!(agent.response(1)["result"]["protocolVersion"], 1);
        agent.send(json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}));
        let session = agent.response(2)["result"]["sessionId"]
            .as_str()
            .expect("session id")
            .to_owned();
        agent.send(
            json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
                "sessionId":session,"prompt":[{"type":"text","text":"write approved.txt"}]
            }}),
        );
        let permission = loop {
            let frame = agent.next();
            if frame["method"] == "session/request_permission" {
                break frame;
            }
            assert_eq!(frame["method"], "session/update", "{frame}");
        };
        assert_eq!(permission["params"]["sessionId"], session);
        let details = permission["params"]["toolCall"]["content"].to_string();
        assert!(details.contains("filesystem.write"), "{details}");
        assert!(details.contains("approved.txt"), "{details}");
        if cancel {
            agent.send(
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}),
            );
            agent.send(json!({"jsonrpc":"2.0","id":permission["id"],"result":{
                "outcome":{"outcome":"cancelled"}
            }}));
        } else {
            agent.send(json!({"jsonrpc":"2.0","id":permission["id"],"result":{
                "outcome":{"outcome":"selected","optionId":option}
            }}));
        }
        let response = agent.response(3);
        if cancel {
            assert_eq!(response["result"]["stopReason"], "cancelled", "{response}");
            assert!(!workspace.join("approved.txt").exists());
        } else if should_write {
            assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
            assert_eq!(
                fs::read_to_string(workspace.join("approved.txt")).expect("approved file"),
                "written after approval"
            );
        } else {
            assert!(response.get("error").is_some(), "{response}");
            assert!(!workspace.join("approved.txt").exists());
        }
        agent.finish();
        provider.join().expect("provider thread");
    }
}
