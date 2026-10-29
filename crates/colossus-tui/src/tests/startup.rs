use super::*;

fn armed_startup(source: InteractiveSnapshot) -> TuiState {
    let mut state = TuiState::from_snapshot(source);
    state.startup_animation = Some(crate::startup::StartupAnimation::new());
    state
}

fn draw_startup(state: &mut TuiState, width: u16, height: u16, mode: ScreenMode) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| render(frame, state, 0, mode))
        .expect("startup frame");
    terminal.backend().to_string()
}

#[test]
fn snapshots_never_rearm_launch_branding() {
    for source in [empty_snapshot(), snapshot()] {
        let state = TuiState::from_snapshot(source);
        assert!(state.startup_animation.is_none());
    }
}

#[test]
fn entry_screen_owns_the_viewport_without_hidden_workspace_targets() {
    for (width, height) in [(40, 24), (80, 24), (140, 40)] {
        let mut state = armed_startup(empty_snapshot());
        let hidden = Rect::new(1, 1, 5, 5);
        state.agents.area = Some(hidden);
        state.text_selection.regions.push(hidden);
        state
            .workspace_hits
            .push((hidden, crate::workspace_view::WorkspaceAction::New));
        let rendered = draw_startup(&mut state, width, height, ScreenMode::Alternate);
        assert!(state.startup_animation.is_some(), "{rendered}");
        assert!(rendered.contains("Press any key to continue"), "{rendered}");
        for hidden_text in [
            "Enter sends",
            "approval ask",
            "Workspace sessions",
            "Agents ·",
            "What do you want to work on?",
        ] {
            assert!(
                !rendered.contains(hidden_text),
                "unexpected {hidden_text}: {rendered}"
            );
        }
        assert!(state.agents.area.is_none());
        assert!(state.workspace_hits.is_empty());
        assert!(state.text_selection.regions.is_empty());
    }
}

#[test]
fn startup_findings_preempt_the_entry_screen() {
    let mut source = empty_snapshot();
    source.security_posture.findings = vec![colossus_contracts::SecurityPostureFinding {
        code: "storage.plaintext".into(),
        severity: colossus_contracts::SecurityPostureSeverity::Warning,
        summary: "Journal payloads use plaintext storage.".into(),
        remediation: "Configure platform storage keys.".into(),
    }];
    let mut state = armed_startup(source);
    let rendered = draw_startup(&mut state, 140, 40, ScreenMode::Alternate);
    assert!(state.startup_animation.is_none(), "{rendered}");
    assert!(!rendered.contains("Press any key to continue"));
    for text in [
        "Security posture · 1 warning",
        "Journal payloads use plaintext storage.",
        "Configure platform storage keys.",
        "approval ask",
        "Enter sends",
    ] {
        assert!(rendered.contains(text), "missing {text}: {rendered}");
    }
}

#[test]
fn an_arriving_notice_ends_the_entry_screen_immediately() {
    let mut state = armed_startup(empty_snapshot());
    assert!(
        draw_startup(&mut state, 140, 40, ScreenMode::Alternate)
            .contains("Press any key to continue")
    );
    handle_host_event(
        &mut state,
        HostEvent::Notice(PresentationDocument::from_block(PresentationBlock::Text(
            "Startup notice must be visible now".into(),
        ))),
    );
    let rendered = draw_startup(&mut state, 140, 40, ScreenMode::Alternate);
    assert!(state.startup_animation.is_none());
    assert!(
        rendered.contains("Startup notice must be visible now"),
        "{rendered}"
    );
    assert!(rendered.contains("Enter sends"), "{rendered}");
}

#[test]
fn compact_inline_resumed_and_editing_views_end_the_intro() {
    for (width, height, mode) in [
        (39, 24, ScreenMode::Alternate),
        (140, 23, ScreenMode::Alternate),
        (140, 40, ScreenMode::Inline),
    ] {
        let mut state = armed_startup(empty_snapshot());
        let rendered = draw_startup(&mut state, width, height, mode);
        assert!(state.startup_animation.is_none());
        assert!(!rendered.contains("Press any key to continue"));
    }
    let mut resumed = armed_startup(snapshot());
    let rendered = draw_startup(&mut resumed, 140, 40, ScreenMode::Alternate);
    assert!(resumed.startup_animation.is_none());
    assert!(rendered.contains("durable row marker"));

    let mut editing = armed_startup(empty_snapshot());
    editing.composer.insert("first key retained");
    let rendered = draw_startup(&mut editing, 140, 40, ScreenMode::Alternate);
    assert!(editing.startup_animation.is_none());
    assert!(rendered.contains("first key retained"));
}

#[test]
fn sessions_decisions_and_text_selection_preempt_branding() {
    for interrupt in 0..5 {
        let mut state = armed_startup(empty_snapshot());
        match interrupt {
            0 => state.workspace_center = true,
            1 => state.overlay = Some(Overlay::QueuePaused),
            2 => state.operation = Some(OperationKind::Run),
            3 => state.sandbox_boundary_acknowledgement_in_progress = true,
            _ => state.selecting_text = true,
        }
        let rendered = draw_startup(&mut state, 140, 40, ScreenMode::Alternate);
        assert!(state.startup_animation.is_none());
        assert!(!rendered.contains("Press any key to continue"));
    }
}

#[test]
fn completed_intro_restores_the_workspace_without_leaving_splash_cells() {
    let mut state = armed_startup(empty_snapshot());
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).expect("terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("entry screen");
    assert!(
        terminal
            .backend()
            .to_string()
            .contains("Press any key to continue")
    );
    state.startup_animation = Some(crate::startup::StartupAnimation::new());
    state
        .startup_animation
        .as_mut()
        .expect("armed")
        .prepare(Instant::now() - Duration::from_secs(5));
    for _ in 0..2 {
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("workspace");
        let rendered = terminal.backend().to_string();
        assert!(state.startup_animation.is_none());
        assert!(rendered.contains("What do you want to work on?"));
        assert!(rendered.contains("Enter sends"));
        assert!(!rendered.contains("Press any key to continue"));
        assert!(!rendered.contains("OBSCURITY LABS // COLOSSUS"));
    }
}
