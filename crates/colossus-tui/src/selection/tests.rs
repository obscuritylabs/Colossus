use super::*;
use crate::app::handle_text_selection_key;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn select(selection: &mut TextSelection, start: (u16, u16), end: (u16, u16)) {
    assert!(selection.handle_mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        start.0,
        start.1
    )));
    assert!(selection.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), end.0, end.1)));
    assert!(selection.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), end.0, end.1)));
}

#[test]
fn drag_highlights_exact_cells_and_copies_in_both_directions() {
    let original = Buffer::with_lines(["first line", "second line"]);
    for (start, end) in [((2, 0), (5, 1)), ((5, 1), (2, 0))] {
        let mut selection = TextSelection::default();
        selection.remember(&original);
        select(&mut selection, start, end);
        let mut live = Buffer::with_lines(["new output!", "changed row"]);
        assert!(selection.render(&mut live));
        assert_eq!(live.cell((0, 0)).unwrap().symbol(), "f");
        assert!(
            !live
                .cell((1, 0))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            live.cell((2, 0))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            live.cell((5, 1))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !live
                .cell((6, 1))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
        selection.request_copy();
        assert_eq!(selection.take_copy().as_deref(), Some("rst line\nsecond"));
        assert!(selection.active());
        assert!(selection.take_copy().is_none());
    }
}

#[test]
fn selection_preserves_unicode_graphemes_and_stays_inside_its_pane() {
    let mut selection = TextSelection::default();
    let buffer = Buffer::with_lines(["界e\u{301} 👩‍💻   │ Agent", "next     │ Agent"]);
    selection.regions.push(Rect::new(0, 0, 8, 2));
    selection.remember(&buffer);
    // Starting in the second half of a wide glyph includes it exactly once.
    select(&mut selection, (1, 0), (15, 1));
    assert_eq!(selection.text().as_deref(), Some("界e\u{301} 👩‍💻\nnext"));
    assert!(!selection.text().unwrap().contains("Agent"));
}

#[test]
fn click_scroll_and_resize_clear_selection_without_changing_the_draft() {
    let mut selection = TextSelection::default();
    selection.remember(&Buffer::with_lines(["select this"]));
    select(&mut selection, (2, 0), (2, 0));
    assert!(!selection.active());
    select(&mut selection, (0, 0), (5, 0));
    assert!(!selection.handle_mouse(mouse(MouseEventKind::ScrollUp, 2, 0)));
    assert!(!selection.active());
    select(&mut selection, (0, 0), (5, 0));
    assert!(!selection.render(&mut Buffer::empty(Rect::new(0, 0, 5, 2))));
    assert!(!selection.active());
}

#[test]
fn copy_has_priority_over_interrupt_and_dismissal_cannot_answer_an_unseen_approval() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    state.composer.insert("keep draft");
    state.control = Some(RunControl::default());
    state
        .text_selection
        .remember(&Buffer::with_lines(["copy me"]));
    select(&mut state.text_selection, (0, 0), (6, 0));
    assert!(handle_text_selection_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
    assert_eq!(state.text_selection.take_copy().as_deref(), Some("copy me"));
    assert_eq!(state.draft(), "keep draft");
    assert!(!state.control.as_ref().unwrap().is_cancelled());

    let (response, mut received) = oneshot::channel();
    state.overlay = Some(Overlay::Prompt {
        request: InteractivePrompt {
            id: "new-approval".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval".into(),
            document: PresentationDocument::default(),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: Some(0),
            allow_free_form: false,
            response,
        },
        selected: Some(0),
        input: String::new(),
        approval_section: ApprovalSection::Summary,
        document_scroll: 0,
    });
    assert!(handle_text_selection_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    ));
    assert!(!state.text_selection.active());
    assert!(received.try_recv().is_err());
    assert!(state.overlay.is_some());
}
