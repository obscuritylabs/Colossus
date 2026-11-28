use super::*;
use crate::tests::snapshot;
use ratatui::backend::TestBackend;

fn card(id: &str) -> ProviderModelInfo {
    ProviderModelInfo {
        id: id.into(),
        display_name: Some("Discovered model".into()),
        context_window_tokens: Some(65_536),
        max_output_tokens: Some(8_192),
        ..ProviderModelInfo::default()
    }
}

#[test]
fn asynchronous_catalog_results_preserve_the_selected_model_and_filter() {
    let mut browser = ModelCatalogState::new(1);
    browser.update(ModelCatalogUpdate::Overview(Ok(InteractiveModelCatalog {
        providers: vec!["first".into(), "second".into()],
        ..InteractiveModelCatalog::default()
    })));
    for group in browser.groups.values_mut() {
        group.expanded = true;
    }
    browser.update(ModelCatalogUpdate::Provider {
        profile: "second".into(),
        result: Ok(vec![card("wanted")]),
    });
    browser.selected = browser.rows.len() - 1;
    browser.paste("discovered");
    let selected = browser.rows[browser.selected].clone();
    assert!(matches!(selected, Row::Model { ref id, .. } if id == "wanted"));
    browser.update(ModelCatalogUpdate::Provider {
        profile: "first".into(),
        result: Ok(vec![card("also-matches")]),
    });
    assert!(browser.rows[browser.selected] == selected);
    browser.update(ModelCatalogUpdate::Provider {
        profile: "second".into(),
        result: Err("Refresh timed out".into()),
    });
    assert!(browser.rows[browser.selected] == selected);
    assert_eq!(
        browser.groups["second"].error.as_deref(),
        Some("Refresh timed out")
    );
}

#[test]
fn delayed_picker_results_require_the_same_session_and_generation() {
    let mut state = TuiState::from_snapshot(snapshot());
    let current = state.session_id.clone();
    state.model_catalog_generation = 2;
    state.overlay = Some(Overlay::ModelCatalog(Box::new(ModelCatalogState::new(2))));
    let rejected = || ModelCatalogUpdate::Overview(Err("old error".into()));
    updated(&mut state, "another session".into(), 2, rejected());
    updated(&mut state, current.clone(), 1, rejected());
    let Some(Overlay::ModelCatalog(browser)) = &state.overlay else {
        panic!("picker stays open");
    };
    assert!(browser.error.is_none());
    updated(
        &mut state,
        current,
        2,
        ModelCatalogUpdate::Applied(Ok(ModelCatalogApplied {
            role: "primary".into(),
            route: "catalog-new via first".into(),
            input_budget_tokens: 32_000,
            revision: 2,
        })),
    );
    assert!(state.overlay.is_none());
    state.replace_footer(FooterState {
        route: "old route".into(),
        model_revision: 1,
        context: Some((200, 4_000)),
        status: "ok".into(),
        ..FooterState::default()
    });
    assert_eq!(state.footer.route, "catalog-new via first");
    assert_eq!(state.footer.context, Some((200, 32_000)));
    assert_eq!(state.footer.status, "ok");
}

#[test]
fn a_failed_confirmation_with_the_picker_gone_pauses_waiting_turns() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.model_catalog_generation = 2;
    state.model_catalog_applying = true;
    state.queue.push_back("Use the chosen model".into());
    let session_id = state.session_id.clone();
    updated(
        &mut state,
        session_id,
        2,
        ModelCatalogUpdate::Applied(Err("Worker disconnected".into())),
    );
    assert!(!state.model_catalog_applying);
    assert!(state.queue_paused);
    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.transcript.last().unwrap().kind, TranscriptKind::Error);
}

#[test]
fn discovered_model_editor_renders_both_limits_in_a_compact_terminal() {
    let mut state = TuiState::from_snapshot(snapshot());
    let mut browser = ModelCatalogState::new(1);
    let mut editor = ModelEditor::new("first".into(), &card("first/model"));
    editor.output = "2048".into();
    let ModelCatalogSelection::Discovered(profile) = editor.selection().unwrap() else {
        panic!("expected a discovered model");
    };
    assert_eq!(profile.model, "first/model");
    assert_eq!(profile.context_window_tokens, 65_536);
    assert_eq!(profile.max_output_tokens, 2_048);
    browser.editor = Some(editor);
    state.overlay = Some(Overlay::ModelCatalog(Box::new(browser)));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, frame.area()))
        .unwrap();
    let screen = terminal.backend().to_string();
    for visible in ["Configure model", "65536", "2048", "Use in conversation"] {
        assert!(screen.contains(visible), "{screen}");
    }
}
