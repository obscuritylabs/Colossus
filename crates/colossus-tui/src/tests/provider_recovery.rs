use super::*;
use std::sync::Mutex;

fn failure_event(code: &str, http_status: Option<u16>) -> RunEventEnvelope {
    RunEventEnvelope {
        schema_version: 1,
        run_id: "failed-provider-run".into(),
        session_id: "019f-test".into(),
        event: RunEvent::Error {
            code: code.into(),
            message: "The request failed.".into(),
            recoverable: false,
            http_status,
            retry_after_ms: None,
            turn: Some(3),
            elapsed_seconds: 1.0,
        },
    }
}

fn finish_failure(state: &mut TuiState, code: &str, status: Option<u16>) {
    state.operation = Some(OperationKind::Run);
    handle_run_event(state, failure_event(code, status));
    assert!(
        !provider_recovery_visible(state),
        "the run must settle first"
    );
    handle_host_event(
        state,
        HostEvent::OperationFinished(Box::new(Err("The request failed.".into()))),
    );
}

#[test]
fn provider_failure_actions_require_known_provider_evidence_and_a_terminal_run() {
    for (code, status, offered) in [
        ("provider.failed", Some(404), true),
        ("provider.unavailable", None, true),
        ("provider.temporarily_unavailable", Some(503), true),
        ("provider.invalid_tool_arguments", None, true),
        ("provider.failed", None, false),
        ("provider.configuration", None, false),
        ("provider.outcome_unknown", Some(503), false),
        ("tool.outcome_unknown", None, false),
        ("tool.denied", None, false),
        ("session.integrity", None, false),
        ("agent.cancelled", None, false),
    ] {
        let mut state = TuiState::from_snapshot(snapshot());
        finish_failure(&mut state, code, status);
        assert_eq!(provider_recovery_visible(&state), offered, "{code}");
    }

    let mut state = TuiState::from_snapshot(snapshot());
    state.operation = Some(OperationKind::Run);
    handle_run_event(&mut state, failure_event("provider.failed", Some(404)));
    handle_run_event(
        &mut state,
        RunEventEnvelope {
            event: RunEvent::Provider {
                event: ProviderEvent::ModelDelta {
                    text: "Recovered".into(),
                },
            },
            ..failure_event("provider.failed", None)
        },
    );
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("Later tool failure".into()))),
    );
    assert!(!provider_recovery_visible(&state));
}

#[test]
fn provider_failure_actions_preserve_composer_focus_and_completion_navigation() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("preserved draft");
    finish_failure(&mut state, "provider.failed", Some(404));
    assert!(!provider_recovery_focused(&state));
    assert!(matches!(
        provider_recovery_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        ProviderRecoveryKey::Ignored
    ));
    for focused in [true, true, false] {
        assert!(matches!(
            provider_recovery_key(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            ProviderRecoveryKey::Handled
        ));
        assert_eq!(provider_recovery_focused(&state), focused);
    }
    provider_recovery_key(
        &mut state,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert!(matches!(
        provider_recovery_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
        ),
        ProviderRecoveryKey::Action(ProviderRecoveryAction::Dismiss)
    ));
    assert!(matches!(
        provider_recovery_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ),
        ProviderRecoveryKey::Ignored
    ));
    assert!(!provider_recovery_focused(&state));
    assert_eq!(state.draft(), "preserved draft");

    state.composer.take();
    state.composer.insert("/t");
    assert!(!state.completion_menu_candidates().is_empty());
    assert!(matches!(
        provider_recovery_key(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        ProviderRecoveryKey::Ignored
    ));
    assert!(!provider_recovery_focused(&state));
}

#[test]
fn provider_failure_actions_render_inline_at_supported_widths_with_pointer_targets() {
    for width in [40, 80, 120] {
        for mode in [ScreenMode::Inline, ScreenMode::Alternate] {
            let mut state = TuiState::from_snapshot(snapshot());
            state.composer.insert("preserved draft");
            finish_failure(&mut state, "provider.failed", Some(404));
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).expect("terminal");
            terminal
                .draw(|frame| render(frame, &mut state, 0, mode))
                .expect("draw");
            let rendered = terminal.backend().to_string();
            assert!(rendered.contains("Provider request failed."), "{rendered}");
            assert!(rendered.contains("[ Retry ]  [ Dismiss ]"), "{rendered}");
            assert!(rendered.contains("preserved draft"), "{rendered}");
            assert!(state.overlay.is_none());
            let buffer = terminal.backend().buffer();
            for (label, action) in [
                ("Retry", ProviderRecoveryAction::Retry),
                ("Dismiss", ProviderRecoveryAction::Dismiss),
            ] {
                let position = (0..24)
                    .find_map(|y| {
                        let row = (0..width)
                            .map(|x| buffer.cell((x, y)).unwrap().symbol())
                            .collect::<String>();
                        row.find(label)
                            .filter(|_| row.contains("[ Retry ]"))
                            .map(|x| Position::new(x as u16, y))
                    })
                    .expect("button position");
                assert_eq!(
                    provider_recovery_mouse(
                        &state,
                        MouseEvent {
                            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                            column: position.x,
                            row: position.y,
                            modifiers: KeyModifiers::NONE,
                        }
                    ),
                    Some(action)
                );
            }
            let height = desired_inline_viewport_height(&state, width, 24, state.transcript.len());
            state.provider_recovery = None;
            assert_eq!(
                height,
                desired_inline_viewport_height(&state, width, 24, state.transcript.len()) + 3
            );
        }
    }
}

#[test]
fn provider_failure_actions_keep_a_long_draft_usable_at_minimum_size() {
    let mut state = TuiState::from_snapshot(snapshot());
    state
        .composer
        .insert("line one\nline two\nline three\nline four\nline five\nline six");
    finish_failure(&mut state, "provider.failed", Some(404));
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("[ Retry ]  [ Dismiss ]"), "{rendered}");
    assert!(rendered.contains("line six"), "{rendered}");
    let cursor = terminal.backend().cursor_position();
    assert_eq!(
        terminal
            .backend()
            .buffer()
            .cell((cursor.x - 1, cursor.y))
            .unwrap()
            .symbol(),
        "x"
    );
    state.selecting_text = true;
    for row in 0..12 {
        assert!(
            provider_recovery_mouse(
                &state,
                MouseEvent {
                    kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    column: 3,
                    row,
                    modifiers: KeyModifiers::NONE,
                }
            )
            .is_none()
        );
    }
}

#[derive(Default)]
struct RecoveryHost {
    requests: Mutex<Vec<InteractiveRunRequest>>,
}

#[async_trait]
impl InteractiveHost for RecoveryHost {
    async fn bootstrap(&self, _: BootstrapRequest) -> Result<InteractiveSnapshot, String> {
        Ok(snapshot())
    }
    async fn acknowledge_sandbox_boundary(
        &self,
        _: &str,
        _: SandboxBoundaryMode,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<bool, String> {
        Ok(false)
    }
    async fn execute_command(
        &self,
        _: RuntimeCommand,
        _: &str,
        _: &[String],
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostCommandResult, String> {
        Err("not a command".into())
    }
    async fn run_turn(
        &self,
        request: InteractiveRunRequest,
        events: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostRunResult, String> {
        self.requests.lock().unwrap().push(request);
        events
            .send(HostEvent::Run(failure_event("provider.failed", Some(404))))
            .await
            .unwrap();
        Err("The request failed.".into())
    }
    async fn run_plan_execution(
        &self,
        _: InteractivePlanExecutionRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        Err("not a Plan".into())
    }
    async fn append_history(
        &self,
        _: &str,
        _: String,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn save_preferences(
        &self,
        _: &str,
        preferences: TerminalPreferences,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<TerminalPreferences, String> {
        Ok(preferences)
    }
    async fn older_messages(&self, _: &str, _: u64) -> Result<SessionMessagePage, String> {
        Err("no older messages".into())
    }
}

#[tokio::test]
async fn provider_failure_retry_submits_continue_once_in_the_same_session_and_can_be_dismissed() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("draft for later");
    state.composer.move_to(5);
    state.pending_images.push(image_reference(0));
    finish_failure(&mut state, "provider.failed", Some(404));
    assert!(state.pending_images.is_empty());
    let host = Arc::new(RecoveryHost::default());
    let (events, mut receiver) = mpsc::channel(16);
    apply_provider_recovery_action(
        &mut state,
        ProviderRecoveryAction::Retry,
        host.clone(),
        events.clone(),
    );
    apply_provider_recovery_action(
        &mut state,
        ProviderRecoveryAction::Retry,
        host.clone(),
        events.clone(),
    );
    assert!(state.is_busy());
    assert!(!provider_recovery_visible(&state));
    loop {
        let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let finished = matches!(event, HostEvent::OperationFinished(_));
        handle_host_event(&mut state, event);
        if finished {
            break;
        }
    }
    let requests = host.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].session_id, "019f-test");
    assert_eq!(requests[0].prompt, "Continue");
    assert!(requests[0].images.is_empty());
    assert_eq!(requests[0].mode, AgentRunMode::Execute);
    drop(requests);
    assert!(
        provider_recovery_visible(&state),
        "another failure offers recovery again"
    );
    assert_eq!(state.draft(), "draft for later");
    assert_eq!(state.cursor(), 5);
    apply_provider_recovery_action(
        &mut state,
        ProviderRecoveryAction::Dismiss,
        host.clone(),
        events,
    );
    assert!(!provider_recovery_visible(&state));
    assert!(!provider_recovery_focused(&state));
    assert!(state.overlay.is_none());
    assert_eq!(state.draft(), "draft for later");
    assert_eq!(host.requests.lock().unwrap().len(), 1);
}
