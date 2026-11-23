//! Support reporting through the real CLI, native PTY, and local worker IPC.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;
#[path = "support/terminal.rs"]
#[allow(dead_code)]
mod terminal_support;

use portable_pty::CommandBuilder;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use terminal_support::Terminal;

const CONFIG: &str = "config.yaml";

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    binary: PathBuf,
    home: process_support::IsolatedUserHome,
}

impl Fixture {
    fn new() -> Self {
        let root = process_support::tempdir().expect("private test directory");
        let workspace = root.path().canonicalize().unwrap().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let home = process_support::isolated_user_home(root.path());
        let binary = workspace.join(if cfg!(windows) {
            "colossus-report-test.exe"
        } else {
            "colossus-report-test"
        });
        fs::copy(env!("CARGO_BIN_EXE_colossus"), &binary).expect("copy binary outside checkout");
        let config = include_str!("../../../release/smoke-config.yaml").replace(
            "allow: [plugin.list]",
            "allow: [context.show, presentation.history.append, plugin.list]",
        );
        fs::write(workspace.join(CONFIG), config).expect("offline test configuration");
        Self {
            _root: root,
            workspace,
            binary,
            home,
        }
    }

    fn environment(&self) -> impl IntoIterator<Item = (&'static str, OsString)> {
        [
            ("HOME", self.home.path().into()),
            ("COLOSSUS_HOME", self.home.colossus_home().into()),
            ("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64).into()),
            ("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64).into()),
            #[cfg(windows)]
            ("USERPROFILE", self.home.path().into()),
            #[cfg(windows)]
            ("LOCALAPPDATA", self.home.local_app_data().into()),
            #[cfg(windows)]
            ("TEMP", self.home.temporary_directory().into()),
            #[cfg(windows)]
            ("TMP", self.home.temporary_directory().into()),
        ]
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .current_dir(&self.workspace)
            .args(["--config", CONFIG])
            .envs(self.environment());
        command
    }

    fn terminal(&self, worker: bool) -> Terminal {
        let mut command = CommandBuilder::new(&self.binary);
        command.cwd(&self.workspace);
        command.args(["--config", CONFIG, "--alt-screen"]);
        if worker {
            command.arg("--worker-required");
        }
        command.arg("tui");
        for (key, value) in self.environment() {
            command.env(key, value);
        }
        let terminal = Terminal::start(command);
        terminal.wait("Enter sends");
        terminal
    }

    fn start_worker(&self) -> Worker {
        let mut worker = Worker(
            self.command()
                .env_remove("RUST_MIN_STACK")
                .arg("worker")
                .stdout(Stdio::null())
                .spawn()
                .expect("start offline worker"),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = self
                .command()
                .args(["worker", "--status"])
                .output()
                .expect("worker status");
            if output.status.success() {
                return worker;
            }
            assert!(
                worker.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "worker readiness failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn assert_no_durable_input(&self) {
        for arguments in [["sessions", "list"], ["preferences", "history"]] {
            let output = self.command().args(arguments).output().unwrap();
            assert!(
                output.status.success(),
                "{arguments:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                json!([]),
                "{arguments:?}"
            );
        }
    }
}

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn report_command(terminal: &Terminal, arguments: &str, notice: &str) {
    let input = format!("/support-report {arguments}");
    terminal.send(format!("\x1b[200~{input}\x1b[201~").as_bytes());
    terminal.wait_until("report command in the composer", |screen| {
        screen.contains("Support report · Enter saves") && screen.contains(&input)
    });
    terminal.send(b"\r");
    let screen = terminal.wait_until(notice, |screen| {
        screen.contains(notice) && !screen.contains("Support report · Enter saves")
    });
    assert!(
        !screen.contains(&input),
        "submitted report path was rendered"
    );
}

fn assert_report(path: &Path, runtime_mode: &str, worker: Value) {
    let report: Value = serde_json::from_slice(&fs::read(path).expect("local report")).unwrap();
    assert_eq!(
        report,
        json!({
            "schema_version": 1,
            "colossus_version": env!("CARGO_PKG_VERSION"),
            "platform": {
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
            },
            "runtime_mode": runtime_mode,
            "worker": worker,
        })
    );
}

#[test]
fn embedded_tui_report_preserves_the_fresh_draft_and_history() {
    let fixture = Fixture::new();
    let terminal = fixture.terminal(false);
    let output = fixture.workspace.join("private report.json");
    fs::write(&output, b"previous report").unwrap();

    report_command(
        &terminal,
        r#""private report.json""#,
        "File exists · add --replace",
    );
    assert_eq!(fs::read(&output).unwrap(), b"previous report");
    report_command(
        &terminal,
        r#""private report.json" --replace"#,
        "Support report saved.",
    );
    assert_report(&output, "embedded", json!({"status": "not_applicable"}));

    terminal.send(b"\x12"); // Ctrl+R opens the current prompt history.
    terminal.wait_until("empty prompt history after reporting", |screen| {
        screen.contains("Prompt history") && screen.contains("No matching prompts")
    });
    drop(terminal);
    fixture.assert_no_durable_input();
}

#[test]
fn worker_tui_saves_a_local_report_after_an_observed_disconnect() {
    let fixture = Fixture::new();
    let worker = fixture.start_worker();
    let terminal = fixture.terminal(true);
    report_command(&terminal, "before.json", "Support report saved.");
    assert_report(
        &fixture.workspace.join("before.json"),
        "worker",
        json!({"status": "last_observed_available"}),
    );
    fixture.assert_no_durable_input();

    drop(worker);
    terminal.command(
        r#"/integration call github.pull_request {"owner":"acme","repo":"tools","number":42}"#,
        "worker is unavailable",
    );
    report_command(&terminal, "after.json", "Support report saved.");
    assert_report(
        &fixture.workspace.join("after.json"),
        "worker",
        json!({"status": "unavailable", "reason": "connection_unavailable"}),
    );
}
