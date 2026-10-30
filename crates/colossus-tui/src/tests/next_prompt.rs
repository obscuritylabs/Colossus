use super::*;

fn completed(output: &str) -> AgentRunResult {
    AgentRunResult {
        run_id: "suggestion-run".into(),
        session_id: Some("019f-test".into()),
        role: "primary".into(),
        profile: "test".into(),
        model_profile: "test".into(),
        provider_profile: "test".into(),
        model: "test".into(),
        plan: None,
        output: output.into(),
        event_count: 1,
        elapsed_seconds: 0.1,
    }
}

#[test]
fn next_prompt_tab_inserts_draft_without_submitting_and_escape_dismisses_source() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.next_prompt.completed(&completed(
        "Would you like me to review the cookie restore path?",
    ));
    state.composer.completion_hidden = false;
    assert_eq!(state.ghost_text(), Some("Review the cookie restore path"));
    assert!(state.accept_completion());
    assert_eq!(state.composer.draft, "Review the cookie restore path");
    assert!(state.operation.is_none());
    assert_eq!(state.history, snapshot().history);
    state.composer.clear();
    state.composer.completion_hidden = false;
    assert!(state.hide_completion());
    state.composer.insert("temporary");
    state.composer.clear();
    assert!(state.ghost_text().is_none());
}

#[test]
fn next_prompt_respects_mode_existing_draft_busy_and_generation_fences() {
    let mut state = TuiState::from_snapshot(snapshot());
    state
        .next_prompt
        .completed(&completed("## Next steps\n- Add cookie restoration tests."));
    state.composer.completion_hidden = false;
    state.preferences.next_prompt_mode = colossus_contracts::NextPromptMode::Off;
    assert!(state.ghost_text().is_none());
    state.preferences.next_prompt_mode = colossus_contracts::NextPromptMode::Model;
    state.composer.insert("my existing draft");
    assert!(state.suggested_next_prompt().is_none());
    state.composer.clear();
    state.operation = Some(OperationKind::Run);
    assert!(state.suggested_next_prompt().is_none());
    state.operation = None;
    crate::next_prompt::apply_next_prompt(
        &mut state,
        "another-session".into(),
        1,
        Some("Stale prediction".into()),
        Some(12),
    );
    assert_eq!(
        state.suggested_next_prompt(),
        Some("Add cookie restoration tests")
    );
    state.next_prompt.clear();
    crate::next_prompt::apply_next_prompt(
        &mut state,
        "019f-test".into(),
        1,
        Some("Stale prediction".into()),
        Some(12),
    );
    assert!(state.suggested_next_prompt().is_none());
}
