//! Real provider → recovery → prepared approval → process, with a private home.
use super::*;
use std::sync::{Arc, Mutex};

const REASON: &str = "Verify command approval with a local marker.";

pub(super) fn server(allow: bool, arguments: Value) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback provider");
    listener.set_nonblocking(true).expect("nonblocking");
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let count = if allow { 3 } else { 2 };
        let mut requests = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while requests.len() < count {
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "provider timed out after {} requests",
                        requests.len()
                    );
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("provider: {error}"),
            };
            stream.set_nonblocking(false).unwrap();
            let input = read_request(&mut stream);
            let turn = requests.len();
            if turn == 0 {
                assert!(
                    input.contains("justification"),
                    "model was not told about task intent"
                );
            } else if turn == 1 {
                assert!(
                    input.contains("justification"),
                    "missing reason was not returned as invalid arguments"
                );
            }
            requests.push(input);
            let delta = if turn == 2 {
                json!({"content": "Command finished."})
            } else {
                let mut arguments = arguments.clone();
                if turn == 1 {
                    arguments["justification"] = json!(REASON);
                }
                json!({"tool_calls": [{"index": 0, "id": format!("command-{turn}"), "type": "function", "function": {"name": "shell_run", "arguments": arguments.to_string()}}]})
            };
            respond_sse(
                &mut stream,
                &format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"id": format!("turn-{turn}"), "choices": [{"index": 0, "delta": delta, "finish_reason": if turn == 2 { "stop" } else { "tool_calls" }}]})
                ),
            );
        }
        requests
    });
    (format!("http://{address}"), task)
}

struct RunGuard(std::process::Child);
impl Drop for RunGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_output(output: &Arc<Mutex<String>>, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let text = output.lock().unwrap().clone();
        if text.contains(expected) {
            return;
        }
        assert!(Instant::now() < deadline, "missing {expected}: {text}");
        thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn command_reason_recovery_and_full_details_precede_allow_or_deny() {
    for allow in [false, true] {
        let directory = tempdir().unwrap();
        let workspace = directory.path().canonicalize().unwrap();
        let marker = workspace.join("approved-marker.txt");
        let script = if cfg!(windows) {
            format!(
                "echo approved>>approved-marker.txt & rem {} COMMAND_TAIL",
                "x".repeat(1500)
            )
        } else {
            format!(
                "printf 'approved\\n' >> approved-marker.txt # {} COMMAND_TAIL",
                "x".repeat(6000)
            )
        };
        let (origin, provider) = server(allow, json!({"command": script, "cwd": "."}));
        let config = config(&workspace, &origin);
        let mut process = command(Path::new(env!("CARGO_BIN_EXE_colossus")), &config);
        process
            .current_dir(&workspace)
            .args([
                "--approval-mode",
                "ask",
                "run",
                "Verify the command approval marker.",
                "--max-turns",
                "4",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = RunGuard(process.spawn().unwrap());
        let mut stderr = child.0.stderr.take().unwrap();
        let output = Arc::new(Mutex::new(String::new()));
        let transcript = Arc::clone(&output);
        let reader = thread::spawn(move || {
            let mut bytes = [0; 4096];
            while let Ok(count) = stderr.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                transcript
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&bytes[..count]));
            }
        });
        wait_output(&output, "[y/N/details]");
        assert!(!marker.exists(), "command ran before explicit approval");
        let before = output.lock().unwrap().clone();
        assert!(
            before.contains(REASON),
            "task reason missing from prompt: {before}"
        );
        assert!(before.contains("Working directory"));
        assert!(
            !before.contains("COMMAND_TAIL"),
            "long preview was not bounded"
        );
        let mut stdin = child.0.stdin.take().unwrap();
        stdin.write_all(b"details\n").unwrap();
        stdin.flush().unwrap();
        wait_output(&output, "COMMAND_TAIL");
        assert!(!marker.exists(), "viewing details authorized execution");
        stdin
            .write_all(if allow { b"yes\n" } else { b"no\n" })
            .unwrap();
        drop(stdin);
        let deadline = Instant::now() + Duration::from_secs(45);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "command did not finish: {}",
                output.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        };
        reader.join().unwrap();
        assert_eq!(status.success(), allow, "{}", output.lock().unwrap());
        if allow {
            assert_eq!(fs::read_to_string(&marker).unwrap().trim(), "approved");
        } else {
            assert!(!marker.exists());
        }
        assert_eq!(provider.join().unwrap().len(), if allow { 3 } else { 2 });
    }
}

pub(super) fn config(workspace: &Path, origin: &str) -> std::path::PathBuf {
    let config = write_tool_config(workspace, origin, false, false);
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["access"]["tools"]["include"] = json!(["shell.run"]);
    value["access"]["actions"]["requireApproval"] = json!(["shell.run"]);
    value["sandbox"]["backend"] = json!("danger_full_access");
    value["sandbox"]["acknowledgeDangerFullAccess"] = json!(true);
    fs::write(&config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    config
}
