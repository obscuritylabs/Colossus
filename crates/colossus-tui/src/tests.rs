use super::*;
use colossus_contracts::{
    CustomTheme, EventDisplayMode, ModelMessage, ModelToolCall, SessionMessage, StreamDisplayMode,
    ThemeColor, ThemeSpinner, ThemeTextStyle, TranscriptDensity,
};
use ratatui::{Terminal, backend::TestBackend};

fn snapshot() -> InteractiveSnapshot {
    InteractiveSnapshot {
        session_id: "019f-test".into(),
        transcript: SessionMessagePage {
            messages: vec![SessionMessage {
                session_id: "019f-test".into(),
                run_id: "run".into(),
                sequence: 1,
                message: ModelMessage {
                    role: ModelMessageRole::Assistant,
                    content: "durable row marker".into(),
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
                created_at: "2026-07-15T00:00:00Z".into(),
            }],
            before_sequence: Some(1),
            has_more: true,
        },
        preferences: TerminalPreferences::default(),
        history: vec!["older prompt".into()],
        completions: vec!["/tools".into(), "/tui prefs".into()],
        footer: FooterState {
            role: "primary".into(),
            route: "echo@echo".into(),
            context: Some((1, 32_768)),
            message_count: 1,
            status: "ready".into(),
            approval_mode: "ask".into(),
        },
    }
}

fn custom_theme() -> CustomTheme {
    let primary = ThemeTextStyle {
        foreground: Some(ThemeColor {
            red: 64,
            green: 200,
            blue: 255,
        }),
        bold: false,
        dim: false,
        italic: false,
    };
    let muted = ThemeTextStyle {
        foreground: Some(ThemeColor {
            red: 100,
            green: 110,
            blue: 120,
        }),
        bold: false,
        dim: true,
        italic: true,
    };
    CustomTheme {
        schema_version: 1,
        name: "test_ocean".into(),
        source_hash: "a".repeat(64),
        base: colossus_contracts::ThemeName::Default,
        title: "Ocean".into(),
        caret: "›".into(),
        continuation: "…".into(),
        prompt_left: primary.foreground,
        prompt_right: muted.foreground,
        indicator: primary.foreground,
        continuation_color: muted.foreground,
        assistant: primary,
        activity: primary,
        thinking: muted,
        tool: primary,
        success: primary,
        warning: primary,
        error: primary,
        meta: muted,
        spinner: ThemeSpinner::Line,
    }
}

#[test]
fn parser_handles_tui_commands_without_a_repl_alias() {
    assert_eq!(
        parse_interactive_command("/tui prefs"),
        InteractiveCommand::Local(LocalCommand::Preferences)
    );
    assert_eq!(
        parse_interactive_command("/tui reset"),
        InteractiveCommand::Local(LocalCommand::ResetPreferences)
    );
    assert_eq!(
        parse_interactive_command("/repl reset"),
        InteractiveCommand::Runtime(RuntimeCommand::Known {
            name: "repl".into(),
            arguments: "reset".into(),
        })
    );
    assert_eq!(
        parse_interactive_command("/plans"),
        InteractiveCommand::Runtime(RuntimeCommand::Known {
            name: "plans".into(),
            arguments: String::new(),
        })
    );
}

#[test]
fn unicode_editing_never_splits_a_character() {
    let mut composer = Composer::default();
    composer.insert("a🦀界");
    composer.move_left();
    composer.backspace();
    assert_eq!(composer.draft, "a界");
    assert!(composer.draft.is_char_boundary(composer.cursor));
    composer.delete();
    assert_eq!(composer.draft, "a");
}

#[test]
fn multiline_history_search_and_boundary_navigation_preserve_the_draft() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.preferences.multiline = true;
    state.composer.insert("first");
    state.composer.insert("\n");
    state.composer.insert("界");
    assert_eq!(state.draft(), "first\n界");
    state.composer.cursor = 0;
    state.previous_history();
    assert_eq!(state.draft(), "older prompt");
    state.next_history();
    assert!(state.draft().is_empty());
    state.composer.insert("unsent draft");
    state.overlay = Some(Overlay::HistorySearch {
        query: "older".into(),
    });
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.draft(), "unsent draft");
}

#[test]
fn completion_ghost_is_separate_and_right_accepts_it() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("/to");
    assert_eq!(state.ghost_text(), Some("ols"));
    assert!(state.accept_completion());
    assert_eq!(state.draft(), "/tools");
}

#[test]
fn structured_completion_tracks_slash_commands_and_skill_tokens() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions.extend([
        "@coding".into(),
        "@offline-dev".into(),
        "@security-review".into(),
    ]);

    state.composer.insert("/");
    assert_eq!(
        state.structured_completion_context(),
        Some(CompletionContext {
            prefix: "/",
            kind: CompletionKind::Command,
        })
    );
    assert_eq!(
        state.completion_menu_candidates(),
        vec!["/tools", "/tui prefs"]
    );

    state.composer.clear();
    state.composer.insert("please @off");
    assert_eq!(
        state.structured_completion_context(),
        Some(CompletionContext {
            prefix: "@off",
            kind: CompletionKind::Skill,
        })
    );
    assert_eq!(state.ghost_text(), Some("line-dev"));
    assert!(state.accept_completion());
    assert_eq!(state.draft(), "please @offline-dev ");
}

#[test]
fn completion_selection_moves_in_both_directions_and_can_be_dismissed() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("/");
    state.advance_completion();
    assert_eq!(state.composer.completion_index, Some(0));
    state.advance_completion();
    assert_eq!(state.composer.completion_index, Some(1));
    state.previous_completion();
    assert_eq!(state.composer.completion_index, Some(0));
    assert!(state.hide_completion());
    assert!(state.completion_menu_candidates().is_empty());
    state.composer.insert("to");
    assert_eq!(state.completion_menu_candidates(), vec!["/tools"]);
}

#[test]
fn visible_completion_menu_is_adaptive_at_minimum_size() {
    for (width, height) in [(40, 12), (60, 16)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(snapshot());
        state.composer.insert("/");
        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw completion menu");
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("Commands"),
            "{width}x{height}: {rendered}"
        );
        assert!(rendered.contains("/tools"), "{width}x{height}: {rendered}");
        assert!(
            rendered.contains("/tui prefs"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("durable row marker"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Message · Enter sends"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Colossus"),
            "{width}x{height}: {rendered}"
        );
    }
}

#[test]
fn completion_ghost_uses_a_distinct_low_emphasis_style() {
    let backend = TestBackend::new(40, 3);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions = vec!["candy".into()];
    state.composer.insert("can");
    terminal
        .draw(|frame| render_composer(frame, &state, frame.area()))
        .expect("draw composer");
    let buffer = terminal.backend().buffer();
    let typed = buffer.cell((1, 1)).expect("typed cell").style();
    let ghost = buffer.cell((4, 1)).expect("ghost cell").style();
    assert_ne!(typed, ghost);
    assert!(ghost.add_modifier.contains(Modifier::DIM));
}

#[test]
fn canonical_system_messages_are_excluded() {
    let mut source = snapshot();
    source.transcript.messages.insert(
        0,
        SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 0,
            message: ModelMessage {
                role: ModelMessageRole::System,
                content: "hidden instructions".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
    );
    let state = TuiState::from_snapshot(source);
    assert_eq!(state.transcript.len(), 1);
    assert!(
        !transcript_lines(&state, 80)
            .iter()
            .any(|line| line.to_string().contains("hidden instructions"))
    );
}

#[test]
fn historical_tool_results_are_correlated_with_assistant_calls() {
    let mut source = snapshot();
    source.transcript.messages = vec![
        SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 1,
            message: ModelMessage {
                role: ModelMessageRole::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-1".into(),
                    name: "filesystem.search".into(),
                    arguments: serde_json::json!({"query": "needle"}),
                }],
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
        SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 2,
            message: ModelMessage {
                role: ModelMessageRole::Tool,
                content: "found".into(),
                tool_call_id: Some("call-1".into()),
                tool_calls: Vec::new(),
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
    ];
    let state = TuiState::from_snapshot(source);
    let rendered = transcript_lines(&state, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("Completed filesystem.search"),
        "{rendered}"
    );
}

#[test]
fn historical_web_fetch_results_keep_compact_preview_semantics() {
    let mut source = snapshot();
    let body = format!("preview-start\n{}FULL-BODY-TAIL", "line\n".repeat(100));
    source.transcript.messages = vec![
        SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 1,
            message: ModelMessage {
                role: ModelMessageRole::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: vec![ModelToolCall {
                    call_id: "call-fetch".into(),
                    name: "web.fetch".into(),
                    arguments: serde_json::json!({"url": "https://example.com"}),
                }],
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
        SessionMessage {
            session_id: "019f-test".into(),
            run_id: "run".into(),
            sequence: 2,
            message: ModelMessage {
                role: ModelMessageRole::Tool,
                content: body.clone(),
                tool_call_id: Some("call-fetch".into()),
                tool_calls: Vec::new(),
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
    ];

    let compact = TuiState::from_snapshot(source.clone());
    let compact = transcript_lines(&compact, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("Response preview"), "{compact}");
    assert!(compact.contains("preview only"), "{compact}");
    assert!(!compact.contains("FULL-BODY-TAIL"), "{compact}");

    source.preferences.events_mode = EventDisplayMode::Verbose;
    let verbose = TuiState::from_snapshot(source);
    let verbose = transcript_lines(&verbose, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(verbose.contains("FULL-BODY-TAIL"), "{verbose}");
}

#[test]
fn page_boundary_tool_results_remain_bounded_before_their_call_is_loaded() {
    let mut source = snapshot();
    let body = format!("preview-start\n{}FULL-BODY-TAIL", "line\n".repeat(100));
    source.transcript.messages = vec![SessionMessage {
        session_id: "019f-test".into(),
        run_id: "run".into(),
        sequence: 101,
        message: ModelMessage {
            role: ModelMessageRole::Tool,
            content: body.clone(),
            tool_call_id: Some("call-from-older-page".into()),
            tool_calls: Vec::new(),
        },
        created_at: "2026-07-15T00:00:00Z".into(),
    }];
    source.transcript.before_sequence = Some(101);
    source.transcript.has_more = true;

    let compact = TuiState::from_snapshot(source.clone());
    let compact = transcript_lines(&compact, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("Output preview"), "{compact}");
    assert!(compact.contains("preview only"), "{compact}");
    assert!(!compact.contains("FULL-BODY-TAIL"), "{compact}");

    source.preferences.events_mode = EventDisplayMode::Verbose;
    let verbose = TuiState::from_snapshot(source);
    let verbose = transcript_lines(&verbose, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(verbose.contains("FULL-BODY-TAIL"), "{verbose}");
}

#[test]
fn session_switch_replaces_transcript_and_resets_live_scroll_state() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.page_up();
    state.new_items = 3;
    apply_command_result(
        &mut state,
        HostCommandResult {
            document: PresentationDocument::new(),
            session: Some((
                "019f-other".into(),
                SessionMessagePage {
                    messages: vec![SessionMessage {
                        session_id: "019f-other".into(),
                        run_id: "other-run".into(),
                        sequence: 1,
                        message: ModelMessage {
                            role: ModelMessageRole::Assistant,
                            content: "other transcript".into(),
                            tool_call_id: None,
                            tool_calls: Vec::new(),
                        },
                        created_at: "2026-07-15T00:00:00Z".into(),
                    }],
                    before_sequence: Some(1),
                    has_more: true,
                },
            )),
            preferences: None,
            completions: None,
            sticky_skills: None,
            footer: None,
            clear_transcript: false,
        },
    );
    assert_eq!(state.session_id, "019f-other");
    assert_eq!(state.transcript.len(), 1);
    assert_eq!(state.scroll_from_bottom, 0);
    assert_eq!(state.new_items, 0);
    assert!(
        transcript_lines(&state, 80)
            .iter()
            .any(|line| line.to_string().contains("other transcript"))
    );
}

#[test]
fn on_raw_and_off_stream_modes_keep_their_distinct_transcript_contracts() {
    let envelope = |event| RunEventEnvelope {
        schema_version: 1,
        run_id: "run-stream".into(),
        session_id: "019f-test".into(),
        event,
    };
    for mode in [
        StreamDisplayMode::On,
        StreamDisplayMode::Raw,
        StreamDisplayMode::Off,
    ] {
        let mut state = TuiState::from_snapshot(snapshot());
        state.preferences.stream_mode = mode;
        let starting = state.transcript.len();
        handle_run_event(
            &mut state,
            envelope(RunEvent::Provider {
                event: ProviderEvent::ModelDelta {
                    text: "**partial**".into(),
                },
            }),
        );
        assert_eq!(
            state.transcript.len(),
            starting + usize::from(mode != StreamDisplayMode::Off)
        );
        handle_run_event(
            &mut state,
            envelope(RunEvent::Provider {
                event: ProviderEvent::FinalOutput {
                    text: "**complete**".into(),
                },
            }),
        );
        let block = state
            .transcript
            .last()
            .and_then(|entry| entry.document.blocks.first())
            .expect("final block");
        if mode == StreamDisplayMode::Raw {
            assert!(matches!(block, PresentationBlock::Text(_)));
        } else {
            assert!(matches!(block, PresentationBlock::Markdown(_)));
        }
    }
}

#[test]
fn prompt_cancel_is_one_use_and_preserves_the_composer_draft() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("draft stays here");
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "prompt-1".into(),
            title: "Approval".into(),
            document: PresentationDocument::from_block(PresentationBlock::Text("Allow?".into())),
            choices: vec!["allow".into(), "deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
    assert_eq!(state.draft(), "draft stays here");
    assert!(state.overlay.is_none());
}

#[test]
fn prompt_keyboard_selection_returns_the_highlighted_choice() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "session-picker".into(),
            title: "Resume session".into(),
            document: PresentationDocument::new(),
            choices: vec![
                "First session\nFirst preview".into(),
                "Second session\nSecond preview".into(),
            ],
            initial_choice: Some(0),
            allow_free_form: false,
            response,
        }),
    );
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        received.try_recv(),
        Ok(PromptResponse::Answer(
            "Second session\nSecond preview".into()
        ))
    );
    assert!(state.overlay.is_none());
}

#[test]
fn blank_approval_submission_still_fails_closed() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "approval".into(),
            title: "Approval".into(),
            document: PresentationDocument::from_block(PresentationBlock::Text("Allow?".into())),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
}

#[test]
fn resume_picker_is_responsive_and_keeps_the_selected_preview_visible() {
    for (width, height) in [(40, 12), (80, 24)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(snapshot());
        let choices = (0..10)
                .map(|index| {
                    let message_count = index + 1;
                    format!(
                        "Session {index} · {message_count} msgs · 2026-07-18 01:4{index} · 019f72e{index}\nPrior user message {index}"
                    )
                })
                .collect::<Vec<_>>();
        let (response, _received) = oneshot::channel();
        handle_host_event(
            &mut state,
            HostEvent::Prompt(InteractivePrompt {
                id: "session-picker".into(),
                title: "Resume session".into(),
                document: PresentationDocument::new(),
                choices,
                initial_choice: Some(7),
                allow_free_form: false,
                response,
            }),
        );
        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw resume picker");
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("Resume session · 8/10"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Prior user message 7"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Enter select"),
            "{width}x{height}: {rendered}"
        );
        assert!(!rendered.contains("Message count"), "{rendered}");
        assert!(!rendered.contains("Created at"), "{rendered}");
        assert!(!rendered.contains("Prior user message 0"), "{rendered}");
    }
}

#[test]
fn scrolled_up_state_counts_new_items_without_losing_position() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript_height = 4;
    state.transcript_width = 80;
    for index in 0..8 {
        state.append_entry(user_entry(
            &format!("old row {index}"),
            TranscriptKind::User,
        ));
    }
    state.page_up();
    let before_lines = transcript_lines(&state, state.transcript_width).len();
    let before_top = before_lines
        .saturating_sub(state.transcript_height)
        .saturating_sub(state.scroll_from_bottom);
    state.append_entry(user_entry("new row", TranscriptKind::User));
    let after_lines = transcript_lines(&state, state.transcript_width).len();
    let after_top = after_lines
        .saturating_sub(state.transcript_height)
        .saturating_sub(state.scroll_from_bottom);
    assert_eq!(after_top, before_top);
    assert_eq!(state.new_items, 1);
    state.end();
    assert_eq!(state.scroll_from_bottom, 0);
    assert_eq!(state.new_items, 0);
}

#[test]
fn queue_is_bounded_to_eight_future_turns() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.operation = Some(OperationKind::Run);
    for index in 0..10 {
        if state.queue.len() < MAX_QUEUED_TURNS {
            state.queue.push_back(format!("turn {index}"));
        }
    }
    assert_eq!(state.queue.len(), MAX_QUEUED_TURNS);
}

#[test]
fn failed_or_cancelled_runs_pause_the_queue_and_cancellation_is_cooperative() {
    let mut state = TuiState::from_snapshot(snapshot());
    let control = RunControl::default();
    state.operation = Some(OperationKind::Run);
    state.control = Some(control.clone());
    state.queue.push_back("next turn".into());
    assert!(state.cancel_focus());
    assert!(control.is_cancelled());
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("cancelled".into()))),
    );
    assert!(state.queue_paused);
    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
}

#[test]
fn hostile_controls_are_removed_and_minimum_size_preserves_state() {
    assert_eq!(
        sanitize_input("safe\u{1b}]8;;evil\u{7}text\r\n"),
        "safe]8;;eviltext\n"
    );
    let backend = TestBackend::new(39, 11);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("preserved draft");
    terminal
        .draw(|frame| render(frame, &mut state))
        .expect("draw");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Resize terminal"));
    assert_eq!(state.draft(), "preserved draft");
    assert_eq!(state.transcript.len(), 1);
}

#[test]
fn every_theme_keeps_transcript_and_composer_at_all_required_sizes() {
    let themes = [
        colossus_contracts::ThemeName::Default,
        colossus_contracts::ThemeName::Mono,
        colossus_contracts::ThemeName::HighContrast,
        colossus_contracts::ThemeName::Carrot,
        colossus_contracts::ThemeName::Hacker,
    ];
    for custom in [false, true] {
        for theme in themes {
            for (width, height) in [(40, 12), (60, 20), (80, 24), (120, 40), (160, 50)] {
                let mut source = snapshot();
                source.preferences = TerminalPreferences {
                    theme,
                    custom_theme: custom.then(custom_theme),
                    stream_mode: StreamDisplayMode::On,
                    events_mode: EventDisplayMode::Compact,
                    transcript_density: TranscriptDensity::Comfortable,
                    ..TerminalPreferences::default()
                };
                let backend = TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).expect("test terminal");
                let mut state = TuiState::from_snapshot(source);
                state.composer.insert("draft marker");
                terminal
                    .draw(|frame| render(frame, &mut state))
                    .expect("draw");
                let rendered = terminal.backend().to_string();
                assert!(rendered.contains("durable row marker"), "{width}x{height}");
                assert!(rendered.contains("draft marker"), "{width}x{height}");
                assert!(rendered.contains("Colossus"), "{width}x{height}");
            }
        }
    }
}

#[test]
fn transcript_is_borderless_and_uses_distinct_speaker_and_semantic_cues() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.append_entry(user_entry("question", TranscriptKind::User));
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Command,
        document: help_document(),
        temporary: false,
    });
    let lines = transcript_lines(&state, 80);
    let rendered = lines
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("● Colossus"), "{rendered}");
    assert!(rendered.contains("› You"), "{rendered}");
    assert!(rendered.contains("◆ Colossus terminal"), "{rendered}");
    assert!(!rendered.contains("Command\n"), "{rendered}");
    assert!(!rendered.contains("┌─Colossus terminal"), "{rendered}");
    assert!(!rendered.contains("│ Field"), "{rendered}");

    let assistant = lines
        .iter()
        .find(|line| line.to_string().contains("● Colossus"))
        .expect("assistant label");
    let user = lines
        .iter()
        .find(|line| line.to_string().contains("› You"))
        .expect("user label");
    assert_ne!(assistant.spans[0].style, user.spans[0].style);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state))
        .expect("draw");
    let screen = terminal.backend().to_string();
    assert!(!screen.contains("┌─Transcript"), "{screen}");
    assert!(screen.contains("Message · Enter sends"), "{screen}");
}

#[test]
fn labeled_transcript_content_reserves_its_indent_within_the_viewport() {
    let mut source = snapshot();
    source.transcript.messages = vec![SessionMessage {
        session_id: "019f-test".into(),
        run_id: "run".into(),
        sequence: 1,
        message: ModelMessage {
            role: ModelMessageRole::Assistant,
            content: String::new(),
            tool_call_id: None,
            tool_calls: vec![ModelToolCall {
                call_id: "call-wide".into(),
                name: "web.fetch".into(),
                arguments: serde_json::json!({
                    "url": "https://example.com/a/long/path/that/exercises/the/full/card/width"
                }),
            }],
        },
        created_at: "2026-07-15T00:00:00Z".into(),
    }];
    let state = TuiState::from_snapshot(source);
    for width in [40, 60, 80, 112] {
        let lines = transcript_lines(&state, width);
        assert!(
            lines.iter().all(|line| {
                unicode_width::UnicodeWidthStr::width(line.to_string().as_str()) <= width
            }),
            "transcript exceeded {width} columns:\n{}",
            lines
                .iter()
                .map(Line::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
