//! PTY regression for durable transcript preservation during typing and resize.

use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, AgentRunResult, ModelMessage, ModelMessageRole, ProviderEvent, RunEvent,
    RunEventEnvelope, SandboxBoundaryMode, SessionMessage, SessionMessagePage, SessionSummary,
    TerminalPreferences, ToolCall, ToolResult,
};
use colossus_ports::RunControl;
use colossus_presentation::{PresentationBlock, PresentationDocument, PresentationTone};
use colossus_tui::{
    BootstrapRequest, FooterState, HostCommandResult, HostEvent, HostPlanExecutionResult,
    HostRunResult, InteractiveHost, InteractivePlanExecutionRequest, InteractiveRunRequest,
    InteractiveSessionBrowser, InteractiveSessionBrowserEntry, InteractiveSessionBrowserMessage,
    InteractiveSnapshot, InteractiveThemePicker, InteractiveThemePickerEntry, PlanSelectionUpdate,
    PromptResponse, RuntimeCommand, ScreenMode, TuiOptions, run_tui,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read as _, Write as _},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

struct FixtureHost;

#[async_trait]
impl InteractiveHost for FixtureHost {
    async fn bootstrap(&self, _request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let inline = std::env::var("COLOSSUS_TUI_MODE").as_deref() == Ok("inline");
        let first_sequence = if inline { 2 } else { 1 };
        let last_sequence = if std::env::var_os("COLOSSUS_TUI_LONG_HISTORY").is_some() {
            30
        } else {
            5
        };
        let messages = (first_sequence..=last_sequence)
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
        let history_navigation_fixture =
            std::env::var_os("COLOSSUS_TUI_HISTORY_NAVIGATION_FIXTURE").is_some();
        Ok(InteractiveSnapshot {
            session_id: "019f-pty".into(),
            transcript: SessionMessagePage {
                messages,
                before_sequence: inline.then_some(2),
                has_more: inline,
            },
            preferences: TerminalPreferences::default(),
            history: if history_navigation_fixture {
                vec![
                    "first prompt".into(),
                    "second prompt".into(),
                    "third prompt".into(),
                ]
            } else {
                Vec::new()
            },
            completions: if history_navigation_fixture {
                vec!["/tools".into(), "@repo-review".into()]
            } else {
                vec!["/tools".into()]
            },
            footer: FooterState {
                role: "primary".into(),
                route: "fixture@local".into(),
                context: Some((5, 32_768)),
                message_count: 5,
                status: "ready".into(),
                approval_mode: "ask".into(),
            },
            pending_sandbox_boundary_acknowledgement: None,
            security_posture: Default::default(),
        })
    }

    async fn acknowledge_sandbox_boundary(
        &self,
        _session_id: &str,
        _mode: SandboxBoundaryMode,
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        Ok(true)
    }

    async fn execute_command(
        &self,
        command: RuntimeCommand,
        _session_id: &str,
        _sticky_skills: &[String],
        events: mpsc::Sender<HostEvent>,
        _control: RunControl,
    ) -> Result<HostCommandResult, String> {
        if matches!(
            command,
            RuntimeCommand::Known { ref name, .. } if name == "resume"
        ) {
            let (response, answer) = oneshot::channel();
            events
                .send(HostEvent::SessionBrowser(InteractiveSessionBrowser {
                    current_session_id: "019f-pty".into(),
                    sessions: vec![
                        fixture_session_browser_entry(
                            "019f-pty",
                            "Current PTY session",
                            5,
                            "Current session content",
                        ),
                        fixture_session_browser_entry(
                            "019f-resume",
                            "Resume target session",
                            13,
                            "Recent conversation preview",
                        ),
                    ],
                    response,
                }))
                .await
                .map_err(|_| "session browser receiver closed")?;
            let result = match answer.await.map_err(|_| "session browser dropped")? {
                PromptResponse::Answer(session_id) => format!("resumed {session_id}"),
                PromptResponse::Cancelled => "resume cancelled".into(),
            };
            return Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Text(result)),
            ));
        }
        if matches!(
            command,
            RuntimeCommand::Known { ref name, .. } if name == "theme"
        ) {
            let default = TerminalPreferences::default();
            let mut hacker = default.clone();
            hacker.select_builtin_theme(colossus_contracts::ThemeName::Hacker);
            let (response, answer) = oneshot::channel();
            events
                .send(HostEvent::ThemePicker(InteractiveThemePicker {
                    current_theme: "default".into(),
                    themes: vec![
                        InteractiveThemePickerEntry {
                            name: "default".into(),
                            preferences: default,
                        },
                        InteractiveThemePickerEntry {
                            name: "hacker".into(),
                            preferences: hacker,
                        },
                    ],
                    response,
                }))
                .await
                .map_err(|_| "theme picker receiver closed")?;
            let result = match answer.await.map_err(|_| "theme picker dropped")? {
                PromptResponse::Answer(theme) => format!("theme selected {theme}"),
                PromptResponse::Cancelled => "theme cancelled".into(),
            };
            return Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Text(result)),
            ));
        }
        if matches!(
            command,
            RuntimeCommand::Known { ref name, .. } if name == "missing"
        ) {
            return Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Card {
                    title: "Unknown command".into(),
                    tone: PresentationTone::Warning,
                    body: vec![PresentationBlock::Text(
                        "/missing is not available; use /help".into(),
                    )],
                }),
            ));
        }
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

    async fn append_history(
        &self,
        _session_id: &str,
        _entry: String,
        _events: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn save_preferences(
        &self,
        _session_id: &str,
        preferences: TerminalPreferences,
        _events: mpsc::Sender<HostEvent>,
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

fn fixture_session_browser_entry(
    id: &str,
    title: &str,
    message_count: u64,
    preview: &str,
) -> InteractiveSessionBrowserEntry {
    InteractiveSessionBrowserEntry {
        summary: SessionSummary {
            id: id.into(),
            title: Some(title.into()),
            created_at: "2026-08-08T01:00:00Z".into(),
            updated_at: "2026-08-08T02:05:00Z".into(),
            message_count,
            last_run_id: None,
            last_user_preview: Some(preview.into()),
        },
        recent_messages: vec![InteractiveSessionBrowserMessage {
            role: ModelMessageRole::User,
            content: preview.into(),
        }],
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
                background_notice: None,
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
fn submitted_input_history_traverses_repeatedly_and_completion_keeps_key_precedence() {
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
    command.env("COLOSSUS_TUI_HISTORY_NAVIGATION_FIXTURE", "1");
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

    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    writer.write_all(b"/").expect("open slash completion");
    writer
        .write_all(b"\x1b[A\x1b[B")
        .expect("navigate slash completion");
    writer.flush().expect("flush slash completion navigation");
    wait_for_screen(&output, 24, 80, "Commands");
    assert!(!screen_contents(&output, 24, 80).contains("third prompt"));
    writer
        .write_all(&[27, 127])
        .expect("dismiss and clear slash completion");

    writer.write_all(b"@r").expect("open skill completion");
    writer
        .write_all(b"\x1b[A\x1b[B")
        .expect("navigate skill completion");
    writer.flush().expect("flush skill completion navigation");
    wait_for_screen(&output, 24, 80, "Skills");
    assert!(!screen_contents(&output, 24, 80).contains("third prompt"));
    writer
        .write_all(&[27, 127, 127])
        .expect("dismiss and clear skill completion");

    writer.write_all(b"unsent draft").expect("type draft");
    writer.write_all(b"\x1b[A").expect("recall newest history");
    writer.flush().expect("flush newest history");
    wait_for_screen(&output, 24, 80, "third prompt");
    writer.write_all(b"\x1b[A").expect("recall middle history");
    writer.flush().expect("flush middle history");
    wait_for_screen(&output, 24, 80, "second prompt");
    writer.write_all(b"\x1b[A").expect("recall oldest history");
    writer.flush().expect("flush oldest history");
    wait_for_screen(&output, 24, 80, "first prompt");

    writer
        .write_all(b"\x1b[B")
        .expect("advance to middle history");
    writer.flush().expect("flush middle history");
    wait_for_screen(&output, 24, 80, "second prompt");
    writer
        .write_all(b"\x1b[B")
        .expect("advance to newest history");
    writer.flush().expect("flush newest history");
    wait_for_screen(&output, 24, 80, "third prompt");
    writer.write_all(b"\x1b[B").expect("restore original draft");
    writer.flush().expect("flush restored draft");
    wait_for_screen(&output, 24, 80, "unsent draft");

    writer.write_all(&[3]).expect("Ctrl-C exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(
        status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.lock().expect("output"))
    );
    drop(writer);
    reader_thread.join().expect("reader thread");
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
fn inline_completion_chrome_never_enters_native_scrollback() {
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
    wait_for_screen(&output, 24, 80, "Message · Enter sends");

    writer.write_all(b"/").expect("open completion");
    writer.flush().expect("flush completion open");
    wait_for_screen(&output, 24, 80, "Commands");
    writer.write_all(b"t").expect("filter completion");
    writer.flush().expect("flush completion filter");
    wait_for_screen(&output, 24, 80, "/tools");
    writer.write_all(&[127]).expect("grow completion again");
    writer.flush().expect("flush completion growth");
    wait_for_screen(&output, 24, 80, "Commands");
    writer.write_all(&[27]).expect("dismiss completion");
    writer.flush().expect("flush completion dismissal");
    thread::sleep(Duration::from_millis(100));
    writer.write_all(&[127]).expect("clear dismissed draft");
    writer.flush().expect("flush dismissed draft clear");
    thread::sleep(Duration::from_millis(50));

    writer
        .write_all(b"/missing\r")
        .expect("submit fixture command");
    writer.flush().expect("flush fixture command");
    wait_for_screen(&output, 24, 80, "Unknown command");
    thread::sleep(Duration::from_millis(150));

    let command_screen = screen_rows(&output, 24, 80);
    let command_body_row = command_screen
        .iter()
        .position(|row| row.contains("/missing is not available"))
        .expect("command result body");
    let composer_row = command_screen
        .iter()
        .position(|row| row.contains("Message · Enter sends"))
        .expect("composer row");
    assert!(
        composer_row <= command_body_row + 2,
        "completion dismissal must restore the command result directly above the composer: {command_screen:?}"
    );

    writer
        .write_all(b"/t")
        .expect("open completion after command");
    writer.flush().expect("flush post-command completion");
    wait_for_screen(&output, 24, 80, "Commands");
    wait_for_screen(&output, 24, 80, "/tools");

    let open_history = native_history_rows(&output, 24, 80, "Commands");
    let open_history_text = open_history.join("\n");
    assert!(
        open_history_text.contains("durable-row-05"),
        "{open_history_text}"
    );
    assert_eq!(
        open_history
            .iter()
            .filter(|row| row.contains("Unknown command"))
            .count(),
        1,
        "{open_history_text}"
    );
    let latest_durable_row = open_history
        .iter()
        .position(|row| row.contains("durable-row-05"))
        .expect("latest durable transcript row");
    let command_result_row = open_history
        .iter()
        .position(|row| row.contains("Unknown command"))
        .expect("command result row");
    assert!(
        command_result_row <= latest_durable_row + 2,
        "completion chrome must not leave a blank band between durable transcript entries: {open_history:?}"
    );
    for transient in [
        "Commands",
        "/tools",
        "Message · Enter sends",
        "fixture@local",
    ] {
        assert!(
            !open_history_text.contains(transient),
            "transient {transient:?} leaked into native history: {open_history_text}"
        );
    }
    assert_eq!(
        open_history
            .iter()
            .filter(|row| row.trim() == "/missing")
            .count(),
        0,
        "submitted command input leaked into native history: {open_history_text}"
    );

    writer.write_all(b"\t").expect("accept completion");
    writer.flush().expect("flush completion acceptance");
    thread::sleep(Duration::from_millis(50));
    writer.write_all(b"\r").expect("submit accepted command");
    writer.flush().expect("flush accepted command");
    wait_for_screen(&output, 24, 80, "ok");
    thread::sleep(Duration::from_millis(150));

    let settled_history = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let settled_history_text = settled_history.join("\n");
    assert_eq!(
        settled_history
            .iter()
            .filter(|row| row.trim() == "ok")
            .count(),
        1,
        "{settled_history_text}"
    );
    for transient in [
        "Commands",
        "/tools",
        "Message · Enter sends",
        "fixture@local",
    ] {
        assert!(
            !settled_history_text.contains(transient),
            "transient {transient:?} leaked into native history: {settled_history_text}"
        );
    }

    writer
        .write_all(b"/")
        .expect("open completion before resize");
    writer.flush().expect("flush pre-resize completion");
    wait_for_screen(&output, 24, 80, "Commands");
    let resize_output_offset = output.lock().expect("output").len();
    pair.master
        .resize(PtySize {
            rows: 12,
            cols: 40,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize while completion is open");
    wait_for_resized_screen(
        &output,
        (24, 80),
        resize_output_offset,
        (12, 40),
        "Commands",
    );
    wait_for_resized_screen(
        &output,
        (24, 80),
        resize_output_offset,
        (12, 40),
        "Message · Enter sends",
    );
    let resized_history = resized_native_history_rows(
        &output,
        (24, 80),
        resize_output_offset,
        (12, 40),
        "Commands",
    );
    let resized_history_text = resized_history.join("\n");
    for transient in ["Commands", "Message · Enter sends", "fixture@local"] {
        assert!(
            !resized_history_text.contains(transient),
            "resize leaked transient {transient:?} into native history: {resized_history_text}"
        );
    }

    writer.write_all(&[3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
}

#[test]
fn inline_session_browser_uses_a_transient_screen_without_polluting_history() {
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
    command.env("COLOSSUS_TUI_LONG_HISTORY", "1");
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
    thread::sleep(Duration::from_millis(150));

    let history_before = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let history_before_text = history_before.join("\n");
    for sequence in 1..=30 {
        let row = format!("durable-row-{sequence:02}");
        assert_eq!(
            history_before_text.matches(&row).count(),
            1,
            "{row} was not present exactly once before opening the browser: {history_before_text}"
        );
    }
    let browser_output_offset = output.lock().expect("output").len();
    writer
        .write_all(b"/resume\r")
        .expect("open session browser");
    writer.flush().expect("flush session browser command");
    wait_for_screen(&output, 24, 80, "Resume session");
    wait_for_screen(&output, 24, 80, "Resume target session");
    writer.write_all(&[27]).expect("dismiss session browser");
    writer.flush().expect("flush session browser dismissal");
    wait_for_screen(&output, 24, 80, "resume cancelled");
    thread::sleep(Duration::from_millis(150));

    let bytes = output.lock().expect("output").clone();
    let browser_output = &bytes[browser_output_offset..];
    assert!(
        browser_output
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h"),
        "session browser did not enter its transient screen"
    );
    assert!(
        browser_output
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l"),
        "session browser did not restore the main screen"
    );
    drop(bytes);

    let history_after = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let history_after_text = history_after.join("\n");
    for transient in [
        "Resume session",
        "Current PTY session",
        "Resume target session",
    ] {
        assert!(
            !history_after_text.contains(transient),
            "session browser leaked {transient:?} into native history: {history_after_text}"
        );
    }
    for sequence in 1..=30 {
        let row = format!("durable-row-{sequence:02}");
        assert_eq!(
            history_after_text.matches(&row).count(),
            1,
            "restored history changed row {row:?}: {history_after_text}"
        );
    }

    writer.write_all(&[3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
}

#[test]
fn inline_theme_picker_uses_a_transient_screen_without_polluting_history() {
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
    command.env("COLOSSUS_TUI_LONG_HISTORY", "1");
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
    thread::sleep(Duration::from_millis(150));

    let history_before = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let history_before_text = history_before.join("\n");
    for sequence in 1..=30 {
        let row = format!("durable-row-{sequence:02}");
        assert_eq!(
            history_before_text.matches(&row).count(),
            1,
            "{row} was not present exactly once before opening the theme picker: {history_before_text}"
        );
    }
    let picker_output_offset = output.lock().expect("output").len();
    writer.write_all(b"/theme\r").expect("open theme picker");
    writer.flush().expect("flush theme picker command");
    wait_for_screen(&output, 24, 80, "Choose theme");
    writer.write_all(b"\x1b[B").expect("preview hacker theme");
    writer.flush().expect("flush hacker theme preview");
    wait_for_screen(&output, 24, 80, "hacker preview");
    writer.write_all(&[27]).expect("dismiss theme picker");
    writer.flush().expect("flush theme picker dismissal");
    wait_for_screen(&output, 24, 80, "theme cancelled");
    thread::sleep(Duration::from_millis(150));

    let bytes = output.lock().expect("output").clone();
    let picker_output = &bytes[picker_output_offset..];
    assert!(
        picker_output
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h"),
        "theme picker did not enter its transient screen"
    );
    assert!(
        picker_output
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l"),
        "theme picker did not restore the main screen"
    );
    drop(bytes);

    let history_after = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let history_after_text = history_after.join("\n");
    for transient in ["Choose theme", "default preview", "hacker preview"] {
        assert!(
            !history_after_text.contains(transient),
            "theme picker leaked {transient:?} into native history: {history_after_text}"
        );
    }
    for sequence in 1..=30 {
        let row = format!("durable-row-{sequence:02}");
        assert_eq!(
            history_after_text.matches(&row).count(),
            1,
            "restored history changed row {row:?}: {history_after_text}"
        );
    }

    writer.write_all(&[3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    reader_thread.join().expect("reader thread");
}

#[test]
fn inline_completion_restores_a_full_main_screen_without_changing_history() {
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
    command.env("COLOSSUS_TUI_LONG_HISTORY", "1");
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
    thread::sleep(Duration::from_millis(150));

    let history_before = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let history_before_text = history_before.join("\n");
    for sequence in 1..=30 {
        let row = format!("durable-row-{sequence:02}");
        assert_eq!(
            history_before
                .iter()
                .filter(|line| line.contains(&row))
                .count(),
            1,
            "{row} was not present exactly once before completion: {history_before_text}"
        );
    }

    let completion_output_offset = output.lock().expect("output").len();
    writer.write_all(b"/").expect("open completion");
    writer.flush().expect("flush completion open");
    wait_for_screen(&output, 24, 80, "Commands");
    writer.write_all(&[27]).expect("dismiss completion");
    writer.flush().expect("flush completion dismissal");
    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    thread::sleep(Duration::from_millis(150));

    let history_after = native_history_rows(&output, 24, 80, "Message · Enter sends");
    assert_eq!(
        history_after, history_before,
        "transient completion changed the restored main-screen history"
    );
    let completion_output = output.lock().expect("output").clone();
    let completion_output = &completion_output[completion_output_offset..];
    assert!(
        completion_output
            .windows(b"\x1b[?1049h".len())
            .any(|window| window == b"\x1b[?1049h"),
        "completion did not enter its transient screen"
    );
    assert!(
        completion_output
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l"),
        "completion did not restore the main screen"
    );

    writer.write_all(&[3]).expect("exit");
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

fn wait_for_resized_screen(
    output: &Arc<Mutex<Vec<u8>>>,
    initial: (u16, u16),
    resize_output_offset: usize,
    resized: (u16, u16),
    needle: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let bytes = output.lock().expect("output").clone();
        let mut parser = resized_parser(&bytes, initial, resize_output_offset, resized);
        parser.screen_mut().set_scrollback(0);
        if parser.screen().contents().contains(needle) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("resized screen never contained {needle}");
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

fn native_scrollback_rows(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16) -> Vec<String> {
    let bytes = output.lock().expect("output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 512);
    parser.process(&bytes);
    parser.screen_mut().set_scrollback(usize::MAX);
    let scrollback_len = parser.screen().scrollback();
    (1..=scrollback_len)
        .rev()
        .filter_map(|offset| {
            parser.screen_mut().set_scrollback(offset);
            parser.screen().rows(0, cols).next()
        })
        .collect()
}

fn native_history_rows(
    output: &Arc<Mutex<Vec<u8>>>,
    rows: u16,
    cols: u16,
    live_marker: &str,
) -> Vec<String> {
    let mut history = native_scrollback_rows(output, rows, cols);
    let visible = screen_rows(output, rows, cols);
    let live_viewport = visible
        .iter()
        .rposition(|row| row.contains(live_marker))
        .expect("live viewport marker");
    history.extend_from_slice(&visible[..live_viewport]);
    history
}

fn resized_native_history_rows(
    output: &Arc<Mutex<Vec<u8>>>,
    initial: (u16, u16),
    resize_output_offset: usize,
    resized: (u16, u16),
    live_marker: &str,
) -> Vec<String> {
    let bytes = output.lock().expect("output").clone();
    let mut parser = resized_parser(&bytes, initial, resize_output_offset, resized);
    parser.screen_mut().set_scrollback(usize::MAX);
    let scrollback_len = parser.screen().scrollback();
    let mut history = (1..=scrollback_len)
        .rev()
        .filter_map(|offset| {
            parser.screen_mut().set_scrollback(offset);
            parser.screen().rows(0, resized.1).next()
        })
        .collect::<Vec<_>>();
    parser.screen_mut().set_scrollback(0);
    let visible = parser.screen().rows(0, resized.1).collect::<Vec<_>>();
    let live_viewport = visible
        .iter()
        .rposition(|row| row.contains(live_marker))
        .expect("resized live viewport marker");
    history.extend_from_slice(&visible[..live_viewport]);
    history
}

fn resized_parser(
    bytes: &[u8],
    initial: (u16, u16),
    resize_output_offset: usize,
    resized: (u16, u16),
) -> vt100::Parser {
    let split = resize_output_offset.min(bytes.len());
    let mut parser = vt100::Parser::new(initial.0, initial.1, 512);
    parser.process(&bytes[..split]);
    parser.screen_mut().set_size(resized.0, resized.1);
    parser.process(&bytes[split..]);
    parser
}
