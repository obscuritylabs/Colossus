use super::*;

fn prompt(id: &str) -> (HostEvent, oneshot::Receiver<PromptResponse>) {
    let (response, receiver) = oneshot::channel();
    (
        HostEvent::Prompt(InteractivePrompt {
            id: id.into(),
            kind: InteractivePromptKind::Approval,
            title: id.into(),
            document: PresentationDocument::new(),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
        receiver,
    )
}

#[test]
fn simultaneous_child_prompts_wait_for_each_own_decision() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("keep this draft");
    let (first, mut first_answer) = prompt("first child");
    let (second, mut second_answer) = prompt("second child");
    handle_host_event(&mut state, first);
    handle_host_event(&mut state, second);
    assert!(first_answer.try_recv().is_err());
    assert!(second_answer.try_recv().is_err());
    assert_eq!(state.pending_prompts.len(), 1);
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(first_answer.try_recv(), Ok(PromptResponse::Cancelled));
    crate::app::show_queued_prompt(&mut state);
    assert!(
        matches!(&state.overlay, Some(Overlay::Prompt { request, .. }) if request.id == "second child")
    );
    assert!(second_answer.try_recv().is_err());
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        second_answer.try_recv(),
        Ok(PromptResponse::Answer("Allow once".into()))
    );
    assert_eq!(state.draft(), "keep this draft");
}

#[test]
fn queued_prompts_are_bounded_and_terminal_failure_denies_all() {
    let mut state = TuiState::from_snapshot(snapshot());
    let mut responses = Vec::new();
    for index in 0..=MAX_QUEUED_TURNS {
        let (request, response) = prompt(&format!("child {index}"));
        handle_host_event(&mut state, request);
        responses.push(response);
    }
    let (overflow, mut denied) = prompt("overflow");
    handle_host_event(&mut state, overflow);
    assert_eq!(state.pending_prompts.len(), MAX_QUEUED_TURNS);
    assert_eq!(denied.try_recv(), Ok(PromptResponse::Cancelled));
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("disconnected".into()))),
    );
    assert!(state.pending_prompts.is_empty());
    for response in &mut responses {
        assert_eq!(response.try_recv(), Ok(PromptResponse::Cancelled));
    }
}
