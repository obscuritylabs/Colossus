use super::*;
use colossus_contracts::CommandApprovalContext;
use colossus_presentation::command_approval_document;

#[test]
fn command_tail_remains_reachable_beyond_u16_wrapped_lines_and_after_resize() {
    // Four accepted 64-KiB arguments can expand this far after replacing a
    // one-character known credential. The display remains below its 4-MiB bound.
    let context = CommandApprovalContext {
        justification: "Check the requested build output.".into(),
        executable: "/bin/echo".into(),
        arguments: vec!["[REDACTED]".repeat(65_536); 4]
            .into_iter()
            .chain(["ENDTAIL".into()])
            .collect(),
        working_directory: "/work".into(),
        redacted: true,
    };
    context.validate().unwrap();
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "large-command".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval required".into(),
            document: command_approval_document(&context, None, None, true).unwrap(),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    let Some(Overlay::Prompt {
        document_scroll, ..
    }) = state.overlay.as_mut()
    else {
        panic!("expected pending approval");
    };
    *document_scroll = usize::from(u16::MAX);
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    let Some(Overlay::Prompt {
        document_scroll, ..
    }) = state.overlay.as_mut()
    else {
        panic!("expected pending approval");
    };
    assert!(*document_scroll > usize::from(u16::MAX));
    // A requested offset beyond the document clamps to its final viewport.
    *document_scroll = usize::MAX;
    let mut terminal = Terminal::new(TestBackend::new(40, 32)).unwrap();
    for width in [40, 60] {
        terminal.backend_mut().resize(width, 32);
        terminal.resize(Rect::new(0, 0, width, 32)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .unwrap();
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("ENDTAIL"),
            "tail unreachable at width {width}: {rendered}"
        );
        assert!(
            received.try_recv().is_err(),
            "inspection must not submit approval"
        );
    }
}
