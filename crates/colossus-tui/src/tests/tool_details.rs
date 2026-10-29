use super::*;

fn open_latest(state: &mut TuiState) {
    let index = (0..state.transcript_sources.len())
        .rev()
        .find(|&index| {
            state.transcript_sources[index]
                .as_ref()
                .is_some_and(TranscriptRenderSource::inspectable)
        })
        .expect("inspectable event");
    crate::tool_details::open(state, index);
}

fn state_with_result(output: String) -> TuiState {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    result(&mut state, "read-1", "filesystem.read", output);
    state
}

fn result(state: &mut TuiState, id: &str, name: &str, output: String) {
    let envelope = |event| RunEventEnvelope {
        schema_version: 1,
        run_id: "run-details".into(),
        session_id: "019f-test".into(),
        event,
    };
    handle_run_event(
        state,
        envelope(RunEvent::ToolStarted {
            turn: 1,
            call: ToolCall {
                call_id: id.into(),
                name: name.into(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
            },
            elapsed_seconds: 0.1,
        }),
    );
    handle_run_event(
        state,
        envelope(RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                images: vec![],
                call_id: id.into(),
                name: name.into(),
                output,
                exit_code: 0,
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.3,
        }),
    );
}

fn draw(state: &mut TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, state, 0, ScreenMode::Alternate))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(Cell::symbol).collect()
}

fn mouse(state: &mut TuiState, kind: MouseEventKind, x: u16, y: u16) {
    handle_mouse(
        state,
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        },
    );
}

#[test]
fn large_results_are_one_row_until_inspected_and_raw_source_is_retained() {
    let payload = format!(
        "HEAD-DETAIL\n{}TAIL-NOT-IN-PREVIEW",
        "long file line\n".repeat(150_000)
    );
    let length = payload.len();
    let mut state = state_with_result(payload);
    state.composer.insert("draft survives inspection");
    for width in [40, 80, 160] {
        let lines = transcript_lines(&state, width);
        assert_eq!(lines.len(), 1, "collapsed events occupy one physical row");
        assert!(!lines[0].to_string().contains("HEAD-DETAIL"));
        assert!(UnicodeWidthStr::width(lines[0].to_string().as_str()) <= width);
    }
    let before = draw(&mut state, 120, 32);
    assert!(!text(&before).contains("HEAD-DETAIL"));
    open_latest(&mut state);
    assert!(state.transient_inline_screen_active());
    let details = text(&draw(&mut state, 120, 32));
    assert!(details.contains("HEAD-DETAIL"), "{details}");
    assert!(!details.contains("TAIL-NOT-IN-PREVIEW"));
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let tail = text(&draw(&mut state, 80, 24));
    assert!(tail.to_lowercase().contains("truncat"), "{tail}");
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.draft(), "draft survives inspection");
    assert!(state.overlay.is_none());
    let Some(TranscriptRenderSource::RunEvent { event, .. }) = &state.transcript_sources[0] else {
        panic!("source")
    };
    let RunEvent::ToolCompleted { result, .. } = event.as_ref() else {
        panic!("result")
    };
    assert_eq!(result.output.len(), length);
    assert!(result.output.ends_with("TAIL-NOT-IN-PREVIEW"));
}

#[test]
fn a_stationary_click_opens_details_but_dragging_selects_text() {
    let mut state = state_with_result("CLICK-DETAIL".into());
    draw(&mut state, 100, 24);
    let area = state.tool_hits[0].area;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x + 2,
        area.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        area.x + 8,
        area.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        area.x + 8,
        area.y,
    );
    assert!(state.overlay.is_none());
    assert!(state.text_selection.active());
    state.text_selection.clear();
    draw(&mut state, 100, 24);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x + 2,
        area.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        area.x + 2,
        area.y,
    );
    assert!(matches!(state.overlay, Some(Overlay::ToolDetails(_))));
    assert!(!state.text_selection.active());
    assert!(text(&draw(&mut state, 100, 24)).contains("CLICK-DETAIL"));
}

#[test]
fn stale_clicks_and_hidden_rows_do_not_open_details() {
    let mut state = state_with_result("FIRST".into());
    draw(&mut state, 100, 24);
    let area = state.tool_hits[0].area;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y,
    );
    result(&mut state, "second", "repo.map", "{}".into());
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        area.x,
        area.y,
    );
    assert!(
        state.overlay.is_none(),
        "new transcript invalidates a pending click"
    );
    state.overlay = Some(Overlay::QueuePaused);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        area.x,
        area.y,
    );
    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
}

#[test]
fn inspector_navigation_and_approval_preemption_preserve_the_draft() {
    let mut state = state_with_result("FIRST-DETAIL".into());
    result(
        &mut state,
        "second",
        "filesystem.read",
        "SECOND-DETAIL".into(),
    );
    state.composer.insert("keep draft");
    open_latest(&mut state);
    assert!(text(&draw(&mut state, 80, 24)).contains("SECOND-DETAIL"));
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert!(text(&draw(&mut state, 80, 24)).contains("FIRST-DETAIL"));
    insert_active_text(&mut state, "must not paste");
    let (response, _receiver) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            kind: InteractivePromptKind::Approval,
            title: "Approve?".into(),
            document: PresentationDocument::new(),
            choices: vec!["Approve".into(), "Deny".into()],
            id: "inspect-approval".into(),
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    assert!(matches!(state.overlay, Some(Overlay::Prompt { .. })));
    assert!(state.pending_prompts.is_empty());
    assert_eq!(state.draft(), "keep draft");
}

#[test]
fn transcript_has_a_blank_line_above_the_composer_in_compact_and_roomy_terminals() {
    for (width, height) in [(40, 12), (80, 24), (160, 40)] {
        let mut state = state_with_result("detail".into());
        let buffer = draw(&mut state, width, height);
        // The composer occupies three rows; the footer one. Its preceding row is clear.
        let gap_y = height - 5;
        let content_width = width - state.sidebar_width(width);
        assert!(
            (0..content_width).all(|x| buffer[(x, gap_y)].symbol() == " "),
            "{width}x{height}"
        );
        assert!(!state.tool_hits.is_empty());
    }
}

#[test]
fn many_large_results_keep_redraw_work_bounded_by_summary_rows() {
    let mut state = state_with_result("x".repeat(256 * 1024));
    for index in 1..100 {
        result(
            &mut state,
            &format!("read-{index}"),
            "filesystem.read",
            "x".repeat(256 * 1024),
        );
    }
    let started = Instant::now();
    for _ in 0..20 {
        let lines = transcript_lines(&state, 120);
        assert_eq!(
            lines.len(),
            199,
            "one row per tool plus comfortable spacing"
        );
        assert!(lines.iter().all(|line| line.width() <= 120));
    }
    eprintln!(
        "100 retained 256KiB tool outputs, 20 compact redraws: {:?}",
        started.elapsed()
    );
    assert!(state.overlay.is_none(), "details remain lazy");
}

#[test]
fn child_agent_status_uses_the_shared_phase_not_only_the_tool_exit_code() {
    for (status, marker, failure) in [
        ("failed", "!", true),
        ("running", "◌", false),
        ("cancelled", "×", false),
    ] {
        let mut state = state_with_result("file".into());
        result(
            &mut state,
            "agent-result",
            "agent.result",
            serde_json::json!({"status":status}).to_string(),
        );
        let source = state.transcript_sources.last().unwrap().as_ref().unwrap();
        assert_eq!(source.status_marker(), marker);
        assert_eq!(
            state.transcript.last().unwrap().kind == TranscriptKind::Error,
            failure
        );
    }
}

#[test]
fn asynchronous_pickers_preempt_read_only_inspection() {
    for theme in [false, true] {
        let mut state = state_with_result("detail".into());
        state.composer.insert("keep this draft");
        open_latest(&mut state);
        let (response, mut receiver) = oneshot::channel();
        let event = if theme {
            HostEvent::ThemePicker(theme_picker(response))
        } else {
            HostEvent::SessionBrowser(session_browser(response))
        };
        handle_host_event(&mut state, event);
        assert!(!matches!(
            state.overlay,
            None | Some(Overlay::ToolDetails(_))
        ));
        assert!(
            matches!(
                receiver.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "requested picker remains live until operator chooses"
        );
        assert_eq!(state.draft(), "keep this draft");
    }
}

#[test]
fn resizing_between_mouse_down_and_up_invalidates_the_old_row() {
    let mut state = state_with_result("detail".into());
    draw(&mut state, 100, 24);
    let area = state.tool_hits[0].area;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        area.x,
        area.y,
    );
    draw(&mut state, 60, 12);
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        area.x,
        area.y,
    );
    assert!(state.overlay.is_none());
}

#[test]
fn expanded_file_details_can_scroll_to_middle_lines() {
    let body = (1..=180)
        .map(|line| format!("DETAIL-LINE-{line:03}\n"))
        .collect::<String>();
    let mut state = state_with_result(body);
    open_latest(&mut state);
    assert!(text(&draw(&mut state, 80, 24)).contains("[ Close ]"));
    for _ in 0..2 {
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        );
    }
    let middle = text(&draw(&mut state, 80, 24));
    assert!(middle.contains("DETAIL-LINE-040"), "{middle}");
    assert!(!middle.contains("lines omitted"));
}

#[test]
fn inspector_keeps_repository_metadata_and_readable_rows_out_of_the_compact_stream() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    result(
        &mut state,
        "map",
        "repo.map",
        serde_json::json!({
            "root": ".", "file_count": 2, "truncated": false,
            "extension_counts": {"rs": 2},
            "files": ["src/main.rs", "src/lib.rs"]
        })
        .to_string(),
    );
    assert_eq!(transcript_lines(&state, 120).len(), 1);
    assert!(!text(&draw(&mut state, 120, 40)).contains("src/lib.rs"));
    open_latest(&mut state);
    let visible = text(&draw(&mut state, 120, 40));
    for expected in [
        "Map repository",
        "Status",
        "Duration",
        "0.20s",
        "Exit",
        "Root",
        "File count",
        "Extension counts",
        "src/main.rs",
        "src/lib.rs",
    ] {
        assert!(visible.contains(expected), "missing {expected}: {visible}");
    }
    assert!(!visible.contains("json preview"), "{visible}");
    assert!(!visible.contains("\"file_count\""), "{visible}");
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.overlay.is_none());
    assert_eq!(transcript_lines(&state, 120).len(), 1);
}
