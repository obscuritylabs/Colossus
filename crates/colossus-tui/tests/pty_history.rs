//! PTY regression for durable transcript preservation during typing and resize.

use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, AgentRunResult, ModelMessage, ModelMessageRole, ProviderEvent, RunEvent,
    RunEventEnvelope, SessionMessage, SessionMessagePage, TerminalPreferences, ToolCall,
    ToolResult,
};
use colossus_ports::RunControl;
use colossus_presentation::{PresentationBlock, PresentationDocument};
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostPlanExecutionResult,
    HostRunResult, InteractiveHost, InteractivePlanExecutionRequest, InteractiveRunRequest,
    InteractiveSnapshot, PlanSelectionUpdate, RuntimeCommand, ScreenMode, TuiOptions, run_tui,
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
        let inline = std::env::var("COLOSSUS_TUI_MODE").as_deref() == Ok("inline");
        let first_sequence = if inline { 2 } else { 1 };
        let messages = (first_sequence..=5)
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
                before_sequence: inline.then_some(2),
                has_more: inline,
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
        _control: RunControl,
    ) -> Result<HostCommandResult, String> {
        Ok(HostCommandResult::document(
            PresentationDocument::from_block(PresentationBlock::Text("ok".into())),
        ))
    }

    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        _control: RunControl,
    ) -> Result<HostRunResult, String> {
        if std::env::var_os("COLOSSUS_TUI_STREAM_FIXTURE").is_none() {
            return Err("fixture does not run model turns".into());
        }
        let output = (1..=30)
            .map(|row| format!("stream-final-row-{row:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let call = ToolCall {
            call_id: "call-scrollback".into(),
            name: "filesystem.search".into(),
            arguments: serde_json::json!({"query": "Runtime"}),
        };
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: request.session_id.clone(),
                event: RunEvent::Provider {
                    event: ProviderEvent::ModelDelta {
                        text: "commentary-before-tool".into(),
                    },
                },
            }))
            .await
            .map_err(|_| "stream receiver closed")?;
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: request.session_id.clone(),
                event: RunEvent::ToolStarted {
                    turn: 1,
                    call: call.clone(),
                    elapsed_seconds: 0.1,
                },
            }))
            .await
            .map_err(|_| "stream receiver closed")?;
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: request.session_id.clone(),
                event: RunEvent::ToolCompleted {
                    turn: 1,
                    result: ToolResult {
                        call_id: call.call_id,
                        name: call.name,
                        output: serde_json::json!({"matches": ["runtime.rs"]}).to_string(),
                        exit_code: 0,
                    },
                    duration_seconds: 0.2,
                    elapsed_seconds: 0.3,
                },
            }))
            .await
            .map_err(|_| "stream receiver closed")?;
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: request.session_id.clone(),
                event: RunEvent::Provider {
                    event: ProviderEvent::ModelDelta {
                        text: output.clone(),
                    },
                },
            }))
            .await
            .map_err(|_| "stream receiver closed")?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        events
            .send(HostEvent::Run(RunEventEnvelope {
                schema_version: 1,
                run_id: "run-stream-pty".into(),
                session_id: request.session_id.clone(),
                event: RunEvent::Provider {
                    event: ProviderEvent::FinalOutput {
                        text: output.clone(),
                    },
                },
            }))
            .await
            .map_err(|_| "stream receiver closed")?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(HostRunResult {
            outcome: AgentRunOutcome::Completed {
                result: AgentRunResult {
                    run_id: "run-stream-pty".into(),
                    session_id: Some(request.session_id),
                    role: "primary".into(),
                    profile: "fixture".into(),
                    model_profile: "fixture".into(),
                    provider_profile: "local".into(),
                    model: "fixture".into(),
                    plan: None,
                    output,
                    event_count: 2,
                    elapsed_seconds: 0.3,
                },
            },
            footer: FooterState {
                role: "primary".into(),
                route: "fixture@local".into(),
                context: Some((35, 32_768)),
                message_count: 7,
                status: "ready".into(),
                approval_mode: "ask".into(),
            },
            plan_selection: PlanSelectionUpdate::Unchanged,
        })
    }

    async fn run_plan_execution(
        &self,
        _request: InteractivePlanExecutionRequest,
        _events: mpsc::Sender<HostEvent>,
        _control: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        Err("fixture does not execute plans".into())
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
        before_sequence: u64,
    ) -> Result<SessionMessagePage, String> {
        assert_eq!(before_sequence, 2);
        Ok(SessionMessagePage {
            messages: vec![SessionMessage {
                session_id: "019f-pty".into(),
                run_id: "run-pty".into(),
                sequence: 1,
                message: ModelMessage {
                    role: ModelMessageRole::Assistant,
                    content: "durable-row-01".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            }],
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
    wait_for_screen(&output, 24, 80, "fixture@local");
    let rows = screen_rows(&output, 24, 80);
    let latest_row = rows
        .iter()
        .position(|row| row.contains("durable-row-05"))
        .expect("newest transcript row");
    let composer_row = rows
        .iter()
        .position(|row| row.contains("Message · Enter sends"))
        .expect("composer row");
    assert_eq!(
        composer_row,
        latest_row + 1,
        "newest output should end directly above the composer: {rows:?}"
    );
    writer.write_all(&[4]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(
        status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.lock().expect("output"))
    );
    drop(writer);
    reader_thread.join().expect("reader thread");

    let raw = output.lock().expect("output");
    assert_eq!(
        raw.windows(b"\x1b[6n".len())
            .filter(|window| *window == b"\x1b[6n")
            .count(),
        1,
        "inline rendering should only query the cursor during startup"
    );
    assert!(
        raw.windows(b"durable-row-01".len())
            .any(|window| window == b"durable-row-01"),
        "inline startup did not preload the older transcript page"
    );
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
    assert!(
        !raw.windows(b"\x1b[?1000h".len())
            .any(|window| window == b"\x1b[?1000h"),
        "inline mode must preserve native mouse scrollback"
    );
}

#[test]
fn completed_streaming_output_moves_immediately_into_native_scrollback() {
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
    command.env("COLOSSUS_TUI_STREAM_FIXTURE", "1");
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
    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    writer.write_all(b"go\r").expect("submit fixture turn");
    writer.flush().expect("flush fixture turn");

    wait_for_screen(&output, 24, 80, "stream-final-row-30");
    thread::sleep(Duration::from_millis(400));
    let rows = screen_rows(&output, 24, 80);
    let latest_row = rows
        .iter()
        .position(|row| row.contains("stream-final-row-30"))
        .expect("final output row");
    let composer_row = rows
        .iter()
        .position(|row| row.contains("Message · Enter sends"))
        .expect("composer row");
    assert!(
        composer_row <= latest_row + 2,
        "completed output should leave at most the comfortable-density separator above the composer: {rows:?}"
    );
    assert!(
        scrollback_contains(&output, 24, 80, "stream-final-row-01"),
        "early completed rows should be present in native terminal scrollback"
    );
    assert!(
        scrollback_contains(&output, 24, 80, "commentary-before-tool"),
        "commentary preceding a tool call should be present in native terminal scrollback"
    );
    assert!(
        scrollback_contains(&output, 24, 80, "Completed filesystem.search"),
        "completed tool activity should be present in native terminal scrollback"
    );

    writer.write_all(&[4]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
}

#[test]
fn typing_tab_completion_and_resize_never_erase_visible_transcript_rows() {
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
    writer.write_all(b"/to\t").expect("type completion");
    writer.flush().expect("flush completion");
    wait_for_screen(&output, 24, 80, "/tools");
    writer
        .write_all(&[127; 6])
        .expect("erase completed draft with Backspace");
    writer.flush().expect("flush clear");
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

    writer.write_all(&[3]).expect("Ctrl-C exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
    let raw = output.lock().expect("output");
    assert!(
        raw.windows(b"\x1b[?1000h".len())
            .any(|window| window == b"\x1b[?1000h"),
        "alternate-screen mouse capture was not enabled"
    );
    assert!(
        raw.windows(b"\x1b[?1000l".len())
            .any(|window| window == b"\x1b[?1000l"),
        "alternate-screen mouse capture was not restored"
    );
}

fn wait_for_raw(output: &Arc<Mutex<Vec<u8>>>, needle: &[u8]) {
    wait_for_raw_count(output, needle, 1);
}

fn wait_for_raw_count(output: &Arc<Mutex<Vec<u8>>>, needle: &[u8], expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let count = output
            .lock()
            .expect("output")
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count();
        if count >= expected {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("PTY never emitted {expected} instances of expected terminal query");
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

fn screen_rows(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16) -> Vec<String> {
    let bytes = output.lock().expect("output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(&bytes);
    parser.screen().rows(0, cols).collect()
}

fn scrollback_contains(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16, needle: &str) -> bool {
    let bytes = output.lock().expect("output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 256);
    parser.process(&bytes);
    (0..=256).any(|offset| {
        parser.screen_mut().set_scrollback(offset);
        parser.screen().contents().contains(needle)
    })
}
