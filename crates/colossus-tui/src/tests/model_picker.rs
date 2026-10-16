use super::*;

fn picker() -> (TuiState, oneshot::Receiver<PromptResponse>) {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.set("Preserve this draft".into());
    let (response, receiver) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "model-picker".into(),
            kind: InteractivePromptKind::ModelSelection,
            title: "Choose model".into(),
            document: PresentationDocument::new(),
            choices: vec![
                "Role default".into(),
                "fast · model-one".into(),
                "deep · model-two".into(),
            ],
            initial_choice: Some(1),
            allow_free_form: false,
            response,
        }),
    );
    (state, receiver)
}

#[test]
fn model_picker_search_applies_exact_choice_and_keeps_conversation() {
    let (mut state, mut receiver) = picker();
    let original_session = state.session_id.clone();
    for character in "deep".chars() {
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    for (width, height) in [(120, 32), (52, 14)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .unwrap();
        let display = terminal.backend().to_string();
        assert!(display.contains("Choose model"), "{display}");
        assert!(display.contains("deep · model-two"), "{display}");
        assert!(!display.contains("fast · model-one"), "{display}");
    }
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        receiver.try_recv().unwrap(),
        PromptResponse::Answer("deep · model-two".into())
    );
    assert_eq!(state.session_id, original_session);
    assert_eq!(state.composer.draft, "Preserve this draft");
}

#[test]
fn pasted_search_reconciles_the_visible_choice_and_never_applies_an_unmatched_model() {
    let (mut state, mut receiver) = picker();
    insert_active_text(&mut state, "deep");
    let mut terminal = Terminal::new(TestBackend::new(72, 24)).unwrap();
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .unwrap();
    let display = terminal.backend().to_string();
    assert!(display.contains("› deep · model-two"), "{display}");
    assert!(!display.contains("fast · model-one"), "{display}");
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        receiver.try_recv().unwrap(),
        PromptResponse::Answer("deep · model-two".into())
    );

    let (mut state, mut receiver) = picker();
    insert_active_text(&mut state, &"é".repeat(1000));
    if let Some(Overlay::Prompt {
        input, selected, ..
    }) = &state.overlay
    {
        assert_eq!(input.len(), 128);
        assert_eq!(*selected, None);
    } else {
        panic!("picker should remain open");
    }
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn model_picker_escape_and_no_matches_do_not_apply_a_choice() {
    let (mut state, mut receiver) = picker();
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(receiver.try_recv().is_err());
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.overlay.is_some(), "first Escape clears search");
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(receiver.try_recv().unwrap(), PromptResponse::Cancelled);
    assert_eq!(state.composer.draft, "Preserve this draft");
}
