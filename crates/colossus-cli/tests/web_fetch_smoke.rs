//! Real provider-to-tool-to-gateway redirect acceptance with isolated durable state.

#[path = "support/process.rs"]
mod process_support;

use process_support::tempdir;
use serde_json::{Value, json};
use std::{
    fs,
    io::{ErrorKind, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0_u8; 4096];
        let count = stream.read(&mut buffer).expect("request");
        assert_ne!(count, 0, "incomplete request");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024, "bounded request");
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("length"))
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return String::from_utf8(bytes).expect("UTF-8 request");
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("response");
}

fn run(binary: &Path, config: &Path, directory: &Path, arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(binary);
    let _home = process_support::isolate_user_home(&mut command, directory);
    command
        .current_dir(directory)
        .arg("--config")
        .arg(config)
        .args(arguments)
        .output()
        .expect("run Colossus")
}

fn exercise_fetch(tool: &str, limit: Option<usize>) {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    let directory = tempdir().expect("isolated workspace");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture");
    listener.set_nonblocking(true).expect("nonblocking fixture");
    let origin = format!("http://{}", listener.local_addr().expect("origin"));
    let success = limit.is_none_or(|value| value >= 2);
    let expected_requests = if success {
        5
    } else {
        limit.expect("explicit limit") + 2
    };
    let tool_call = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "fetch-tool", "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "id": "fetch-call", "type": "function", "function": {
                    "name": tool.replace('.', "_"),
                    "arguments": json!({"url": format!("{origin}/start")}).to_string()
                }}]
            }, "finish_reason": "tool_calls"}]
        })
    );
    let final_answer = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({
            "id": "fetch-final", "choices": [{"index": 0, "delta": {"content": "redirect-finished"}, "finish_reason": "stop"}]
        })
    );
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut requests = Vec::new();
        while requests.len() < expected_requests && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("accept: {error}"),
            };
            stream.set_nonblocking(false).expect("blocking stream");
            let request = read_request(&mut stream);
            match requests.len() {
                0 => {
                    assert!(request.starts_with("POST /v1/chat/completions "));
                    respond(
                        &mut stream,
                        "200 OK",
                        "Content-Type: text/event-stream\r\n",
                        &tool_call,
                    );
                }
                1 => {
                    assert!(request.starts_with("GET /start "));
                    respond(
                        &mut stream,
                        "302 Found",
                        "Location: /idp?SAMLRequest=a%2Bb&RelayState=x%2Fy\r\nSet-Cookie: saml=private\r\n",
                        "",
                    );
                }
                2 => {
                    assert!(request.starts_with("GET /idp?SAMLRequest=a%2Bb&RelayState=x%2Fy "));
                    assert!(!request.to_ascii_lowercase().contains("\r\ncookie:"));
                    respond(&mut stream, "303 See Other", "Location: /callback\r\n", "");
                }
                3 => {
                    assert!(request.starts_with("GET /callback "));
                    respond(
                        &mut stream,
                        "200 OK",
                        "Content-Type: text/plain\r\n",
                        "SAML redirect landing page",
                    );
                }
                4 => {
                    assert!(request.starts_with("POST /v1/chat/completions "));
                    let (_, body) = request
                        .split_once("\r\n\r\n")
                        .expect("provider request body");
                    let body: Value = serde_json::from_str(body).expect("provider JSON");
                    let observation = body["messages"]
                        .as_array()
                        .expect("messages")
                        .iter()
                        .find(|message| message["role"] == "tool")
                        .expect("released tool observation");
                    assert_eq!(observation["tool_call_id"], "fetch-call");
                    assert!(
                        observation["content"]
                            .as_str()
                            .expect("observation text")
                            .contains("SAML redirect landing page")
                    );
                    respond(
                        &mut stream,
                        "200 OK",
                        "Content-Type: text/event-stream\r\n",
                        &final_answer,
                    );
                }
                _ => unreachable!("bounded sequence"),
            }
            requests.push(request);
        }
        assert_eq!(requests.len(), expected_requests, "request count");
        requests
    });
    let config = directory.path().join("config.json");
    let mut document = json!({
        "schemaVersion": 3,
        "storage": {"path": directory.path().join("state.redb"), "keys": {"kind": "none"}},
        "access": {"profile": "pinned", "tools": {"include": [tool]}, "actions": {"allow": ["network.http", "provider.openai.chat"]}},
        "policy": {"kind": "built_in", "require_post_effect": true},
        "providers": {"profiles": {"fixture": {"kind": "open_ai_compatible", "baseUrl": format!("{origin}/v1"), "timeoutMs": 5000}}},
        "models": {"profiles": {"fixture": {"providerProfile": "fixture", "model": "fetch-test", "contextWindowTokens": 32768, "maxOutputTokens": 4096, "capabilities": {"toolCalls": true, "streaming": true}}}, "roles": {"primary": "fixture"}},
        "sandbox": {"backend": "external", "profile": "fetch-acceptance", "acknowledgeExternalBoundary": true,
            "filesystem": [{"root": directory.path(), "mode": "write"}], "networkDestinations": [origin],
            "timeoutMs": 5000, "maxOutputBytes": 1048576, "maxProcesses": 2, "maxMemoryBytes": 67108864, "maxConcurrency": 1}
    });
    if let Some(limit) = limit {
        document["network"] = json!({"maxRedirects": limit});
    }
    fs::write(
        &config,
        serde_json::to_vec_pretty(&document).expect("config JSON"),
    )
    .expect("config");
    let output = run(
        binary,
        &config,
        directory.path(),
        &[
            "run",
            "Fetch the URL through the offered tool.",
            "--stream",
            "--max-turns",
            "3",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.success(),
        success,
        "stdout: {stdout}\nstderr: {stderr}"
    );
    if success {
        assert!(stdout.contains("redirect-finished"), "{stdout}");
    } else {
        assert!(stderr.contains("redirect limit exceeded"), "{stderr}");
    }
    server.join().expect("HTTP fixture");

    let output = run(
        binary,
        &config,
        directory.path(),
        &["audit", "show", "--limit", "200"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<Value> = serde_json::from_slice(&output.stdout).expect("audit events");
    let fetch_events: Vec<_> = events
        .iter()
        .filter(|event| event["actor"]["id"] == "tool-call:fetch-call")
        .collect();
    assert!(
        fetch_events
            .iter()
            .any(|event| event["event_type"] == "effect.started.v1")
    );
    assert_eq!(
        fetch_events
            .iter()
            .any(|event| event["event_type"] == "effect.release_requested.v1"),
        success
    );
    let output = run(binary, &config, directory.path(), &["audit", "verify"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fetch_aliases_follow_default_redirects_through_real_cli_and_release() {
    for tool in ["web.fetch", "docs.fetch", "network.http"] {
        exercise_fetch(tool, None);
    }
}

#[test]
fn configured_redirect_limit_is_used_by_the_real_cli() {
    for limit in [0, 1, 2] {
        exercise_fetch("web.fetch", Some(limit));
    }
}
