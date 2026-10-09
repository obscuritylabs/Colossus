use super::*;

#[test]
fn settings_choose_current_values_and_cancel_without_changes() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("preserved draft");
    let original = state.preferences.clone();
    for (command, current_command) in [
        ("/permissions", "/permissions ask"),
        ("/stream", "/stream on"),
        ("/events", "/events compact"),
        ("/reasoning", "/reasoning on"),
        ("/transcript", "/transcript comfortable"),
        ("/multiline", "/multiline off"),
        ("/provider diagnostics", "/provider diagnostics off"),
    ] {
        let mut picker = SettingsPickerState::for_command(command, &state).expect("setting picker");
        assert_eq!(
            picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(current_command)
        );
        state.overlay = Some(Overlay::SettingsPicker(picker));
        assert!(state.transient_inline_screen_active());
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.overlay.is_none());
        assert!(state.pending_setting_command.is_none());
        assert_eq!(state.preferences, original);
        assert_eq!(state.footer.approval_mode, "ask");
        assert_eq!(state.draft(), "preserved draft");
    }
    state.overlay = Some(Overlay::SettingsPicker(
        SettingsPickerState::for_command("/permissions", &state).expect("picker"),
    ));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE),
    );
    assert!(state.pending_setting_command.is_none());
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_none());
    assert_eq!(
        state.pending_setting_command,
        Some("/permissions risk-auto")
    );
    assert_eq!(
        state.footer.approval_mode, "ask",
        "host applies the mode after confirmation"
    );
}

#[test]
fn setting_completion_shows_roots_then_explicit_argument_shortcuts() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions = [
        "/permissions",
        "/permissions ask",
        "/permissions full-access",
        "/events",
        "/events compact",
        "/theme",
        "/theme list",
    ]
    .map(String::from)
    .into();
    state.composer.insert("/");
    assert_eq!(
        state.completion_menu_candidates(),
        vec!["/permissions", "/events", "/theme", "/theme list"]
    );
    state.composer.insert("permissions");
    assert!(state.completion_menu_candidates().is_empty());
    assert!(state.ghost_text().is_none());
    state.composer.insert(" ");
    assert_eq!(
        state.completion_menu_candidates(),
        vec!["/permissions ask", "/permissions full-access"]
    );
}

#[test]
fn permission_choices_and_controls_remain_readable_after_resize() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.overlay = Some(Overlay::SettingsPicker(
        SettingsPickerState::for_command("/permissions", &state).expect("picker"),
    ));
    for (width, height) in [(120, 32), (80, 16), (80, 12), (40, 12)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw picker");
        let rendered = terminal.backend().to_string();
        for label in [
            "Update permissions",
            "Deny approvals",
            "Ask for approval (current)",
            "Approve low risk",
            "Full access",
            "Enter",
            "Apply",
            "Esc",
            "Back",
        ] {
            assert!(
                rendered.contains(label),
                "{width}x{height}: missing {label}\n{rendered}"
            );
        }
        assert!(!rendered.contains("Message · Enter sends"), "{rendered}");
    }
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("narrow terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw full-access choice");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("and sandbox limits."), "{rendered}");
}

#[test]
fn history_search_browses_filtered_prompts_and_preserves_the_cancelled_draft() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.history = vec![
        "Build first\nwith more detail".into(),
        "unrelated".into(),
        "build latest".into(),
    ];
    state.composer.insert("unfinished draft");
    state.overlay = Some(Overlay::HistorySearch(HistorySearchState::new(
        &state.history,
    )));
    assert!(state.transient_inline_screen_active());
    insert_active_text(&mut state, "BUILD");
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.draft(), "Build first\nwith more detail");
    assert_eq!(state.cursor(), state.draft().len());

    state.overlay = Some(Overlay::HistorySearch(HistorySearchState::new(
        &state.history,
    )));
    insert_active_text(&mut state, "no matching prompt");
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_some());
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.draft(), "Build first\nwith more detail");
}

#[test]
fn history_search_uses_the_full_screen_and_returns_to_the_preserved_conversation() {
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        let mut state = TuiState::from_snapshot(snapshot());
        state.history = vec![
            "Inspect the runtime\nThen explain the boundary".into(),
            "Fix the build".into(),
        ];
        state.composer.insert("draft below browser");
        state.overlay = Some(Overlay::HistorySearch(HistorySearchState::new(
            &state.history,
        )));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw browser");
        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("Prompt history"), "{rendered}");
        assert!(rendered.contains("Inspect the runtime"), "{rendered}");
        assert!(rendered.contains("Enter Use prompt"), "{rendered}");
        assert!(!rendered.contains("draft below browser"), "{rendered}");
        assert!(!rendered.contains("Message · Enter sends"), "{rendered}");
        if width >= 80 {
            assert!(rendered.contains("View"), "{rendered}");
        }
        handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("restore conversation");
        assert!(
            terminal
                .backend()
                .to_string()
                .contains("draft below browser")
        );
    }
}

#[test]
fn welcome_mark_fills_only_unused_space_and_never_enters_inline_scrollback() {
    let braille = |rendered: &str| {
        rendered
            .chars()
            .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
    };
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).expect("terminal");
    let mut state = TuiState::from_snapshot(empty_snapshot());
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("welcome");
    assert!(braille(&terminal.backend().to_string()));
    state.workspace = "long-workspace/".repeat(15);
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("long workspace");
    assert!(!braille(&terminal.backend().to_string()));
    state.workspace = "/workspace".into();
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Inline))
        .expect("inline welcome");
    assert!(!braille(&terminal.backend().to_string()));
}

#[test]
fn startup_warnings_do_not_hide_the_welcome_mark() {
    let mut source = empty_snapshot();
    source
        .preferences
        .select_builtin_theme(colossus_contracts::ThemeName::Hacker);
    source.security_posture.findings = vec![
        SecurityPostureFinding {
            code: "storage.plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Journal payloads are stored as plaintext canonical JSON.".into(),
            remediation: "Use platform or environment storage keys.".into(),
        },
        SecurityPostureFinding {
            code: "sandbox.danger_full_access".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Danger full access is enabled.".into(),
            remediation: "Use an isolating execution boundary.".into(),
        },
    ];
    let mut state = TuiState::from_snapshot(source);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("welcome with startup warnings");
    let rendered = terminal.backend().to_string();
    assert!(
        rendered.contains("Security posture · 2 warnings"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Danger full access is enabled."),
        "{rendered}"
    );
    assert!(
        rendered
            .chars()
            .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch)),
        "{rendered}"
    );

    state.dismiss_welcome();
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("conversation after welcome");
    let rendered = terminal.backend().to_string();
    assert!(
        rendered.contains("Security posture · 2 warnings"),
        "{rendered}"
    );
    assert!(
        !rendered
            .chars()
            .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch)),
        "{rendered}"
    );
}

fn rendered_footer(state: &TuiState, width: u16) -> String {
    let mut terminal =
        Terminal::new(TestBackend::new(width, FOOTER_HEIGHT)).expect("footer terminal");
    terminal
        .draw(|frame| render_footer(frame, state, frame.area(), true))
        .expect("draw footer");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn footer_combines_status_and_runtime_in_one_row_with_readable_highlights() {
    let mut source = snapshot();
    source
        .preferences
        .select_builtin_theme(colossus_contracts::ThemeName::Hacker);
    source.footer.route = "gpt-5.6-sol@codex via codex-provider".into();
    source
        .security_posture
        .findings
        .push(SecurityPostureFinding {
            code: "storage.plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Plaintext journal".into(),
            remediation: "Use storage keys".into(),
        });
    let state = TuiState::from_snapshot(source);
    let mut terminal = Terminal::new(TestBackend::new(120, FOOTER_HEIGHT)).expect("terminal");
    terminal
        .draw(|frame| render_footer(frame, &state, frame.area(), true))
        .expect("footer");
    let row = |y| {
        (0..120)
            .map(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, y))
                    .expect("cell")
                    .symbol()
            })
            .collect::<String>()
    };
    let status = row(0);
    assert!(status.contains("ready · approval ask"), "{status}");
    assert!(status.contains("⚠ Security: 1"), "{status}");
    assert!(
        status.contains("execute") && status.contains("gpt-5.6-sol@codex via codex-provider · ctx"),
        "{status}"
    );
    assert_eq!(terminal.backend().buffer().area.height, 1);
    assert!(status.contains("  ⚠"), "{status}");
    let buffer = terminal.backend().buffer();
    let warning_x = (0..120)
        .find(|x| buffer.cell((*x, 0)).expect("cell").symbol() == "⚠")
        .expect("warning badge");
    let warning = buffer.cell((warning_x, 0)).expect("warning cell");
    assert_eq!(warning.fg, Color::Black);
    assert_ne!(warning.bg, Color::Reset);
    assert!(!warning.modifier.contains(Modifier::DIM));
    let runtime_x = (0..120)
        .find(|x| buffer.cell((*x, 0)).expect("cell").symbol() == "g")
        .expect("runtime text");
    let runtime = buffer.cell((runtime_x, 0)).expect("runtime cell");
    assert_eq!(runtime.bg, buffer.cell((0, 0)).expect("surface cell").bg);
    assert!(!runtime.modifier.contains(Modifier::DIM));
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| !cell.modifier.contains(Modifier::REVERSED))
    );
}

#[test]
fn footer_keeps_status_and_permissions_visible_with_long_runtime_metadata() {
    let mut source = snapshot();
    source.footer.route = "a-very-long-provider-model-name@a-long-profile via provider".into();
    source.footer.approval_mode = "full-access".into();
    source
        .security_posture
        .findings
        .push(SecurityPostureFinding {
            code: "storage.plaintext".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Plaintext journal".into(),
            remediation: "Use storage keys".into(),
        });
    let mut state = TuiState::from_snapshot(source);
    for width in [40, 42, 60, 80, 120] {
        let footer = rendered_footer(&state, width);
        assert!(footer.contains("ready"), "{width}: {footer}");
        assert!(footer.contains("full-access"), "{width}: {footer}");
        assert!(
            footer.contains("Security: 1") || footer.contains("⚠ 1"),
            "{width}: {footer}"
        );
    }
    state.queue.push_back("next turn".into());
    state.queue_paused = true;
    let footer = rendered_footer(&state, 40);
    assert!(footer.contains("queue paused"), "{footer}");
    assert!(footer.contains("full-access"), "{footer}");
    assert!(footer.contains("⚠ 1"), "{footer}");
    state.queue_paused = false;
    state.operation = Some(OperationKind::Run);
    assert!(rendered_footer(&state, 40).contains("working"));
}

#[test]
fn footer_removes_terminal_controls_from_host_metadata() {
    let mut source = snapshot();
    source.footer.route = "model\u{1b}[31m\nprofile".into();
    source.footer.approval_mode = "ask\u{7}\u{200d}".into();
    source.footer.status = "ready\rnow".into();
    let state = TuiState::from_snapshot(source);
    let footer = rendered_footer(&state, 160);
    assert!(footer.contains("ready now"), "{footer}");
    assert!(footer.contains("approval ask"), "{footer}");
    assert!(!footer.chars().any(char::is_control), "{footer}");
    assert!(!footer.contains('\u{200d}'), "{footer}");
}

#[test]
fn composer_keeps_send_action_and_newline_hint_visible_in_narrow_terminals() {
    for (theme, multiline) in [
        colossus_contracts::ThemeName::Default,
        colossus_contracts::ThemeName::Mono,
        colossus_contracts::ThemeName::HighContrast,
        colossus_contracts::ThemeName::Carrot,
        colossus_contracts::ThemeName::Hacker,
    ]
    .into_iter()
    .flat_map(|theme| {
        [false, true]
            .into_iter()
            .map(move |multiline| (theme, multiline))
    }) {
        let mut state = TuiState::from_snapshot(snapshot());
        state.preferences.select_builtin_theme(theme);
        state.preferences.multiline = multiline;
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).expect("composer terminal");
        terminal
            .draw(|frame| render_composer(frame, &mut state, frame.area()))
            .expect("draw composer");
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains(if multiline {
                "Ctrl+D sends"
            } else {
                "Enter sends"
            }),
            "{rendered}"
        );
        assert!(
            rendered.contains(if multiline {
                "Enter newline"
            } else {
                "Ctrl+J newline"
            }),
            "{rendered}"
        );
        let buffer = terminal.backend().buffer();
        let prompt = buffer.cell((1, 1)).expect("prompt cell");
        let hint = buffer.cell((2, 2)).expect("hint cell");
        assert_ne!(prompt.bg, Color::Reset);
        assert_eq!(prompt.bg, hint.bg);
        assert_ne!(hint.fg, prompt.bg);
        assert!(!hint.modifier.contains(Modifier::DIM), "{theme:?}");
    }
}

#[test]
fn custom_theme_dark_ink_stays_readable_on_shaded_chrome() {
    for (assistant, meta) in [(0, 0), (230, 0), (0, 230), (100, 100), (230, 230)] {
        let mut theme = custom_theme();
        let foreground = |channel| ThemeColor {
            red: channel,
            green: channel,
            blue: channel,
        };
        theme.assistant.foreground = Some(foreground(assistant));
        theme.warning.foreground = theme.assistant.foreground;
        theme.meta.foreground = Some(foreground(meta));
        let mut source = snapshot();
        source.preferences.select_custom_theme(theme);
        let mut state = TuiState::from_snapshot(source);
        state.completions = vec!["draft suggestion".into()];
        state.composer.insert("draft");
        let mut terminal = Terminal::new(TestBackend::new(80, 4)).expect("terminal");
        terminal
            .draw(|frame| {
                render_composer(frame, &mut state, Rect::new(0, 0, 80, 3));
                render_footer(frame, &state, Rect::new(0, 3, 80, 1), false);
            })
            .expect("custom theme chrome");
        let buffer = terminal.backend().buffer();
        let expected = |channel| {
            if channel == 230 {
                Color::Rgb(230, 230, 230)
            } else {
                Color::Rgb(230, 237, 243)
            }
        };
        for position in [(2, 0), (1, 1), (12, 3)] {
            let cell = buffer.cell(position).expect("draft, title, or status");
            assert_eq!(cell.fg, expected(assistant), "{position:?}");
            assert_ne!(cell.fg, cell.bg);
            assert!(!cell.modifier.contains(Modifier::DIM));
        }
        for position in [(0, 1), (2, 2), (6, 1)] {
            let cell = buffer.cell(position).expect("border, hint, or completion");
            assert_eq!(cell.fg, expected(meta), "{position:?}");
            assert_ne!(cell.fg, cell.bg);
        }
        assert!(
            buffer
                .cell((6, 1))
                .expect("completion")
                .modifier
                .contains(Modifier::DIM)
        );
        assert!(
            !buffer
                .cell((2, 2))
                .expect("hint")
                .modifier
                .contains(Modifier::DIM)
        );
        assert_eq!(state.composer.draft, "draft");
        state.footer.status = "waiting".into();
        terminal
            .draw(|frame| render_footer(frame, &state, Rect::new(0, 3, 80, 1), false))
            .expect("waiting footer");
        assert_eq!(
            terminal
                .backend()
                .buffer()
                .cell((12, 3))
                .expect("waiting status")
                .fg,
            expected(assistant)
        );
    }
}

#[test]
fn composer_shows_queue_action_and_scrolled_draft_position() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.operation = Some(OperationKind::Run);
    state
        .composer
        .insert("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight");
    let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("composer terminal");
    terminal
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw composer");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Enter queues"), "{rendered}");
    assert!(rendered.contains("row 8/8"), "{rendered}");
    assert!(rendered.contains("eight"), "{rendered}");
}

#[test]
fn completion_ghost_does_not_count_as_hidden_draft_rows() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions = vec![format!("prefix{}", "long suggestion ".repeat(20))];
    state.composer.insert("prefix");
    let mut terminal = Terminal::new(TestBackend::new(40, 3)).expect("composer terminal");
    terminal
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw completion ghost");
    let rendered = terminal.backend().to_string();
    assert!(!rendered.contains("row 1/"), "{rendered}");
    assert!(rendered.contains("Ctrl+J newline"), "{rendered}");
}

#[test]
fn runtime_input_prompt_shows_waiting_status() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.operation = Some(OperationKind::Run);
    let (response, _answer) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "input".into(),
            kind: InteractivePromptKind::UserInput,
            title: "Choose a name".into(),
            document: PresentationDocument::from_block(PresentationBlock::Markdown("Name".into())),
            choices: Vec::new(),
            initial_choice: None,
            allow_free_form: true,
            response,
        }),
    );
    let footer = rendered_footer(&state, 40);
    assert!(footer.contains("waiting"), "{footer}");
    assert!(!footer.contains("working"), "{footer}");
}

#[test]
fn vertical_editing_preserves_the_column_through_short_unicode_rows() {
    let mut composer = Composer::default();
    composer.insert("abcdef\n界\n123456");
    assert!(composer.move_vertical(ComposerVerticalDirection::Previous, 78));
    assert_eq!(composer.cursor, "abcdef\n界".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Previous, 78));
    assert_eq!(composer.cursor, "abcdef".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 78));
    assert_eq!(composer.cursor, "abcdef\n界".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 78));
    assert_eq!(composer.cursor, composer.draft.len());
    assert!(!composer.move_vertical(ComposerVerticalDirection::Next, 78));
    composer.move_to("abc".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 78));
    assert_eq!(composer.cursor, "abcdef\n界".len());
    composer.move_left();
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 78));
    assert_eq!(composer.cursor, "abcdef\n界\n".len());
}

#[test]
fn vertical_editing_uses_wrapped_rows_without_splitting_graphemes() {
    let mut composer = Composer::default();
    composer.insert("abcde界❤️z");
    assert!(composer.move_vertical(ComposerVerticalDirection::Previous, 5));
    assert_eq!(composer.cursor, "abcde".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Previous, 5));
    assert_eq!(composer.cursor, 0);
    assert!(!composer.move_vertical(ComposerVerticalDirection::Previous, 5));
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 5));
    assert_eq!(composer.cursor, "abcde".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 5));
    assert_eq!(composer.cursor, composer.draft.len());
    composer.move_to(3);
    assert!(composer.move_vertical(ComposerVerticalDirection::Next, 5));
    assert_eq!(composer.cursor, "abcde界".len());
    assert!(composer.move_vertical(ComposerVerticalDirection::Previous, 5));
    assert_eq!(composer.cursor, 3);
}

#[test]
fn top_bar_keeps_workspace_and_session_separate_from_the_footer() {
    for (width, height, mode, has_header) in [
        (120, 40, ScreenMode::Alternate, true),
        (80, 24, ScreenMode::Alternate, true),
        (40, 18, ScreenMode::Alternate, true),
        (40, 12, ScreenMode::Alternate, false),
        (120, 40, ScreenMode::Inline, false),
    ] {
        let mut source = snapshot();
        source.workspace = "/workspace/Colossus".into();
        source.session_id = "019f-test-session".into();
        let mut state = TuiState::from_snapshot(source);
        state.composer.insert("preserved draft");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, &mut state, 0, mode))
            .expect("layout");
        let text = terminal.backend().to_string();
        assert!(text.contains("preserved draft"), "{width}x{height}: {text}");
        assert!(
            text.contains("durable row marker"),
            "{width}x{height}: {text}"
        );
        let header = (0..width)
            .map(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, 0))
                    .expect("header cell")
                    .symbol()
            })
            .collect::<String>();
        assert_eq!(
            header.contains(env!("CARGO_PKG_VERSION")),
            has_header,
            "{header}"
        );
        if has_header {
            assert!(header.contains("019f-tes"), "{header}");
            let footer = (height - FOOTER_HEIGHT..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|pos| {
                    terminal
                        .backend()
                        .buffer()
                        .cell(pos)
                        .expect("footer cell")
                        .symbol()
                })
                .collect::<String>();
            assert!(!footer.contains("session"), "{footer}");
        }
    }
}

#[test]
fn top_bar_sanitizes_and_bounds_runtime_labels() {
    let mut source = snapshot();
    source.workspace = "/workspace/".to_owned() + &"long-name".repeat(30) + "\u{1b}[31m\n";
    source.session_id = "019f-test\u{7}session".into();
    let state = TuiState::from_snapshot(source);
    for width in [40, 80, 120] {
        let mut terminal = Terminal::new(TestBackend::new(width, HEADER_HEIGHT)).expect("terminal");
        terminal
            .draw(|frame| render_header(frame, &state, frame.area()))
            .expect("header");
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("019f-tes"), "{text}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(!text.chars().any(char::is_control), "{text}");
    }
}

#[test]
fn history_preview_scrolls_back_immediately_after_repeated_page_down_and_resize() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.history = vec![
        (0..100)
            .map(|n| format!("line {n:03}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ];
    state.overlay = Some(Overlay::HistorySearch(HistorySearchState::new(
        &state.history,
    )));
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("history");
    for _ in 0..100 {
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        );
    }
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("last page");
    let bottom = terminal.backend().to_string();
    assert!(bottom.contains("line 099"), "{bottom}");
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("previous page");
    let previous = terminal.backend().to_string();
    assert_ne!(previous, bottom);
    assert!(!previous.contains("line 099"), "{previous}");
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    let mut taller = Terminal::new(TestBackend::new(120, 40)).expect("taller terminal");
    taller
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("resized last page");
    let bottom = taller.backend().to_string();
    assert!(bottom.contains("line 099"), "{bottom}");
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );
    taller
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("resized previous page");
    assert_ne!(taller.backend().to_string(), bottom);
}

#[test]
fn history_search_filters_large_histories_only_when_the_query_changes() {
    let history = (0..1_000)
        .map(|n| format!("Prompt {n}: {}", "Résumé detail ".repeat(128)))
        .collect::<Vec<_>>();
    let mut search = HistorySearchState::new(&history);
    assert_eq!(search.filtered_indices().len(), 1_000);
    search.query = "RÉSUMÉ".into();
    search.reconcile_selection(&history);
    assert_eq!(search.filtered_indices().len(), 1_000);
    search.query = "prompt 998:".into();
    search.reconcile_selection(&history);
    assert_eq!(search.filtered_indices(), &[998]);
    search.query.clear();
    search.reconcile_selection(&history);
    assert_eq!(search.filtered_indices().len(), 1_000);
    assert_eq!(search.filtered_indices()[0], 999);
}
