use super::*;
use crate::{
    app::{handle_host_event, handle_run_event},
    tests::snapshot,
};
use ratatui::backend::TestBackend;

fn metadata() -> SidebarSnapshot {
    SidebarSnapshot {
        tasks: vec![
            ("Review runtime".into(), TaskStatus::InProgress),
            ("Waiting on input".into(), TaskStatus::Blocked),
        ],
        open_task_count: 7,
        goals: vec![("Ship the release".into(), GoalStatus::Active)],
        goal_count: 1,
        memory_count: Some((24, false)),
        plan_count: 2,
        mcp_servers: vec![("atlas".into(), true), ("offline".into(), false)],
    }
}

fn text(state: &SidebarState) -> String {
    state
        .lines(
            25,
            TerminalPalette::for_preferences(&TerminalPreferences::default()),
        )
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn call() -> colossus_contracts::ToolCall {
    colossus_contracts::ToolCall {
        call_id: "mcp-call".into(),
        name: "mcp.call".into(),
        arguments: serde_json::json!({"server":"atlas", "tool":"search", "arguments":{"query":"test"}}),
    }
}

#[test]
fn metadata_shows_bounded_work_previews_and_explicit_unknown_memory_counts() {
    let mut unavailable = SidebarState::default();
    unavailable.refresh(Err("private backend failure".into()));
    assert!(unavailable.loaded());
    assert_eq!(text(&unavailable), "Status unavailable");
    let mut state = SidebarState::default();
    state.refresh(Ok(Some(metadata())));
    let rendered = text(&state);
    for expected in [
        "Tasks 7 · Goals 1",
        "Memory 24 · Plans 2",
        "Review runtime",
        "blocked",
        "+5 · /tasks",
        "Ship the release",
        "MCP · 2 configured",
        "configured",
        "unavailable",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
    state.refresh(Err("private backend failure".into()));
    assert!(text(&state).contains("Status · stale"));
    assert!(text(&state).contains("Memory 24"));
    assert!(!text(&state).contains("private backend"));
    let mut data = metadata();
    data.memory_count = None;
    state.refresh(Ok(Some(data)));
    assert!(text(&state).contains("Memory —"));
}

#[test]
fn mcp_bubble_only_uses_success_style_after_a_released_successful_result() {
    let mut state = SidebarState::default();
    state.refresh(Ok(Some(metadata())));
    assert!(text(&state).contains("configured"));
    assert!(!text(&state).contains("used"));
    state.observe(&RunEvent::ToolStarted {
        turn: 1,
        call: call(),
        elapsed_seconds: 0.0,
    });
    assert!(text(&state).contains("in use"));
    state.observe(&RunEvent::ToolCompleted {
        turn: 1,
        result: colossus_contracts::ToolResult {
            call_id: "mcp-call".into(),
            name: "mcp.call".into(),
            output: "private result not retained".into(),
            exit_code: 0,
            images: Vec::new(),
        },
        duration_seconds: 0.1,
        elapsed_seconds: 0.1,
    });
    let palette = TerminalPalette::for_preferences(&TerminalPreferences::default());
    let lines = state.lines(25, palette);
    let used = lines
        .iter()
        .find(|line| line.to_string().contains("atlas"))
        .unwrap();
    assert!(used.to_string().contains("●"));
    assert!(used.to_string().contains("used"));
    assert_eq!(
        used.spans[1].style,
        ratatui_style(palette.tone_style(PresentationTone::Success))
    );
    assert!(!text(&state).contains("private result"));
    state.observe(&RunEvent::ToolStarted {
        turn: 1,
        call: call(),
        elapsed_seconds: 0.2,
    });
    state.observe(&RunEvent::ToolCancelled {
        turn: 1,
        call: call(),
        elapsed_seconds: 0.3,
    });
    assert!(text(&state).contains("cancelled"));
    assert!(!text(&state).contains("used"));
}

#[test]
fn sidebar_refreshes_and_mcp_events_are_fenced_to_the_selected_session() {
    let mut state = TuiState::from_snapshot(snapshot());
    for (session_id, epoch) in [("other", 0), ("019f-test", 1)] {
        handle_host_event(
            &mut state,
            HostEvent::Sidebar {
                session_id: session_id.into(),
                session_epoch: epoch,
                result: Ok(Some(metadata())),
            },
        );
        assert!(!state.sidebar.loaded());
    }
    handle_host_event(
        &mut state,
        HostEvent::Sidebar {
            session_id: "019f-test".into(),
            session_epoch: 0,
            result: Ok(Some(metadata())),
        },
    );
    handle_run_event(
        &mut state,
        RunEventEnvelope {
            schema_version: 1,
            session_id: "other".into(),
            run_id: "run".into(),
            event: RunEvent::ToolStarted {
                turn: 1,
                call: call(),
                elapsed_seconds: 0.0,
            },
        },
    );
    assert!(!text(&state.sidebar).contains("in use"));
    assert!(text(&state.sidebar).contains("configured"));
}

#[test]
fn status_is_visible_without_subagents_and_scrolls_independently_in_both_screen_modes() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.sidebar.refresh(Ok(Some(metadata())));
    state
        .sidebar
        .snapshot
        .as_mut()
        .unwrap()
        .mcp_servers
        .extend((0..30).map(|i| (format!("server-{i:02}"), true)));
    state.composer.insert("keep draft");
    for mode in [ScreenMode::Alternate, ScreenMode::Inline] {
        for width in [120, 80, 120] {
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal
                .draw(|frame| render(frame, &mut state, 0, mode))
                .unwrap();
            let rendered = terminal.backend().to_string();
            assert_eq!(rendered.contains("Memory 24"), width >= 92, "{rendered}");
            assert_eq!(state.agents.area.is_some(), width >= 92);
            assert_eq!(state.draft(), "keep draft");
        }
    }
    let panel = state.agents.area.unwrap();
    crate::app::handle_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panel.x + 2,
            row: panel.y + 2,
            modifiers: KeyModifiers::NONE,
        },
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .unwrap();
    assert!(!terminal.backend().to_string().contains("Memory 24"));
    assert_eq!(state.scroll_from_bottom, 0);
}
