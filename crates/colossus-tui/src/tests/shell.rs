use super::*;

#[test]
fn bang_commands_retain_shell_source_and_never_become_provider_turns() {
    for (input, command) in [
        ("!pwd", "pwd"),
        (
            " ! source .venv/bin/activate && python -V ",
            "source .venv/bin/activate && python -V ",
        ),
        ("!printf %s foo\\ ", "printf %s foo\\ "),
        ("!printf done \t\n", "printf done \t\n"),
        ("!printf done\\\n", "printf done\\\n"),
        (
            "!printf '%s\\n' 'a  b' | sort\nprintf done",
            "printf '%s\\n' 'a  b' | sort\nprintf done",
        ),
    ] {
        assert_eq!(
            parse_interactive_command(input),
            InteractiveCommand::Runtime(RuntimeCommand::Shell {
                command: command.into()
            })
        );
    }
    for input in ["!", "!  ", "!echo\0secret"] {
        assert!(matches!(
            parse_interactive_command(input),
            InteractiveCommand::Invalid(_)
        ));
    }
    assert!(matches!(
        parse_interactive_command(&format!("!{}", "x".repeat(64 * 1024 + 1))),
        InteractiveCommand::Invalid(_)
    ));
    assert!(matches!(
        parse_interactive_command(&format!("!{} ", "x".repeat(64 * 1024))),
        InteractiveCommand::Invalid(_)
    ));
    assert!(matches!(
        parse_interactive_command("Explain !pwd"),
        InteractiveCommand::Turn(_)
    ));
}

#[test]
fn non_shell_input_keeps_its_whitespace_normalization() {
    for input in ["", " \t\r\n"] {
        assert_eq!(parse_interactive_command(input), InteractiveCommand::Empty);
    }
    assert_eq!(
        parse_interactive_command(" \tExplain !pwd \r\n"),
        InteractiveCommand::Turn("Explain !pwd".into())
    );
    assert_eq!(
        parse_interactive_command(" \t/tools  filesystem.search \r\n"),
        InteractiveCommand::Runtime(RuntimeCommand::Known {
            name: "tools".into(),
            arguments: "filesystem.search".into(),
        })
    );
}

#[tokio::test]
async fn shell_submission_preserves_trailing_source_through_history_queue_and_dispatch() {
    for queued in [false, true] {
        let mut state = TuiState::from_snapshot(snapshot());
        let (commands, mut received_commands) = mpsc::unbounded_channel();
        let (history, mut received_history) = mpsc::unbounded_channel();
        let host = Arc::new(ShellHost { commands, history });
        let (events, _receiver) = mpsc::channel(16);
        let input = " \t!printf %s foo\\ ";
        state.composer.insert(input);
        let line = if queued {
            state.operation = Some(OperationKind::Run);
            state.preferences.multiline = true;
            handle_composer_ctrl_d(&mut state)
        } else {
            handle_composer_enter(
                &mut state,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            )
        }
        .expect("submitted composer source");
        submit_line(&mut state, line, host.clone(), events.clone());
        assert!(state.draft().is_empty());
        assert_eq!(state.history.last().unwrap(), "!printf %s foo\\ ");
        if queued {
            assert!(received_commands.try_recv().is_err());
            assert_eq!(state.queue.front().unwrap(), "!printf %s foo\\ ");
            state.operation = None;
            let line = state.queue.pop_front().expect("queued shell command");
            start_line(&mut state, line, host, events);
        }
        let saved = tokio::time::timeout(Duration::from_secs(1), received_history.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved, "!printf %s foo\\ ");
        let command = tokio::time::timeout(Duration::from_secs(1), received_commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            command,
            RuntimeCommand::Shell {
                command: "printf %s foo\\ ".into(),
            }
        );
    }
}

struct ShellHost {
    commands: mpsc::UnboundedSender<RuntimeCommand>,
    history: mpsc::UnboundedSender<String>,
}

#[async_trait]
impl InteractiveHost for ShellHost {
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
        command: RuntimeCommand,
        _: &str,
        _: &[String],
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostCommandResult, String> {
        self.commands.send(command).unwrap();
        Err("shell fixture finished".into())
    }

    async fn run_turn(
        &self,
        _: InteractiveRunRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostRunResult, String> {
        panic!("shell commands must not become provider turns")
    }

    async fn run_plan_execution(
        &self,
        _: InteractivePlanExecutionRequest,
        _: mpsc::Sender<HostEvent>,
        _: RunControl,
    ) -> Result<HostPlanExecutionResult, String> {
        panic!("shell commands must not become plan executions")
    }

    async fn append_history(
        &self,
        _: &str,
        entry: String,
        _: mpsc::Sender<HostEvent>,
    ) -> Result<(), String> {
        self.history.send(entry).unwrap();
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

#[test]
fn shell_intent_is_visible_before_submitting_in_every_mode() {
    for mode in [
        InteractiveMode::Execute,
        InteractiveMode::Plan,
        InteractiveMode::Research,
    ] {
        let mut state = TuiState::from_snapshot(snapshot());
        state.mode = mode;
        state.composer.insert("!pwd");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw");
        assert!(
            terminal
                .backend()
                .to_string()
                .contains("Shell · Enter runs")
        );
        assert_eq!(state.draft(), "!pwd");
    }
}

#[test]
fn shell_arguments_do_not_trigger_skill_completion() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("!printf @plugin/skill");
    assert!(state.structured_completion_context().is_none());
}
