//! PTY regression for durable transcript preservation during typing and resize.

use async_trait::async_trait;
use colossus_contracts::{
    ModelMessage, ModelMessageRole, SessionMessage, SessionMessagePage, TerminalPreferences,
};
use colossus_ports::RunControl;
use colossus_presentation::{PresentationBlock, PresentationDocument};
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostRunResult, InteractiveHost,
    InteractiveRunRequest, InteractiveSnapshot, RuntimeCommand, ScreenMode, TuiOptions, run_tui,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read as _, Write as _},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

struct FixtureHost;

#[async_trait]
impl InteractiveHost for FixtureHost {
    async fn bootstrap(&self, _request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let messages = (1..=5)
            .map(|sequence| SessionMessage {
                session_id: "019f-pty".into(),
                run_id: "run-pty".into(),
                sequence,
                message: ModelMessage {
                    role: ModelMessageRole::Assistant,
                    content: format!("durable-row-{sequence:02}"),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            })
            .collect();
        Ok(InteractiveSnapshot {
            session_id: "019f-pty".into(),
            transcript: SessionMessagePage {
                messages,
                before_sequence: None,
                has_more: false,
            },
            preferences: TerminalPreferences::default(),
            history: Vec::new(),
            completions: vec!["/tools".into()],
            footer: FooterState {
                role: "primary".into(),
                route: "fixture@local".into(),
                context: Some((5, 32_768)),
                message_count: 5,
                status: "ready".into(),
                approval_mode: "ask".into(),
            },
        })
    }

    async fn execute_command(
        &self,
        _command: RuntimeCommand,
        _session_id: &str,
        _sticky_skills: &[String],
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<HostCommandResult, String> {
        Ok(HostCommandResult::document(
            PresentationDocument::from_block(PresentationBlock::Text("ok".into())),
        ))
    }

    async fn run_turn(
        &self,
        _request: InteractiveRunRequest,
        _events: mpsc::Sender<HostEvent>,
        _control: RunControl,
    ) -> Result<HostRunResult, String> {
        Err("fixture does not run model turns".into())
    }

    async fn append_history(&self, _entry: String) -> Result<(), String> {
        Ok(())
    }

    async fn save_preferences(
        &self,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, String> {
        Ok(preferences)
    }

    async fn older_messages(
        &self,
        _session_id: &str,
        _before_sequence: u64,
    ) -> Result<SessionMessagePage, String> {
        Ok(SessionMessagePage {
            messages: Vec::new(),
            before_sequence: None,
            has_more: false,
        })
    }
}

#[test]
fn fixture_process() {
    if std::env::var_os("COLOSSUS_TUI_PTY_FIXTURE").is_none() {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let screen_mode = if std::env::var("COLOSSUS_TUI_MODE").as_deref() == Ok("inline") {
        ScreenMode::Inline
    } else {
        ScreenMode::Alternate
    };
    runtime
        .block_on(run_tui(
            Arc::new(FixtureHost),
            TuiOptions {
                bootstrap: BootstrapRequest::default(),
                screen_mode,
            },
        ))
        .expect("fixture TUI");
}

#[test]
fn inline_mode_preserves_rows_and_restores_terminal_controls() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = CommandBuilder::new(std::env::current_exe().expect("test executable"));
    command.arg("--exact");
    command.arg("fixture_process");
    command.arg("--nocapture");
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "inline");
    let mut child = pair.slave.spawn_command(command).expect("spawn fixture");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8_192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_output
                    .lock()
                    .expect("output")
                    .extend_from_slice(&buffer[..read]),
            }
        }
    });
    let mut writer = pair.master.take_writer().expect("PTY writer");
    wait_for_raw(&output, b"\x1b[6n");
    writer
        .write_all(b"\x1b[1;1R")
        .expect("answer cursor-position query");
    writer.flush().expect("flush cursor-position answer");
    wait_for_screen(&output, 24, 80, "durable-row-05");
    writer.write_all(&[4]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");

    let raw = output.lock().expect("output");
    assert!(
        raw.windows(b"\x1b[?2004h".len())
            .any(|window| window == b"\x1b[?2004h"),
        "bracketed paste was not enabled"
    );
    assert!(
        raw.windows(b"\x1b[?2004l".len())
            .any(|window| window == b"\x1b[?2004l"),
        "bracketed paste was not restored"
    );
    assert!(
        raw.windows(b"\x1b[?25h".len())
            .any(|window| window == b"\x1b[?25h"),
        "cursor visibility was not restored"
    );
}

#[test]
fn typing_and_resize_never_erase_visible_transcript_rows() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = CommandBuilder::new(std::env::current_exe().expect("test executable"));
    command.arg("--exact");
    command.arg("fixture_process");
    command.arg("--nocapture");
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    let mut child = pair.slave.spawn_command(command).expect("spawn fixture");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8_192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => reader_output
                    .lock()
                    .expect("output")
                    .extend_from_slice(&buffer[..read]),
            }
        }
    });
    let mut writer = pair.master.take_writer().expect("PTY writer");

    wait_for_screen(&output, 24, 80, "durable-row-01");
    for character in "typing-preserves-history".chars() {
        writer
            .write_all(character.to_string().as_bytes())
            .expect("type character");
        writer.flush().expect("flush character");
        thread::sleep(Duration::from_millis(8));
    }
    wait_for_screen(&output, 24, 80, "typing-preserves-history");
    let before_resize = screen_contents(&output, 24, 80);
    assert!(before_resize.contains("durable-row-01"), "{before_resize}");
    assert!(before_resize.contains("durable-row-05"), "{before_resize}");

    pair.master
        .resize(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize");
    thread::sleep(Duration::from_millis(150));
    let after_resize = screen_contents(&output, 30, 100);
    assert!(after_resize.contains("durable-row-01"), "{after_resize}");
    assert!(after_resize.contains("durable-row-05"), "{after_resize}");
    assert!(
        after_resize.contains("typing-preserves-history"),
        "{after_resize}"
    );

    writer.write_all(&[3, 4]).expect("clear and exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
}

fn wait_for_raw(output: &Arc<Mutex<Vec<u8>>>, needle: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if output
            .lock()
            .expect("output")
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("PTY never emitted expected terminal query");
}

fn wait_for_screen(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if screen_contents(output, rows, cols).contains(needle) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "screen never contained {needle}: {}",
        screen_contents(output, rows, cols)
    );
}

fn screen_contents(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16) -> String {
    let bytes = output.lock().expect("output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(&bytes);
    parser.screen().contents()
}
