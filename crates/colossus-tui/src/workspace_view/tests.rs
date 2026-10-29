use super::*;
use crate::tests::snapshot;
use ratatui::backend::TestBackend;

fn row(id: &str, status: WorkspaceSessionStatus) -> WorkspaceSessionRow {
    WorkspaceSessionRow {
        id: id.into(),
        title: format!("Session {id}"),
        status,
        unread: 0,
    }
}

fn state() -> TuiState {
    let mut state = TuiState::from_snapshot(snapshot());
    state.workspace_sessions = vec![
        row("019f-test", WorkspaceSessionStatus::Running),
        row("review", WorkspaceSessionStatus::Waiting),
        row("tests", WorkspaceSessionStatus::Complete),
    ];
    state
}

fn draw(state: &mut TuiState, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| crate::render::render(frame, state, 0, ScreenMode::Alternate))
        .expect("draw");
    terminal.backend().to_string()
}

#[test]
fn workspace_sessions_are_pinned_below_current_agent_details_and_click_exact_session() {
    let mut state = state();
    state.composer.insert("unfinished draft");
    let rendered = draw(&mut state, 120, 24);
    assert!(rendered.contains("Workspace sessions"), "{rendered}");
    assert!(rendered.contains("Session rev"), "{rendered}");
    assert!(rendered.contains("decision"), "{rendered}");
    let (hit, _) = state
        .workspace_hits
        .iter()
        .find(|(_, action)| *action == WorkspaceAction::Switch("review".into()))
        .expect("review session hit");
    let hit = *hit;
    assert!(hit.y >= state.agents.area.expect("agent details").bottom());
    assert!(handle_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        }
    ));
    assert_eq!(
        state.workspace_action,
        Some(WorkspaceAction::Switch("review".into()))
    );
    assert_eq!(state.draft(), "unfinished draft");
    assert_eq!(state.session_id, "019f-test");
}

#[test]
fn narrow_terminal_keeps_command_center_navigation_and_draft() {
    let mut state = state();
    state.composer.insert("keep draft");
    assert!(!draw(&mut state, 60, 16).contains("Workspace sessions"));
    assert!(state.workspace_hits.is_empty());
    state.workspace_center = true;
    let rendered = draw(&mut state, 40, 12);
    assert!(rendered.contains("Sessions"), "{rendered}");
    assert!(rendered.contains("Session review"), "{rendered}");
    assert!(handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    ));
    assert!(handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    ));
    assert_eq!(
        state.workspace_action,
        Some(WorkspaceAction::Switch("review".into()))
    );
    assert_eq!(state.draft(), "keep draft");
}

#[test]
fn scrolling_roster_preserves_conversation_scroll_and_reveals_last_session() {
    let mut state = state();
    state.workspace_sessions = (0..100)
        .map(|index| row(&format!("{index:03}"), WorkspaceSessionStatus::Ready))
        .collect();
    state.workspace_center = true;
    state.scroll_from_bottom = 7;
    state.composer.insert("keep editing");
    handle_key(&mut state, KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let rendered = draw(&mut state, 60, 16);
    assert!(rendered.contains("Session 099"), "{rendered}");
    assert!(rendered.contains("100 open"), "{rendered}");
    assert_eq!(state.scroll_from_bottom, 7);
    assert_eq!(state.draft(), "keep editing");
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        state.workspace_action,
        Some(WorkspaceAction::Switch("099".into()))
    );
}

#[test]
fn command_center_controls_do_not_stop_an_idle_session() {
    let mut state = state();
    state.workspace_center = true;
    state.workspace_selected = 1;
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    assert_eq!(
        state.workspace_action.take(),
        Some(WorkspaceAction::Stop("review".into()))
    );
    state.workspace_manager.select(Some("tests".into()));
    handle_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    assert_eq!(state.workspace_action, None);
    handle_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.workspace_action, Some(WorkspaceAction::CloseCenter));
}

#[test]
fn session_titles_are_sanitized_and_statuses_survive_supported_palettes() {
    use colossus_contracts::ThemeName;
    let mut state = state();
    state.workspace_sessions = [
        WorkspaceSessionStatus::Ready,
        WorkspaceSessionStatus::Running,
        WorkspaceSessionStatus::Waiting,
        WorkspaceSessionStatus::Complete,
        WorkspaceSessionStatus::Failed,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, status)| WorkspaceSessionRow {
        id: index.to_string(),
        title: "界👩‍💻 e\u{301}\n\u{1b}[31m title".into(),
        status,
        unread: index,
    })
    .collect();
    state.workspace_center = true;
    for theme in [
        ThemeName::Default,
        ThemeName::Mono,
        ThemeName::HighContrast,
        ThemeName::Carrot,
        ThemeName::Hacker,
    ] {
        state.preferences.select_builtin_theme(theme);
        for (index, label) in ["ready", "working", "decision", "done", "failed"]
            .into_iter()
            .enumerate()
        {
            draw(&mut state, 120, 24);
            state.workspace_manager.select(Some(index.to_string()));
            let rendered = draw(&mut state, 120, 24);
            assert!(rendered.contains(label), "{label}: {rendered}");
            assert!(!rendered.contains('\u{1b}'), "{rendered}");
        }
    }
}

#[test]
fn closing_or_resizing_center_clears_obsolete_click_targets() {
    let mut state = state();
    state.workspace_center = true;
    draw(&mut state, 100, 24);
    assert!(!state.workspace_hits.is_empty());
    state.workspace_center = false;
    draw(&mut state, 40, 12);
    assert!(state.workspace_hits.is_empty());
}

#[test]
fn inline_command_center_uses_the_available_terminal_height() {
    let mut state = state();
    state.workspace_center = true;
    assert_eq!(
        crate::render::desired_inline_viewport_height(&state, 40, 24, 0),
        24
    );
}
