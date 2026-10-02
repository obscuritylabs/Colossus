//! Best-effort Herdr presence and session restore for terminal Colossus.

use crate::{ApprovalMode, Cli, OutputMode};
use colossus_tui::InteractiveLifecycleObserver;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SOURCE: &str = "colossus";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AgentState {
    Idle,
    Working,
    Blocked,
}

impl AgentState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
        }
    }
}

struct HerdrEnvironment {
    binary: PathBuf,
    pane_id: String,
}

impl HerdrEnvironment {
    fn detect() -> Option<Self> {
        (std::env::var_os("HERDR_ENV").as_deref() == Some(OsStr::new("1"))).then_some(())?;
        let binary = PathBuf::from(std::env::var_os("HERDR_BIN_PATH")?);
        let pane_id = std::env::var("HERDR_PANE_ID").ok()?;
        let socket = std::env::var_os("HERDR_SOCKET_PATH")?;
        if binary.as_os_str().is_empty() || pane_id.is_empty() || socket.is_empty() {
            return None;
        }
        Some(Self { binary, pane_id })
    }
}

#[derive(Clone)]
struct ResumeOptions {
    workspace: Option<String>,
    config: Option<String>,
    approval_mode: Option<&'static str>,
    output: Option<&'static str>,
    alt_screen: bool,
    worker_required: bool,
    resumable: bool,
}

impl ResumeOptions {
    fn from_cli(cli: &Cli, workspace: &Path, config: &Path, interactive_tui: bool) -> Self {
        Self {
            workspace: workspace.to_str().map(str::to_owned),
            config: config.to_str().map(str::to_owned),
            approval_mode: cli.approval_mode.map(|mode| match mode {
                ApprovalMode::Deny => "deny",
                ApprovalMode::Ask => "ask",
                ApprovalMode::RiskAuto => "risk-auto",
                ApprovalMode::FullAccess => "full-access",
            }),
            output: match cli.output {
                OutputMode::Auto => None,
                OutputMode::Human => Some("human"),
                OutputMode::Json => Some("json"),
            },
            alt_screen: cli.alt_screen,
            worker_required: cli.worker_required,
            // The managed Desktop TUI receives a one-time authenticated channel
            // that cannot be recreated by a Herdr resume command.
            resumable: interactive_tui && !cli.desktop_worker_auth,
        }
    }

    fn argv(&self, session_id: &str, approval_mode: Option<&str>) -> Option<Vec<String>> {
        if !self.resumable {
            return None;
        }
        let mut argv = vec![
            "colossus".to_owned(),
            "--workspace".to_owned(),
            self.workspace.clone()?,
        ];
        if let Some(config) = &self.config {
            argv.extend(["--config".to_owned(), config.clone()]);
        }
        let mode = approval_mode
            .and_then(|mode| match mode {
                "deny" => Some("deny"),
                "ask" => Some("ask"),
                "risk-auto" => Some("risk-auto"),
                "full-access" => Some("full-access"),
                _ => None,
            })
            .or(self.approval_mode);
        if let Some(mode) = mode {
            argv.extend(["--approval-mode".to_owned(), mode.to_owned()]);
        }
        if let Some(output) = self.output {
            argv.extend(["--output".to_owned(), output.to_owned()]);
        }
        if self.alt_screen {
            argv.push("--alt-screen".to_owned());
        }
        if self.worker_required {
            argv.push("--worker-required".to_owned());
        }
        argv.extend([
            "tui".to_owned(),
            "--session".to_owned(),
            session_id.to_owned(),
        ]);
        let bytes = argv.iter().map(String::len).sum::<usize>();
        (argv.len() <= 64
            && bytes <= 8 * 1024
            && argv
                .iter()
                .all(|arg| !arg.contains('\'') && !arg.chars().any(char::is_control)))
        .then_some(argv)
    }
}

struct Report {
    session_id: String,
    state: AgentState,
    approval_mode: Option<String>,
}

#[derive(Default)]
struct Queue {
    pending: Option<Report>,
    last_observed: Option<(String, AgentState, Option<String>)>,
    closing: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

/// Pane-local reporter. Its worker sends only the latest queued state and exits
/// after a bounded release attempt when the terminal closes.
pub(super) struct HerdrReporter {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl HerdrReporter {
    pub(super) fn from_env(
        cli: &Cli,
        workspace: &Path,
        config: &Path,
        interactive_tui: bool,
    ) -> Option<Arc<Self>> {
        let environment = HerdrEnvironment::detect()?;
        let resume = ResumeOptions::from_cli(cli, workspace, config, interactive_tui);
        Self::start(environment, resume)
    }

    fn start(environment: HerdrEnvironment, resume: ResumeOptions) -> Option<Arc<Self>> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("colossus-herdr".into())
            .spawn(move || run_reporter(environment, resume, worker_shared))
            .ok()?;
        Some(Arc::new(Self {
            shared,
            worker: Some(worker),
        }))
    }

    pub(super) fn report(&self, session_id: &str, state: AgentState) {
        self.report_with_mode(session_id, state, None);
    }

    fn report_with_mode(&self, session_id: &str, state: AgentState, approval_mode: Option<&str>) {
        let Ok(mut queue) = self.shared.queue.lock() else {
            return;
        };
        let approval_mode = approval_mode.map(str::to_owned);
        if queue.closing
            || queue.last_observed.as_ref().is_some_and(|previous| {
                previous.0 == session_id && previous.1 == state && previous.2 == approval_mode
            })
        {
            return;
        }
        queue.last_observed = Some((session_id.to_owned(), state, approval_mode.clone()));
        queue.pending = Some(Report {
            session_id: session_id.to_owned(),
            state,
            approval_mode,
        });
        self.shared.wake.notify_one();
    }
}

impl InteractiveLifecycleObserver for HerdrReporter {
    fn observe(&self, session_id: &str, working: bool, blocked: bool, approval_mode: &str) {
        let state = if blocked {
            AgentState::Blocked
        } else if working {
            AgentState::Working
        } else {
            AgentState::Idle
        };
        self.report_with_mode(session_id, state, Some(approval_mode));
    }
}

impl Drop for HerdrReporter {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.closing = true;
            queue.pending = None;
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_reporter(environment: HerdrEnvironment, resume: ResumeOptions, shared: Arc<Shared>) {
    let mut sequence = 0;
    loop {
        let report = {
            let Ok(mut queue) = shared.queue.lock() else {
                break;
            };
            while queue.pending.is_none() && !queue.closing {
                let Ok(next) = shared.wake.wait(queue) else {
                    return;
                };
                queue = next;
            }
            if queue.closing {
                None
            } else {
                queue.pending.take()
            }
        };
        let Some(report) = report else {
            break;
        };
        sequence = next_sequence(sequence);
        let mut args = vec![
            "pane".to_owned(),
            "report-agent".to_owned(),
            environment.pane_id.clone(),
            "--source".to_owned(),
            SOURCE.to_owned(),
            "--agent".to_owned(),
            SOURCE.to_owned(),
            "--state".to_owned(),
            report.state.as_str().to_owned(),
            "--seq".to_owned(),
            sequence.to_string(),
        ];
        if report.state == AgentState::Blocked {
            args.extend([
                "--message".to_owned(),
                "Waiting for a decision in Colossus".to_owned(),
            ]);
        }
        if let Some(argv) = resume.argv(&report.session_id, report.approval_mode.as_deref()) {
            args.extend([
                "--agent-session-id".to_owned(),
                report.session_id,
                "--".to_owned(),
            ]);
            args.extend(argv);
        }
        run_command(&environment.binary, &args);
    }
    sequence = next_sequence(sequence);
    run_command(
        &environment.binary,
        &[
            "pane".to_owned(),
            "release-agent".to_owned(),
            environment.pane_id,
            "--source".to_owned(),
            SOURCE.to_owned(),
            "--agent".to_owned(),
            SOURCE.to_owned(),
            "--seq".to_owned(),
            sequence.to_string(),
        ],
    );
}

fn next_sequence(previous: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .unwrap_or(0);
    now.max(previous.saturating_add(1))
}

fn run_command(binary: &Path, args: &[String]) {
    let Ok(mut child) = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    #[test]
    fn resume_uses_the_selected_config_even_when_cli_did_not_name_one() {
        let cli = Cli::parse_from(["colossus", "tui"]);
        let options = ResumeOptions::from_cli(
            &cli,
            Path::new("/work/repo"),
            Path::new("/home/user/.config/colossus/config.yaml"),
            true,
        );
        let argv = options.argv("session-1", Some("ask")).expect("resume argv");
        assert_eq!(
            argv.windows(2)
                .find(|pair| pair[0] == "--config")
                .map(|pair| pair[1].as_str()),
            Some("/home/user/.config/colossus/config.yaml")
        );
    }

    #[test]
    fn resume_argv_preserves_terminal_selection_and_rejects_herdr_invalid_arguments() {
        let options = ResumeOptions {
            workspace: Some("/work/repo".into()),
            config: Some("/work/config.yaml".into()),
            approval_mode: Some("risk-auto"),
            output: None,
            alt_screen: true,
            worker_required: false,
            resumable: true,
        };
        assert_eq!(
            options.argv("session-1", None),
            Some(
                vec![
                    "colossus",
                    "--workspace",
                    "/work/repo",
                    "--config",
                    "/work/config.yaml",
                    "--approval-mode",
                    "risk-auto",
                    "--alt-screen",
                    "tui",
                    "--session",
                    "session-1",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            )
        );
        assert!(options.argv("bad'session", None).is_none());
        assert!(options.argv("bad\nsession", None).is_none());
        assert!(
            ResumeOptions {
                workspace: Some("a".repeat(8192)),
                ..options.clone()
            }
            .argv("session-1", None)
            .is_none()
        );
        assert!(
            ResumeOptions {
                resumable: false,
                ..options
            }
            .argv("session-1", None)
            .is_none()
        );
    }

    #[test]
    fn sequence_increases_within_a_process() {
        let first = next_sequence(0);
        assert!(next_sequence(first) > first);
    }

    #[cfg(unix)]
    #[test]
    fn reports_state_session_switch_and_release_through_herdr_cli() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let directory = tempfile::tempdir().expect("temporary Herdr fixture");
        let binary = directory.path().join("herdr");
        let output = directory.path().join("reports");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
                output.display()
            ),
        )
        .expect("write fake Herdr CLI");
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))
            .expect("make fake Herdr executable");
        let reporter = HerdrReporter::start(
            HerdrEnvironment {
                binary,
                pane_id: "pane-1".into(),
            },
            ResumeOptions {
                workspace: Some("/work/repo".into()),
                config: None,
                approval_mode: None,
                output: None,
                alt_screen: false,
                worker_required: false,
                resumable: true,
            },
        )
        .expect("start reporter");
        reporter.report_with_mode("session-1", AgentState::Idle, Some("ask"));
        wait_for_lines(&output, 1);
        reporter.report_with_mode("session-1", AgentState::Blocked, Some("deny"));
        wait_for_lines(&output, 2);
        reporter.report_with_mode("session-2", AgentState::Working, Some("deny"));
        wait_for_lines(&output, 3);
        drop(reporter);

        let lines = fs::read_to_string(output).expect("read fake Herdr reports");
        let lines = lines.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains(
            "pane report-agent pane-1 --source colossus --agent colossus --state idle --seq"
        ));
        assert!(lines[0].contains("--agent-session-id session-1 -- colossus --workspace /work/repo --approval-mode ask tui --session session-1"));
        assert!(lines[1].contains("--state blocked"));
        assert!(lines[1].contains("--message Waiting for a decision in Colossus"));
        assert!(lines[1].contains("--approval-mode deny"));
        assert!(lines[2].contains("--state working"));
        assert!(lines[2].contains("--agent-session-id session-2"));
        assert!(
            lines[3].contains("pane release-agent pane-1 --source colossus --agent colossus --seq")
        );
    }

    #[cfg(unix)]
    fn wait_for_lines(path: &Path, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if std::fs::read_to_string(path).is_ok_and(|content| content.lines().count() >= count) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("fake Herdr CLI did not receive {count} reports");
    }
}
