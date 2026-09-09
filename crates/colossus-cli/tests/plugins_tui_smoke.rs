//! The production TUI, native PTY, and both runtime hosts with an isolated offline home.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;

#[path = "support/terminal.rs"]
mod terminal_support;
use portable_pty::CommandBuilder;
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminal_support::Terminal;

struct WorkerGuard(Child);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn embedded_tui_discovers_selects_and_manages_the_offline_core() {
    exercise(false);
}

#[test]
fn worker_tui_discovers_selects_and_manages_the_offline_core() {
    exercise(true);
}

fn exercise(worker_host: bool) {
    let temporary = process_support::tempdir().expect("private temporary");
    let workspace = temporary.path().canonicalize().expect("workspace");
    let home = process_support::isolated_user_home(&workspace);
    let binary = workspace.join(if cfg!(windows) {
        "copied-colossus.exe"
    } else {
        "copied-colossus"
    });
    fs::copy(env!("CARGO_BIN_EXE_colossus"), &binary).expect("copy executable outside checkout");
    let configuration = include_str!("../../../release/smoke-config.yaml")
        .replace("allow: [plugin.list]", "allow: [context.show, presentation.history.append, plugin.list, plugin.inspect, plugin.skill.read, plugin.resource.list, plugin.resource.read, plugin.enable, plugin.disable]");
    let config = workspace.join("config.yaml");
    fs::write(&config, configuration).expect("isolated configuration");
    let mut command = CommandBuilder::new(&binary);
    command.cwd(&workspace);
    command.args(["--config", "config.yaml", "--alt-screen"]);
    if !worker_host {
        command.args(["--approval-mode", "ask"]);
    }
    if worker_host {
        command.arg("--worker-required");
    }
    command.arg("tui");
    command.env("HOME", home.path());
    command.env("COLOSSUS_HOME", home.colossus_home());
    command.env("TERM", "xterm-256color");
    command.env("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64));
    command.env("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64));
    command.env("HTTP_PROXY", "http://127.0.0.1:1");
    command.env("HTTPS_PROXY", "http://127.0.0.1:1");
    #[cfg(windows)]
    {
        command.env("USERPROFILE", home.path());
        command.env("LOCALAPPDATA", home.local_app_data());
        command.env("TEMP", home.temporary_directory());
        command.env("TMP", home.temporary_directory());
    }
    let _worker = worker_host.then(|| start_worker(&binary, &workspace, &home));
    let terminal = Terminal::start(command);
    terminal.wait("Enter sends");
    terminal.send(b"/plu\t");
    terminal.wait("/plugins");
    terminal.send(b"\x1b\x03");
    let inventory = terminal.command("/plugins", "Bundled with Colossus");
    assert!(inventory.contains("colossus"));
    assert!(!inventory.contains("Item 1"));
    terminal.command("/plugin use colossus", "colossus/plugin-authoring");
    terminal.command("/plugin use colossus/coding", "Active plugin skills");
    terminal.wait("Skills: colossus/coding");
    terminal.command(
        "/plugin resources colossus/plugin-authoring",
        "oci-distribution.md",
    );
    terminal.command(
        "/plugin read colossus/plugin-authoring references/oci-distribution.md",
        "whole-plugin unit",
    );
    terminal.command("/plugin remove colossus/coding", "Active plugin skills");
    terminal.command("/plugins disable colossus", "Active  no");
    terminal.command("/plugins", "disabled");
    terminal.resize(32, 55);
    terminal.command("/plugins", "colossus");
    terminal.command("/plugin invalid", "/plugin expects");
    terminal.command("/plu", "Unknown command");
}

fn start_worker(
    binary: &Path,
    workspace: &Path,
    home: &process_support::IsolatedUserHome,
) -> WorkerGuard {
    let mut command = Command::new(binary);
    command
        .current_dir(workspace)
        .env_remove("RUST_MIN_STACK")
        .args([
            "--config",
            "config.yaml",
            "--approval-mode",
            "ask",
            "worker",
        ])
        .env("HOME", home.path())
        .env("COLOSSUS_HOME", home.colossus_home())
        .env("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64))
        .env("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64))
        .stdout(Stdio::null())
        // Preserve crash diagnostics; an unread pipe hides worker stack aborts.
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    command
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.local_app_data())
        .env("TEMP", home.temporary_directory())
        .env("TMP", home.temporary_directory());
    let worker = WorkerGuard(command.spawn().expect("worker"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let mut ping = Command::new(binary);
        let status = ping
            .current_dir(workspace)
            .args(["--config", "config.yaml", "worker", "--status"])
            .env("HOME", home.path())
            .env("COLOSSUS_HOME", home.colossus_home())
            .env("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64))
            .env("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64))
            .output()
            .expect("worker ping");
        if status.status.success() {
            return worker;
        }
        assert!(
            Instant::now() < deadline,
            "worker readiness: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        thread::sleep(Duration::from_millis(50));
    }
}
