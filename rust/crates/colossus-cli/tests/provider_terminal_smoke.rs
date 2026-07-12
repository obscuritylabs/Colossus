//! Loopback-live provider terminal, credential, worker, and tool-loop acceptance.

use serde_json::Value;
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

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

fn command(binary: &Path, config: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("--config")
        .arg(config)
        .env("COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY", SIGNING_KEY)
        .env("COLOSSUS_PROVIDER_TERMINAL_API_KEY", "terminal-secret");
    command
}

fn run(binary: &Path, config: &Path, arguments: &[&str]) -> std::process::Output {
    command(binary, config)
        .args(arguments)
        .output()
        .expect("run Colossus")
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
    let repl_answer = r#"data: {"id":"chat-repl","choices":[{"index":0,"delta":{"content":"repl-"},"finish_reason":null}]}

data: {"id":"chat-repl","choices":[{"index":0,"delta":{"content":"connected"},"finish_reason":"stop"}]}

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
        repl_answer,
        worker_answer,
    ])
}

fn responses_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let tool_call = r#"data: {"type":"response.created","response":{"id":"resp-tool"}}

data: {"type":"response.output_item.done","item":{"type":"function_call","call_id":"call-r","name":"echo","arguments":"{\"text\":\"responses-tool\"}"}}

data: {"type":"response.completed","response":{"id":"resp-tool","status":"completed","output":[{"type":"function_call","call_id":"call-r","name":"echo","arguments":"{\"text\":\"responses-tool\"}"}],"usage":{"input_tokens":9,"output_tokens":1,"total_tokens":10}}}

data: [DONE]

"#;
    let final_answer = r#"data: {"type":"response.created","response":{"id":"resp-final"}}

data: {"type":"response.output_text.delta","delta":"responses-"}

data: {"type":"response.output_text.delta","delta":"connected"}

data: {"type":"response.completed","response":{"id":"resp-final","status":"completed","output":[],"usage":{"input_tokens":12,"output_tokens":2,"total_tokens":14}}}

data: [DONE]

"#;
    sse_server(vec![tool_call, final_answer])
}

#[test]
fn compatible_provider_streams_tool_use_and_repl_output_through_terminal_surfaces() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
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
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: provider-terminal-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [provider.openai.chat]
  approval_actions: []
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    live:
      kind: open_ai_compatible
      model: terminal-model
      baseUrl: {origin}/v1
      credentialReference: null
      timeoutMs: 10000
  roles:
    primary: live
agent:
  maxTurns: 4
  tools: [echo]
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
    assert_eq!(
        terminal.matches("[tool] complete echo status=ok").count(),
        2,
        "{terminal}"
    );
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

    let mut repl = command(binary, &config);
    repl.arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut repl = repl.spawn().expect("spawn REPL");
    repl.stdin
        .take()
        .expect("REPL stdin")
        .write_all(b"Reply from the live endpoint.\n/exit\n")
        .expect("write REPL script");
    let repl = repl.wait_with_output().expect("REPL output");
    assert!(
        repl.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&repl.stdout),
        String::from_utf8_lossy(&repl.stderr)
    );
    let repl_output = String::from_utf8_lossy(&repl.stdout);
    assert!(repl_output.contains("Colossus Rust alpha."));
    assert!(repl_output.contains("repl-connected"));
    assert!(!repl_output.contains("\x1b["));

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
fn responses_provider_keeps_credentials_out_of_streamed_tool_terminal_output() {
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus-rs"));
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
            r#"schemaVersion: 1
storage:
  path: {state}
  keys:
    kind: environment
    journal_variable: COLOSSUS_PROVIDER_TERMINAL_JOURNAL_KEY
    journal_key_id: responses-terminal-journal-v1
    signing_variable: COLOSSUS_PROVIDER_TERMINAL_SIGNING_KEY
    anchor_path: {anchor}
policy:
  kind: built_in
  allow_actions: [provider.openai.responses]
  approval_actions: []
  require_post_effect: true
workflows:
  repository: {workflows}
  user: {workflows}
providers:
  profiles:
    responses:
      kind: open_ai_responses
      model: responses-model
      baseUrl: {origin}/v1
      credentialReference: env:COLOSSUS_PROVIDER_TERMINAL_API_KEY
      timeoutMs: 10000
  roles:
    primary: responses
agent:
  maxTurns: 4
  tools: [echo]
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
    assert!(
        terminal.contains("[tool] complete echo status=ok"),
        "{terminal}"
    );
    assert!(terminal.contains("responses-connected"), "{terminal}");
    assert!(!terminal.contains("terminal-secret"));
    assert!(!terminal.contains("\x1b["));
    let result: Value = serde_json::from_slice(&output.stdout).expect("run JSON");
    assert_eq!(result["profile"], "responses");
    assert_eq!(result["output"], "responses-connected");
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
    assert!(requests[0].contains(r#""strict":true"#));
    assert!(requests[0].contains("Use the Responses tool path and answer."));
    assert!(requests[1].contains(r#""type":"function_call_output""#));
    assert!(requests[1].contains(r#""call_id":"call-r""#));
    assert!(requests[1].contains("responses-tool"));
}
