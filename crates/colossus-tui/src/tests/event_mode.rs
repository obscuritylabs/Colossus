use super::*;

fn state() -> TuiState {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    state.welcome_visible = false;
    state.security_posture_entry = None;
    state.transcript_width = 80;
    state.transcript_height = 5;
    state
}

fn file_result(state: &mut TuiState, output: String) {
    let source = TranscriptRenderSource::live(
        RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                images: vec![],
                call_id: "inline-file".into(),
                name: "filesystem.read".into(),
                output,
                exit_code: 0,
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.3,
        },
        Some(ToolCall {
            call_id: "inline-file".into(),
            name: "filesystem.read".into(),
            arguments: serde_json::json!({"path": "src/inline.rs"}),
        }),
    );
    state.append_entry_with_source(
        TranscriptEntry {
            sequence: Some(1),
            kind: TranscriptKind::Tool,
            document: source.render(&state.preferences).unwrap_or_default(),
            temporary: false,
        },
        Some(source),
    );
}

fn text_entry(state: &mut TuiState, text: String) {
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Assistant,
        document: PresentationDocument::from_block(PresentationBlock::Text(text)),
        temporary: false,
    });
}

fn rows(state: &TuiState) -> Vec<String> {
    transcript_lines(state, state.transcript_width)
        .into_iter()
        .map(|line| line.to_string())
        .collect()
}

fn top_row(state: &TuiState) -> String {
    let rows = rows(state);
    let top = rows
        .len()
        .saturating_sub(state.transcript_height)
        .saturating_sub(state.scroll_from_bottom);
    rows[top].clone()
}

fn scroll_to(state: &mut TuiState, marker: &str) {
    let rows = rows(state);
    let top = rows
        .iter()
        .position(|row| row.contains(marker))
        .expect("anchor marker");
    state.scroll_from_bottom = rows.len() - state.transcript_height - top;
    assert!(state.scroll_from_bottom > 0, "fixture must be scrolled");
}

#[test]
fn event_toggle_expands_inline_and_preserves_session_work_and_draft() {
    let mut state = state();
    file_result(&mut state, "INLINE_CONTENT_MARKER".into());
    state.composer.insert("draft survives");
    state.composer.completion_index = Some(1);
    state.queue.push_back("queued turn".into());
    state.operation = Some(OperationKind::Run);
    state.activity = Some("active operation".into());
    state.new_items = 3;
    state.tool_hits.push(crate::tool_details::ToolHit::new(
        Rect::new(0, 0, 80, 1),
        0,
        &state,
    ));
    state.tool_click = Some((state.tool_hits[0].clone(), Position::new(0, 0)));
    state
        .text_selection
        .remember(&Buffer::empty(Rect::new(0, 0, 80, 5)));
    state.text_selection.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert!(state.text_selection.active());
    let session = state.session_id.clone();
    let collapsed = rows(&state).join("\n");
    assert!(!collapsed.contains("INLINE_CONTENT_MARKER"));

    state.toggle_event_details();
    assert_eq!(state.preferences.events_mode, EventDisplayMode::Verbose);
    let expanded = rows(&state).join("\n");
    assert!(expanded.contains("INLINE_CONTENT_MARKER"), "{expanded}");
    assert!(expanded.contains("src/inline.rs"), "{expanded}");
    assert!(expanded.contains("Duration"), "{expanded}");
    assert!(state.overlay.is_none(), "expansion stays in the transcript");
    assert_eq!(state.draft(), "draft survives");
    assert_eq!(state.session_id, session);
    assert_eq!(state.queue.front().map(String::as_str), Some("queued turn"));
    assert_eq!(state.operation, Some(OperationKind::Run));
    assert_eq!(state.activity.as_deref(), Some("active operation"));
    assert_eq!(state.new_items, 3);
    assert_eq!(state.scroll_from_bottom, 0, "live view follows the bottom");
    assert!(state.tool_hits.is_empty());
    assert!(state.tool_click.is_none());
    assert!(!state.text_selection.active());
    assert!(state.composer.completion_index.is_none());
    assert!(state.composer.completion_hidden);

    state.toggle_event_details();
    assert_eq!(state.preferences.events_mode, EventDisplayMode::Compact);
    assert_eq!(rows(&state).join("\n"), collapsed);
    assert_eq!(state.draft(), "draft survives");
}

#[test]
fn event_toggle_keeps_a_scrolled_entry_and_its_intra_entry_line() {
    let mut state = state();
    file_result(&mut state, "expanded content\n".repeat(80));
    text_entry(&mut state, "ANCHOR_0\nANCHOR_1\nANCHOR_2\nANCHOR_3".into());
    for index in 0..20 {
        text_entry(&mut state, format!("TAIL_{index:02}"));
    }
    scroll_to(&mut state, "ANCHOR_2");
    let before = top_row(&state);
    let old_scroll = state.scroll_from_bottom;
    let old_line_count = rows(&state).len();
    state.toggle_event_details();
    assert_eq!(top_row(&state), before);
    assert!(rows(&state).len() > old_line_count);
    // Expansion above the anchor moves its line index with the transcript height;
    // the unchanged content below it keeps the same distance from live output.
    assert_eq!(state.scroll_from_bottom, old_scroll);
    state.toggle_event_details();
    assert_eq!(top_row(&state), before);
    assert_eq!(state.scroll_from_bottom, old_scroll);
}

#[test]
fn collapsing_an_anchor_inside_tool_details_keeps_the_same_tool_entry() {
    let mut state = state();
    file_result(
        &mut state,
        (0..80)
            .map(|index| format!("SOURCE_LINE_{index:04}\n"))
            .collect(),
    );
    for index in 0..20 {
        text_entry(&mut state, format!("TAIL_{index:02}"));
    }
    state.toggle_event_details();
    scroll_to(&mut state, "SOURCE_LINE_0010");
    state.toggle_event_details();
    let top = top_row(&state);
    assert!(top.contains("Read file"), "{top}");
    assert!(!top.contains("TAIL_"), "{top}");
}

#[test]
fn inline_verbose_code_uses_snippets_while_click_details_retain_middle_lines() {
    let mut state = state();
    file_result(
        &mut state,
        (0..80)
            .map(|index| format!("SOURCE_LINE_{index:04}\n"))
            .collect(),
    );
    state.toggle_event_details();
    let inline = rows(&state).join("\n");
    assert!(inline.contains("SOURCE_LINE_0000"), "{inline}");
    assert!(inline.contains("SOURCE_LINE_0079"), "{inline}");
    assert!(!inline.contains("SOURCE_LINE_0040"), "{inline}");
    assert!(inline.contains("lines omitted"), "{inline}");
    assert!(
        rows(&state)
            .iter()
            .all(|row| UnicodeWidthStr::width(row.as_str()) <= 80)
    );
    let document = state.transcript_sources[0]
        .as_ref()
        .expect("source")
        .render(&state.preferences)
        .expect("details");
    let detail = crate::render::styled_document_lines(&document, &state.preferences, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(detail.contains("SOURCE_LINE_0040"), "{detail}");
}

#[test]
fn event_toggle_reveals_previously_hidden_events_without_a_modal() {
    let mut state = state();
    state.preferences.events_mode = EventDisplayMode::Off;
    file_result(&mut state, "OFF_TO_VERBOSE".into());
    assert!(rows(&state).is_empty());
    state.toggle_event_details();
    assert_eq!(state.preferences.events_mode, EventDisplayMode::Verbose);
    assert!(rows(&state).join("\n").contains("OFF_TO_VERBOSE"));
    assert!(state.overlay.is_none());
}
