//! Loopback-live provider terminal, credential, worker, and tool-loop acceptance.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const JOURNAL_KEY: &str = "abababababababababababababababababababababababababababababababab";
const SIGNING_KEY: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn command(binary: &Path, config: &Path) -> process_support::IsolatedCommand {
    let mut command = Command::new(binary);
    let isolated_home = process_support::isolate_user_home(
        &mut command,
        config.parent().expect("provider test workspace"),
    );
    command
        .current_dir(config.parent().expect("provider test workspace"))
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY", SIGNING_KEY)
        .env("COLOSSUS_PROVIDER_TERMINAL_API_KEY", "terminal-secret");
    process_support::IsolatedCommand::new(command, isolated_home)
}

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    command(binary, config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

fn write_failure_config(directory: &Path, origin: &str, tool: &str) -> std::path::PathBuf {
    let workflows = directory.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.join("config.json");
    let document = json!({
        "schemaVersion": 2,
        "storage": {
            "path": directory.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY",
                "journal_key_id": "provider-failure-journal-v1",
                "signing_variable": "COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY",
                "anchor_path": directory.join("anchor.json")
            }
        },
        "access": {
            "profile": "pinned",
            "tools": {"include": [tool], "exclude": []},
            "actions": {
                "allow": ["provider.openai.chat", "filesystem.write"],
                "requireApproval": [],
                "deny": []
            }
        },
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {
            "profiles": {
                "failure": {
                    "kind": "open_ai_compatible",
                    "baseUrl": format!("{origin}/v1"),
                    "credentialReference": null,
                    "timeoutMs": 5000
                }
            }
        },
        "models": {
            "profiles": {
                "failure": {
                    "providerProfile": "failure",
                    "model": "failure-model",
                    "contextWindowTokens": 32768,
                    "maxOutputTokens": 4096,
                    "capabilities": {"toolCalls": true, "streaming": true}
                }
            },
            "roles": {"primary": "failure"}
        },
        "agent": {"maxTurns": 4},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "provider-failure-v1",
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

fn failed_run(
    binary: &Path,
    directory: &Path,
    config: &Path,
    max_turns: &str,
) -> std::process::Output {
    command(binary, config)
        .current_dir(directory)
        .args([
            "run",
            "Exercise one terminal failure classification.",
            "--stream",
            "--max-turns",
            max_turns,
        ])
        .output()
        .expect("failed run")
}

fn audited_run_events(binary: &Path, config: &Path) -> Vec<Value> {
    let audit = run(binary, config, &["audit", "show", "--limit", "200"]);
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let stream_id = events
        .iter()
        .filter_map(|event| event["stream_id"].as_str())
        .find(|stream_id| stream_id.starts_with("run:"))
        .expect("run stream")
        .to_owned();
    events
        .into_iter()
        .filter(|event| event["stream_id"] == stream_id)
        .collect()
}

fn event_count(events: &[Value], event_type: &str) -> usize {
    events
        .iter()
        .filter(|event| event["event_type"] == event_type)
        .count()
}

fn wait_for_worker(binary: &Path, config: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if run(binary, config, &["worker", "--status"])
            .status
            .success()
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("worker did not become ready");
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().expect("worker status").is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(child.try_wait().expect("worker status").is_some());
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
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
    let headers = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).expect("headers");
    for chunk in body.as_bytes().chunks(23) {
        stream.write_all(chunk).expect("SSE chunk");
    }
    stream.flush().expect("flush response");
}

fn respond_json(stream: &mut TcpStream, status: &str, body: &str) {
    let headers = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).expect("headers");
    stream.write_all(body.as_bytes()).expect("JSON body");
    stream.flush().expect("flush response");
}

fn doctor_server(
    generation_status: &'static str,
    generation_body: &'static str,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        let (mut catalog, _) = listener.accept().expect("catalog accept");
        let catalog_request = read_request(&mut catalog);
        respond_json(
            &mut catalog,
            "200 OK",
            r#"{"data":[{"id":"terminal-model","object":"model","owned_by":"test"}]}"#,
        );

        let (mut generation, _) = listener.accept().expect("generation accept");
        let generation_request = read_request(&mut generation);
        respond_json(&mut generation, generation_status, generation_body);
        vec![catalog_request, generation_request]
    });
    (format!("http://{address}"), task)
}

fn write_doctor_config(directory: &Path, origin: &str) -> std::path::PathBuf {
    let workflows = directory.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.join("config.json");
    let document = json!({
        "schemaVersion": 2,
        "storage": {
            "path": directory.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY",
                "journal_key_id": "provider-doctor-journal-v1",
                "signing_variable": "COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY",
                "anchor_path": directory.join("anchor.json")
            }
        },
        "access": {
            "profile": "pinned",
            "tools": {"include": [], "exclude": []},
            "actions": {
                "allow": ["provider.models", "provider.openai.chat"],
                "requireApproval": [],
                "deny": []
            }
        },
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {
            "profiles": {
                "live": {
                    "kind": "open_ai_compatible",
                    "baseUrl": format!("{origin}/v1"),
                    "credentialReference": "env:COLOSSUS_PROVIDER_TERMINAL_API_KEY",
                    "timeoutMs": 10000
                }
            }
        },
        "models": {
            "profiles": {
                "live": {
                    "providerProfile": "live",
                    "model": "terminal-model",
                    "contextWindowTokens": 32768,
                    "maxOutputTokens": 4096,
                    "capabilities": {"toolCalls": true, "streaming": true}
                }
            },
            "roles": {"primary": "live"}
        },
        "agent": {"maxTurns": 4},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "provider-doctor-v1",
            "allowBrokerFallback": false,
            "helperPath": null,
            "ociRuntime": null,
            "ociImage": null,
            "ociProxyImage": null,
            "filesystem": [],
            "executables": [],
            "environment": [],
            "networkDestinations": [origin],
            "timeoutMs": 10000,
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

fn write_provider_timeout_config(directory: &Path, origin: &str) -> std::path::PathBuf {
    let workflows = directory.join("workflows");
    fs::create_dir_all(&workflows).expect("workflows");
    let config = directory.join("config.json");
    let document = json!({
        "schemaVersion": 2,
        "storage": {
            "path": directory.join("state.redb"),
            "keys": {
                "kind": "environment",
                "journal_variable": "COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY",
                "journal_key_id": "provider-timeout-journal-v1",
                "signing_variable": "COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY",
                "anchor_path": directory.join("anchor.json")
            }
        },
        "access": {
            "profile": "pinned",
            "tools": {"include": [], "exclude": []},
            "actions": {
                "allow": ["provider.openai.chat"],
                "requireApproval": [],
                "deny": []
            }
        },
        "policy": {"kind": "built_in", "require_post_effect": true},
        "workflows": {"repository": workflows, "user": workflows},
        "providers": {
            "profiles": {
                "live": {
                    "kind": "open_ai_compatible",
                    "baseUrl": format!("{origin}/v1"),
                    "credentialReference": null,
                    "timeoutMs": 500
                }
            }
        },
        "models": {
            "profiles": {
                "live": {
                    "providerProfile": "live",
                    "model": "terminal-model",
                    "contextWindowTokens": 32768,
                    "maxOutputTokens": 4096,
                    "capabilities": {"toolCalls": true, "streaming": true}
                }
            },
            "roles": {"primary": "live"}
        },
        "agent": {"maxTurns": 2},
        "subagents": {"maxConcurrent": 1},
        "sandbox": {
            "backend": "native",
            "profile": "provider-timeout-v1",
            "allowBrokerFallback": false,
            "helperPath": null,
            "ociRuntime": null,
            "ociImage": null,
            "ociProxyImage": null,
            "filesystem": [],
            "executables": [],
            "environment": [],
            "networkDestinations": [origin],
            "timeoutMs": 10,
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

fn sse_server(responses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        responses
            .into_iter()
            .map(|body| {
                let (mut stream, _) = listener.accept().expect("provider accept");
                let request = read_request(&mut stream);
                respond_sse(&mut stream, body);
                request
            })
            .collect()
    });
    (format!("http://{address}"), task)
}

fn status_server(status: &'static str, body: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider accept");
        let request = read_request(&mut stream);
        respond_json(&mut stream, status, body);
        request
    });
    (format!("http://{address}"), task)
}

fn delayed_sse_server(delay: Duration, body: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider accept");
        let request = read_request(&mut stream);
        thread::sleep(delay);
        respond_sse(&mut stream, body);
        request
    });
    (format!("http://{address}"), task)
}

fn request_body(request: &str) -> Value {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .expect("provider request body");
    serde_json::from_str(body).expect("provider request JSON")
}

fn subagent_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let task = thread::spawn(move || {
        let mut requests = Vec::new();

        let (mut parent_delegate, _) = listener.accept().expect("parent delegate accept");
        requests.push(read_request(&mut parent_delegate));
        let delegate = json!({
            "id": "chat-parent-delegate",
            "choices": [{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": 0,
                    "id": "call-delegate",
                    "type": "function",
                    "function": {
                        "name": "agent_delegate",
                        "arguments": "{\"task\":\"Say hi to Alex and confirm the ping.\"}"
                    }
                }]},
                "finish_reason": "tool_calls"
            }]
        });
        respond_sse(
            &mut parent_delegate,
            &format!("data: {delegate}\n\ndata: [DONE]\n\n"),
        );

        let (mut child, _) = listener.accept().expect("child accept");
        let child_request = read_request(&mut child);
        assert!(child_request.contains("durable Colossus child agent"));
        requests.push(child_request);
        let child_answer = r#"data: {"id":"chat-child-final","choices":[{"index":0,"delta":{"content":"Hi, Alex! Ping received."},"finish_reason":"stop"}]}

data: [DONE]

"#;
        respond_sse(&mut child, child_answer);

        let (mut parent_result, _) = listener.accept().expect("parent result accept");
        let parent_result_request = read_request(&mut parent_result);
        let body = request_body(&parent_result_request);
        let child_id = body["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter(|message| message["role"] == "tool")
            .filter_map(|message| message["content"].as_str())
            .filter_map(|content| serde_json::from_str::<Value>(content).ok())
            .find_map(|result| result["id"].as_str().map(str::to_owned))
            .expect("delegated child id");
        requests.push(parent_result_request);
        let result_arguments = json!({"id": child_id}).to_string();
        let result_call = json!({
            "id": "chat-parent-result",
            "choices": [{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": 0,
                    "id": "call-result",
                    "type": "function",
                    "function": {
                        "name": "agent_result",
                        "arguments": result_arguments
                    }
                }]},
                "finish_reason": "tool_calls"
            }]
        });
        respond_sse(
            &mut parent_result,
            &format!("data: {result_call}\n\ndata: [DONE]\n\n"),
        );

        let (mut parent_final, _) = listener.accept().expect("parent final accept");
        let parent_final_request = read_request(&mut parent_final);
        assert!(parent_final_request.contains("Hi, Alex! Ping received."));
        requests.push(parent_final_request);
        let final_answer = r#"data: {"id":"chat-parent-final","choices":[{"index":0,"delta":{"content":"The subagent said hi."},"finish_reason":"stop"}]}

data: [DONE]

"#;
        respond_sse(&mut parent_final, final_answer);

        requests
    });
    (format!("http://{address}"), task)
}

fn live_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let first_tool_call = r#"data: {"id":"chat-tool-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"echo","arguments":"{\"text\":\"terminal-tool-one\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let second_tool_call = r#"data: {"id":"chat-tool-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-2","type":"function","function":{"name":"echo","arguments":"{\"text\":\"terminal-tool-two\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let final_answer = r#"data: {"id":"chat-final","choices":[{"index":0,"delta":{"content":"con"},"finish_reason":null}]}

data: {"id":"chat-final","choices":[{"index":0,"delta":{"content":"nected"},"finish_reason":"stop"}]}

data: {"id":"chat-final","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":2,"total_tokens":13}}

data: [DONE]

"#;
    let terminal_answer = r#"data: {"id":"chat-terminal","choices":[{"index":0,"delta":{"content":"terminal-"},"finish_reason":null}]}

data: {"id":"chat-terminal","choices":[{"index":0,"delta":{"content":"connected"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let worker_answer = r#"data: {"id":"chat-worker","choices":[{"index":0,"delta":{"content":"worker-"},"finish_reason":null}]}

data: {"id":"chat-worker","choices":[{"index":0,"delta":{"content":"connected"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    sse_server(vec![
        first_tool_call,
        second_tool_call,
        final_answer,
        terminal_answer,
        worker_answer,
    ])
}

fn responses_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let tool_call = r#"data: {"type":"response.created","response":{"id":"resp-tool"}}

data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"call-r","name":"echo","arguments":"{\"text\":\"responses-tool terminal-secret\"}"}}

data: {"type":"response.completed","response":{"id":"resp-tool","status":"completed","output":[{"type":"function_call","call_id":"call-r","name":"echo","arguments":"{\"text\":\"responses-tool terminal-secret\"}"}],"usage":{"input_tokens":9,"output_tokens":1,"total_tokens":10}}}

data: [DONE]

"#;
    let final_answer = r#"data: {"type":"response.created","response":{"id":"resp-final"}}

data: {"type":"response.output_text.delta","delta":"responses-"}

data: {"type":"response.output_text.delta","delta":"connected"}

data: {"type":"response.output_text.delta","delta":" terminal-secret"}

data: {"type":"response.completed","response":{"id":"resp-final","status":"completed","output":[],"usage":{"input_tokens":12,"output_tokens":2,"total_tokens":14}}}

data: [DONE]

"#;
    sse_server(vec![tool_call, final_answer])
}

fn malformed_arguments_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let first_invalid = r#"data: {"id":"invalid-tool-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"invalid-call-1","type":"function","function":{"name":"filesystem_write","arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"first\",\"mode\":\"create\""}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let second_invalid = r#"data: {"id":"invalid-tool-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"invalid-call-2","type":"function","function":{"name":"filesystem_write","arguments":"[\"must-not-exist.txt\"]"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let recovered = r#"data: {"id":"recovered-final","choices":[{"index":0,"delta":{"content":"recovered"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    sse_server(vec![first_invalid, second_invalid, recovered])
}

fn max_turn_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let first = r#"data: {"id":"max-turn-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"max-call-1","type":"function","function":{"name":"echo","arguments":"{\"text\":\"first\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let second = r#"data: {"id":"max-turn-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"max-call-2","type":"function","function":{"name":"echo","arguments":"{\"text\":\"second\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    sse_server(vec![first, second])
}

fn empty_output_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let empty = r#"data: {"id":"empty-output","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]

"#;
    sse_server(vec![empty])
}

fn malformed_exhaustion_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let first = r#"data: {"id":"malformed-exhaust-1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"malformed-exhaust-call-1","type":"function","function":{"name":"filesystem_write","arguments":"{\"path\":\"exhausted-must-not-exist.txt\""}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let second = r#"data: {"id":"malformed-exhaust-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"malformed-exhaust-call-2","type":"function","function":{"name":"filesystem_write","arguments":"[]"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    let third = r#"data: {"id":"malformed-exhaust-3","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"malformed-exhaust-call-3","type":"function","function":{"name":"filesystem_write","arguments":"not-json"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;
    sse_server(vec![first, second, third])
}

#[test]
fn provider_doctor_does_not_treat_a_public_catalog_as_credential_readiness() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let (origin, server) = doctor_server(
        "401 Unauthorized",
        r#"{"error":{"message":"invalid credential"}}"#,
    );
    let config = write_doctor_config(directory.path(), &origin);

    let output = run(binary, &config, &["provider", "doctor", "live"]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let provider_readiness: Value =
        serde_json::from_slice(&output.stdout).expect("provider readiness JSON");
    assert_eq!(provider_readiness["ready"], true);
    assert_eq!(provider_readiness["checks"][0]["name"], "models_endpoint");
    assert_eq!(provider_readiness["checks"][0]["status"], "pass");

    let output = run(binary, &config, &["models", "doctor", "live"]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let model_readiness: Value =
        serde_json::from_slice(&output.stdout).expect("model readiness JSON");
    assert_eq!(model_readiness["ready"], false);
    assert_eq!(model_readiness["checks"][0]["name"], "metadata");
    assert_eq!(model_readiness["checks"][0]["status"], "pass");
    assert_eq!(model_readiness["checks"][1]["name"], "generation");
    assert_eq!(model_readiness["checks"][1]["status"], "fail");
    assert!(
        model_readiness["checks"][1]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("HTTP 401")),
        "{model_readiness}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("invalid credential"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("terminal-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("terminal-secret"));

    let requests = server.join().expect("doctor server");
    assert!(requests[0].starts_with("GET /v1/models "));
    assert!(requests[1].starts_with("POST /v1/chat/completions "));
    assert!(requests[1].contains(r#""name":"colossus_readiness""#));
    assert!(!requests[1].contains(r#""maxLength""#));
    assert!(
        requests
            .iter()
            .all(|request| request.contains("authorization: Bearer terminal-secret"))
    );
}

#[test]
fn model_doctor_can_release_bounded_redacted_provider_response_diagnostics() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let response = format!(
        "tool schema rejected terminal-secret\n{}",
        "x".repeat(20 * 1024)
    );
    let response = Box::leak(response.into_boxed_str());
    let (origin, server) = doctor_server("400 Bad Request", response);
    let config = write_doctor_config(directory.path(), &origin);

    let worker = command(binary, &config)
        .arg("worker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start worker");
    let mut worker = ChildGuard(worker);
    wait_for_worker(binary, &config);

    let catalog = run(binary, &config, &["provider", "doctor", "live"]);
    assert!(
        catalog.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&catalog.stdout),
        String::from_utf8_lossy(&catalog.stderr)
    );

    let output = run(
        binary,
        &config,
        &["models", "doctor", "live", "--include-provider-response"],
    );
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let readiness: Value = serde_json::from_slice(&output.stdout).expect("model readiness JSON");
    let diagnostic = &readiness["checks"][1]["provider_response"];
    assert_eq!(readiness["ready"], false);
    assert_eq!(diagnostic["request_method"], "POST");
    assert_eq!(
        diagnostic["request_url"],
        format!("{origin}/v1/chat/completions")
    );
    assert_eq!(
        diagnostic["request_body"]["tools"][0]["function"]["name"],
        "colossus_readiness"
    );
    assert_eq!(diagnostic["status"], 400);
    assert_eq!(diagnostic["content_type"], "application/json");
    assert_eq!(diagnostic["body_encoding"], "utf8");
    assert_eq!(diagnostic["body_truncated"], true);
    let body = diagnostic["body"].as_str().expect("diagnostic body");
    assert!(body.starts_with("tool schema rejected [REDACTED]"));
    assert!(body.len() <= 16 * 1024);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("terminal-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("terminal-secret"));

    let requests = server.join().expect("doctor server");
    assert!(requests[0].starts_with("GET /v1/models "));
    assert!(requests[1].starts_with("POST /v1/chat/completions "));
    assert!(
        run(binary, &config, &["worker", "--shutdown"])
            .status
            .success()
    );
    wait_for_exit(&mut worker.0);
}

#[test]
fn tui_model_doctor_releases_bounded_redacted_provider_response_diagnostics() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let (origin, server) = doctor_server("400 Bad Request", "tool schema rejected terminal-secret");
    let config = write_doctor_config(directory.path(), &origin);

    let worker = command(binary, &config)
        .arg("worker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start worker");
    let mut worker = ChildGuard(worker);
    wait_for_worker(binary, &config);

    let catalog = run(binary, &config, &["provider", "doctor", "live"]);
    assert!(
        catalog.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&catalog.stdout),
        String::from_utf8_lossy(&catalog.stderr)
    );

    let mut terminal = command(binary, &config);
    terminal
        .args(["--output", "json", "tui"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut terminal = terminal.spawn().expect("spawn terminal line runner");
    terminal
        .stdin
        .take()
        .expect("terminal stdin")
        .write_all(b"/models doctor live\n/exit\n")
        .expect("write terminal script");
    let terminal = terminal.wait_with_output().expect("terminal output");
    assert!(
        terminal.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&terminal.stdout),
        String::from_utf8_lossy(&terminal.stderr)
    );
    let stdout = String::from_utf8_lossy(&terminal.stdout);
    let stderr = String::from_utf8_lossy(&terminal.stderr);
    assert!(
        stdout.contains("tool schema rejected [REDACTED]"),
        "{stdout}"
    );
    assert!(stdout.contains("/v1/chat/completions"), "{stdout}");
    assert!(stdout.contains("\"status\": 400"), "{stdout}");
    assert!(!stdout.contains("terminal-secret"), "{stdout}");
    assert!(!stderr.contains("terminal-secret"), "{stderr}");
    assert!(!stdout.contains("\x1b["), "{stdout}");

    let requests = server.join().expect("doctor server");
    assert!(requests[0].starts_with("GET /v1/models "));
    assert!(requests[1].starts_with("POST /v1/chat/completions "));
    assert!(requests[1].contains(r#""name":"colossus_readiness""#));
    assert!(
        run(binary, &config, &["worker", "--shutdown"])
            .status
            .success()
    );
    wait_for_exit(&mut worker.0);
}

#[test]
fn provider_doctor_reports_ready_through_worker_after_catalog_and_generation_succeed() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let (origin, server) = doctor_server(
        "200 OK",
        r#"{"id":"chat-doctor","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
    );
    let config = write_doctor_config(directory.path(), &origin);

    let worker = command(binary, &config)
        .arg("worker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start worker");
    let mut worker = ChildGuard(worker);
    wait_for_worker(binary, &config);
    let output = run(binary, &config, &["provider", "doctor", "live"]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let provider_readiness: Value =
        serde_json::from_slice(&output.stdout).expect("provider readiness JSON");
    assert_eq!(provider_readiness["ready"], true);
    assert_eq!(provider_readiness["checks"][0]["name"], "models_endpoint");
    assert_eq!(provider_readiness["checks"][0]["status"], "pass");

    let output = run(binary, &config, &["models", "doctor", "live"]);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let model_readiness: Value =
        serde_json::from_slice(&output.stdout).expect("model readiness JSON");
    assert_eq!(model_readiness["ready"], true);
    assert_eq!(model_readiness["checks"][0]["name"], "metadata");
    assert_eq!(model_readiness["checks"][0]["status"], "pass");
    assert_eq!(model_readiness["checks"][1]["name"], "generation");
    assert_eq!(model_readiness["checks"][1]["status"], "pass");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"ok\""));

    let requests = server.join().expect("doctor server");
    assert!(requests[0].starts_with("GET /v1/models "));
    assert!(requests[1].starts_with("POST /v1/chat/completions "));
    assert!(requests[1].contains(r#""name":"colossus_readiness""#));
    assert!(!requests[1].contains(r#""maxLength""#));
    assert!(
        run(binary, &config, &["worker", "--shutdown"])
            .status
            .success()
    );
    wait_for_exit(&mut worker.0);
}

#[test]
fn provider_profile_timeout_is_not_silently_capped_by_the_sandbox_timeout() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let delayed = r#"data: {"id":"chat-delayed","choices":[{"index":0,"delta":{"content":"delayed-ready"},"finish_reason":"stop"}]}

data: [DONE]

"#;
    let (origin, server) = delayed_sse_server(Duration::from_millis(75), delayed);
    let config = write_provider_timeout_config(directory.path(), &origin);

    let output = run(
        binary,
        &config,
        &[
            "run",
            "Exercise the configured provider timeout.",
            "--stream",
        ],
    );
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "delayed-ready");
    assert!(String::from_utf8_lossy(&output.stderr).contains("delayed-ready"));

    let request = server.join().expect("provider server");
    assert!(request.starts_with("POST /v1/chat/completions "));
}

#[test]
fn service_unavailable_is_visible_as_a_recoverable_provider_error() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let (origin, server) = status_server(
        "503 Service Unavailable",
        r#"{"error":{"message":"private local loading detail"}}"#,
    );
    let config = write_failure_config(directory.path(), &origin, "echo");

    let output = failed_run(binary, directory.path(), &config, "1");
    assert!(!output.status.success(), "unavailable provider succeeded");
    let terminal = String::from_utf8_lossy(&output.stderr);
    assert!(terminal.contains("Run error"), "{terminal}");
    assert!(
        terminal.contains("provider.temporarily_unavailable"),
        "{terminal}"
    );
    assert!(terminal.contains("Recoverable"), "{terminal}");
    assert!(terminal.contains("yes"), "{terminal}");
    assert!(terminal.contains("HTTP 503"), "{terminal}");
    assert!(
        terminal.contains("retry after the endpoint reports ready"),
        "{terminal}"
    );
    assert!(!terminal.contains("private local loading detail"));

    let request = server.join().expect("provider server");
    assert!(request.starts_with("POST /v1/chat/completions "));
    let events = audited_run_events(binary, &config);
    assert_eq!(event_count(&events, "error.v1"), 1);
}

#[test]
fn compatible_provider_streams_tool_use_and_tui_output_through_terminal_surfaces() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    let (origin, server) = live_server();
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: provider-terminal-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [provider.openai.chat]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    live:
      kind: open_ai_compatible
      baseUrl: {origin}/v1
      credentialReference: null
      timeoutMs: 10000
models:
  profiles:
    live:
      providerProfile: live
      model: terminal-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: live
agent:
  maxTurns: 4
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: provider-terminal-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: [{origin}]
  timeoutMs: 10000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
        ),
    )
    .expect("config");

    let run_output = run(
        binary,
        &config,
        &[
            "run",
            "Use the echo tool and then answer.",
            "--stream",
            "--max-turns",
            "4",
        ],
    );
    assert!(
        run_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );
    let terminal = String::from_utf8_lossy(&run_output.stderr);
    assert!(
        terminal.contains("[activity] waiting_for_model terminal-model"),
        "{terminal}"
    );
    assert_eq!(
        terminal.matches("[tool] start echo").count(),
        2,
        "{terminal}"
    );
    assert_eq!(terminal.matches("Completed echo").count(), 2, "{terminal}");
    assert!(terminal.contains("terminal-tool-one"), "{terminal}");
    assert!(terminal.contains("terminal-tool-two"), "{terminal}");
    assert!(terminal.contains("connected"), "{terminal}");
    assert!(!terminal.contains("\x1b["));
    let result: Value = serde_json::from_slice(&run_output.stdout).expect("run JSON");
    let run_id = result["run_id"].as_str().expect("run id");
    let session_id = result["session_id"].as_str().expect("session id");
    assert!(!run_id.is_empty());
    assert!(!session_id.is_empty());
    assert_ne!(run_id, session_id);
    assert_eq!(result["profile"], "live");
    assert_eq!(result["model"], "terminal-model");
    assert_eq!(result["output"], "connected");
    assert!(
        result["event_count"]
            .as_u64()
            .is_some_and(|count| count >= 14)
    );
    assert!(
        result["elapsed_seconds"]
            .as_f64()
            .is_some_and(|seconds| seconds > 0.0)
    );

    let mut terminal = command(binary, &config);
    terminal
        .arg("tui")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut terminal = terminal.spawn().expect("spawn terminal line runner");
    terminal
        .stdin
        .take()
        .expect("terminal stdin")
        .write_all(b"/session bogus\n/session resume\n1\nReply from the live endpoint.\n/exit\n")
        .expect("write terminal script");
    let terminal = terminal.wait_with_output().expect("terminal output");
    assert!(
        terminal.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&terminal.stdout),
        String::from_utf8_lossy(&terminal.stderr)
    );
    let terminal_output = String::from_utf8_lossy(&terminal.stdout);
    assert!(terminal_output.contains(&format!("Colossus Rust {}.", env!("CARGO_PKG_VERSION"))));
    assert!(terminal_output.contains("unknown terminal command: /session bogus"));
    assert!(terminal_output.contains("Choose a session to resume:"));
    assert!(terminal_output.contains("terminal-connected"));
    assert!(!terminal_output.contains("\x1b["));

    let worker = command(binary, &config)
        .arg("worker")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start worker");
    let mut worker = ChildGuard(worker);
    wait_for_worker(binary, &config);
    let worker_run = run(
        binary,
        &config,
        &["run", "Reply through the worker endpoint.", "--stream"],
    );
    assert!(
        worker_run.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&worker_run.stdout),
        String::from_utf8_lossy(&worker_run.stderr)
    );
    let worker_terminal = String::from_utf8_lossy(&worker_run.stderr);
    assert!(worker_terminal.contains("worker-connected"));
    assert!(!worker_terminal.contains("\x1b["));
    let worker_result: Value = serde_json::from_slice(&worker_run.stdout).expect("worker run JSON");
    assert_eq!(worker_result["output"], "worker-connected");
    assert!(
        run(binary, &config, &["worker", "--shutdown"])
            .status
            .success()
    );
    wait_for_exit(&mut worker.0);

    let requests = server.join().expect("provider server");
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(requests[0].contains("Use the echo tool and then answer."));
    assert!(requests[0].contains(r#""name":"echo""#));
    assert!(requests[0].contains(r#""stream":true"#));
    assert!(requests[1].contains(r#""tool_call_id":"call-1""#));
    assert!(requests[1].contains(r#""role":"tool""#));
    assert!(requests[1].contains("terminal-tool-one"));
    assert!(requests[2].contains(r#""tool_call_id":"call-2""#));
    assert!(requests[2].contains("terminal-tool-one"));
    assert!(requests[2].contains("terminal-tool-two"));
    assert!(requests[3].contains("Reply from the live endpoint."));
    assert!(requests[4].contains("Reply through the worker endpoint."));
}

#[test]
fn model_delegation_runs_the_child_before_agent_result_in_the_same_turn() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    let (origin, server) = subagent_server();
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: provider-subagent-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [agent.delegate, agent.result, agent.list]
    exclude: []
  actions:
    allow: [provider.openai.chat, subagent.create, subagent.read, subagent.list, subagent.start, subagent.complete, subagent.fail, subagent.cancel, subagent.interrupt, subagent.requeue]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    delegated:
      kind: open_ai_compatible
      baseUrl: {origin}/v1
      credentialReference: null
      timeoutMs: 10000
models:
  profiles:
    delegated:
      providerProfile: delegated
      model: delegated-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: delegated
    subagent_default: delegated
agent:
  maxTurns: 4
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: provider-subagent-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: [{origin}]
  timeoutMs: 10000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
        ),
    )
    .expect("config");

    let output = command(binary, &config)
        .current_dir(directory.path())
        .args([
            "run",
            "Ask a subagent to say hi, then report its actual response.",
            "--stream",
            "--max-turns",
            "4",
        ])
        .output()
        .expect("delegated run");
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "The subagent said hi.");
    let terminal = String::from_utf8_lossy(&output.stderr);
    assert!(terminal.contains("Completed agent.delegate"), "{terminal}");
    assert!(terminal.contains("Completed agent.result"), "{terminal}");
    assert!(!terminal.contains("Failed agent.result"), "{terminal}");
    assert!(terminal.contains("Hi, Alex! Ping received."), "{terminal}");

    let requests = server.join().expect("subagent provider server");
    assert_eq!(requests.len(), 4);
    let session_id = result["session_id"].as_str().expect("session id");
    let agents = run(
        binary,
        &config,
        &["agents", "list", "--session", session_id],
    );
    assert!(agents.status.success());
    let agents: Value = serde_json::from_slice(&agents.stdout).expect("agents JSON");
    assert_eq!(agents.as_array().map(Vec::len), Some(1));
    assert_eq!(agents[0]["status"], "completed");
    assert_eq!(agents[0]["final_output"], "Hi, Alex! Ping received.");
}

#[test]
fn responses_provider_keeps_credentials_out_of_streamed_tool_terminal_output() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    let (origin, server) = responses_server();
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: responses-terminal-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [echo]
    exclude: []
  actions:
    allow: [provider.openai.responses]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    responses:
      kind: open_ai_responses
      baseUrl: {origin}/v1
      credentialReference: env:COLOSSUS_PROVIDER_TERMINAL_API_KEY
      timeoutMs: 10000
models:
  profiles:
    responses:
      providerProfile: responses
      model: responses-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: responses
agent:
  maxTurns: 4
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: responses-terminal-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem: []
  executables: []
  environment: []
  networkDestinations: [{origin}]
  timeoutMs: 10000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
        ),
    )
    .expect("config");

    let output = run(
        binary,
        &config,
        &[
            "run",
            "Use the Responses tool path and answer.",
            "--stream",
            "--max-turns",
            "4",
        ],
    );
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let terminal = String::from_utf8_lossy(&output.stderr);
    assert!(terminal.contains("[tool] start echo"), "{terminal}");
    assert!(terminal.contains("Completed echo"), "{terminal}");
    assert!(
        terminal.contains("responses-connected [REDACTED]"),
        "{terminal}"
    );
    assert!(!terminal.contains("terminal-secret"));
    assert!(!terminal.contains("\x1b["));
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["profile"], "responses");
    assert_eq!(result["output"], "responses-connected [REDACTED]");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("terminal-secret"));

    let requests = server.join().expect("Responses server");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /v1/responses HTTP/1.1"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer terminal-secret")
    );
    assert!(requests[0].contains(r#""store":false"#));
    assert!(requests[0].contains(r#""strict":false"#));
    assert!(requests[0].contains("Use the Responses tool path and answer."));
    let first_body = requests[0]
        .split("\r\n\r\n")
        .nth(1)
        .expect("first request body");
    assert!(!first_body.contains("terminal-secret"));
    assert!(requests[1].contains(r#""type":"function_call_output""#));
    assert!(requests[1].contains(r#""call_id":"call-r""#));
    assert!(requests[1].contains("responses-tool"));
    let continuation_body = requests[1]
        .split("\r\n\r\n")
        .nth(1)
        .expect("continuation request body");
    assert!(!continuation_body.contains("terminal-secret"));
    assert!(continuation_body.contains("[REDACTED]"));

    let session_id = result["session_id"].as_str().expect("session id");
    for arguments in [
        vec!["sessions", "messages", session_id],
        vec![
            "telemetry",
            "show",
            result["run_id"].as_str().expect("run id"),
        ],
        vec!["audit", "show", "--limit", "200"],
    ] {
        let diagnostic = run(binary, &config, &arguments);
        assert!(
            diagnostic.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&diagnostic.stdout),
            String::from_utf8_lossy(&diagnostic.stderr)
        );
        assert!(!String::from_utf8_lossy(&diagnostic.stdout).contains("terminal-secret"));
        assert!(!String::from_utf8_lossy(&diagnostic.stderr).contains("terminal-secret"));
    }
}

#[test]
fn malformed_provider_tool_arguments_retry_twice_without_executing_the_tool() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("directory");
    let state = directory.path().join("state.redb");
    let anchor = directory.path().join("anchor.json");
    let workflows = directory.path().join("workflows");
    fs::create_dir(&workflows).expect("workflows");
    let config = directory.path().join("config.yaml");
    let (origin, server) = malformed_arguments_server();
    fs::write(
        &config,
        format!(
            r#"schemaVersion: 2
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: malformed-terminal-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
access:
  profile: pinned
  tools:
    include: [filesystem.write]
    exclude: []
  actions:
    allow: [provider.openai.chat, filesystem.write]
    requireApproval: []
    deny: []
policy:
  kind: built_in
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    malformed:
      kind: open_ai_compatible
      baseUrl: {origin}/v1
      credentialReference: null
      timeoutMs: 10000
models:
  profiles:
    malformed:
      providerProfile: malformed
      model: malformed-tool-model
      contextWindowTokens: 32768
      maxOutputTokens: 4096
      capabilities:
        toolCalls: true
        streaming: true
  roles:
    primary: malformed
agent:
  maxTurns: 4
subagents:
  maxConcurrent: 1
sandbox:
  backend: native
  profile: malformed-terminal-v1
  allowBrokerFallback: false
  helperPath: null
  ociRuntime: null
  ociImage: null
  ociProxyImage: null
  filesystem:
    - root: {workspace}
      mode: write
  executables: []
  environment: []
  networkDestinations: [{origin}]
  timeoutMs: 10000
  maxOutputBytes: 1048576
  maxProcesses: 2
  maxMemoryBytes: 67108864
  maxConcurrency: 1
"#,
            state = state.display(),
            anchor = anchor.display(),
            workflows = workflows.display(),
            workspace = directory.path().display(),
        ),
    )
    .expect("config");

    let output = command(binary, &config)
        .current_dir(directory.path())
        .args([
            "run",
            "Attempt the requested write and recover malformed arguments.",
            "--stream",
            "--max-turns",
            "4",
        ])
        .output()
        .expect("run Colossus");
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !directory.path().join("must-not-exist.txt").exists(),
        "malformed tool arguments reached the filesystem adapter"
    );
    let terminal = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        terminal.matches("provider.invalid_tool_arguments").count(),
        2,
        "{terminal}"
    );
    assert!(!terminal.contains("[file] start filesystem.write"));
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["output"], "recovered");
    let run_id = result["run_id"].as_str().expect("run id");

    let requests = server.join().expect("malformed provider server");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("Attempt the requested write"));
    assert!(requests[1].contains("No tool was executed"));
    assert!(requests[1].contains("Recovery attempt 1/2"));
    assert!(requests[2].contains("Recovery attempt 1/2"));
    assert!(requests[2].contains("Recovery attempt 2/2"));

    let audit = run(binary, &config, &["audit", "show", "--limit", "200"]);
    assert!(audit.status.success());
    let events: Vec<Value> = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    let stream_id = format!("run:{run_id}");
    let run_events = events
        .iter()
        .filter(|event| event["stream_id"] == stream_id)
        .collect::<Vec<_>>();
    assert_eq!(
        run_events
            .iter()
            .filter(|event| event["event_type"] == "model.request.prepared.v1")
            .count(),
        3
    );
    assert_eq!(
        run_events
            .iter()
            .filter(|event| event["event_type"] == "error.v1")
            .count(),
        2
    );
    assert!(
        run_events
            .iter()
            .all(|event| event["event_type"] != "tool.call.started.v1")
    );
    assert_eq!(
        result["event_count"].as_u64(),
        Some(run_events.len() as u64)
    );
    assert_eq!(
        run_events.last().map(|event| &event["event_type"]),
        Some(&Value::String("run.completed.v1".into()))
    );
}

#[test]
fn max_turn_empty_output_and_malformed_recovery_have_distinct_terminal_states() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let root = tempdir().expect("root directory");

    let max_turn_directory = root.path().join("max-turn");
    fs::create_dir(&max_turn_directory).expect("max-turn directory");
    let (max_turn_origin, max_turn_server) = max_turn_server();
    let max_turn_config = write_failure_config(&max_turn_directory, &max_turn_origin, "echo");
    let max_turn = failed_run(binary, &max_turn_directory, &max_turn_config, "2");
    assert!(!max_turn.status.success(), "turn exhaustion succeeded");
    let max_turn_terminal = String::from_utf8_lossy(&max_turn.stderr);
    assert!(
        max_turn_terminal.contains("Run error"),
        "{max_turn_terminal}"
    );
    assert!(
        max_turn_terminal.contains("agent.max_turns"),
        "{max_turn_terminal}"
    );
    assert!(
        max_turn_terminal.contains("Recoverable"),
        "{max_turn_terminal}"
    );
    assert!(
        max_turn_terminal.contains("model turn limit exhausted after 2 turns"),
        "{max_turn_terminal}"
    );
    assert!(!max_turn_terminal.contains("provider.empty_turn"));
    assert!(!max_turn_terminal.contains("provider.invalid_tool_arguments"));
    let max_turn_requests = max_turn_server.join().expect("max-turn provider");
    assert_eq!(max_turn_requests.len(), 2);
    assert!(max_turn_requests[1].contains(r#""tool_call_id":"max-call-1""#));
    let max_turn_events = audited_run_events(binary, &max_turn_config);
    assert_eq!(
        event_count(&max_turn_events, "model.request.prepared.v1"),
        2
    );
    assert_eq!(event_count(&max_turn_events, "tool.call.started.v1"), 2);
    assert_eq!(event_count(&max_turn_events, "tool.call.completed.v1"), 2);
    assert_eq!(event_count(&max_turn_events, "run.max_turns.v1"), 1);
    assert_eq!(event_count(&max_turn_events, "error.v1"), 0);
    assert_eq!(
        max_turn_events.last().map(|event| &event["event_type"]),
        Some(&Value::String("run.max_turns.v1".into()))
    );

    let empty_directory = root.path().join("empty-output");
    fs::create_dir(&empty_directory).expect("empty-output directory");
    let (empty_origin, empty_server) = empty_output_server();
    let empty_config = write_failure_config(&empty_directory, &empty_origin, "echo");
    let empty = failed_run(binary, &empty_directory, &empty_config, "4");
    assert!(!empty.status.success(), "empty provider output succeeded");
    let empty_terminal = String::from_utf8_lossy(&empty.stderr);
    assert!(empty_terminal.contains("Run error"), "{empty_terminal}");
    assert!(
        empty_terminal.contains("provider.failed"),
        "{empty_terminal}"
    );
    assert!(empty_terminal.contains("Recoverable"), "{empty_terminal}");
    assert!(
        empty_terminal.contains("chat stream completed without visible text or tool calls"),
        "{empty_terminal}"
    );
    assert!(!empty_terminal.contains("agent.max_turns"));
    assert_eq!(empty_server.join().expect("empty provider").len(), 1);
    let empty_events = audited_run_events(binary, &empty_config);
    assert_eq!(event_count(&empty_events, "model.request.prepared.v1"), 1);
    assert_eq!(event_count(&empty_events, "error.v1"), 1);
    assert_eq!(event_count(&empty_events, "run.max_turns.v1"), 0);
    assert_eq!(event_count(&empty_events, "tool.call.started.v1"), 0);

    let malformed_directory = root.path().join("malformed-exhaustion");
    fs::create_dir(&malformed_directory).expect("malformed directory");
    let (malformed_origin, malformed_server) = malformed_exhaustion_server();
    let malformed_config =
        write_failure_config(&malformed_directory, &malformed_origin, "filesystem.write");
    let malformed = failed_run(binary, &malformed_directory, &malformed_config, "4");
    assert!(
        !malformed.status.success(),
        "malformed recovery exhaustion succeeded"
    );
    assert!(
        !malformed_directory
            .join("exhausted-must-not-exist.txt")
            .exists(),
        "malformed recovery reached the filesystem"
    );
    let malformed_terminal = String::from_utf8_lossy(&malformed.stderr);
    assert_eq!(
        malformed_terminal
            .matches("provider.invalid_tool_arguments")
            .count(),
        3,
        "{malformed_terminal}"
    );
    assert!(
        malformed_terminal.contains("ToolArgumentRecoveryExhausted { attempts: 3 }"),
        "{malformed_terminal}"
    );
    assert!(!malformed_terminal.contains("agent.max_turns"));
    let malformed_requests = malformed_server.join().expect("malformed provider");
    assert_eq!(malformed_requests.len(), 3);
    assert!(malformed_requests[1].contains("Recovery attempt 1/2"));
    assert!(malformed_requests[2].contains("Recovery attempt 2/2"));
    let malformed_events = audited_run_events(binary, &malformed_config);
    assert_eq!(
        event_count(&malformed_events, "model.request.prepared.v1"),
        3
    );
    assert_eq!(event_count(&malformed_events, "error.v1"), 3);
    assert_eq!(event_count(&malformed_events, "run.max_turns.v1"), 0);
    assert_eq!(event_count(&malformed_events, "tool.call.started.v1"), 0);
}
