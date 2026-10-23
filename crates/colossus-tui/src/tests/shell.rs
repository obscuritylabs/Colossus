use super::*;

#[test]
fn bang_commands_retain_shell_source_and_never_become_provider_turns() {
    for (input, command) in [
        ("!pwd", "pwd"),
        (
            " ! source .venv/bin/activate && python -V ",
            "source .venv/bin/activate && python -V",
        ),
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
        parse_interactive_command("Explain !pwd"),
        InteractiveCommand::Turn(_)
    ));
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
