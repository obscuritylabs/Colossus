use super::*;
use portable_pty::CommandBuilder;
use terminal_support::Terminal;

struct Worker(std::process::Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn process(workspace: &Path, home: &process_support::IsolatedUserHome) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_colossus"));
    command
        .current_dir(workspace)
        .args(["--config", "config.json"])
        .env("HOME", home.path())
        .env("COLOSSUS_HOME", home.colossus_home())
        .env("COLOSSUS_APPROVAL_TEST_JOURNAL_KEY", JOURNAL_KEY)
        .env("COLOSSUS_APPROVAL_TEST_SIGNING_KEY", SIGNING_KEY);
    #[cfg(windows)]
    command
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.local_app_data())
        .env("TEMP", home.temporary_directory())
        .env("TMP", home.temporary_directory());
    command
}

#[test]
fn both_tui_hosts_review_long_argv_before_deciding_in_a_real_pty() {
    for worker_host in [false, true] {
        let directory = tempdir().unwrap();
        let workspace = directory.path().canonicalize().unwrap();
        let home = process_support::isolated_user_home(&workspace);
        // Explicit argv form: no model-provided display command or inferred interpreter.
        let marker = workspace.join("approved-marker.txt");
        let executable = if cfg!(windows) { "cmd.exe" } else { "/bin/sh" };
        let script = if cfg!(windows) {
            format!(
                "echo approved>>approved-marker.txt & rem {} COMMAND_TAIL",
                "x".repeat(1500)
            )
        } else {
            format!(
                "printf 'approved\\n' >> approved-marker.txt # {} COMMAND_TAIL",
                "x".repeat(2500)
            )
        };
        let (origin, provider) = command_approval::server(
            worker_host,
            json!({
                "argv": [executable, if cfg!(windows) { "/C" } else { "-c" }, script], "cwd": "."
            }),
        );
        let config = command_approval::config(&workspace, &origin);
        let mut document: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        document["access"]["actions"]["allow"] = json!([
            "provider.openai.chat",
            "context.show",
            "presentation.history.append",
            "plugin.list"
        ]);
        fs::write(config, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
        let _worker = worker_host.then(|| {
            let child = process(&workspace, &home)
                .args(["--approval-mode", "ask", "worker"])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let worker = Worker(child);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let status = process(&workspace, &home)
                    .args(["worker", "--status"])
                    .output()
                    .unwrap();
                if status.status.success() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "worker did not start: {}",
                    String::from_utf8_lossy(&status.stderr)
                );
                thread::sleep(Duration::from_millis(50));
            }
            worker
        });
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_colossus"));
        command.cwd(&workspace);
        command.args(["--config", "config.json", "--alt-screen"]);
        if worker_host {
            command.arg("--worker-required");
        } else {
            command.args(["--approval-mode", "ask"]);
        }
        command.arg("tui");
        command.env("HOME", home.path());
        command.env("COLOSSUS_HOME", home.colossus_home());
        command.env("COLOSSUS_APPROVAL_TEST_JOURNAL_KEY", JOURNAL_KEY);
        command.env("COLOSSUS_APPROVAL_TEST_SIGNING_KEY", SIGNING_KEY);
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
        terminal.send(b"Verify the command approval marker.\r");
        terminal.wait("Approval required");
        assert!(!marker.exists());
        terminal.resize_frame(32, 60);
        terminal.wait_until("approval after narrow resize", |screen| {
            screen.contains("Approval required")
                && screen
                    .lines()
                    .any(|line| line.ends_with('┐') && line.chars().count() == 60)
        });
        terminal.send(b"r");
        // PageDown scrolls only the request, never changes the decision.
        for _ in 0..35 {
            terminal.send(b"\x1b[6~");
        }
        terminal.wait("COMMAND_TAIL");
        assert!(!marker.exists());
        terminal.send(if worker_host { b"a" } else { b"d" });
        assert!(!marker.exists(), "selecting a decision must not submit it");
        terminal.send(b"\r");
        if worker_host {
            terminal.wait("Command finished.");
            assert_eq!(fs::read_to_string(&marker).unwrap().trim(), "approved");
        } else {
            terminal.wait("declined");
            assert!(!marker.exists());
        }
        assert_eq!(
            provider.join().unwrap().len(),
            if worker_host { 3 } else { 2 }
        );
    }
}
