//! Model changes through the production terminal, both hosts, and provider HTTP.
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
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminal_support::Terminal;

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
        .env("COLOSSUS_HOME", home.colossus_home());
    #[cfg(windows)]
    command
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.local_app_data())
        .env("TEMP", home.temporary_directory())
        .env("TMP", home.temporary_directory());
    command
}

fn provider(replies: &'static [&'static str]) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "provider request timed out");
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("provider accept: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let (offset, length) = loop {
                let mut chunk = [0; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0 && bytes.len() < 1_048_576);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse::<usize>()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break (end + 4, length);
                    }
                }
            };
            requests.push(serde_json::from_slice(&bytes[offset..offset + length]).unwrap());
            let body = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"id": format!("turn-{}", requests.len()), "choices": [{"index": 0, "delta": {"content": reply}, "finish_reason": "stop"}]})
            );
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
        }
        requests
    });
    (origin, task)
}

fn open_picker(terminal: &Terminal, command: &str, title: &str) {
    terminal.send(format!("\x1b[200~{command}\x1b[201~").as_bytes());
    terminal.wait_for_composer("picker command is ready", |draft| draft == command);
    terminal.send(b"\r");
    terminal.wait(title);
}

#[test]
fn embedded_and_worker_tui_switch_models_and_reasoning_without_losing_history() {
    for worker_host in [false, true] {
        let directory = process_support::tempdir().unwrap();
        let workspace = directory.path().canonicalize().unwrap();
        let home = process_support::isolated_user_home(&workspace);
        let binary = workspace.join(if cfg!(windows) {
            "colossus-model-test.exe"
        } else {
            "colossus-model-test"
        });
        fs::copy(env!("CARGO_BIN_EXE_colossus"), &binary).unwrap();
        let (deep_origin, deep_requests) = provider(&["MODEL_TUI_TURN_1_OK"]);
        let (fast_origin, fast_requests) =
            provider(&["MODEL_TUI_TURN_2_OK", "MODEL_TUI_TURN_3_OK"]);
        let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
        config.workflows.repository = workspace.join("workflows");
        config.workflows.user = workspace.join("workflows");
        fs::create_dir_all(&config.workflows.repository).unwrap();
        config.providers = serde_json::from_value(json!({"profiles": {
            "fast": { "kind": "open_ai_compatible", "baseUrl": format!("{fast_origin}/v1"), "timeoutMs": 5000 },
            "deep": { "kind": "open_ai_compatible", "baseUrl": format!("{deep_origin}/v1"), "timeoutMs": 5000 },
        }}))
        .unwrap();
        config.models = serde_json::from_value(json!({"profiles": {
            "fast": {"providerProfile": "fast", "model": "live-fast", "reasoningEffort": "low", "contextWindowTokens": 32000, "maxOutputTokens": 4000, "capabilities": {"toolCalls": true, "streaming": true}},
            "deep": {"providerProfile": "deep", "model": "live-deep", "reasoningEffort": "high", "contextWindowTokens": 32000, "maxOutputTokens": 4000, "capabilities": {"toolCalls": true, "streaming": true}},
        }, "roles": {"primary": "fast"}})).unwrap();
        config.access = serde_json::from_value(json!({"profile": "pinned", "tools": {"include": []}, "actions": {"allow": ["provider.openai.chat", "presentation.history.append", "context.show", "plugin.list"]}})).unwrap();
        config.sandbox.network_destinations = vec![deep_origin, fast_origin];
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
                    "{}",
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
        #[cfg(windows)]
        {
            command.env("USERPROFILE", home.path());
            command.env("LOCALAPPDATA", home.local_app_data());
            command.env("TEMP", home.temporary_directory());
            command.env("TMP", home.temporary_directory());
        }
        let terminal = Terminal::start(command);
        terminal.wait("Enter sends");
        open_picker(&terminal, "/model", "Choose model");
        terminal.send(b"\x1b[200~deep\x1b[201~");
        // The unfiltered list already contains live-deep. Observe the pasted
        // query and its single match before resizing or applying a choice.
        terminal.wait_until("filtered model picker", |screen| {
            screen.contains("Choose model · 1 choices")
                && screen.contains("/ deep")
                && !screen.contains("fast ·")
        });
        terminal.resize_frame(24, 55);
        // A clipped wide frame still contains the title. The browser moves its
        // left margin from two columns to one only after the narrow redraw.
        terminal.wait_until_visual_rows("narrow filtered model picker", |screen, rows| {
            screen.contains("/ deep")
                && rows
                    .iter()
                    .any(|row| row.starts_with(" Choose model · 1 choices"))
        });
        terminal.send(b"\r");
        terminal.wait("Enter sends");
        terminal.resize(40, 140);
        open_picker(&terminal, "/effort", "Choose reasoning effort");
        terminal.send(b"low");
        terminal.wait_until("filtered reasoning picker", |screen| {
            screen.contains("Choose reasoning effort · 1 choices") && screen.contains("/ low")
        });
        terminal.send(b"\r");
        terminal.wait("Enter sends");
        terminal.command("Remember this first turn", "MODEL_TUI_TURN_1_OK");
        terminal.command("/effort high", "high reasoning");
        terminal.command("/model fast", "low reasoning");
        terminal.command("/effort high", "high reasoning");
        terminal.command("/model fast", "high reasoning");
        terminal.command("Continue this same conversation", "MODEL_TUI_TURN_2_OK");
        terminal.command("/model default", "low reasoning");
        terminal.command("Return to the configured default", "MODEL_TUI_TURN_3_OK");
        let mut requests = deep_requests.join().unwrap();
        requests.extend(fast_requests.join().unwrap());
        assert_eq!(requests[0]["model"], "live-deep");
        assert_eq!(requests[0]["reasoning_effort"], "low");
        assert_eq!(requests[1]["model"], "live-fast");
        assert_eq!(requests[1]["reasoning_effort"], "high");
        assert_eq!(requests[2]["model"], "live-fast");
        assert_eq!(requests[2]["reasoning_effort"], "low");
        assert!(
            requests[1]["messages"]
                .to_string()
                .contains("Remember this first turn")
        );
        assert!(
            requests
                .iter()
                .all(|request| request["tools"].as_array().is_none_or(Vec::is_empty))
        );
        println!(
            "{} TUI verified three provider requests, changed model/effort, restored defaults, retained history and tool ceiling",
            if worker_host { "worker" } else { "embedded" }
        );
    }
}
