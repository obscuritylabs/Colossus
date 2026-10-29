use super::*;
use crate::{
    app::{apply_command_result, handle_host_event, handle_run_event},
    tests::snapshot,
};
use ratatui::backend::TestBackend;

fn job(id: &str, status: SubagentStatus) -> SubagentJob {
    SubagentJob {
        id: id.into(),
        session_id: "019f-test".into(),
        parent_run_id: "parent-run".into(),
        parent_call_id: format!("call-{id}"),
        task: format!("Inspect {id}"),
        role: "subagent_default".into(),
        allowed_tools: None,
        status,
        child_session_id: format!("session-{id}"),
        child_run_id: None,
        final_output: "released result stays in transcript".into(),
        error: String::new(),
        created_at: "2026-10-23T18:00:00Z".into(),
        updated_at: "2026-10-23T18:00:00Z".into(),
        started_at: None,
        completed_at: None,
    }
}

fn update(state: &mut TuiState, job: SubagentJob) {
    handle_run_event(
        state,
        RunEventEnvelope {
            schema_version: 1,
            run_id: "parent-run".into(),
            session_id: job.session_id.clone(),
            event: RunEvent::SubagentUpdated { job: Box::new(job) },
        },
    );
}

fn draw(state: &mut TuiState, width: u16, height: u16, mode: ScreenMode) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| render(frame, state, 0, mode))
        .expect("draw");
    terminal.backend().to_string()
}

#[test]
fn lifecycle_updates_are_retained_as_one_tree_row_per_agent() {
    let mut state = TuiState::from_snapshot(snapshot());
    for status in [
        SubagentStatus::Queued,
        SubagentStatus::Running,
        SubagentStatus::Completed,
    ] {
        update(&mut state, job("code", status));
        assert_eq!(state.agents.nodes.len(), 1);
        assert_eq!(state.agents.nodes["code"].status, status);
    }
    update(&mut state, job("tests", SubagentStatus::Running));
    let rendered = draw(&mut state, 120, 24, ScreenMode::Alternate);
    assert!(rendered.contains("Agents · 1 active"), "{rendered}");
    assert!(rendered.contains("├─ ● Inspect tests"), "{rendered}");
    assert!(rendered.contains("└─ ✓ Inspect code"), "{rendered}");
    assert!(rendered.contains("working"), "{rendered}");
    assert!(rendered.contains("done"), "{rendered}");
    let panel = state.agents.area.expect("visible panel");
    assert_eq!(panel.width, PANEL_WIDTH);
    assert_eq!(panel.right(), 120);
    assert_eq!(state.transcript_width, 84);
}

#[test]
fn refreshed_history_cannot_regress_newer_live_updates_or_cross_sessions() {
    let mut state = TuiState::from_snapshot(snapshot());
    update(&mut state, job("code", SubagentStatus::Completed));
    handle_host_event(
        &mut state,
        HostEvent::Subagents {
            session_id: "019f-test".into(),
            revision: 0,
            session_epoch: 0,
            result: Ok(vec![job("code", SubagentStatus::Running)]),
        },
    );
    assert_eq!(state.agents.nodes["code"].status, SubagentStatus::Completed);
    let mut foreign = job("foreign", SubagentStatus::Running);
    foreign.session_id = "other-session".into();
    update(&mut state, foreign.clone());
    state.agents.refresh("019f-test", 1, Ok(vec![foreign]));
    assert_eq!(state.agents.nodes.len(), 1);

    let mut command = HostCommandResult::document(PresentationDocument::default());
    command.session = Some((
        "other-session".into(),
        SessionMessagePage {
            messages: vec![],
            before_sequence: None,
            has_more: false,
        },
        None,
    ));
    apply_command_result(&mut state, command);
    assert!(state.agents.nodes.is_empty());
    // A reply from the previous visit to this session must also be ignored.
    state.session_id = "019f-test".into();
    handle_host_event(
        &mut state,
        HostEvent::Subagents {
            session_id: "019f-test".into(),
            revision: 0,
            session_epoch: 0,
            result: Ok(vec![job("late", SubagentStatus::Running)]),
        },
    );
    assert!(state.agents.nodes.is_empty());
}

#[test]
fn terminal_history_is_bounded_while_active_jobs_remain_visible() {
    let mut tree = AgentTree::default();
    tree.update(&job("active", SubagentStatus::Running));
    for i in 0..MAX_AGENTS + 20 {
        tree.update(&job(&format!("done-{i:03}"), SubagentStatus::Completed));
    }
    assert_eq!(tree.nodes.len(), MAX_AGENTS);
    assert!(tree.nodes.contains_key("active"));
    assert!(!tree.nodes.contains_key("done-000"));
}

#[test]
fn recent_completion_stays_visible_even_for_an_old_long_running_agent() {
    let mut state = TuiState::from_snapshot(snapshot());
    let mut finished = job("fresh", SubagentStatus::Completed);
    finished.created_at = "2026-10-22T18:00:00Z".into();
    finished.updated_at = "2026-10-23T19:00:00Z".into();
    state.agents.update(&finished);
    for i in 0..MAX_AGENTS {
        state
            .agents
            .update(&job(&format!("done-{i:03}"), SubagentStatus::Completed));
    }
    assert!(state.agents.nodes.contains_key("fresh"));
    let rendered = draw(&mut state, 120, 12, ScreenMode::Alternate);
    assert!(rendered.contains("├─ ✓ Inspect fresh"), "{rendered}");
    let fresh = rendered.find("Inspect fresh").expect("fresh completion");
    let older = rendered.find("Inspect done").expect("older completion");
    assert!(fresh < older, "{rendered}");
}

#[test]
fn narrow_terminal_hides_the_tree_and_restores_it_without_losing_the_draft() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("keep this draft");
    update(&mut state, job("code", SubagentStatus::Running));
    for mode in [ScreenMode::Alternate, ScreenMode::Inline] {
        for (width, expected_panel_width) in [
            (120, Some(36)),
            (100, Some(36)),
            (96, Some(32)),
            (92, Some(28)),
            (91, None),
            (80, None),
            (40, None),
            (120, Some(36)),
        ] {
            let height = if mode == ScreenMode::Inline {
                desired_inline_viewport_height(&state, width, 24, 0)
            } else {
                24
            };
            let rendered = draw(&mut state, width, height, mode);
            assert_eq!(
                rendered.contains("Agents · 1 active"),
                expected_panel_width.is_some(),
                "{width}: {rendered}"
            );
            assert_eq!(
                state.agents.area.map(|area| area.width),
                expected_panel_width
            );
            if expected_panel_width.is_some() {
                assert!(state.transcript_width >= usize::from(MIN_CHAT_WIDTH));
            }
            assert!(rendered.contains("keep this draft"), "{width}: {rendered}");
            assert_eq!(state.draft(), "keep this draft");
        }
    }
}

#[test]
fn tree_scrolling_does_not_move_the_transcript_or_composer() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("unchanged draft");
    for i in 0..30 {
        state
            .agents
            .update(&job(&format!("{i:02}"), SubagentStatus::Running));
    }
    draw(&mut state, 120, 12, ScreenMode::Alternate);
    let panel = state.agents.area.expect("panel");
    assert!(!crate::app::handle_mouse(
        &mut state,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panel.x + 2,
            row: panel.y + 3,
            modifiers: KeyModifiers::NONE,
        }
    ));
    assert_eq!(state.scroll_from_bottom, 0);
    assert_eq!(state.agents.scroll, MOUSE_SCROLL_LINES);
    assert!(
        state
            .agents
            .handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::ALT))
    );
    let rendered = draw(&mut state, 120, 12, ScreenMode::Alternate);
    assert!(rendered.contains("Inspect 12"), "{rendered}");
    assert!(rendered.contains("Alt+PgUp/PgDn"), "{rendered}");
    assert_eq!(state.draft(), "unchanged draft");
    assert_eq!(state.scroll_from_bottom, 0);
}

#[test]
fn failed_refresh_preserves_last_known_status_and_marks_it_stale() {
    let mut state = TuiState::from_snapshot(snapshot());
    update(&mut state, job("code", SubagentStatus::Running));
    state
        .agents
        .refresh("019f-test", 1, Err("private transport failure".into()));
    let rendered = draw(&mut state, 120, 24, ScreenMode::Alternate);
    assert!(rendered.contains("Agents · stale"), "{rendered}");
    assert!(!rendered.contains("private transport"));
    assert_eq!(state.agents.nodes["code"].status, SubagentStatus::Running);
    state.agents.refresh(
        "019f-test",
        1,
        Ok(vec![job("code", SubagentStatus::Completed)]),
    );
    assert_eq!(state.agents.nodes["code"].status, SubagentStatus::Completed);
    assert!(!state.agents.stale);
}

#[test]
fn theme_palettes_and_unicode_tasks_keep_every_status_readable() {
    use colossus_contracts::ThemeName;
    let mut state = TuiState::from_snapshot(snapshot());
    for (i, status) in [
        SubagentStatus::Queued,
        SubagentStatus::Running,
        SubagentStatus::Completed,
        SubagentStatus::Failed,
        SubagentStatus::Cancelled,
        SubagentStatus::Interrupted,
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = job(&i.to_string(), status);
        entry.task = "Inspect 界👩‍💻 e\u{301}\n\u{1b}[31munsafe".into();
        update(&mut state, entry);
    }
    for theme in [
        ThemeName::Default,
        ThemeName::Mono,
        ThemeName::HighContrast,
        ThemeName::Carrot,
        ThemeName::Hacker,
    ] {
        state.preferences.select_builtin_theme(theme);
        let rendered = draw(&mut state, 120, 24, ScreenMode::Alternate);
        for label in [
            "queued",
            "working",
            "done",
            "failed",
            "cancelled",
            "interrupted",
        ] {
            assert!(rendered.contains(label), "{label}: {rendered}");
        }
        assert!(
            state
                .agents
                .nodes
                .values()
                .all(|node| !node.label.contains(['\n', '\u{1b}']))
        );
    }
}
