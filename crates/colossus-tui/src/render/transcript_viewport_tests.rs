use super::*;
use ratatui::backend::TestBackend;

const VIEWPORT_HEIGHT: u16 = 12;

fn draw(terminal: &mut Terminal<TestBackend>, state: &mut TuiState, width: u16) -> String {
    terminal
        .draw(|frame| {
            state.tool_hits.clear();
            render_transcript(frame, state, Rect::new(0, 0, width, VIEWPORT_HEIGHT), 0, 24);
        })
        .expect("draw transcript");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(Cell::symbol)
        .collect()
}

#[test]
fn viewport_reuses_wrapped_rows_during_edits_and_updates_on_scroll_and_streaming() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    for index in 0..40 {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::User,
            document: PresentationDocument::from_block(PresentationBlock::Text(format!(
                "synthetic history row {index}"
            ))),
            temporary: false,
        });
    }
    update_streaming_assistant(&mut state, "streaming prefix");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    let live = draw(&mut terminal, &mut state, 80);
    assert!(live.contains("streaming prefix"));
    assert!(!live.contains("durable row marker"));
    let layout = state.transcript_layout.as_ref().expect("cached viewport");
    assert_eq!(layout.rows.len(), usize::from(VIEWPORT_HEIGHT));
    let cached_rows = layout.rows.as_ptr();

    state.composer.insert("edited draft");
    assert_eq!(draw(&mut terminal, &mut state, 80), live);
    assert_eq!(
        state.transcript_layout.as_ref().unwrap().rows.as_ptr(),
        cached_rows
    );

    state.scroll_up_lines(usize::MAX);
    assert!(draw(&mut terminal, &mut state, 80).contains("durable row marker"));
    state.end();
    assert_eq!(draw(&mut terminal, &mut state, 80), live);

    // A streaming edit changes an existing transcript entry in place.
    update_streaming_assistant(&mut state, " with new content");
    assert!(state.transcript_layout.is_none());
    assert!(draw(&mut terminal, &mut state, 80).contains("prefix with new content"));
    draw(&mut terminal, &mut state, 40);
    assert_eq!(state.transcript_layout.as_ref().unwrap().viewport.width, 40);
}

fn append_tool_result(state: &mut TuiState) {
    state.append_entry_with_source(
        TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Tool,
            document: PresentationDocument::from_block(PresentationBlock::Text("tool row".into())),
            temporary: false,
        },
        Some(TranscriptRenderSource::RetainedToolResult {
            name: None,
            output: "tool output".into(),
        }),
    );
}

#[test]
fn cached_tool_placements_are_bounded_by_the_viewport() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    for _ in 0..512 {
        append_tool_result(&mut state);
    }
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    draw(&mut terminal, &mut state, 80);
    let placements = &state.transcript_layout.as_ref().unwrap().tool_placements;
    assert!(!placements.is_empty());
    assert!(placements.len() <= usize::from(VIEWPORT_HEIGHT));
    assert!(placements.capacity() <= 4 * usize::from(VIEWPORT_HEIGHT));
}

#[test]
fn cached_viewport_updates_tool_positions_when_image_preview_changes() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    let digest = format!("{:064x}", 1);
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::User,
        document: PresentationDocument::from_block(PresentationBlock::Image(ModelImageReference {
            artifact_id: format!("artifact-{digest}"),
            file_name: "preview.png".into(),
            media_type: "image/png".into(),
            size_bytes: 128,
            sha256: digest.clone(),
            width_pixels: 16,
            height_pixels: 16,
            detail: colossus_contracts::ModelImageDetail::Auto,
        })),
        temporary: false,
    });
    append_tool_result(&mut state);
    let size = Size::new(12, 2);
    state.preview_transcript_size = size;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    draw(&mut terminal, &mut state, 80);
    let before = state.tool_hits[0].area.y;

    state
        .preview_cache
        .insert(digest.clone(), image::DynamicImage::new_rgba8(16, 16));
    state.preview_cache.prepare(&digest, size);
    draw(&mut terminal, &mut state, 80);
    assert_eq!(state.tool_hits[0].area.y, before + size.height);

    state.preview_cache.set_picker(Picker::halfblocks());
    draw(&mut terminal, &mut state, 80);
    assert_eq!(state.tool_hits[0].area.y, before);
}
