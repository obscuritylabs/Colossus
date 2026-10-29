//! Concurrent workspace conversations through the real TUI, runtime and worker.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;
#[path = "support/terminal.rs"]
#[allow(dead_code)]
mod terminal_support;

use colossus_runtime::RuntimeConfig;
use portable_pty::CommandBuilder;
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
use terminal_support::Terminal;

const PROVIDER_WAIT: Duration = Duration::from_secs(30);

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn process(binary: &Path, workspace: &Path, home: &process_support::IsolatedUserHome) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(workspace)
        .args(["--config", "config.json"])
        .env("HOME", home.path())
        .env("COLOSSUS_HOME", home.colossus_home())
        .env("NO_PROXY", "127.0.0.1");
    #[cfg(windows)]
    command
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.local_app_data())
        .env("TEMP", home.temporary_directory())
        .env("TMP", home.temporary_directory());
    command
}

fn accept_request(listener: &TcpListener) -> (TcpStream, Value) {
    let deadline = Instant::now() + PROVIDER_WAIT;
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "concurrent provider request timed out"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("provider accept: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).expect("provider request bytes");
        assert!(
            read > 0 && bytes.len() + read <= 1_048_576,
            "bounded provider request"
        );
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .expect("content length")
                .trim()
                .parse::<usize>()
                .unwrap();
            assert!(length <= 1_048_576, "bounded provider body");
            if bytes.len() >= end + 4 + length {
                let request = serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                return (stream, request);
            }
        }
    }
}

fn event(text: &str, finished: bool) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"concurrent-turn","choices":[{"index":0,"delta":{"content":text},"finish_reason":if finished {json!("stop")} else {Value::Null}}]})
    )
}

fn start_stream(stream: &mut TcpStream, start: &str, end: &str) -> String {
    let initial = event(start, false);
    let final_part = format!("{}data: [DONE]\n\n", event(end, true));
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{initial}", initial.len() + final_part.len()).unwrap();
    stream.flush().unwrap();
    final_part
}

fn overlapping_provider() -> (String, thread::JoinHandle<[Value; 2]>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let (mut alpha, alpha_request) = accept_request(&listener);
        assert!(
            alpha_request["messages"]
                .to_string()
                .contains("ALPHA_REQUEST")
        );
        let alpha_final = start_stream(&mut alpha, "ALPHA_RUNNING\n", "ALPHA_FINAL_OUTPUT");
        // Alpha's HTTP response remains incomplete until Beta reaches the
        // provider. A globally serialized host cannot pass this barrier.
        let (mut beta, beta_request) = accept_request(&listener);
        assert!(
            beta_request["messages"]
                .to_string()
                .contains("BETA_REQUEST")
        );
        assert!(
            !beta_request["messages"]
                .to_string()
                .contains("ALPHA_REQUEST"),
            "independent sessions must not share canonical conversation context"
        );
        let beta_final = start_stream(&mut beta, "BETA_RUNNING\n", "BETA_FINAL_OUTPUT");
        alpha.write_all(alpha_final.as_bytes()).unwrap();
        alpha.flush().unwrap();
        beta.write_all(beta_final.as_bytes()).unwrap();
        beta.flush().unwrap();
        [alpha_request, beta_request]
    });
    (origin, task)
}

fn submit(terminal: &Terminal, prompt: &str) {
    terminal.send(format!("\x1b[200~{prompt}\x1b[201~").as_bytes());
    terminal.wait_for_composer("complete prompt before submission", |draft| draft == prompt);
    terminal.send(b"\r");
}

fn center(terminal: &Terminal) {
    terminal.send(if cfg!(windows) {
        b"\x1b[113;60;0;1;0;1_\x1b[113;60;0;0;0;1_"
    } else {
        b"\x1bOQ"
    });
    terminal.wait("Sessions ·");
}

#[test]
fn embedded_tui_runs_independent_sessions_concurrently() {
    exercise(false);
}

#[test]
fn authenticated_worker_tui_runs_independent_sessions_concurrently() {
    exercise(true);
}

fn exercise(worker_host: bool) {
    let directory = process_support::tempdir().expect("isolated fixture directory");
    let workspace = directory.path().canonicalize().unwrap();
    let home = process_support::isolated_user_home(&workspace);
    let binary = workspace.join(if cfg!(windows) {
        "colossus-concurrency-test.exe"
    } else {
        "colossus-concurrency-test"
    });
    fs::copy(env!("CARGO_BIN_EXE_colossus"), &binary).unwrap();
    let (origin, requests) = overlapping_provider();
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.workflows.repository = workspace.join("workflows");
    config.workflows.user = workspace.join("workflows");
    fs::create_dir_all(&config.workflows.repository).unwrap();
    config.providers = serde_json::from_value(json!({"profiles":{
        "fixture":{"kind":"open_ai_compatible","baseUrl":format!("{origin}/v1"),"timeoutMs":30000}
    }}))
    .unwrap();
    config.models = serde_json::from_value(json!({"profiles":{
        "fixture":{"providerProfile":"fixture","model":"fixture","contextWindowTokens":32000,"maxOutputTokens":4000,"capabilities":{"toolCalls":true,"streaming":true}}
    },"roles":{"primary":"fixture"}})).unwrap();
    config.access = serde_json::from_value(json!({"profile":"pinned","tools":{"include":[]},"actions":{"allow":["provider.openai.chat","presentation.history.append","context.show","plugin.list"]}})).unwrap();
    config.sandbox.network_destinations = vec![origin];
    fs::write(
        workspace.join("config.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    let _worker = worker_host.then(|| {
        let child = process(&binary, &workspace, &home)
            .args(["--approval-mode", "deny", "worker"])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let worker = Worker(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let status = process(&binary, &workspace, &home)
                .args(["worker", "--status"])
                .output()
                .unwrap();
            if status.status.success() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "worker readiness: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            thread::sleep(Duration::from_millis(50));
        }
        worker
    });
    let mut command = CommandBuilder::new(&binary);
    command.cwd(&workspace);
    command.args([
        "--config",
        "config.json",
        "--alt-screen",
        "--approval-mode",
        "deny",
    ]);
    if worker_host {
        command.arg("--worker-required");
    }
    command.arg("tui");
    command.env("HOME", home.path());
    command.env("COLOSSUS_HOME", home.colossus_home());
    command.env("TERM", "xterm-256color");
    command.env("NO_PROXY", "127.0.0.1");
    #[cfg(windows)]
    {
        command.env("USERPROFILE", home.path());
        command.env("LOCALAPPDATA", home.local_app_data());
        command.env("TEMP", home.temporary_directory());
        command.env("TMP", home.temporary_directory());
    }
    let terminal = Terminal::start(command);
    terminal.wait("Enter sends");
    terminal.resize(40, 140);
    submit(&terminal, "ALPHA_REQUEST");
    terminal.wait("ALPHA_RUNNING");
    terminal.send(b"\x1b[200~ALPHA_UNSENT_DRAFT\x1b[201~");
    terminal.wait_for_composer("Alpha unsent draft", |draft| draft == "ALPHA_UNSENT_DRAFT");
    center(&terminal);
    terminal.send(b"n");
    terminal.wait_for_composer("new session composer", |draft| {
        draft.contains("Implement {feature}")
    });
    submit(&terminal, "BETA_REQUEST");
    let beta = terminal.wait("BETA_FINAL_OUTPUT");
    assert!(
        !beta.contains("ALPHA_FINAL_OUTPUT"),
        "Alpha streamed into Beta: {beta}"
    );
    center(&terminal);
    terminal.send(b"\x1b[H\r");
    let alpha = terminal.wait("ALPHA_FINAL_OUTPUT");
    terminal.wait_for_composer("retained Alpha draft", |draft| {
        draft == "ALPHA_UNSENT_DRAFT"
    });
    assert!(
        !alpha.contains("BETA_FINAL_OUTPUT"),
        "Beta streamed into Alpha: {alpha}"
    );
    center(&terminal);
    terminal.send(b"\x1b[F\r");
    terminal.wait("BETA_FINAL_OUTPUT");
    terminal.wait_for_composer("independent empty Beta draft", str::is_empty);
    let requests = requests.join().expect("overlapping local provider");
    assert!(
        requests
            .iter()
            .all(|request| request["tools"].as_array().is_none_or(Vec::is_empty))
    );
    println!(
        "{} TUI: two overlapping real provider requests, independent contexts, routed output and retained drafts",
        if worker_host {
            "authenticated worker"
        } else {
            "embedded"
        }
    );
}
