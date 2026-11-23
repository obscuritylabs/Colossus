//! Exercise GitHub pull request inspection through the production PTY and both TUI hosts.

#[path = "support/process.rs"]
#[allow(dead_code)]
mod process_support;
#[path = "support/terminal.rs"]
#[allow(dead_code)]
mod terminal_support;

use portable_pty::CommandBuilder;
use process_support::IsolatedUserHome;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::{ErrorKind, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminal_support::Terminal;

const BINARY: &str = env!("CARGO_BIN_EXE_colossus");
const TOKEN: &str = "fixture-github-token";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const INSPECT: &str =
    r#"/integration call github.pull_request {"owner":"acme","repo":"widget","number":17}"#;

struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn both_tui_hosts_approve_and_render_github_pull_request_evidence() {
    for worker_host in [false, true] {
        let workspace = process_support::tempdir().expect("private workspace");
        let root = workspace.path();
        let home = process_support::isolated_user_home(root);
        let github = TcpListener::bind("127.0.0.1:0").expect("GitHub fixture listener");
        github.set_nonblocking(true).expect("nonblocking listener");
        let origin = format!(
            "http://{}",
            github.local_addr().expect("GitHub fixture origin")
        );
        let base_url = format!("{origin}/api/v3");
        let config = include_str!("../../../release/smoke-config.yaml")
            .replace("profile: pinned", "profile: development")
            .replace(
                "allow: [plugin.list]",
                "allow: [plugin.list, context.show, presentation.history.append]",
            )
            .replace(
                "networkDestinations: []",
                &format!("networkDestinations: [\"{origin}\"]"),
            );
        fs::write(root.join("config.yaml"), config).expect("isolated configuration");

        let connect = process(root, &home)
            .args([
                "--approval-mode",
                "full-access",
                "integrations",
                "connect",
                "github",
                "--base-url",
                &base_url,
                "--credential-reference",
                "env:COLOSSUS_TUI_TEST_GITHUB_TOKEN",
            ])
            .output()
            .expect("connect GitHub fixture");
        assert!(
            connect.status.success(),
            "could not connect GitHub fixture: {}",
            String::from_utf8_lossy(&connect.stderr)
        );

        let _worker = worker_host.then(|| start_worker(root, &home));
        let mut command = CommandBuilder::new(BINARY);
        command.cwd(root);
        command.args([
            "--config",
            "config.yaml",
            "--alt-screen",
            "--approval-mode",
            "ask",
        ]);
        if worker_host {
            command.arg("--worker-required");
        }
        command.arg("tui");
        for (variable, value) in fixture_environment(&home) {
            command.env(variable, value);
        }
        let terminal = Terminal::start(command);
        terminal.wait("Enter sends");

        request_approval(&terminal);
        assert_no_request(&github);
        terminal.send(b"\x1b");
        terminal.wait_until("cancelled approval settles", |screen| {
            screen.contains("declined") && !screen.contains("running /")
        });
        assert_no_request(&github);

        request_approval(&terminal);
        terminal.send(b"a");
        assert_no_request(&github);
        terminal.send(b"\r");
        serve_github_observation(&github);
        let screen = terminal.wait_until(
            "released pull request and both evidence sources",
            |screen| {
                screen.contains("GitHub pull request")
                    && screen.contains(SHA)
                    && screen.contains("Evidence state")
                    && screen.contains("complete")
                    && screen.contains("checks-fixture-build")
                    && screen.contains("status-fixture-deploy")
            },
        );
        assert!(!screen.contains(TOKEN));
        assert_no_request(&github);
    }
}

fn fixture_environment(home: &IsolatedUserHome) -> Vec<(&'static str, OsString)> {
    vec![
        ("HOME", home.path().into()),
        ("COLOSSUS_HOME", home.colossus_home().into()),
        ("COLOSSUS_RELEASE_JOURNAL_KEY", "5".repeat(64).into()),
        ("COLOSSUS_RELEASE_SIGNING_KEY", "6".repeat(64).into()),
        ("COLOSSUS_TUI_TEST_GITHUB_TOKEN", TOKEN.into()),
        #[cfg(windows)]
        ("USERPROFILE", home.path().into()),
        #[cfg(windows)]
        ("LOCALAPPDATA", home.local_app_data().into()),
        #[cfg(windows)]
        ("TEMP", home.temporary_directory().into()),
        #[cfg(windows)]
        ("TMP", home.temporary_directory().into()),
    ]
}

fn process(root: &Path, home: &IsolatedUserHome) -> Command {
    let mut command = Command::new(BINARY);
    command
        .current_dir(root)
        .args(["--config", "config.yaml"])
        .envs(fixture_environment(home));
    command
}

fn start_worker(root: &Path, home: &IsolatedUserHome) -> Worker {
    let worker = Worker(
        process(root, home)
            .env_remove("RUST_MIN_STACK")
            .args(["--approval-mode", "deny", "worker"])
            .stdout(Stdio::null())
            .spawn()
            .expect("start test worker"),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = process(root, home)
            .args(["worker", "--status"])
            .output()
            .expect("worker status");
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

fn request_approval(terminal: &Terminal) {
    terminal.send(format!("\x1b[200~{INSPECT}\x1b[201~").as_bytes());
    terminal.wait_for_composer("GitHub pull request command", |draft| {
        draft.starts_with(INSPECT)
    });
    terminal.send(b"\r");
    let screen = terminal.wait("Approval required");
    assert!(!screen.contains(TOKEN));
}

fn assert_no_request(listener: &TcpListener) {
    let Err(error) = listener.accept() else {
        panic!("GitHub connection opened without a submitted approval or after completion");
    };
    assert_eq!(error.kind(), ErrorKind::WouldBlock);
}

fn serve_github_observation(listener: &TcpListener) {
    let pull = json!({
        "number": 17,
        "state": "open",
        "draft": false,
        "head": {"sha": SHA, "repo": {"full_name": "acme/widget"}}
    });
    let checks = json!({
        "total_count": 1,
        "check_runs": [{
            "id": 1, "name": "checks-fixture-build", "head_sha": SHA,
            "status": "completed", "conclusion": "success"
        }]
    });
    let statuses = json!({
        "sha": SHA,
        "total_count": 1,
        "statuses": [{"id": 2, "context": "status-fixture-deploy", "state": "pending"}]
    });
    let pull_path = "/api/v3/repos/acme/widget/pulls/17";
    let commit_path = format!("/api/v3/repos/acme/widget/commits/{SHA}");
    for (path, response) in [
        (pull_path.into(), &pull),
        (
            format!("{commit_path}/check-runs?per_page=10&page=1"),
            &checks,
        ),
        (
            format!("{commit_path}/status?per_page=10&page=1"),
            &statuses,
        ),
        (pull_path.into(), &pull),
    ] {
        let mut stream = accept_request(listener);
        expect_request(&mut stream, &path, response);
    }
}

fn accept_request(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "approved GitHub request never reached the fixture"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("GitHub fixture accept: {error}"),
        }
    }
}

fn expect_request(stream: &mut TcpStream, path: &str, response: &Value) {
    stream.set_nonblocking(false).expect("blocking stream");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("request deadline");
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.ends_with(b"\r\n\r\n") {
        let count = stream.read(&mut chunk).expect("GitHub request");
        assert!(
            count > 0 && request.len() < 8192,
            "unterminated request headers"
        );
        request.extend_from_slice(&chunk[..count]);
    }
    let request = std::str::from_utf8(&request).expect("HTTP request");
    assert!(
        request.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
        "unexpected GitHub route: {}",
        request.lines().next().unwrap_or_default()
    );
    assert!(
        request
            .lines()
            .any(|line| line.eq_ignore_ascii_case(&format!("authorization: bearer {TOKEN}"))),
        "missing fixture authorization"
    );
    let response = response.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    )
    .expect("reply to GitHub request");
}
