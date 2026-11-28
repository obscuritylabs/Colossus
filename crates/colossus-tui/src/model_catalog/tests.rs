use super::*;
use crate::tests::snapshot;
use colossus_contracts::{ModelCapabilities, ModelFeatureMode, ModelLimits, ModelRoute};
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

fn editable_card() -> TerminalModelCard {
    let limits = ModelLimits {
        context_window_tokens: 65_536,
        max_output_tokens: 8_192,
        safety_margin_tokens: 6_554,
        input_budget_tokens: 50_790,
    };
    TerminalModelCard {
        target: TerminalModelTarget::Discovered {
            provider_profile: "first".into(),
            model: "first/model".into(),
        },
        binding: "1".repeat(64),
        route: ModelRoute {
            role: "primary".into(),
            profile: "first-model".into(),
            model_profile: "first-model".into(),
            provider_profile: "first".into(),
            provider: "open_ai_compatible".into(),
            model: "first/model".into(),
            limits,
            capabilities: ModelCapabilities {
                tool_calls: true,
                streaming: true,
                image_inputs: true,
            },
            reasoning_effort: None,
        },
        default_limits: limits,
        default_features: Default::default(),
        default_reasoning_effort: None,
        advertised: Some(card("first/model")),
        overrides: Default::default(),
        revision: 7,
    }
}

fn editor_key(editor: &mut ModelEditor, code: KeyCode) -> EditorAction {
    editor.key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap()
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
    state.model_catalog_control = Some(RunControl::default());
    state.queue.push_back("Use the chosen model".into());
    let session_id = state.session_id.clone();
    updated(
        &mut state,
        session_id,
        2,
        ModelCatalogUpdate::Applied(Err("Worker disconnected".into())),
    );
    assert!(state.model_catalog_control.is_none());
    assert!(state.queue_paused);
    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.transcript.last().unwrap().kind, TranscriptKind::Error);
}

#[test]
fn full_editor_saves_limits_and_capabilities_and_scrolls_in_a_compact_terminal() {
    let mut state = TuiState::from_snapshot(snapshot());
    let mut browser = ModelCatalogState::new(1);
    let mut editor = ModelEditor::new(editable_card());
    assert!(editor.paste("65k").is_err());
    editor_key(&mut editor, KeyCode::Tab);
    editor.paste("48,000").unwrap();
    editor_key(&mut editor, KeyCode::Tab);
    editor.paste("2048").unwrap();
    editor_key(&mut editor, KeyCode::Tab);
    editor_key(&mut editor, KeyCode::Tab);
    editor_key(&mut editor, KeyCode::Left);
    editor_key(&mut editor, KeyCode::Tab);
    editor_key(&mut editor, KeyCode::Right);
    editor_key(&mut editor, KeyCode::Right);
    let EditorAction::Save(update) = editor_key(&mut editor, KeyCode::Enter) else {
        panic!("expected an editable model update");
    };
    assert_eq!(update.target, editable_card().target);
    assert_eq!(update.expected_revision, 7);
    assert_eq!(update.expected_binding, editable_card().binding);
    assert_eq!(update.overrides.context_window_tokens, None);
    assert_eq!(update.overrides.input_budget_tokens, Some(48_000));
    assert_eq!(update.overrides.max_output_tokens, Some(2_048));
    assert_eq!(update.overrides.streaming, Some(ModelFeatureMode::Off));
    assert_eq!(update.overrides.image_inputs, Some(ModelFeatureMode::On));
    editor_key(&mut editor, KeyCode::Tab);
    editor_key(&mut editor, KeyCode::Tab);
    browser.editor = Some(editor);
    state.overlay = Some(Overlay::ModelCatalog(Box::new(browser)));
    let mut terminal = ratatui::Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| render(frame, &state, frame.area()))
        .unwrap();
    let screen = terminal.backend().to_string();
    for visible in [
        "Configure model",
        "Image inputs",
        "Server compaction",
        "Reasoning effort",
        "Save and use",
    ] {
        assert!(screen.contains(visible), "{screen}");
    }
}

#[test]
fn saved_discoveries_can_be_selected_before_the_first_provider_request() {
    let mut browser = ModelCatalogState::new(1);
    let saved = editable_card();
    let target = saved.target.clone();
    browser.update(ModelCatalogUpdate::Overview(Ok(InteractiveModelCatalog {
        providers: vec!["first".into()],
        saved: vec![saved],
        supports_model_cards: true,
        ..InteractiveModelCatalog::default()
    })));
    browser.paste("discovered model");
    assert!(matches!(
        browser.activate_row(),
        CatalogAction::Select(ModelCatalogSelection::Saved(selected))
            if selected.target == target && selected.binding == editable_card().binding
    ));
    let edit = browser.browse_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert!(matches!(edit, CatalogAction::Edit(selected) if selected == target));
    assert!(!browser.groups["first"].loaded && !browser.groups["first"].loading);
    browser.loading_editor = Some((4, target));
    browser.update(ModelCatalogUpdate::Card {
        request: 3,
        result: Ok(Box::new(editable_card())),
    });
    assert!(browser.editor.is_none());
    browser.update(ModelCatalogUpdate::Card {
        request: 4,
        result: Ok(Box::new(editable_card())),
    });
    assert!(browser.editor.is_some());
}

#[test]
fn a_legacy_worker_can_still_select_one_of_its_discovered_models() {
    let mut browser = ModelCatalogState::new(1);
    browser.update(ModelCatalogUpdate::Overview(Ok(InteractiveModelCatalog {
        providers: vec!["first".into()],
        ..InteractiveModelCatalog::default()
    })));
    browser.groups.get_mut("first").unwrap().expanded = true;
    browser.update(ModelCatalogUpdate::Provider {
        profile: "first".into(),
        result: Ok(vec![ProviderModelInfo {
            context_window_tokens: Some(1_024),
            max_output_tokens: Some(512),
            ..card("first/model")
        }]),
    });
    browser.selected = browser.rows.len() - 1;
    assert!(matches!(
        browser.activate_row(),
        CatalogAction::Select(ModelCatalogSelection::Discovered(profile))
            if profile.model == "first/model"
                && profile.context_window_tokens == 1_024
                && profile.max_output_tokens == 511
    ));
}
