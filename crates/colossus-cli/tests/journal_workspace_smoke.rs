//! Native terminal and worker ownership acceptance for local journal adapters.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;
#[path = "support/terminal.rs"]
#[allow(dead_code)]
mod terminal_support;

use colossus_runtime::{KeyConfig, RuntimeConfig, StorageAdapter};
use portable_pty::CommandBuilder;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Read as _, Seek as _},
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminal_support::Terminal;

const WORKER_TIMEOUT: Duration = Duration::from_secs(60);
const PROMPTS: [&str; 2] = ["Workspace terminal one", "Workspace terminal two"];

#[test]
fn same_workspace_terminals_share_one_worker_across_local_journals() {
    let backends = [
        (StorageAdapter::Redb, "redb"),
        #[cfg(feature = "journal-startup-benchmark")]
        (StorageAdapter::BenchmarkSqlite, "sqlite"),
        #[cfg(feature = "journal-startup-benchmark")]
        (StorageAdapter::BenchmarkTurso, "turso"),
    ];
    for (adapter, name) in backends {
        exercise_workspace(adapter, name);
    }
}

fn exercise_workspace(adapter: StorageAdapter, name: &str) {
    let directory = process_support::tempdir().expect("private workspace");
    let workspace = directory
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let home = process_support::isolated_user_home(&workspace);
    let binary = Path::new(env!("CARGO_BIN_EXE_colossus"));
    // Windows uses the process temp directory for workspace coordination. All
    // participants need the same directory to exercise the real ownership check.
    let environment = [
        ("HOME", home.path().as_os_str().to_owned()),
        ("COLOSSUS_HOME", home.colossus_home().into_os_string()),
        #[cfg(windows)]
        ("USERPROFILE", home.path().as_os_str().to_owned()),
        #[cfg(windows)]
        ("LOCALAPPDATA", home.local_app_data().into_os_string()),
        #[cfg(windows)]
        ("TEMP", home.temporary_directory().into_os_string()),
        #[cfg(windows)]
        ("TMP", home.temporary_directory().into_os_string()),
        ("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64).into()),
        ("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64).into()),
        ("COLOSSUS_JOURNAL_STARTUP_BENCHMARK", "1".into()),
        ("HTTP_PROXY", "http://127.0.0.1:1".into()),
        ("HTTPS_PROXY", "http://127.0.0.1:1".into()),
    ];
    let mut config = RuntimeConfig::from_yaml(include_str!("../../../release/smoke-config.yaml"))
        .expect("offline Echo configuration");
    config.storage.adapter = adapter;
    config.access.actions.allow.extend(
        [
            "context.show",
            "presentation.history.append",
            "provider.echo",
        ]
        .map(String::from),
    );
    write_config(&workspace, "worker.yaml", &config);

    // A distinct journal and anchor isolate workspace ownership from database locking.
    config.storage.path = "competing.state".into();
    let KeyConfig::Environment { anchor_path, .. } = &mut config.storage.keys else {
        panic!("protected fixture requires environment keys");
    };
    *anchor_path = "competing.anchor".into();
    write_config(&workspace, "competing.yaml", &config);

    let cli = |config: &str| {
        let mut command = Command::new(binary);
        command
            .current_dir(&workspace)
            .envs(environment.iter().map(|(key, value)| (*key, value)))
            .args(["--config", config]);
        command
    };
    let run =
        |config, args: &[&str]| bounded_output(cli(config).args(args), &workspace, WORKER_TIMEOUT);
    let mut worker = Worker(
        cli("worker.yaml")
            .args(["--approval-mode", "ask", "worker"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start workspace worker"),
    );
    let deadline = Instant::now() + WORKER_TIMEOUT;
    loop {
        let status = bounded_output(
            cli("worker.yaml").args(["worker", "--status"]),
            &workspace,
            deadline.saturating_duration_since(Instant::now()),
        );
        if status.status.success() {
            break;
        }
        assert!(
            worker.0.try_wait().expect("worker status").is_none() && Instant::now() < deadline,
            "{name} worker failed to become ready: {}",
            String::from_utf8_lossy(&status.stderr),
        );
        thread::sleep(Duration::from_millis(25));
    }
    let doctor = json_output(name, run("worker.yaml", &["state", "doctor"]));
    assert_eq!(doctor["storage"]["adapter"], name);
    assert_eq!(doctor["recovery_mode"], false);

    let competing = run("competing.yaml", &["worker"]);
    let error = String::from_utf8_lossy(&competing.stderr);
    assert!(
        !competing.status.success() && error.contains("runtime writer lease is already held"),
        "{name} competing worker exited with {}: {error}",
        competing.status,
    );
    assert!(!workspace.join("competing.state").exists());

    let terminals = PROMPTS.map(|_| {
        let mut command = CommandBuilder::new(binary);
        command.cwd(&workspace);
        command.args(["--config", "worker.yaml", "--worker-required", "tui"]);
        for (key, value) in &environment {
            command.env(key, value);
        }
        Terminal::start(command)
    });
    for (terminal, prompt) in terminals.iter().zip(PROMPTS) {
        terminal.wait("Enter sends");
        terminal.send(prompt.as_bytes());
        terminal.wait_for_composer(prompt, |draft| draft == prompt);
    }
    for terminal in &terminals {
        terminal.send(b"\r");
    }
    for (terminal, prompt) in terminals.iter().zip(PROMPTS) {
        terminal.wait_until_visual_rows(prompt, |screen, rows| {
            screen.contains("ok · approval")
                && rows.windows(2).any(|pair| {
                    pair[0].starts_with("● Colossus") && pair[1].trim_start().starts_with(prompt)
                })
        });
    }

    let sessions = json_output(name, run("worker.yaml", &["sessions", "list"]));
    let sessions = sessions.as_array().expect("session list");
    assert_eq!(
        sessions.len(),
        PROMPTS.len(),
        "{name} sessions: {sessions:?}"
    );
    for prompt in PROMPTS {
        let session = sessions
            .iter()
            .find(|session| session["last_user_preview"] == prompt)
            .expect("each prompt belongs to its own session");
        let id = session["id"].as_str().expect("session identifier");
        let transcript = json_output(name, run("worker.yaml", &["sessions", "messages", id]));
        assert_eq!(transcript.as_array().map(Vec::len), Some(2));
        assert_eq!(transcript[0]["message"]["role"], "user");
        assert_eq!(transcript[0]["message"]["content"], prompt);
        assert_eq!(transcript[1]["message"]["role"], "assistant");
        assert_eq!(transcript[1]["message"]["content"], prompt);
    }

    drop(terminals);
    json_output(name, run("worker.yaml", &["worker", "--shutdown"]));
    assert!(
        worker.wait(WORKER_TIMEOUT).success(),
        "{name} worker shutdown"
    );
    let reopened = json_output(name, run("competing.yaml", &["state", "doctor"]));
    assert_eq!(reopened["storage"]["adapter"], name);
    assert_eq!(reopened["recovery_mode"], false);
    assert!(workspace.join("competing.state").is_file());
    eprintln!(
        "{name}: two terminal sessions completed; independent worker required release of the workspace"
    );
}

fn write_config(workspace: &Path, file: &str, config: &RuntimeConfig) {
    fs::write(
        workspace.join(file),
        config.to_resolved_yaml().expect("serialize test config"),
    )
    .expect("write test config");
}

fn json_output(backend: &str, output: Output) -> Value {
    assert!(
        output.status.success(),
        "{backend} CLI failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

fn bounded_output(command: &mut Command, workspace: &Path, timeout: Duration) -> Output {
    let stdout = tempfile::tempfile_in(workspace).expect("private CLI stdout");
    let stderr = tempfile::tempfile_in(workspace).expect("private CLI stderr");
    let mut child = Worker(
        command
            .stdin(Stdio::null())
            .stdout(stdout.try_clone().expect("CLI stdout handle"))
            .stderr(stderr.try_clone().expect("CLI stderr handle"))
            .spawn()
            .expect("start CLI command"),
    );
    Output {
        status: child.wait(timeout),
        stdout: read_capture(stdout),
        stderr: read_capture(stderr),
    }
}

fn read_capture(mut file: File) -> Vec<u8> {
    file.rewind().expect("rewind CLI output");
    let mut bytes = Vec::new();
    file.take(1024 * 1024)
        .read_to_end(&mut bytes)
        .expect("read CLI output");
    bytes
}

struct Worker(Child);

impl Worker {
    fn wait(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.0.try_wait().expect("worker wait") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "CLI process did not exit within {timeout:?}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
