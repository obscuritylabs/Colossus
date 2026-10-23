//! PTY regression for durable transcript preservation during typing and resize.

use async_trait::async_trait;
use colossus_contracts::{
    AgentRunOutcome, AgentRunResult, ModelMessage, ModelMessageRole, ProviderEvent, RunEvent,
    RunEventEnvelope, SandboxBoundaryMode, SecurityPostureFinding, SecurityPostureReport,
    SecurityPostureSeverity, SessionMessage, SessionMessagePage, SessionSummary,
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

#[path = "pty_history/agents.rs"]
mod agents;
#[path = "pty_history/selection.rs"]
#[cfg(unix)] // This fixture injects SGR mouse input; Windows uses console input records.
mod selection;
#[path = "pty_history/status.rs"]
mod status;

// ConPTY decodes this Win32-input-mode record into a real Ctrl+J key event.
// A bare LF is translated into Enter before crossterm's Windows backend reads it.
#[cfg(windows)]
const WINDOWS_CTRL_J: &[u8] = b"\x1b[74;36;10;1;8;1_\x1b[74;36;10;0;8;1_";

const MULTILINE_DRAFT: &str = "# Heading\n\n  - **nested**\nunicode 👩‍💻 界 e\u{301}";

struct FixtureHost;

struct FixtureDictation;
#[async_trait]
impl colossus_tui::LocalDictation for FixtureDictation {
    async fn control(
        &self,
        action: colossus_tui::DictationAction,
    ) -> Result<Vec<colossus_tui::DictationUpdate>, String> {
        use colossus_tui::{DictationAction, DictationUpdate};
        let transcript = |revision, settled, text: &str| DictationUpdate::Transcript {
            segment: 1,
            revision,
            settled,
            text: text.into(),
        };
        Ok(match action {
            DictationAction::Start => vec![
                DictationUpdate::Listening,
                transcript(1, false, "voice draft"),
                DictationUpdate::Level(180),
            ],
            DictationAction::Stop => {
                vec![transcript(2, true, "voice final"), DictationUpdate::Stopped]
            }
            DictationAction::Pause => vec![DictationUpdate::Paused],
            DictationAction::Resume => vec![DictationUpdate::Listening],
        })
    }
    async fn poll(&self) -> Result<Vec<colossus_tui::DictationUpdate>, String> {
        Ok(vec![])
    }
    async fn configure(&self, _: &str) -> Result<String, String> {
        Ok("Saved".into())
    }
    fn cancel(&self) {}
}

#[async_trait]
impl InteractiveHost for FixtureHost {
    async fn sidebar_status(
        &self,
        _: &str,
    ) -> Result<Option<colossus_tui::SidebarSnapshot>, String> {
        use colossus_contracts::{GoalStatus, TaskStatus};
        if std::env::var_os("COLOSSUS_TUI_STATUS_FIXTURE").is_none() {
            return Ok(None);
        }
        Ok(Some(colossus_tui::SidebarSnapshot {
            tasks: vec![("Review runtime".into(), TaskStatus::InProgress)],
            open_task_count: 1,
            goals: vec![("Ship release".into(), GoalStatus::Active)],
            goal_count: 1,
            memory_count: Some((24, false)),
            plan_count: 2,
            mcp_servers: vec![("atlas".into(), true), ("offline".into(), false)],
        }))
    }

    async fn subagents(
        &self,
        session_id: &str,
    ) -> Result<Vec<colossus_contracts::SubagentJob>, String> {
        use colossus_contracts::{SubagentJob, SubagentStatus};
        use std::sync::atomic::{AtomicUsize, Ordering};
        if std::env::var_os("COLOSSUS_TUI_AGENTS_FIXTURE").is_none() {
            return Ok(Vec::new());
        }
        static READS: AtomicUsize = AtomicUsize::new(0);
        let complete = READS.fetch_add(1, Ordering::Relaxed) > 1;
        Ok([
            ("review", SubagentStatus::Running),
            ("checks", SubagentStatus::Queued),
        ]
        .into_iter()
        .map(|(id, status)| SubagentJob {
            id: id.into(),
            session_id: session_id.into(),
            parent_run_id: "parent-run".into(),
            parent_call_id: format!("call-{id}"),
            task: id.into(),
            role: "subagent_default".into(),
            allowed_tools: None,
            status: if complete {
                SubagentStatus::Completed
            } else {
                status
            },
            child_session_id: format!("child-{id}"),
            child_run_id: None,
            final_output: String::new(),
            error: String::new(),
            created_at: "2026-10-23T18:00:00Z".into(),
            updated_at: "2026-10-23T18:00:01Z".into(),
            started_at: None,
            completed_at: None,
        })
        .collect())
    }

    async fn bootstrap(&self, _request: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        let inline = std::env::var("COLOSSUS_TUI_MODE").as_deref() == Ok("inline");
        let empty = std::env::var_os("COLOSSUS_TUI_EMPTY").is_some();
        let first_sequence = if inline { 2 } else { 1 };
        let last_sequence = if std::env::var_os("COLOSSUS_TUI_LONG_HISTORY").is_some() {
            30
        } else {
            5
        };
        let messages = if empty {
            Vec::new()
        } else {
            (first_sequence..=last_sequence)
                .map(|sequence| SessionMessage {
                    session_id: "019f-pty".into(),
                    run_id: "run-pty".into(),
                    sequence,
                    message: ModelMessage {
                        agent_message_origin: None,
                        role: ModelMessageRole::Assistant,
                        content: format!("durable-row-{sequence:02}").into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    created_at: "2026-07-15T00:00:00Z".into(),
                })
                .collect()
        };
        let history_navigation_fixture =
            std::env::var_os("COLOSSUS_TUI_HISTORY_NAVIGATION_FIXTURE").is_some();
        Ok(InteractiveSnapshot {
            session_id: "019f-pty".into(),
            fresh_session: empty,
            workspace: "/workspace/Colossus".into(),
            sandbox_profile: "workspace-development".into(),
            transcript: SessionMessagePage {
                messages,
                before_sequence: (inline && !empty).then_some(2),
                has_more: inline && !empty,
            },
            preferences: TerminalPreferences::default(),
            history: if std::env::var_os("COLOSSUS_TUI_INLINE_DRAFT_FIXTURE").is_some() {
                vec![MULTILINE_DRAFT.into()]
            } else if history_navigation_fixture {
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
                context: Some((u64::from(!empty) * 5, 32_768)),
                message_count: u64::from(!empty) * 5,
                status: "ready".into(),
                approval_mode: "ask".into(),
            },
            pending_sandbox_boundary_acknowledgement: None,
            security_posture: if std::env::var_os("COLOSSUS_TUI_SECURITY_WARNINGS").is_some() {
                SecurityPostureReport {
                    findings: vec![
                        SecurityPostureFinding {
                            code: "storage.plaintext".into(),
                            severity: SecurityPostureSeverity::Warning,
                            summary: "Journal payloads are stored as plaintext canonical JSON."
                                .into(),
                            remediation: "Use platform or environment storage keys.".into(),
                        },
                        SecurityPostureFinding {
                            code: "sandbox.danger_full_access".into(),
                            severity: SecurityPostureSeverity::Warning,
                            summary: "Danger full access is enabled.".into(),
                            remediation: "Use an isolating execution boundary.".into(),
                        },
                    ],
                }
            } else {
                Default::default()
            },
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
        if let RuntimeCommand::Shell { command } = &command {
            return Ok(HostCommandResult::document(
                PresentationDocument::from_block(PresentationBlock::Verbatim(format!(
                    "Shell fixture received: {command}"
                ))),
            ));
        }
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
        if std::env::var_os("COLOSSUS_TUI_UNICODE_ROUNDTRIP_FIXTURE").is_some() {
            return Err(if request.prompt == MULTILINE_DRAFT {
                "Native Unicode draft matched".into()
            } else {
                "Native Unicode draft differed".into()
            });
        }
        if std::env::var_os("COLOSSUS_TUI_STREAM_FIXTURE").is_none() {
            return Err("fixture does not run model turns".into());
        }
        let output = if std::env::var_os("COLOSSUS_TUI_DICTATION_FIXTURE").is_some() {
            format!("Dictation accepted: {}", request.prompt)
        } else {
            (1..=30)
                .map(|row| format!("stream-final-row-{row:02}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let status_fixture = std::env::var_os("COLOSSUS_TUI_STATUS_FIXTURE").is_some();
        let call = ToolCall {
            call_id: "call-scrollback".into(),
            name: if status_fixture {
                "mcp.call"
            } else {
                "filesystem.search"
            }
            .into(),
            arguments: if status_fixture {
                serde_json::json!({"server":"atlas", "tool":"search", "arguments":{}})
            } else {
                serde_json::json!({"query": "Runtime"})
            },
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
        if std::env::var_os("COLOSSUS_TUI_SELECTION_FIXTURE").is_some() {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
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
                        images: Vec::new(),
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
                    agent_message_origin: None,
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

fn fixture_command() -> CommandBuilder {
    let mut command = CommandBuilder::new(std::env::current_exe().expect("test executable"));
    // Match the synthetic VT100 terminal rather than inheriting outer graphics hints.
    // Unsupported graphics probes can consume input and restore stale terminal modes.
    for variable in [
        "KITTY_WINDOW_ID",
        "TERM_PROGRAM",
        "WEZTERM_EXECUTABLE",
        "KONSOLE_VERSION",
        "VTE_VERSION",
        "WINDOWID",
        "FOOT_CLIENT_PID",
        "MLTERM",
    ] {
        command.env_remove(variable);
    }
    command.env("TERM", "xterm-256color");
    command
}

#[test]
fn inline_empty_session_shows_launch_rail_and_recedes_after_the_first_turn() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = fixture_command();
    command.arg("--exact");
    command.arg("fixture_process");
    command.arg("--nocapture");
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "inline");
    command.env("COLOSSUS_TUI_EMPTY", "1");
    command.env("COLOSSUS_TUI_SECURITY_WARNINGS", "1");
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

    let welcome_ready =
        screen_contains_within(&output, 32, 120, "⚠ Security: 2", Duration::from_secs(10));
    let welcome = screen_contents(&output, 32, 120);

    writer.write_all(b"go\r").expect("submit first turn");
    writer.flush().expect("flush first turn");
    let output_ready = screen_contains_within(
        &output,
        32,
        120,
        "stream-final-row-30",
        Duration::from_secs(10),
    );
    let composer_ready = screen_contains_within(
        &output,
        32,
        120,
        "Message · Enter sends",
        Duration::from_secs(10),
    );
    let after_turn = screen_contents(&output, 32, 120);

    writer.write_all(&[3, 3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");

    assert!(welcome_ready, "{welcome}");
    assert!(welcome.contains("Build or change something"), "{welcome}");
    assert!(welcome.contains("Implement {feature}"), "{welcome}");
    assert!(welcome.contains("workspace-development"), "{welcome}");
    assert!(
        welcome.contains("// SESSION   durable · ready"),
        "{welcome}"
    );
    assert!(welcome.contains("SECURITY  2 warnings"), "{welcome}");
    assert!(welcome.contains("OL //  COLOSSUS"), "{welcome}");
    assert!(welcome.contains("OBSCURITY LABS"), "{welcome}");
    assert!(
        welcome.contains("ready · approval ask") && welcome.contains("⚠ Security: 2"),
        "the footer must retain readable status, permissions, and security count: {welcome}"
    );
    let rail = welcome
        .find("What do you want to work on?")
        .expect("launch rail");
    let warning = welcome
        .find("Journal payloads are stored as plaintext canonical JSON.")
        .expect("security warning");
    assert!(
        rail < warning,
        "the detailed security posture must follow the launch rail: {welcome}"
    );
    assert_eq!(
        welcome.matches("Execute · Enter sends").count(),
        1,
        "the inline composer must render exactly once: {welcome}"
    );
    assert!(
        scrollback_contains(
            &output,
            32,
            120,
            "Journal payloads are stored as plaintext canonical JSON."
        ),
        "the detailed security posture must remain available in native history"
    );
    assert!(output_ready, "{after_turn}");
    assert!(composer_ready, "{after_turn}");
    assert!(
        !after_turn.contains("What do you want to work on?"),
        "{after_turn}"
    );
    assert!(!after_turn.contains("Implement {feature}"), "{after_turn}");
    assert!(status.success());
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
    if std::env::var_os("COLOSSUS_TUI_STARTUP_SENTINEL").is_some() {
        println!("prior-shell-row");
        std::io::stdout().flush().expect("flush startup sentinel");
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
                dictation: std::env::var_os("COLOSSUS_TUI_DICTATION_FIXTURE")
                    .map(|_| Arc::new(FixtureDictation) as Arc<dyn colossus_tui::LocalDictation>),
                bootstrap: BootstrapRequest::default(),
                screen_mode,
                background_notice: None,
                lifecycle: None,
            },
        ))
        .expect("fixture TUI");
}

#[cfg(unix)]
#[test]
fn enhanced_shift_enter_composes_a_multiline_turn_and_restores_keyboard_mode() {
    multiline_turn_and_keyboard_restoration(b"\x1b[13;2u");
}

#[test]
fn text_selection_freezes_stream_redraw_and_restores_captured_input() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("selection PTY");
    let mut command = fixture_command();
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_STREAM_FIXTURE", "1");
    command.env("COLOSSUS_TUI_SELECTION_FIXTURE", "1");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("spawn selection fixture");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("selection reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            reader_output
                .lock()
                .expect("output")
                .extend_from_slice(&buffer[..read]);
        }
    });
    let mut writer = pair.master.take_writer().expect("selection writer");
    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    writer
        .write_all(b"inspect\r")
        .expect("submit streaming turn");
    writer.flush().expect("flush streaming turn");
    wait_for_screen(&output, 24, 80, "commentary-before-tool");
    writer
        .write_all(b"\x1b[17~")
        .expect("F6 selects native text");
    writer.flush().expect("flush F6");
    wait_for_screen(&output, 24, 80, "Select text");
    wait_for_mouse_capture_count(&output, false, 1);
    let frozen = output.lock().expect("output").clone();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        *output.lock().expect("output"),
        frozen,
        "streaming/spinner redraw cleared native selection"
    );
    writer
        .write_all(b"\x1b")
        .expect("Escape returns to live output");
    writer.flush().expect("flush Escape");
    wait_for_screen(&output, 24, 80, "stream-final-row-30");
    wait_for_mouse_capture_count(&output, true, 2);
    writer.write_all(&[3, 3]).expect("ordinary Ctrl+C exits");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("selection fixture status");
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("selection reader thread");
    assert!(status.success());
}

#[test]
fn legacy_newline_composes_a_multiline_turn_without_submitting() {
    #[cfg(unix)]
    multiline_turn_and_keyboard_restoration(b"\n");
    #[cfg(windows)]
    multiline_turn_and_keyboard_restoration(WINDOWS_CTRL_J);
}

fn multiline_turn_and_keyboard_restoration(newline: &[u8]) {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = fixture_command();
    command.arg("--exact");
    command.arg("fixture_process");
    command.arg("--nocapture");
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
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
    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    #[cfg(unix)]
    wait_for_raw(&output, b"\x1b[>1u");

    let composed_input = [b"first".as_slice(), newline, b"second".as_slice()].concat();
    writer
        .write_all(&composed_input)
        .expect("compose modified Enter turn");
    writer.flush().expect("flush modified Enter turn");
    wait_for_screen(&output, 24, 80, "second");
    let composed = screen_contents(&output, 24, 80);
    assert!(composed.contains("first"), "{composed}");
    assert!(!composed.contains("stream-final-row-30"), "{composed}");

    writer
        .write_all(b"\x1b[AX\x1b[BY")
        .expect("edit both composer rows with arrow keys");
    writer.flush().expect("flush vertical editing");
    wait_for_screen(&output, 24, 80, "firstX");
    wait_for_screen(&output, 24, 80, "secondY");

    writer.write_all(b"\r").expect("submit composed turn");
    writer.flush().expect("flush composed turn");
    wait_for_screen(&output, 24, 80, "stream-final-row-30");
    writer.write_all(&[3, 3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");
    assert!(status.success());
    let raw = output.lock().expect("output");
    let offset = |sequence: &[u8]| {
        raw.windows(sequence.len())
            .position(|window| window == sequence)
            .expect("terminal control sequence")
    };
    let enter_screen = offset(b"\x1b[?1049h");
    let leave_screen = offset(b"\x1b[?1049l");
    assert!(
        enter_screen < leave_screen,
        "alternate screen was not restored"
    );
    #[cfg(unix)]
    {
        assert!(
            enter_screen < offset(b"\x1b[>1u"),
            "keyboard enhancement must be enabled on the alternate screen"
        );
        assert!(
            offset(b"\x1b[<1u") < leave_screen,
            "keyboard enhancement must be removed before leaving the alternate screen"
        );
    }
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
    let mut command = fixture_command();
    command.arg("--exact");
    command.arg("fixture_process");
    command.arg("--nocapture");
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "inline");
    command.env("COLOSSUS_TUI_STARTUP_SENTINEL", "1");
    #[cfg(windows)]
    command.env("COLOSSUS_TUI_INLINE_DRAFT_FIXTURE", "1");
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
    assert!(
        !screen_contents(&output, 24, 80).contains("prior-shell-row"),
        "inline startup should begin on a clean visible viewport"
    );
    assert!(
        scrollback_contains(&output, 24, 80, "prior-shell-row"),
        "inline startup should preserve the prior view in native scrollback"
    );
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
    #[cfg(unix)]
    {
        writer
            .write_all(b"\x1b[200~# Heading\r\r  - **nested**\r\nunicode ")
            .expect("start multiline paste");
        writer
            .write_all("👩‍💻 界 e\u{301}".as_bytes())
            .expect("paste Unicode text");
        writer
            .write_all(b"\x1b[201~")
            .expect("finish multiline paste");
    }
    #[cfg(windows)]
    // crossterm's Windows Console backend has no Paste decoder. Recall the
    // complete UTF-8 history draft in one reducer update to verify rendering.
    writer.write_all(b"\x1b[A").expect("recall multiline draft");
    writer.flush().expect("flush multiline draft");
    wait_for_screen(&output, 24, 80, "- **nested**");
    wait_for_screen(&output, 24, 80, "界 e\u{301}");
    let pasted_rows = screen_rows(&output, 24, 80);
    let heading_row = pasted_rows
        .iter()
        .position(|row| row.contains("# Heading"))
        .expect("pasted heading row");
    let nested_row = pasted_rows
        .iter()
        .position(|row| row.contains("- **nested**"))
        .expect("pasted nested Markdown row");
    let unicode_row = pasted_rows
        .iter()
        .position(|row| row.contains("unicode") && row.contains('界'))
        .expect("pasted Unicode row");
    #[cfg(windows)]
    {
        writer.write_all(b"\x1b[H!").expect("edit recalled draft");
        writer.flush().expect("flush recalled draft edit");
        wait_for_screen(&output, 24, 80, "│!# Heading ");
        writer
            .write_all(&[127])
            .expect("remove recalled draft edit");
        writer.flush().expect("flush recalled draft edit removal");
        // The border followed by '#' cannot match the pre-delete '!'.
        wait_for_screen(&output, 24, 80, "│# Heading ");
        assert!(!screen_contents(&output, 24, 80).contains("│!# Heading "));
    }
    assert_eq!(nested_row, heading_row + 2, "blank line was not preserved");
    assert_eq!(unicode_row, nested_row + 1, "composed lines changed");

    writer.write_all(&[3]).expect("Ctrl-C clear draft");
    writer.flush().expect("flush draft clear");
    thread::sleep(Duration::from_millis(50));
    let cleared_screen = screen_contents(&output, 24, 80);
    assert!(
        cleared_screen.contains("Message · Enter sends"),
        "first Ctrl-C should keep the TUI open: {cleared_screen}"
    );
    assert!(
        !cleared_screen.contains("# Heading"),
        "first Ctrl-C should clear the draft: {cleared_screen}"
    );
    writer.write_all(&[3]).expect("second Ctrl-C exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(
        status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.lock().expect("output"))
    );
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");

    let raw = output.lock().expect("output");
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(raw.as_slice());
    let final_screen = parser.screen().contents();
    assert!(final_screen.contains("durable-row-05"), "{final_screen}");
    assert!(
        !final_screen.contains("Message · Enter sends"),
        "{final_screen}"
    );
    assert!(!final_screen.contains("fixture@local"), "{final_screen}");
    assert!(!final_screen.contains("# Heading"), "{final_screen}");
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
        !raw_contains(raw.as_slice(), b"\x1b[?1000h")
            && !raw_contains(raw.as_slice(), b"\x1b[?1003;1006h"),
        "inline mode must preserve native mouse scrollback"
    );
    drop(raw);
    #[cfg(windows)]
    {
        // ConPTY's glyph serialization cannot prove the complete ZWJ string.
        // Verify its exact UTF-8 data through the real TUI host boundary instead.
        unicode_history_draft_roundtrip(true);
        unicode_history_draft_roundtrip(false);
    }
}

#[cfg(windows)]
fn unicode_history_draft_roundtrip(restore_edit: bool) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("Unicode proof PTY");
    let mut command = fixture_command();
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_MODE", "alternate");
    command.env("COLOSSUS_TUI_INLINE_DRAFT_FIXTURE", "1");
    command.env("COLOSSUS_TUI_UNICODE_ROUNDTRIP_FIXTURE", "1");
    let mut child = pair
        .slave
        .spawn_command(command)
        .expect("spawn Unicode proof");
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .expect("Unicode proof reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8_192];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            reader_output
                .lock()
                .expect("output")
                .extend_from_slice(&buffer[..read]);
        }
    });
    let mut writer = pair.master.take_writer().expect("Unicode proof writer");
    answer_cursor_position_query(&output, &mut writer);
    wait_for_screen(&output, 24, 80, "Message · Enter sends");
    writer.write_all(b"\x1b[A").expect("recall Unicode draft");
    writer.flush().expect("flush Unicode draft recall");
    wait_for_screen(&output, 24, 80, "界 e\u{301}");
    writer.write_all(b"\x1b[H!").expect("edit Unicode draft");
    writer.flush().expect("flush Unicode draft edit");
    wait_for_screen(&output, 24, 80, "│!# Heading ");
    if restore_edit {
        writer.write_all(&[127]).expect("restore Unicode draft");
        writer.flush().expect("flush Unicode draft restoration");
        wait_for_screen(&output, 24, 80, "│# Heading ");
    }
    writer.write_all(b"\r").expect("submit Unicode draft proof");
    writer.flush().expect("flush Unicode draft proof");
    let receipt = if restore_edit {
        "Native Unicode draft matched"
    } else {
        "Native Unicode draft differed"
    };
    wait_for_screen(&output, 24, 80, receipt);
    if !restore_edit {
        assert!(!screen_contents(&output, 24, 80).contains("Native Unicode draft matched"));
    }
    writer.write_all(&[3, 3]).expect("exit Unicode proof");
    writer.flush().expect("flush Unicode proof exit");
    let status = child.wait().expect("Unicode proof status");
    assert!(status.success());
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("Unicode proof reader thread");
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
    let mut command = fixture_command();
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

    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
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
    writer.write_all(&[27]).expect("dismiss skill completion");
    writer.flush().expect("flush skill completion dismissal");
    thread::sleep(Duration::from_millis(50));
    writer
        .write_all(&[127; 16])
        .expect("clear skill completion draft");
    writer.flush().expect("flush skill completion draft clear");

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

    writer.write_all(&[3]).expect("Ctrl-C clear draft");
    writer.flush().expect("flush draft clear");
    thread::sleep(Duration::from_millis(50));
    assert!(
        !screen_contents(&output, 24, 80).contains("unsent draft"),
        "first Ctrl-C should clear the restored draft"
    );
    writer.write_all(&[3]).expect("second Ctrl-C exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(
        status.success(),
        "fixture failed: {}",
        String::from_utf8_lossy(&output.lock().expect("output"))
    );
    drop(writer);
    drop(pair.master);
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
    let mut command = fixture_command();
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
    let history = native_history_rows(&output, 24, 80, "Message · Enter sends");
    let completed_tool_row = history
        .iter()
        .position(|row| row.contains("Completed filesystem.search"))
        .expect("completed tool row");
    let final_output_row = history
        .iter()
        .position(|row| row.contains("stream-final-row-01"))
        .expect("first final output row");
    let largest_blank_run = history[completed_tool_row + 1..final_output_row]
        .split(|row| !row.trim().is_empty())
        .map(<[String]>::len)
        .max()
        .unwrap_or_default();
    assert!(
        largest_blank_run <= 2,
        "finalizing streamed output must not commit cleared viewport rows into native history; blank run was {largest_blank_run}: {history:?}"
    );

    writer.write_all(&[4]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    drop(pair.master);
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
    let mut command = fixture_command();
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

    writer.write_all(&[3]).expect("clear completion draft");
    writer.flush().expect("flush completion draft clear");
    writer.write_all(&[3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");
    #[cfg(unix)]
    {
        let raw = output.lock().expect("output");
        let find_after = |start: usize, sequence: &[u8]| {
            raw[start..]
                .windows(sequence.len())
                .position(|window| window == sequence)
                .map(|offset| start + offset)
                .expect("terminal control sequence")
        };
        let main_push = find_after(0, b"\x1b[>1u");
        let enter = find_after(main_push, b"\x1b[?1049h");
        let alternate_push = find_after(enter, b"\x1b[>1u");
        let alternate_pop = find_after(alternate_push, b"\x1b[<1u");
        let leave = find_after(alternate_pop, b"\x1b[?1049l");
        let main_pop = find_after(leave, b"\x1b[<1u");
        assert!(main_push < enter && enter < alternate_push);
        assert!(alternate_push < alternate_pop && alternate_pop < leave);
        assert!(leave < main_pop);
    }
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
    let mut command = fixture_command();
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
    drop(pair.master);
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
    let mut command = fixture_command();
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
    drop(pair.master);
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
    let mut command = fixture_command();
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

    writer.write_all(&[3]).expect("clear completion draft");
    writer.flush().expect("flush completion draft clear");
    writer.write_all(&[3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    drop(pair.master);
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
    let mut command = fixture_command();
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

    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
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

    writer.write_all(&[3]).expect("Ctrl-C clear draft");
    writer.flush().expect("flush draft clear");
    thread::sleep(Duration::from_millis(50));
    assert!(
        !screen_contents(&output, 30, 100).contains("typing-preserves-history"),
        "first Ctrl-C should clear the draft"
    );
    writer.write_all(&[3]).expect("second Ctrl-C exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    assert!(status.success());
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");
    let raw = output.lock().expect("output");
    assert!(
        raw_contains(raw.as_slice(), b"\x1b[?1000h")
            || raw_contains(raw.as_slice(), b"\x1b[?1003;1006h"),
        "alternate-screen mouse capture was not enabled"
    );
    assert!(
        raw_contains(raw.as_slice(), b"\x1b[?1000l")
            || raw_contains(raw.as_slice(), b"\x1b[?1003;1006l"),
        "alternate-screen mouse capture was not restored"
    );
}

#[test]
fn f4_dictation_displays_a_meter_and_enter_submits_the_final_revision() {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("PTY");
    let mut command = fixture_command();
    command.cwd(std::env::current_dir().expect("fixture directory"));
    command.args(["--exact", "fixture_process", "--nocapture"]);
    command.env("COLOSSUS_TUI_PTY_FIXTURE", "1");
    command.env("COLOSSUS_TUI_DICTATION_FIXTURE", "1");
    command.env("COLOSSUS_TUI_STREAM_FIXTURE", "1");
    let mut child = pair.slave.spawn_command(command).expect("spawn fixture");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("PTY reader");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let reader_output = Arc::clone(&output);
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            reader_output
                .lock()
                .expect("output")
                .extend_from_slice(&buffer[..read]);
        }
    });
    let mut writer = pair.master.take_writer().expect("PTY writer");
    #[cfg(windows)]
    answer_cursor_position_query(&output, &mut writer);
    wait_for_screen(&output, 24, 80, "durable-row-01");
    writer
        .write_all(b"Typed \x1bOS")
        .expect("type prefix and F4");
    writer.flush().expect("flush start");
    wait_for_screen(&output, 24, 80, "Listening");
    wait_for_screen(&output, 24, 80, "Typed voice draft");
    writer.write_all(b"\r").expect("finish and submit");
    writer.flush().expect("flush send");
    wait_for_screen(&output, 24, 80, "Dictation accepted: Typed voice final");
    let final_screen = screen_contents(&output, 24, 80);
    assert!(final_screen.contains("Typed voice final"), "{final_screen}");
    assert!(
        !final_screen.contains("Typed voice draft"),
        "{final_screen}"
    );
    writer.write_all(&[3, 3]).expect("exit");
    writer.flush().expect("flush exit");
    let status = child.wait().expect("fixture status");
    drop(writer);
    drop(pair.master);
    reader_thread.join().expect("reader thread");
    assert!(status.success());
}

fn wait_for_raw(output: &Arc<Mutex<Vec<u8>>>, needle: &[u8]) {
    wait_for_raw_count(output, needle, 1);
}

fn wait_for_mouse_capture_count(output: &Arc<Mutex<Vec<u8>>>, enabled: bool, expected: usize) {
    // ConPTY projects native console-mode changes into a combined sequence;
    // POSIX emulators receive Crossterm's individual mode commands.
    let needles = if enabled {
        [b"\x1b[?1000h".as_slice(), b"\x1b[?1003;1006h".as_slice()]
    } else {
        [b"\x1b[?1000l".as_slice(), b"\x1b[?1003;1006l".as_slice()]
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let raw = output.lock().expect("output");
        let count = needles
            .iter()
            .map(|needle| {
                raw.windows(needle.len())
                    .filter(|bytes| bytes == needle)
                    .count()
            })
            .sum::<usize>();
        if count >= expected {
            return;
        }
        drop(raw);
        thread::sleep(Duration::from_millis(10));
    }
    panic!("PTY did not change mouse capture to {enabled} {expected} times");
}

fn raw_contains(raw: &[u8], needle: &[u8]) -> bool {
    raw.windows(needle.len()).any(|window| window == needle)
}

#[cfg(windows)]
fn answer_cursor_position_query(output: &Arc<Mutex<Vec<u8>>>, writer: &mut dyn std::io::Write) {
    wait_for_raw(output, b"\x1b[6n");
    writer
        .write_all(b"\x1b[1;1R")
        .expect("answer cursor-position query");
    writer.flush().expect("flush cursor-position answer");
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
    let bytes = output.lock().expect("output").clone();
    let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4_096)..])
        .escape_debug()
        .to_string();
    panic!(
        "PTY never emitted {expected} instances of {:?}; raw bytes: {}; raw tail: {tail}",
        String::from_utf8_lossy(needle).escape_debug().to_string(),
        bytes.len()
    );
}

fn wait_for_screen(output: &Arc<Mutex<Vec<u8>>>, rows: u16, cols: u16, needle: &str) {
    if screen_contains_within(output, rows, cols, needle, Duration::from_secs(10)) {
        return;
    }
    let bytes = output.lock().expect("output").clone();
    let tail_start = bytes.len().saturating_sub(4_096);
    let tail = String::from_utf8_lossy(&bytes[tail_start..])
        .escape_debug()
        .to_string();
    panic!(
        "screen never contained {needle}: {}\nraw bytes: {}\nraw tail: {tail}",
        screen_contents(output, rows, cols),
        bytes.len()
    );
}

fn screen_contains_within(
    output: &Arc<Mutex<Vec<u8>>>,
    rows: u16,
    cols: u16,
    needle: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if screen_contents(output, rows, cols).contains(needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
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

fn native_history_rows(
    output: &Arc<Mutex<Vec<u8>>>,
    rows: u16,
    cols: u16,
    live_marker: &str,
) -> Vec<String> {
    // Scrollback and the main screen must come from the same captured PTY frame.
    let bytes = output.lock().expect("output").clone();
    let mut parser = vt100::Parser::new(rows, cols, 512);
    parser.process(&bytes);
    parser.screen_mut().set_scrollback(usize::MAX);
    let scrollback_len = parser.screen().scrollback();
    let mut history = (1..=scrollback_len)
        .rev()
        .filter_map(|offset| {
            parser.screen_mut().set_scrollback(offset);
            parser.screen().rows(0, cols).next()
        })
        .collect::<Vec<_>>();
    parser.screen_mut().set_scrollback(0);
    let visible = parser.screen().rows(0, cols).collect::<Vec<_>>();
    let live_viewport = visible
        .iter()
        .rposition(|row| row.contains(live_marker))
        .expect("live viewport marker");
    history.extend_from_slice(&visible[..live_viewport]);
    // ConPTY may restore an explicitly space-painted blank row with Erase Line.
    // vt100 distinguishes those encodings; preserve every row and all nonblank
    // bytes, including leading/trailing spaces and non-ASCII whitespace.
    for row in &mut history {
        if row.bytes().all(|byte| byte == b' ') {
            row.clear();
        }
    }
    history
}

#[test]
fn native_history_compares_blank_repaints_without_hiding_transcript_changes() {
    let before = b"\x1b[1;1H left \x1b[2;1H                \x1b[3;1Hright \x1b[4;1HMessage";
    let captured = |bytes: Vec<u8>| Arc::new(Mutex::new(bytes));
    let baseline = captured(before.to_vec());
    let expected = native_history_rows(&baseline, 6, 16, "Message");
    assert_eq!(expected, [" left ", "", "right "]);

    // This is the relevant ConPTY main-screen repaint after a real 1049 restore:
    // an explicitly painted separator is re-encoded as an erased, empty row.
    let mut restored = before.to_vec();
    restored.extend_from_slice(b"\x1b[?1049htransient chrome\x1b[?1049l\x1b[2;1H\x1b[K");
    let restored = captured(restored);
    assert_eq!(screen_rows(&baseline, 6, 16)[1], "                ");
    assert_eq!(screen_rows(&restored, 6, 16)[1], "");
    assert_eq!(native_history_rows(&restored, 6, 16, "Message"), expected);

    for (name, mutation) in [
        ("missing row", b"\x1b[2;1H\x1b[M".as_slice()),
        ("duplicated row", b"\x1b[2;1H\x1b[L".as_slice()),
        ("nonblank change", b"\x1b[2;1Hchanged".as_slice()),
        ("leading space lost", b"\x1b[1;1H\x1b[Kleft ".as_slice()),
        ("trailing space lost", b"\x1b[1;1H\x1b[K left".as_slice()),
        ("non-ASCII whitespace", "\x1b[2;1H\u{a0}".as_bytes()),
    ] {
        let mut changed = before.to_vec();
        changed.extend_from_slice(mutation);
        assert_ne!(
            native_history_rows(&captured(changed), 6, 16, "Message"),
            expected,
            "history comparison hid {name}"
        );
    }
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
