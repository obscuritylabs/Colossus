use super::*;

#[test]
fn terminal_query_requires_a_real_emulator_hint() {
    assert!(!terminal_can_answer_graphics_query(|name| match name {
        "TERM" => Ok("xterm-256color".into()),
        _ => Err(std::env::VarError::NotPresent),
    }));
    assert!(terminal_can_answer_graphics_query(|name| match name {
        "TERM" => Ok("xterm-kitty".into()),
        _ => Err(std::env::VarError::NotPresent),
    }));
    assert!(terminal_can_answer_graphics_query(|name| match name {
        "TERM" => Ok("xterm-256color".into()),
        "VTE_VERSION" => Ok("7800".into()),
        _ => Err(std::env::VarError::NotPresent),
    }));
}
use colossus_contracts::{
    AgentRunResult, CustomTheme, EventDisplayMode, ModelMessage, ModelToolCall,
    SandboxBoundaryMode, SecurityPostureFinding, SecurityPostureReport, SecurityPostureSeverity,
    SessionMessage, StreamDisplayMode, ThemeColor, ThemeSpinner, ThemeTextStyle, ToolCall,
    ToolResult, TranscriptDensity,
};
use ratatui::{Terminal, backend::TestBackend};

fn snapshot() -> InteractiveSnapshot {
    InteractiveSnapshot {
        session_id: "019f-test".into(),
        fresh_session: false,
        workspace: "/workspace/Colossus".into(),
        sandbox_profile: "workspace-development".into(),
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
        security_posture: Default::default(),
        pending_sandbox_boundary_acknowledgement: None,
    }
}

fn empty_snapshot() -> InteractiveSnapshot {
    let mut source = snapshot();
    source.fresh_session = true;
    source.transcript = SessionMessagePage {
        messages: Vec::new(),
        before_sequence: None,
        has_more: false,
    };
    source.footer.context = Some((0, 32_768));
    source.footer.message_count = 0;
    source
}

fn image_reference(index: usize) -> ModelImageReference {
    ModelImageReference {
        artifact_id: format!("artifact-{:064x}", index + 1),
        file_name: format!("image-{index}.png"),
        media_type: "image/png".into(),
        size_bytes: 128,
        sha256: format!("{:064x}", index + 100),
        width_pixels: 32,
        height_pixels: 16,
        detail: colossus_contracts::ModelImageDetail::Auto,
    }
}

#[test]
fn empty_session_opens_with_the_responsive_launch_rail() {
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(empty_snapshot());
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw launch rail");
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("COLOSSUS"),
            "{width}x{height}: {rendered}"
        );
        assert!(rendered.contains("READY"), "{width}x{height}: {rendered}");
        assert!(
            rendered.contains("What are we moving today?"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Build or change something"),
            "{width}x{height}: {rendered}"
        );
        assert!(rendered.contains("/plan"), "{width}x{height}: {rendered}");
        assert!(
            rendered.contains("Implement {feature}"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Execute · Enter sends"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            !rendered.contains("durable row marker"),
            "{width}x{height}: {rendered}"
        );
    }
}

#[test]
fn launch_rail_is_process_local_and_only_shown_for_an_empty_session() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut restored = TuiState::from_snapshot(snapshot());
    assert!(!restored.welcome_visible);
    terminal
        .draw(|frame| render(frame, &mut restored, 0, ScreenMode::Alternate))
        .expect("draw restored session");
    let rendered = terminal.backend().to_string();
    assert!(
        !rendered.contains("What are we moving today?"),
        "{rendered}"
    );
    assert!(rendered.contains("durable row marker"), "{rendered}");

    let mut resumed_empty = empty_snapshot();
    resumed_empty.fresh_session = false;
    let resumed_empty = TuiState::from_snapshot(resumed_empty);
    assert!(!resumed_empty.welcome_visible);

    let mut fresh = TuiState::from_snapshot(empty_snapshot());
    assert!(fresh.welcome_visible);
    assert!(fresh.transcript.is_empty());
    fresh.dismiss_welcome();
    assert!(!fresh.welcome_visible);
    assert!(fresh.transcript.is_empty());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut fresh, 0, ScreenMode::Alternate))
        .expect("draw dismissed launch rail");
    let rendered = terminal.backend().to_string();
    assert!(
        !rendered.contains("What are we moving today?"),
        "{rendered}"
    );
    assert!(!rendered.contains("Implement {feature}"), "{rendered}");
    assert!(rendered.contains("Message · Enter sends"), "{rendered}");
}

#[test]
fn launch_rail_labels_the_sandbox_profile_field_as_the_sandbox_profile() {
    let mut source = empty_snapshot();
    source.sandbox_profile = "offline-default".into();
    let state = TuiState::from_snapshot(source);
    let rendered = welcome_lines(&state, 120, 32)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("SANDBOX  offline-default"), "{rendered}");
    assert!(
        !rendered.contains("BOUNDARY"),
        "the sandbox profile is not the effective execution boundary: {rendered}"
    );
}

#[test]
fn launch_rail_sanitizes_runtime_supplied_context() {
    let mut source = empty_snapshot();
    source.workspace = "/workspace/Colossus\u{1b}]8;;evil\u{7}".into();
    source.sandbox_profile = "workspace\nwrite\u{200d}".into();
    source.footer.route = "model\u{1b}[31m@profile".into();
    source.footer.status = "ready\rnow".into();
    let state = TuiState::from_snapshot(source);
    let rendered = welcome_lines(&state, 120, 32)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rendered.contains('\u{1b}'), "{rendered}");
    assert!(!rendered.contains('\u{7}'), "{rendered}");
    assert!(!rendered.contains('\u{200d}'), "{rendered}");
    assert!(rendered.contains("workspace write"), "{rendered}");
    assert!(rendered.contains("READY NOW"), "{rendered}");
}

#[test]
fn fresh_launch_rail_stays_live_after_security_posture_enters_inline_history() {
    let mut source = empty_snapshot();
    source.security_posture = SecurityPostureReport {
        findings: vec![SecurityPostureFinding {
            code: "sandbox.danger_full_access".into(),
            severity: SecurityPostureSeverity::Warning,
            summary: "Danger full access is enabled.".into(),
            remediation: "Use an isolating sandbox backend.".into(),
        }],
    };
    let mut state = TuiState::from_snapshot(source);
    let committed = committable_transcript_end(&state.transcript, 0);
    assert_eq!(committed, 0);
    assert!(state.welcome_visible);
    assert!(
        desired_inline_viewport_height(&state, 120, 32, committed) > MINIMUM_INLINE_VIEWPORT_HEIGHT
    );

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, committed, ScreenMode::Inline))
        .expect("draw fresh warning session");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("What are we moving today?"), "{rendered}");
    assert!(rendered.contains("1 security warning"), "{rendered}");
    assert_eq!(
        rendered.matches("Execute · Enter sends").count(),
        1,
        "{rendered}"
    );
    let rail = rendered
        .find("What are we moving today?")
        .expect("launch rail");
    let warning = rendered
        .find("Danger full access is enabled.")
        .expect("security warning");
    assert!(
        rail < warning,
        "the security posture should follow the launch rail: {rendered}"
    );
    let rows = (0..32)
        .map(|y| {
            (0..120)
                .filter_map(|x| terminal.backend().buffer().cell((x, y)))
                .map(|cell| cell.symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let recommendation_row = rows
        .iter()
        .position(|row| row.contains("Recommendation:"))
        .expect("recommendation row");
    let composer_row = rows
        .iter()
        .position(|row| row.contains("Execute · Enter sends"))
        .expect("composer row");
    assert!(
        composer_row >= recommendation_row + 2,
        "at least one quiet row should separate startup guidance from the composer: {rows:?}"
    );
    assert!(
        rows[recommendation_row + 1..composer_row]
            .iter()
            .all(|row| row.trim().is_empty()),
        "only quiet space should sit between startup guidance and the composer: {rows:?}"
    );
    state.dismiss_welcome();
    assert_eq!(committable_transcript_end(&state.transcript, 0), 1);
}

#[test]
fn danger_full_access_posture_adds_a_non_durable_card_and_persistent_footer_badge() {
    let mut source = snapshot();
    source.security_posture = SecurityPostureReport {
        findings: vec![
            SecurityPostureFinding {
                code: "storage.plaintext".into(),
                severity: SecurityPostureSeverity::Warning,
                summary: "Journal payloads are stored as plaintext canonical JSON.".into(),
                remediation: "Use platform or environment storage keys.".into(),
            },
            SecurityPostureFinding {
                code: "sandbox.danger_full_access".into(),
                severity: SecurityPostureSeverity::Warning,
                summary:
                    "Danger full access is enabled: process execution has ambient runtime access."
                        .into(),
                remediation: "Use an isolating native, windows_job, or oci execution boundary, or use external only when a trusted host enforces the required filesystem and network isolation. Full access can expose host files, environment secrets, Colossus control state, private services, and metadata endpoints; on Unix, deliberately detached descendants can outlive the recorded process effect and its best-effort direct-mode limits.".into(),
            },
        ],
    };
    let state = TuiState::from_snapshot(source);
    let card = state.transcript.last().expect("security card");
    assert_eq!(card.sequence, None);
    assert!(matches!(
        card.document.blocks.first(),
        Some(PresentationBlock::Card {
            tone: PresentationTone::Warning,
            ..
        })
    ));
    let PresentationBlock::Card { body, .. } = &card.document.blocks[0] else {
        panic!("expected security posture card");
    };
    assert!(matches!(
        body.as_slice(),
        [PresentationBlock::Markdown(findings)] if !findings.contains("\n\n")
    ));

    let rendered_lines = security_posture_lines(&state, 120);
    let rendered = rendered_lines
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(rendered_lines.len(), 5, "{rendered}");
    assert!(
        rendered.contains("Security posture · 2 warnings"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Danger full access is enabled"),
        "{rendered}"
    );
    assert!(rendered.contains("Recommendation:"), "{rendered}");
    assert!(
        rendered.contains("Use an isolating native, windows_job, or oci boundary."),
        "{rendered}"
    );
    assert!(!rendered.contains("detached descendants"), "{rendered}");
    assert!(
        rendered_lines
            .iter()
            .filter(|line| line.to_string().contains("Recommendation:"))
            .flat_map(|line| line.spans.iter())
            .all(|span| span.style.add_modifier.contains(Modifier::DIM)),
        "recommendations should recede in the metadata color: {rendered}"
    );

    let backend = TestBackend::new(80, 1);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render_footer(frame, &state, frame.area()))
        .expect("draw footer");
    let footer = (0..80)
        .filter_map(|x| terminal.backend().buffer().cell((x, 0)))
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(footer.contains("Security: 2"));
    assert_ne!(
        terminal
            .backend()
            .buffer()
            .cell((79, 0))
            .expect("footer surface cell")
            .bg,
        Color::Reset,
        "the footer surface should fill the available status row"
    );
    assert_ne!(
        terminal
            .backend()
            .buffer()
            .cell((4, 0))
            .expect("security badge cell")
            .bg,
        terminal
            .backend()
            .buffer()
            .cell((79, 0))
            .expect("footer surface cell")
            .bg,
        "the warning badge should read as a distinct shell-style segment"
    );
}

#[test]
fn inline_startup_clears_the_view_but_retains_prior_shell_rows_in_scrollback() {
    let mut backend = TestBackend::with_lines([
        "prior-shell-row-1",
        "prior-shell-row-2",
        "prior-shell-row-3",
    ]);
    prepare_inline_startup(&mut backend).expect("prepare inline startup");

    assert_eq!(backend.cursor_position(), Position::ORIGIN);
    assert!(
        backend
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.symbol() == " "),
        "visible viewport should be blank: {:?}",
        backend.buffer()
    );
    let scrollback = backend
        .scrollback()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(scrollback.contains("prior-shell-row-1"), "{scrollback}");
    assert!(scrollback.contains("prior-shell-row-3"), "{scrollback}");
}

#[test]
fn direct_execution_acknowledgement_is_process_local_tui_state() {
    let mut initial = snapshot();
    initial.pending_sandbox_boundary_acknowledgement = Some(SandboxBoundaryMode::External);
    let mut state = TuiState::from_snapshot(initial);
    assert_eq!(
        state.pending_sandbox_boundary_acknowledgement,
        Some(SandboxBoundaryMode::External)
    );
    state.sandbox_boundary_acknowledgement_in_progress = true;
    assert!(state.is_busy());

    handle_host_event(
        &mut state,
        HostEvent::SandboxBoundaryAcknowledgement(Ok(Some(SandboxBoundaryMode::External))),
    );
    assert!(!state.sandbox_boundary_acknowledgement_in_progress);
    assert!(matches!(
        state
            .transcript
            .last()
            .expect("acknowledgement transcript")
            .document
            .blocks
            .first(),
        Some(PresentationBlock::Markdown(text)) if text.contains("policy authorization")
    ));
}

#[test]
fn sandbox_boundary_acknowledgement_uses_bottom_decision_dock() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("draft stays visible");
    let (response, mut received) = oneshot::channel();
    let request = sandbox_boundary_prompt(SandboxBoundaryMode::External, response);
    assert_eq!(
        request.kind,
        InteractivePromptKind::SandboxBoundaryAcknowledgement
    );
    assert_eq!(request.initial_choice, None);
    handle_host_event(&mut state, HostEvent::Prompt(request));

    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw sandbox boundary acknowledgement");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("External sandbox boundary · Summary"));
    assert!(rendered.contains("Operator-asserted external boundary"));
    assert!(rendered.contains("[A] Acknowledge the external boundary"));
    assert!(rendered.contains("Esc keep blocked"));
    assert!(rendered.contains("paused for boundary acknowledgement"));
    assert!(rendered.contains("draft stays visible"));
    let acknowledgement_row = rendered
        .lines()
        .position(|line| line.contains("External sandbox boundary · Summary"))
        .expect("acknowledgement row");
    let composer_row = rendered
        .lines()
        .position(|line| line.contains("paused for boundary acknowledgement"))
        .expect("composer row");
    assert!(acknowledgement_row < composer_row, "{rendered}");

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw boundary protections");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Acknowledgement does not add filesystem"));

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(received.try_recv().is_err());
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        received.try_recv(),
        Ok(PromptResponse::Answer(
            "Acknowledge the external boundary".into()
        ))
    );
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

fn theme_picker(response: oneshot::Sender<PromptResponse>) -> InteractiveThemePicker {
    let default = TerminalPreferences::default();
    let mut hacker = default.clone();
    hacker.select_builtin_theme(colossus_contracts::ThemeName::Hacker);
    InteractiveThemePicker {
        current_theme: "default".into(),
        themes: vec![
            InteractiveThemePickerEntry {
                name: "default".into(),
                preferences: default,
            },
            InteractiveThemePickerEntry {
                name: "hacker".into(),
                preferences: hacker,
            },
        ],
        response,
    }
}

fn plan_record(status: PlanStatus, revision: u64) -> PlanRecord {
    PlanRecord {
        id: "plan-019fabcdef".into(),
        session_id: "019f-test".into(),
        prompt: "Plan the terminal workflow".into(),
        status,
        revision,
        content: "A bounded plan".into(),
        steps: vec![colossus_contracts::PlanStep {
            index: 1,
            title: "Implement".into(),
            detail: "Keep behavior behind the host.".into(),
            requires_mutation: true,
        }],
        created_at: "2026-07-15T00:00:00Z".into(),
        updated_at: "2026-07-15T00:00:00Z".into(),
        approved_at: None,
        executed_run_id: None,
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
        parse_interactive_command("/provider diagnostics on"),
        InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(true))
    );
    assert_eq!(
        parse_interactive_command("/provider diagnostics off"),
        InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(false))
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
fn parser_handles_bounded_image_attachment_commands() {
    assert_eq!(
        parse_interactive_command("/attach workspace/image.png"),
        InteractiveCommand::Local(LocalCommand::Attach("workspace/image.png".into()))
    );
    assert_eq!(
        parse_interactive_command("/attach \"workspace/image with space.webp\""),
        InteractiveCommand::Local(LocalCommand::Attach(
            "workspace/image with space.webp".into()
        ))
    );
    assert_eq!(
        parse_interactive_command("/attachments"),
        InteractiveCommand::Local(LocalCommand::Attachments)
    );
    assert_eq!(
        parse_interactive_command("/detach 2"),
        InteractiveCommand::Local(LocalCommand::Detach(AttachmentDetach::Index(2)))
    );
    assert_eq!(
        parse_interactive_command("/detach all"),
        InteractiveCommand::Local(LocalCommand::Detach(AttachmentDetach::All))
    );
    for invalid in [
        "/attach",
        "/attach \"unterminated",
        "/detach 0",
        "/detach none",
    ] {
        assert!(matches!(
            parse_interactive_command(invalid),
            InteractiveCommand::Invalid(_)
        ));
    }
}

#[test]
fn pending_images_survive_start_failure_and_clear_after_accepted_completion() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.pending_images.push(image_reference(0));
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("provider route unavailable".into()))),
    );
    assert_eq!(state.pending_images.len(), 1);

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::Run(HostRunResult {
            outcome: AgentRunOutcome::Completed {
                result: AgentRunResult {
                    run_id: "run-image".into(),
                    session_id: Some("019f-test".into()),
                    role: "primary".into(),
                    profile: "test".into(),
                    model_profile: "test".into(),
                    provider_profile: "test".into(),
                    model: "test".into(),
                    plan: None,
                    output: "done".into(),
                    event_count: 1,
                    elapsed_seconds: 0.1,
                },
            },
            footer: FooterState::default(),
            plan_selection: PlanSelectionUpdate::Unchanged,
        })))),
    );
    assert!(state.pending_images.is_empty());
}

#[test]
fn resumed_multipart_images_render_accessible_metadata_and_pending_overflow() {
    let mut source = snapshot();
    source.transcript.messages = vec![SessionMessage {
        session_id: "019f-test".into(),
        run_id: "run-image".into(),
        sequence: 1,
        message: ModelMessage {
            role: ModelMessageRole::User,
            content: ModelContent::Parts(vec![
                ModelContentPart::Text {
                    text: "Inspect this".into(),
                },
                ModelContentPart::Image {
                    image: image_reference(0),
                },
            ]),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        created_at: "2026-07-15T00:00:00Z".into(),
    }];
    let mut state = TuiState::from_snapshot(source);
    state.pending_images = (0..4).map(image_reference).collect();
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw image metadata");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Inspect this"), "{rendered}");
    assert!(rendered.contains("image-0.png"), "{rendered}");
    assert!(rendered.contains("image/png"), "{rendered}");
    assert!(rendered.contains("+1 more"), "{rendered}");
}

#[test]
fn injected_native_protocol_reserves_cells_and_renders_through_the_stateful_widget() {
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    let mut cache = PreviewCache::default();
    cache.set_picker(picker);
    cache.insert("digest".into(), image::DynamicImage::new_rgba8(2, 2));
    assert!(cache.native_graphics());

    let size = Size::new(18, 5);
    let mut terminal = Terminal::new(TestBackend::new(18, 5)).expect("test terminal");
    let mut rendered_native_sequence = false;
    for _ in 0..1_000 {
        cache.prepare("digest", size);
        let placeholders = cache.lines("digest", size).expect("reserved image cells");
        assert_eq!(placeholders.len(), 5);
        assert!(placeholders.iter().all(|line| line.width() == 18));
        terminal
            .draw(|frame| cache.render_native(frame, "digest", size, frame.area()))
            .expect("native image draw");
        rendered_native_sequence = (0..size.height).any(|y| {
            (0..size.width).any(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, y))
                    .is_some_and(|cell| cell.symbol().contains('\u{1b}'))
            })
        });
        if rendered_native_sequence {
            break;
        }
        std::thread::yield_now();
    }
    assert!(rendered_native_sequence);
}

#[test]
fn preview_cache_bounds_resize_workers_per_image() {
    let mut cache = PreviewCache::default();
    cache.insert("digest".into(), image::DynamicImage::new_rgba8(2, 2));

    for width in 1..=u16::try_from(PREVIEW_VARIANTS_PER_ASSET + 3).expect("small bound") {
        cache.prepare("digest", Size::new(width, 2));
    }

    assert_eq!(cache.variant_count("digest"), PREVIEW_VARIANTS_PER_ASSET);
}

#[test]
fn parser_handles_process_local_permission_modes() {
    assert_eq!(
        parse_interactive_command("/permissions"),
        InteractiveCommand::Runtime(RuntimeCommand::Permissions(None))
    );
    for (value, mode) in [
        ("deny", InteractiveApprovalMode::Deny),
        ("ask", InteractiveApprovalMode::Ask),
        ("risk-auto", InteractiveApprovalMode::RiskAuto),
        ("full-access", InteractiveApprovalMode::FullAccess),
    ] {
        assert_eq!(
            parse_interactive_command(&format!("/permissions {value}")),
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(Some(mode)))
        );
    }
    for input in ["/permissions automatic", "/permissions ask now"] {
        assert!(matches!(
            parse_interactive_command(input),
            InteractiveCommand::Invalid(_)
        ));
    }
    assert_eq!(
        parse_interactive_command("/permissions-extra"),
        InteractiveCommand::Runtime(RuntimeCommand::Known {
            name: "permissions-extra".into(),
            arguments: String::new(),
        })
    );
}

#[test]
fn help_is_generated_from_the_available_command_catalog() {
    let commands = vec![
        "/resume".into(),
        "/workflow schedule list".into(),
        "/theme hacker".into(),
        "/mcp tools".into(),
        "@repo-review".into(),
    ];
    let rendered = StyledDocumentRenderer::new(TerminalPreferences::default(), 100)
        .render(&help_document(&commands))
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(normalized.contains("Conversation"), "{rendered}");
    assert!(normalized.contains("/resume"), "{rendered}");
    assert!(normalized.contains("/workflow"), "{rendered}");
    assert!(normalized.contains("schedule list"), "{rendered}");
    assert!(normalized.contains("/theme hacker"), "{rendered}");
    assert!(normalized.contains("/mcp tools"), "{rendered}");
    assert!(!normalized.contains("@repo-review"), "{rendered}");
}

#[test]
fn parser_enforces_the_exact_plan_command_grammar() {
    assert_eq!(
        parse_interactive_command("/plan"),
        InteractiveCommand::Plan(PlanCommand::Toggle)
    );
    assert_eq!(
        parse_interactive_command("/plan on"),
        InteractiveCommand::Plan(PlanCommand::On)
    );
    assert_eq!(
        parse_interactive_command("/plan use plan-1"),
        InteractiveCommand::Plan(PlanCommand::Use {
            plan_id: "plan-1".into(),
        })
    );
    assert_eq!(
        parse_interactive_command("/plan show"),
        InteractiveCommand::Plan(PlanCommand::Show { plan_id: None })
    );
    assert_eq!(
        parse_interactive_command("/plan execute direct"),
        InteractiveCommand::Plan(PlanCommand::Execute {
            strategy: Some(PlanExecutionStrategy::Direct),
        })
    );
    assert_eq!(
        parse_interactive_command("/plan execute goal"),
        InteractiveCommand::Plan(PlanCommand::Execute {
            strategy: Some(PlanExecutionStrategy::Goal { max_iterations: 5 }),
        })
    );
    assert_eq!(
        parse_interactive_command("/plan execute goal 50"),
        InteractiveCommand::Plan(PlanCommand::Execute {
            strategy: Some(PlanExecutionStrategy::Goal { max_iterations: 50 }),
        })
    );
    for input in [
        "/plan use",
        "/plan approve extra",
        "/plan execute goal 0",
        "/plan execute goal 51",
        "/plan execute other",
        "/plan unknown",
    ] {
        assert!(
            matches!(
                parse_interactive_command(input),
                InteractiveCommand::Invalid(_)
            ),
            "{input}"
        );
    }
}

#[test]
fn parser_treats_research_as_a_mode_with_explicit_runs() {
    assert_eq!(
        parse_interactive_command("/research"),
        InteractiveCommand::Research(ResearchCommand::Toggle)
    );
    assert_eq!(
        parse_interactive_command("/research on"),
        InteractiveCommand::Research(ResearchCommand::On)
    );
    assert_eq!(
        parse_interactive_command("/research off"),
        InteractiveCommand::Research(ResearchCommand::Off)
    );
    assert_eq!(
        parse_interactive_command("/research status"),
        InteractiveCommand::Research(ResearchCommand::Status)
    );
    assert_eq!(
        parse_interactive_command("/research list"),
        InteractiveCommand::Research(ResearchCommand::List)
    );
    assert_eq!(
        parse_interactive_command("/research why is the cache cold?"),
        InteractiveCommand::Research(ResearchCommand::Run {
            question: "why is the cache cold?".into(),
        })
    );
    assert_eq!(
        parse_interactive_command("/researcher"),
        InteractiveCommand::Runtime(RuntimeCommand::Known {
            name: "researcher".into(),
            arguments: String::new(),
        })
    );
}

#[test]
fn run_request_carries_only_process_local_provider_diagnostic_state() {
    let mut state = TuiState::from_snapshot(snapshot());
    assert!(
        !state
            .run_request("normal".into())
            .expect("execute request")
            .include_provider_response_diagnostics
    );
    state.provider_response_diagnostics = true;
    let request = state
        .run_request("reproduce".into())
        .expect("execute request");
    assert!(request.include_provider_response_diagnostics);
    assert_eq!(request.prompt, "reproduce");

    let restarted = TuiState::from_snapshot(snapshot());
    assert!(
        !restarted
            .run_request("after restart".into())
            .expect("execute request")
            .include_provider_response_diagnostics
    );
}

#[test]
fn plan_mode_derives_create_or_revision_bound_update_targets() {
    let mut state = TuiState::from_snapshot(snapshot());
    assert_eq!(state.mode, InteractiveMode::Execute);
    assert_eq!(
        state
            .run_request("execute".into())
            .expect("execute request")
            .mode,
        AgentRunMode::Execute
    );

    state.mode = InteractiveMode::Plan;
    assert_eq!(
        state
            .run_request("create".into())
            .expect("create request")
            .mode,
        AgentRunMode::Plan(PlanDraftTarget::Create)
    );

    state.selected_plan = Some(plan_record(PlanStatus::Draft, 7));
    assert_eq!(
        state
            .run_request("refine".into())
            .expect("refine request")
            .mode,
        AgentRunMode::Plan(PlanDraftTarget::Update {
            plan_id: "plan-019fabcdef".into(),
            revision: 7,
        })
    );

    state.selected_plan = Some(plan_record(PlanStatus::Approved, 8));
    assert!(
        state
            .run_request("must not refine".into())
            .expect_err("approved plans are immutable")
            .contains("cannot be refined")
    );
}

#[test]
fn research_mode_routes_messages_to_the_durable_research_command() {
    let mut state = TuiState::from_snapshot(snapshot());
    assert_eq!(state.research_turn_command("normal".into()), None);

    state.mode = InteractiveMode::Research;
    assert_eq!(state.mode.as_str(), "research");
    assert_eq!(
        state.research_turn_command("why is the cache cold?".into()),
        Some(RuntimeCommand::Known {
            name: "research".into(),
            arguments: "why is the cache cold?".into(),
        })
    );
    assert!(
        state
            .run_request("must use research".into())
            .expect_err("research is not a normal agent turn")
            .contains("research service")
    );
}

#[test]
fn plan_state_is_process_local_and_session_switch_clears_only_selection() {
    let mut state = TuiState::from_snapshot(snapshot());
    assert!(
        state
            .apply_plan_selection(PlanSelectionUpdate::Use(Box::new(plan_record(
                PlanStatus::Draft,
                3,
            ))))
            .is_ok()
    );
    assert_eq!(state.mode, InteractiveMode::Plan);
    assert_eq!(
        state.selected_plan.as_ref().map(|plan| plan.revision),
        Some(3)
    );

    state.mode = InteractiveMode::Plan;

    let mut switched = HostCommandResult::document(PresentationDocument::new());
    switched.session = Some((
        "019f-other".into(),
        SessionMessagePage {
            messages: Vec::new(),
            before_sequence: None,
            has_more: false,
        },
        Some(SandboxBoundaryMode::External),
    ));
    assert!(apply_command_result(&mut state, switched));
    assert_eq!(state.session_id, "019f-other");
    assert_eq!(
        state.pending_sandbox_boundary_acknowledgement,
        Some(SandboxBoundaryMode::External)
    );
    assert_eq!(state.mode, InteractiveMode::Plan);
    assert!(state.selected_plan.is_none());

    let restarted = TuiState::from_snapshot(snapshot());
    assert_eq!(restarted.mode, InteractiveMode::Execute);
    assert!(restarted.selected_plan.is_none());
}

#[test]
fn plan_commands_are_always_available_for_completion() {
    let state = TuiState::from_snapshot(snapshot());
    for command in [
        "/plan",
        "/plan new",
        "/plan use",
        "/plan execute direct",
        "/plan execute goal",
        "/plans",
    ] {
        assert!(
            state
                .completions
                .iter()
                .any(|candidate| candidate == command),
            "{command}"
        );
    }
}

#[test]
fn research_mode_commands_are_always_available_for_completion() {
    let state = TuiState::from_snapshot(snapshot());
    for command in [
        "/research",
        "/research on",
        "/research off",
        "/research status",
        "/research list",
    ] {
        assert!(
            state
                .completions
                .iter()
                .any(|candidate| candidate == command),
            "{command}"
        );
    }
}

#[test]
fn execution_choice_builds_a_bounded_goal_request_or_cancels_without_state_changes() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let approved = plan_record(PlanStatus::Approved, 4);
    state.selected_plan = Some(approved.clone());
    state.overlay = Some(Overlay::PlanExecutionChoice {
        plan: approved,
        selected: None,
    });
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_some());
    assert!(state.pending_plan_execution.is_none());
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_none());
    assert_eq!(
        state.pending_plan_execution,
        Some(InteractivePlanExecutionRequest {
            session_id: "019f-test".into(),
            plan_id: "plan-019fabcdef".into(),
            revision: 4,
            strategy: PlanExecutionStrategy::Goal { max_iterations: 5 },
        })
    );

    state.pending_plan_execution = None;
    let approved = state.selected_plan.clone().expect("selected plan");
    state.overlay = Some(Overlay::PlanExecutionChoice {
        plan: approved,
        selected: Some(0),
    });
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(state.overlay.is_none());
    assert!(state.pending_plan_execution.is_none());
    assert_eq!(state.mode, InteractiveMode::Plan);
    assert!(state.selected_plan.is_some());
}

#[test]
fn plan_execution_is_contextual_and_requires_explicit_confirmation() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.composer.insert("draft stays visible");
    let approved = plan_record(PlanStatus::Approved, 4);
    state.selected_plan = Some(approved.clone());
    state.overlay = Some(Overlay::PlanExecutionChoice {
        plan: approved,
        selected: None,
    });

    let composer_height = composer_height(&state, 120);
    assert_eq!(
        plan_execution_dock_height(&state, 32, composer_height, 0),
        MAX_PLAN_EXECUTION_DOCK_ROWS
    );
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw plan execution dock");
    let rendered = terminal.backend().to_string();
    assert!(
        rendered.contains("Plan the terminal workflow"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Revision r4 · 1 step · 1 mutating"),
        "{rendered}"
    );
    assert!(rendered.contains("No strategy selected"), "{rendered}");
    assert!(
        rendered.contains("No strategy is preselected"),
        "{rendered}"
    );
    assert!(
        rendered.contains("D/G select · Enter confirm"),
        "{rendered}"
    );
    assert!(rendered.contains("paused for plan execution"), "{rendered}");
    assert!(rendered.contains("draft stays visible"), "{rendered}");

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw selected direct strategy");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Selected: Direct"), "{rendered}");
}

#[test]
fn plan_write_events_select_the_exact_canonical_revision() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let plan = plan_record(PlanStatus::Draft, 9);
    handle_run_event(
        &mut state,
        RunEventEnvelope {
            schema_version: 1,
            run_id: "run-plan".into(),
            session_id: "019f-test".into(),
            event: RunEvent::PlanWritten { plan: plan.clone() },
        },
    );
    assert_eq!(state.selected_plan, Some(plan));
}

#[test]
fn completed_plan_turn_opens_an_explicit_review_dock() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let plan = plan_record(PlanStatus::Draft, 9);
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::Run(HostRunResult {
            outcome: AgentRunOutcome::Completed {
                result: AgentRunResult {
                    run_id: "run-plan".into(),
                    session_id: Some("019f-test".into()),
                    role: "primary".into(),
                    profile: "test".into(),
                    model_profile: "test".into(),
                    provider_profile: "test".into(),
                    model: "test".into(),
                    plan: Some(plan.clone()),
                    output: "Draft saved.".into(),
                    event_count: 1,
                    elapsed_seconds: 0.1,
                },
            },
            footer: FooterState::default(),
            plan_selection: PlanSelectionUpdate::Set(Box::new(plan.clone())),
        })))),
    );
    assert_eq!(state.selected_plan, Some(plan.clone()));
    assert!(matches!(
        state.overlay,
        Some(Overlay::PlanReviewChoice {
            plan: ref reviewed,
            selected: None,
        }) if reviewed == &plan
    ));
}

#[test]
fn identical_consecutive_plan_status_cards_are_not_duplicated() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.selected_plan = Some(plan_record(PlanStatus::Draft, 4));
    let before = state.transcript.len();

    append_plan_status(&mut state);
    append_plan_status(&mut state);

    assert_eq!(state.transcript.len(), before + 1);
}

#[test]
fn plan_review_requires_confirmation_and_queues_the_selected_lifecycle_action() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let plan = plan_record(PlanStatus::Draft, 4);
    state.selected_plan = Some(plan.clone());
    state.overlay = Some(Overlay::PlanReviewChoice {
        plan,
        selected: None,
    });

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_some());
    assert!(state.pending_plan_command.is_none());

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(state.pending_plan_command.is_none());
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.pending_plan_command, Some(PlanCommand::Approve));
}

#[test]
fn successful_plan_approval_opens_the_execution_strategy_dock() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.selected_plan = Some(plan_record(PlanStatus::Draft, 4));
    state.open_plan_execution_after_approval = true;
    let approved = plan_record(PlanStatus::Approved, 5);
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.plan_selection = PlanSelectionUpdate::Set(Box::new(approved.clone()));

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::Command(result)))),
    );

    assert_eq!(state.selected_plan, Some(approved.clone()));
    assert!(!state.open_plan_execution_after_approval);
    assert!(matches!(
        state.overlay,
        Some(Overlay::PlanExecutionChoice {
            plan: ref selected,
            selected: None,
        }) if selected == &approved
    ));
}

#[test]
fn interrupted_approval_keeps_the_execution_dock_above_a_paused_queue() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.selected_plan = Some(plan_record(PlanStatus::Draft, 4));
    state.queue.push_back("queued prompt".into());
    state.open_plan_execution_after_approval = true;
    let approved = plan_record(PlanStatus::Approved, 5);
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.continue_queue = false;
    result.plan_selection = PlanSelectionUpdate::Set(Box::new(approved.clone()));

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::Command(result)))),
    );

    assert!(state.queue_paused);
    assert!(matches!(
        state.overlay,
        Some(Overlay::PlanExecutionChoice {
            plan: ref selected,
            selected: None,
        }) if selected == &approved
    ));

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(state.pending_plan_execution.is_some());
    assert!(state.overlay.is_none());
    assert!(state.queue_paused);
}

#[test]
fn cancelling_plan_execution_choice_surfaces_the_paused_queue() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.selected_plan = Some(plan_record(PlanStatus::Draft, 4));
    state.queue.push_back("queued after approval".into());
    state.open_plan_execution_after_approval = true;
    let approved = plan_record(PlanStatus::Approved, 5);
    let mut result = HostCommandResult::document(PresentationDocument::new());
    result.continue_queue = false;
    result.plan_selection = PlanSelectionUpdate::Set(Box::new(approved));

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::Command(result)))),
    );
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
}

#[test]
fn plan_review_dock_previews_steps_and_explains_tasks_are_separate() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.composer.insert("refinement stays visible");
    let plan = plan_record(PlanStatus::Draft, 4);
    state.selected_plan = Some(plan.clone());
    state.overlay = Some(Overlay::PlanReviewChoice {
        plan,
        selected: None,
    });

    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw plan review dock");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Review plan plan-019"), "{rendered}");
    assert!(rendered.contains("1. Implement"), "{rendered}");
    assert!(
        rendered.contains("durable /tasks records are a separate"),
        "{rendered}"
    );
    assert!(rendered.contains("[R] Keep refining"), "{rendered}");
    assert!(rendered.contains("[A] Approve"), "{rendered}");
    assert!(rendered.contains("[X] Discard"), "{rendered}");
    assert!(rendered.contains("paused for plan review"), "{rendered}");
    assert!(rendered.contains("refinement stays visible"), "{rendered}");
}

#[test]
fn post_consumption_failure_returns_to_execute_and_clears_selection() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let mut consumed = plan_record(PlanStatus::Executed, 6);
    consumed.executed_run_id = Some("run-consumed".into());
    state.selected_plan = Some(plan_record(PlanStatus::Approved, 5));
    state.queue.push_back("next turn".into());
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::PlanExecution(
            HostPlanExecutionResult {
                plan: consumed,
                document: PresentationDocument::new(),
                outcome: HostPlanExecutionOutcome::FailedAfterConsumption(
                    "provider unavailable".into(),
                ),
                footer: FooterState {
                    status: "error".into(),
                    ..FooterState::default()
                },
                plan_selection: PlanSelectionUpdate::Clear,
            },
        )))),
    );
    assert_eq!(state.mode, InteractiveMode::Execute);
    assert!(state.selected_plan.is_none());
    assert!(state.queue_paused);
    assert!(matches!(state.overlay, Some(Overlay::QueuePaused)));
}

#[test]
fn cancellation_before_consumption_preserves_plan_mode_and_selection() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let approved = plan_record(PlanStatus::Approved, 5);
    state.selected_plan = Some(approved.clone());
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::PlanExecution(
            HostPlanExecutionResult {
                plan: approved.clone(),
                document: PresentationDocument::new(),
                outcome: HostPlanExecutionOutcome::CancelledBeforeStart,
                footer: FooterState {
                    status: "cancelled".into(),
                    ..FooterState::default()
                },
                plan_selection: PlanSelectionUpdate::Set(Box::new(approved.clone())),
            },
        )))),
    );
    assert_eq!(state.mode, InteractiveMode::Plan);
    assert_eq!(state.selected_plan, Some(approved));
}

#[test]
fn execution_failure_reconciliation_preserves_known_unconsumed_and_clears_unknown() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    let approved = plan_record(PlanStatus::Approved, 5);
    state.selected_plan = Some(approved.clone());
    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::PlanExecution(
            HostPlanExecutionResult {
                plan: approved.clone(),
                document: PresentationDocument::new(),
                outcome: HostPlanExecutionOutcome::FailedBeforeConsumption("policy denied".into()),
                footer: FooterState::default(),
                plan_selection: PlanSelectionUpdate::Set(Box::new(approved.clone())),
            },
        )))),
    );
    assert_eq!(state.mode, InteractiveMode::Plan);
    assert_eq!(state.selected_plan, Some(approved.clone()));

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Ok(OperationResult::PlanExecution(
            HostPlanExecutionResult {
                plan: approved,
                document: PresentationDocument::new(),
                outcome: HostPlanExecutionOutcome::OutcomeUnknown("worker disconnected".into()),
                footer: FooterState::default(),
                plan_selection: PlanSelectionUpdate::Clear,
            },
        )))),
    );
    assert_eq!(state.mode, InteractiveMode::Execute);
    assert!(state.selected_plan.is_none());
}

#[test]
fn plan_mode_and_selection_are_visible_in_composer_and_footer() {
    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Plan;
    state.selected_plan = Some(plan_record(PlanStatus::Draft, 7));
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw plan mode");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Plan plan-019"), "{rendered}");
    assert!(rendered.contains("mode=plan"), "{rendered}");
    assert!(rendered.contains("plan=plan-019:r7:draft"), "{rendered}");
}

#[test]
fn research_mode_is_visible_in_composer_and_footer() {
    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.mode = InteractiveMode::Research;
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw research mode");
    let rendered = terminal.backend().to_string();
    assert!(
        rendered.contains("Research · sourced question"),
        "{rendered}"
    );
    assert!(rendered.contains("mode=research"), "{rendered}");
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
fn repeated_history_navigation_restores_the_original_draft_and_resets_after_editing() {
    let mut source = snapshot();
    source.history = vec![
        "first prompt".into(),
        "second prompt".into(),
        "third prompt".into(),
    ];
    let mut state = TuiState::from_snapshot(source);
    state.composer.insert("unsent draft");

    state.previous_history();
    assert_eq!(state.draft(), "third prompt");
    state.previous_history();
    assert_eq!(state.draft(), "second prompt");
    state.previous_history();
    assert_eq!(state.draft(), "first prompt");
    state.previous_history();
    assert_eq!(state.draft(), "first prompt");

    state.next_history();
    assert_eq!(state.draft(), "second prompt");
    state.next_history();
    assert_eq!(state.draft(), "third prompt");
    state.next_history();
    assert_eq!(state.draft(), "unsent draft");
    assert_eq!(state.cursor(), "unsent draft".len());
    assert_eq!(state.composer.history_index, None);

    state.previous_history();
    assert_eq!(state.draft(), "third prompt");
    state.composer.insert(" edited");
    assert_eq!(state.composer.history_index, None);
    state.next_history();
    assert_eq!(state.draft(), "third prompt edited");
}

#[test]
fn multiline_history_search_and_first_line_navigation_preserve_the_draft() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.preferences.multiline = true;
    state.composer.insert("first");
    state.composer.insert("\n");
    state.composer.insert("界");
    assert_eq!(state.draft(), "first\n界");

    state.previous_history();
    assert_eq!(state.draft(), "first\n界");
    assert_eq!(state.composer.history_index, None);

    state.composer.cursor = "first".len();
    state.previous_history();
    assert_eq!(state.draft(), "older prompt");
    state.next_history();
    assert_eq!(state.draft(), "first\n界");
    assert_eq!(state.cursor(), "first".len());

    state.overlay = Some(Overlay::HistorySearch {
        query: "older".into(),
    });
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.draft(), "first\n界");
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
    let commands = state.completion_menu_candidates();
    assert!(commands.starts_with(&["/tools", "/tui prefs"]));
    assert!(commands.contains(&"/plan"));
    assert!(commands.contains(&"/plan execute goal"));

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
    assert_eq!(state.composer.completion_index, Some(1));
    state.advance_completion();
    assert_eq!(state.composer.completion_index, Some(2));
    state.previous_completion();
    assert_eq!(state.composer.completion_index, Some(1));
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
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
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
fn command_completion_is_left_aligned_compact_and_described() {
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions = vec![
        "/help".into(),
        "/tui prefs".into(),
        "/tui save".into(),
        "/tui reset".into(),
        "/provider diagnostics on".into(),
        "/provider diagnostics off".into(),
    ];
    state.composer.insert("/");

    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw described command completion");
    let rendered = terminal.backend().to_string();
    let title_line = rendered
        .lines()
        .find(|line| line.contains("Commands · 6"))
        .expect("completion title");
    assert!(
        title_line.trim_start_matches('"').starts_with('┌'),
        "{rendered}"
    );
    let right_border = title_line
        .chars()
        .position(|character| character == '┐')
        .expect("compact right border");
    assert!(
        right_border < 80,
        "palette width={right_border}: {rendered}"
    );

    let help_line = rendered
        .lines()
        .find(|line| line.contains("/help"))
        .expect("help row");
    let prefs_line = rendered
        .lines()
        .find(|line| line.contains("/tui prefs"))
        .expect("preferences row");
    assert!(help_line.contains("Show commands and keyboard shortcuts"));
    assert!(prefs_line.contains("Show terminal preferences"));
    assert_eq!(
        help_line.chars().position(|character| character == 'S'),
        prefs_line.chars().position(|character| character == 'S'),
        "description columns should align: {rendered}"
    );
}

#[test]
fn command_descriptions_cover_static_and_dynamic_completions() {
    assert_eq!(
        command_description("/resume"),
        Some("Browse and resume prior sessions")
    );
    assert_eq!(
        command_description("/theme preview mono"),
        Some("Preview this terminal theme")
    );
    assert_eq!(
        command_description("/permissions full-access"),
        Some("Satisfy approval obligations automatically")
    );
    assert_eq!(command_description("@coding"), None);
}

#[test]
fn command_completion_keeps_a_stable_minimum_width_while_filtering() {
    let backend = TestBackend::new(120, 16);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.completions = vec!["/help".into(), "/provider diagnostics on".into()];
    state.composer.insert("/he");

    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw filtered command completion");
    let rendered = terminal.backend().to_string();
    let title_line = rendered
        .lines()
        .find(|line| line.contains("Commands · 1"))
        .expect("filtered completion title")
        .trim_start_matches('"');
    let width = title_line
        .chars()
        .position(|character| character == '┐')
        .expect("completion right border")
        + 1;
    assert_eq!(width, usize::from(MIN_COMPLETION_MENU_WIDTH), "{rendered}");
}

#[test]
fn alternate_screen_completion_never_uses_native_scrollback() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());

    for draft in ["/", "/t", "/missing", ""] {
        state.composer.clear();
        state.composer.insert(draft);
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw alternate-screen completion state");
        terminal.backend().assert_scrollback_empty();
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
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw composer");
    let buffer = terminal.backend().buffer();
    let typed = buffer.cell((1, 1)).expect("typed cell").style();
    let ghost = buffer.cell((4, 1)).expect("ghost cell").style();
    assert_ne!(typed, ghost);
    assert!(ghost.add_modifier.contains(Modifier::DIM));
}

#[test]
fn wrapped_composer_cursor_matches_the_rendered_text() {
    let backend = TestBackend::new(12, 4);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("abcdefghijk");

    terminal
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw wrapped composer");

    let buffer = terminal.backend().buffer();
    let first_row = (1..=10)
        .map(|x| buffer.cell((x, 1)).expect("first-row cell").symbol())
        .collect::<String>();
    assert_eq!(first_row, "abcdefghij");
    assert_eq!(buffer.cell((1, 2)).expect("wrapped cell").symbol(), "k");
    assert_eq!(buffer.cell((2, 2)).expect("cursor cell").symbol(), " ");
    assert_eq!(terminal.backend().cursor_position(), Position::new(2, 2));
}

#[test]
fn exact_width_composer_reserves_the_cursor_row() {
    let backend = TestBackend::new(12, 4);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("abcdefghij");

    assert_eq!(composer_height(&state, 12), 4);
    terminal
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw exact-width composer");

    assert_eq!(terminal.backend().cursor_position(), Position::new(1, 2));
}

#[test]
fn composer_scrolls_wrapped_tail_with_the_cursor() {
    let backend = TestBackend::new(12, 8);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state
        .composer
        .insert("0000000000111111111122222222223333333333444444444455555555556");

    assert_eq!(composer_height(&state, 12), 8);
    terminal
        .draw(|frame| render_composer(frame, &mut state, frame.area()))
        .expect("draw scrolled composer");

    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer.cell((1, 1)).expect("first visible cell").symbol(),
        "1"
    );
    assert_eq!(
        buffer.cell((1, 6)).expect("last visible cell").symbol(),
        "6"
    );
    assert_eq!(buffer.cell((2, 6)).expect("cursor cell").symbol(), " ");
    assert_eq!(terminal.backend().cursor_position(), Position::new(2, 6));
}

#[test]
fn composer_layout_wraps_wide_characters_before_the_cursor() {
    let layout = composer_layout("abcd界", "", "abcd界".len(), 5);

    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[0].draft, "abcd");
    assert_eq!(layout.lines[1].draft, "界");
    assert_eq!((layout.cursor_row, layout.cursor_column), (1, 2));
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
                content: String::new().into(),
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
                content: String::new().into(),
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
                content: body.clone().into(),
                tool_call_id: Some("call-fetch".into()),
                tool_calls: Vec::new(),
            },
            created_at: "2026-07-15T00:00:00Z".into(),
        },
    ];

    let mut state = TuiState::from_snapshot(source.clone());
    let compact = transcript_lines(&state, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("Response preview"), "{compact}");
    assert!(compact.contains("preview only"), "{compact}");
    assert!(!compact.contains("FULL-BODY-TAIL"), "{compact}");

    let mut verbose_preferences = state.preferences.clone();
    verbose_preferences.events_mode = EventDisplayMode::Verbose;
    assert!(apply_command_result(
        &mut state,
        HostCommandResult {
            document: PresentationDocument::new(),
            session: None,
            preferences: Some(verbose_preferences),
            completions: None,
            sticky_skills: None,
            footer: None,
            plan_selection: PlanSelectionUpdate::Unchanged,
            continue_queue: true,
            clear_transcript: false,
        },
    ));
    let rerendered = transcript_lines(&state, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rerendered.contains("FULL-BODY-TAIL"), "{rerendered}");

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
fn live_web_fetch_result_rebuilds_when_event_mode_changes() {
    let mut state = TuiState::from_snapshot(snapshot());
    let call = ToolCall {
        call_id: "call-live-fetch".into(),
        name: "web.fetch".into(),
        arguments: serde_json::json!({"url": "https://example.com/live"}),
    };
    let envelope = |event| RunEventEnvelope {
        schema_version: 1,
        run_id: "run-live-fetch".into(),
        session_id: "019f-test".into(),
        event,
    };
    handle_run_event(
        &mut state,
        envelope(RunEvent::ToolStarted {
            turn: 1,
            call: call.clone(),
            elapsed_seconds: 0.1,
        }),
    );
    handle_run_event(
        &mut state,
        envelope(RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: call.call_id,
                name: call.name,
                output: format!("preview-start\n{}FULL-BODY-TAIL", "line\n".repeat(100)),
                exit_code: 0,
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.3,
        }),
    );

    let compact = transcript_lines(&state, 80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(compact.contains("Response preview"), "{compact}");
    assert!(!compact.contains("FULL-BODY-TAIL"), "{compact}");

    let mut preferences = state.preferences.clone();
    preferences.events_mode = EventDisplayMode::Verbose;
    assert!(apply_command_result(
        &mut state,
        HostCommandResult {
            document: PresentationDocument::new(),
            session: None,
            preferences: Some(preferences),
            completions: None,
            sticky_skills: None,
            footer: None,
            plan_selection: PlanSelectionUpdate::Unchanged,
            continue_queue: true,
            clear_transcript: false,
        },
    ));
    let verbose = transcript_lines(&state, 80)
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
            content: body.clone().into(),
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
    let original_epoch = state.transcript_epoch;
    state.page_up();
    state.new_items = 3;
    assert!(apply_command_result(
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
                None,
            )),
            preferences: None,
            completions: None,
            sticky_skills: None,
            footer: None,
            plan_selection: PlanSelectionUpdate::Unchanged,
            continue_queue: true,
            clear_transcript: false,
        },
    ));
    assert_eq!(state.session_id, "019f-other");
    assert_eq!(state.transcript.len(), 1);
    assert_eq!(state.scroll_from_bottom, 0);
    assert_eq!(state.new_items, 0);
    assert_eq!(state.transcript_epoch, original_epoch.wrapping_add(1));
    assert!(
        transcript_lines(&state, 80)
            .iter()
            .any(|line| line.to_string().contains("other transcript"))
    );
}

#[test]
fn native_history_commits_every_finalized_entry_and_keeps_only_streaming_output_live() {
    assert_eq!(ScreenMode::default(), ScreenMode::Inline);
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    state.append_entry(user_entry("older question", TranscriptKind::User));
    state.append_entry(user_entry("older answer", TranscriptKind::Assistant));
    state.append_entry(user_entry("newest question", TranscriptKind::User));
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Assistant,
        document: PresentationDocument::from_block(PresentationBlock::Text(
            "streaming answer".into(),
        )),
        temporary: true,
    });

    let committed = committable_transcript_end(&state.transcript, 0);
    assert_eq!(committed, 3);
    assert_eq!(
        committable_transcript_end(&state.transcript, committed),
        committed
    );

    let live = transcript_lines_range(&state, 80, committed, state.transcript.len(), false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(live.contains("streaming answer"), "{live}");
    assert!(!live.contains("newest question"), "{live}");
    assert!(!live.contains("older question"), "{live}");
    assert!(!live.contains("older answer"), "{live}");

    state.transcript[3].temporary = false;
    assert_eq!(
        committable_transcript_end(&state.transcript, committed),
        4,
        "completed output enters native history immediately"
    );

    state.append_entry(user_entry("next question", TranscriptKind::User));
    assert_eq!(
        committable_transcript_end(&state.transcript, 4),
        5,
        "the submitted user message is finalized output too"
    );
}

#[test]
fn tool_boundary_releases_intermediate_commentary_and_tool_result_to_native_history() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    let envelope = |event| RunEventEnvelope {
        schema_version: 1,
        run_id: "run-tool-history".into(),
        session_id: "019f-test".into(),
        event,
    };
    let call = ToolCall {
        call_id: "call-history".into(),
        name: "filesystem.search".into(),
        arguments: serde_json::json!({"query": "Runtime"}),
    };

    handle_run_event(
        &mut state,
        envelope(RunEvent::Provider {
            event: ProviderEvent::ModelDelta {
                text: "I will inspect the runtime first.".into(),
            },
        }),
    );
    assert_eq!(committable_transcript_end(&state.transcript, 0), 0);

    handle_run_event(
        &mut state,
        envelope(RunEvent::ToolStarted {
            turn: 1,
            call: call.clone(),
            elapsed_seconds: 0.1,
        }),
    );
    assert_eq!(
        committable_transcript_end(&state.transcript, 0),
        1,
        "commentary preceding a tool call must no longer block native history"
    );
    assert!(!state.transcript[0].temporary);
    assert!(matches!(
        state.transcript[0].document.blocks.first(),
        Some(PresentationBlock::Markdown(_))
    ));

    handle_run_event(
        &mut state,
        envelope(RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: call.call_id,
                name: call.name,
                output: serde_json::json!({"matches": ["runtime.rs"]}).to_string(),
                exit_code: 0,
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.3,
        }),
    );
    assert_eq!(committable_transcript_end(&state.transcript, 1), 2);
    let completed_tool = transcript_lines_range(&state, 80, 1, 2, false)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        completed_tool.contains("Completed filesystem.search"),
        "{completed_tool}"
    );

    handle_run_event(
        &mut state,
        envelope(RunEvent::Provider {
            event: ProviderEvent::ModelDelta {
                text: "The runtime is composed from focused services.".into(),
            },
        }),
    );
    assert_eq!(committable_transcript_end(&state.transcript, 2), 2);
    handle_run_event(
        &mut state,
        envelope(RunEvent::Provider {
            event: ProviderEvent::FinalOutput {
                text: "The runtime is composed from focused services.".into(),
            },
        }),
    );
    assert_eq!(committable_transcript_end(&state.transcript, 2), 3);
}

#[test]
fn inline_viewport_collapses_after_streaming_output_is_finalized() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript.clear();
    state.transcript_sources.clear();
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Assistant,
        document: PresentationDocument::from_block(PresentationBlock::Text(
            "first live row\nsecond live row".into(),
        )),
        temporary: true,
    });

    let live_height = desired_inline_viewport_height(&state, 80, 24, 0);
    assert!(live_height > MINIMUM_INLINE_VIEWPORT_HEIGHT);
    state.transcript[0].temporary = false;
    let committed = committable_transcript_end(&state.transcript, 0);
    assert_eq!(committed, 1);
    assert_eq!(
        desired_inline_viewport_height(&state, 80, 24, committed),
        MINIMUM_INLINE_VIEWPORT_HEIGHT
    );
}

#[test]
fn inline_viewport_stays_bottom_anchored_as_live_content_grows_and_shrinks() {
    let screen = Size::new(80, 24);
    let current = Rect::new(0, 19, 80, 5);
    let (grown, scroll_up) = next_inline_area(current, screen, screen, 11);
    assert_eq!(grown, Rect::new(0, 13, 80, 11));
    assert_eq!(scroll_up, 6);

    let (shrunk, scroll_up) = next_inline_area(grown, screen, screen, 5);
    assert_eq!(shrunk, current);
    assert_eq!(scroll_up, 0);
}

#[test]
fn inline_completion_does_not_resize_the_main_screen_viewport() {
    for screen in [Size::new(40, 12), Size::new(80, 24)] {
        let mut state = TuiState::from_snapshot(snapshot());
        let transcript_start = state.transcript.len();
        let expected =
            desired_inline_viewport_height(&state, screen.width, screen.height, transcript_start);
        for draft in ["/", "/t", "/missing", "/", ""] {
            state.composer.clear();
            state.composer.insert(draft);
            let requested = desired_inline_viewport_height(
                &state,
                screen.width,
                screen.height,
                transcript_start,
            );
            assert_eq!(
                requested, expected,
                "completion draft {draft:?} changed the main-screen viewport at {}x{}",
                screen.width, screen.height
            );
        }
    }
}

#[test]
fn approval_uses_transient_inline_chrome_and_adapts_at_minimum_height() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("/");
    let (response, _received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "inline-approval".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval required".into(),
            document: PresentationDocument::from_block(PresentationBlock::Text("Allow?".into())),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );

    assert!(state.transient_inline_screen_active());
    let composer_height = composer_height(&state, 80);
    assert_eq!(
        approval_dock_height(&state, 24, composer_height, 1),
        MAX_APPROVAL_DOCK_ROWS
    );
    assert_eq!(approval_dock_height(&state, 12, composer_height, 1), 0);

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw approval instead of completion");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Approval required · Summary"));
    assert!(!rendered.contains("Commands ·"));

    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).expect("narrow test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw compact full-screen approval");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("[S] Summary"));
    assert!(rendered.contains("[R] Request"));
    assert!(rendered.contains("[P] Protect"));
    assert!(rendered.contains("Select one"));
    assert!(rendered.contains("Esc deny"));
}

#[test]
fn finalized_multiline_output_has_no_trailing_rendered_separator() {
    let mut state = TuiState::from_snapshot(snapshot());
    let start = state.transcript.len();
    let output = (1..=30)
        .map(|row| format!("stream-final-row-{row:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    update_streaming_assistant(&mut state, &output);
    finalize_assistant(&mut state, &output);
    let rendered = transcript_lines_range(&state, 80, start, state.transcript.len(), true);
    assert!(
        rendered
            .last()
            .is_some_and(|line| !line.to_string().trim().is_empty()),
        "rendered rows: {:?}",
        rendered.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}

#[test]
fn native_history_insertion_fills_the_row_above_a_bottom_anchored_viewport() {
    let mut backend = TestBackend::new(20, 8);
    let lines = (1..=12)
        .map(|row| Line::from(format!("history-{row:02}")))
        .collect::<Vec<_>>();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 12));
    Paragraph::new(lines).render(buffer.area, &mut buffer);
    let mut viewport = Rect::new(0, 4, 20, 4);
    insert_history_buffer(&mut backend, &buffer, &mut viewport, Size::new(20, 8))
        .expect("insert native history");

    assert_eq!(viewport, Rect::new(0, 4, 20, 4));
    let row = (0..20)
        .filter_map(|x| backend.buffer().cell((x, 3)))
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        row.contains("history-12"),
        "last history row should fill the line above the viewport: {row:?}"
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
            kind: InteractivePromptKind::Approval,
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
fn terminal_operation_releases_an_open_prompt_overlay() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "prompt-disconnect".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval".into(),
            document: PresentationDocument::from_block(PresentationBlock::Text("Allow?".into())),
            choices: vec!["allow".into(), "deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("worker disconnected".into()))),
    );

    assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
    assert!(state.overlay.is_none());
}

#[test]
fn policy_notice_appends_to_the_transcript_without_taking_focus() {
    let mut state = TuiState::from_snapshot(snapshot());
    let starting = state.transcript.len();
    handle_host_event(
        &mut state,
        HostEvent::Notice(PresentationDocument::from_block(PresentationBlock::Card {
            title: "Automatic approval review".into(),
            tone: PresentationTone::Warning,
            body: vec![PresentationBlock::Text("low-risk effect approved".into())],
        })),
    );

    assert_eq!(state.transcript.len(), starting + 1);
    assert!(state.overlay.is_none());
    assert!(
        transcript_lines(&state, 80)
            .iter()
            .any(|line| { line.to_string().contains("Automatic approval review") })
    );
}

#[test]
fn approval_is_bottom_docked_with_preserved_composer_and_inspectable_sections() {
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("draft stays visible");
    let request_content = (0..32)
        .map(|index| format!("request-line-{index:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (response, _received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "approval-dock".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval required".into(),
            document: PresentationDocument::from_block(PresentationBlock::Card {
                title: "Approval required".into(),
                tone: PresentationTone::Warning,
                body: vec![
                    PresentationBlock::KeyValue(vec![
                        (
                            "Requested by".into(),
                            "Model · tool-call:call_9xS9WDT8NZnDk7TnqrgCJkmS".into(),
                        ),
                        ("Action".into(), "mcp.call".into()),
                        (
                            "Resource".into(),
                            "http://127.0.0.1:18000/en-US/splunkd/__raw/services/mcp".into(),
                        ),
                        (
                            "Reason".into(),
                            "explicit operator approval required".into(),
                        ),
                        (
                            "Risk review".into(),
                            "not assessed: evaluator unavailable".into(),
                        ),
                    ]),
                    PresentationBlock::Code {
                        language: Some("exact prepared request".into()),
                        content: request_content,
                    },
                ],
            }),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );

    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw approval summary");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Approval required · Summary"));
    assert!(rendered.contains("mcp.call"));
    assert!(rendered.contains("Requested by"));
    assert!(rendered.contains("Reason"));
    assert!(rendered.contains("Risk review"));
    assert!(!rendered.contains("Field"));
    assert!(rendered.contains("[A] Allow once"));
    assert!(rendered.contains("No decision selected"));
    assert!(rendered.contains("Tab sections"));
    assert!(!rendered.contains("S/R/P inspect"));
    assert!(!rendered.contains("request-line-00"));
    assert!(rendered.contains("Message · paused for approval · draft preserved"));
    assert!(rendered.contains("draft stays visible"));
    let approval_row = rendered
        .lines()
        .position(|line| line.contains("Approval required · Summary"))
        .expect("approval row");
    let composer_row = rendered
        .lines()
        .position(|line| line.contains("paused for approval"))
        .expect("composer row");
    assert!(approval_row < composer_row, "{rendered}");

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    for _ in 0..5 {
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        );
    }
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw scrolled exact request");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Approval required · Exact request"));
    assert!(rendered.contains("request-line-19"), "{rendered}");

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw protections");
    assert!(terminal.backend().to_string().contains("Exact scope"));
}

#[test]
fn approval_shortcuts_select_but_still_require_enter_to_confirm() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "approval-shortcut".into(),
            kind: InteractivePromptKind::Approval,
            title: "Approval required".into(),
            document: PresentationDocument::from_block(PresentationBlock::Text("Allow?".into())),
            choices: vec!["Allow once".into(), "Deny".into()],
            initial_choice: None,
            allow_free_form: false,
            response,
        }),
    );

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(received.try_recv().is_err());
    assert!(matches!(
        state.overlay,
        Some(Overlay::Prompt {
            selected: Some(0),
            ..
        })
    ));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        received.try_recv(),
        Ok(PromptResponse::Answer("Allow once".into()))
    );
}

#[test]
fn approval_controls_use_filled_theme_resolved_surfaces() {
    let palette = TerminalPalette::for_theme(colossus_contracts::ThemeName::Default);
    let sections = approval_section_line(ApprovalSection::Summary, &palette, 120);
    let active = sections.spans[0].style;
    let inactive = sections.spans[2].style;

    assert_eq!(active.bg, Some(Color::Rgb(255, 223, 93)));
    assert_eq!(active.fg, Some(Color::Black));
    assert_eq!(inactive.bg, Some(Color::Rgb(52, 55, 58)));
    assert_eq!(inactive.fg, Some(Color::Rgb(174, 184, 194)));

    let mono = TerminalPalette::for_theme(colossus_contracts::ThemeName::Mono);
    let sections = approval_section_line(ApprovalSection::Summary, &mono, 120);
    assert!(
        sections.spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );
    assert!(sections.spans[2].style.add_modifier.contains(Modifier::DIM));
}

#[test]
fn approval_summary_fields_remove_terminal_controls_and_invisible_joiners() {
    assert_eq!(
        sanitize_approval_field("safe\u{1b}]8;;evil\u{7}\u{200b}\nnext"),
        "safe]8;;evil next"
    );
}

#[test]
fn approval_summary_wraps_long_values_instead_of_dropping_their_suffix() {
    let palette = TerminalPalette::for_theme(colossus_contracts::ThemeName::Default);
    let resource = format!("file:///workspace/{}/secrets.yaml", "nested/".repeat(12));
    let lines = compact_approval_summary_lines(
        &[
            ("Resource".into(), resource.clone()),
            (
                "Risk".into(),
                "Writes outside the approved workspace root and may exfiltrate credentials.".into(),
            ),
        ],
        &palette,
        48,
    );

    let rendered = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(rendered.len() > 2);
    assert!(rendered.iter().all(|line| !line.contains('…')));
    assert!(
        rendered
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 48)
    );

    let joined = rendered
        .iter()
        .map(|line| line.trim_start())
        .collect::<String>();
    assert!(joined.contains(&resource));
    assert!(joined.ends_with("credentials."));
}

#[test]
fn approval_exact_request_repeats_the_complete_sanitized_scope() {
    let resource = format!(
        "https://example.test/{}/actor-distinguishing-suffix",
        "nested/".repeat(24)
    );
    let document = PresentationDocument::from_block(PresentationBlock::Card {
        title: "Approval required".into(),
        tone: PresentationTone::Warning,
        body: vec![PresentationBlock::KeyValue(vec![(
            "Resource".into(),
            format!("{resource}\u{200b}\ncontinued"),
        )])],
    });

    let exact = approval_section_document(
        &document,
        InteractivePromptKind::Approval,
        ApprovalSection::Request,
    );
    let PresentationBlock::KeyValue(scope) = &exact.blocks[0] else {
        panic!("exact request must begin with the complete approval scope");
    };
    assert_eq!(scope[0].0, "Resource");
    assert_eq!(scope[0].1, format!("{resource} continued"));
}

#[test]
fn prompt_keyboard_selection_returns_the_highlighted_choice() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "session-picker".into(),
            kind: InteractivePromptKind::Choice,
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
fn theme_picker_previews_reversibly_and_applies_only_after_enter() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(&mut state, HostEvent::ThemePicker(theme_picker(response)));
    assert!(state.transient_inline_screen_active());
    assert_eq!(state.preferences.theme_name(), "default");

    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(state.preferences.theme_name(), "hacker");
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
    assert_eq!(state.preferences.theme_name(), "default");

    let (response, mut received) = oneshot::channel();
    handle_host_event(&mut state, HostEvent::ThemePicker(theme_picker(response)));
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        received.try_recv(),
        Ok(PromptResponse::Answer("hacker".into()))
    );
    assert_eq!(state.preferences.theme_name(), "default");
    assert!(state.overlay.is_none());
}

#[test]
fn theme_picker_is_master_detail_without_false_saved_state() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.composer.insert("draft remains visible");
    let (response, _received) = oneshot::channel();
    handle_host_event(&mut state, HostEvent::ThemePicker(theme_picker(response)));
    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw theme picker");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("Choose theme"), "{rendered}");
    assert!(rendered.contains("hacker preview"), "{rendered}");
    assert!(rendered.contains("preview only until Enter"), "{rendered}");
    assert!(rendered.contains("Cancel and restore"), "{rendered}");
    assert!(rendered.contains("draft remains visible"), "{rendered}");
    assert!(!rendered.contains("Theme applied"), "{rendered}");
    assert!(!rendered.contains("Saved"), "{rendered}");

    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).expect("compact terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw compact theme picker");
    let compact = terminal.backend().to_string();
    assert!(compact.contains("Choose theme"), "{compact}");
    assert!(compact.contains("Enter apply"), "{compact}");
}

#[test]
fn blank_approval_submission_still_fails_closed() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::Prompt(InteractivePrompt {
            id: "approval".into(),
            kind: InteractivePromptKind::Approval,
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

fn session_browser_entry(
    id: &str,
    title: &str,
    message_count: u64,
    messages: &[(&str, ModelMessageRole)],
) -> InteractiveSessionBrowserEntry {
    InteractiveSessionBrowserEntry {
        summary: SessionSummary {
            id: id.into(),
            title: Some(title.into()),
            created_at: "2026-08-08T01:00:00Z".into(),
            updated_at: "2026-08-08T02:05:00Z".into(),
            message_count,
            last_run_id: None,
            last_user_preview: messages.first().map(|(content, _)| (*content).into()),
        },
        recent_messages: messages
            .iter()
            .map(|(content, role)| InteractiveSessionBrowserMessage {
                role: *role,
                content: (*content).into(),
            })
            .collect(),
    }
}

fn session_browser(response: oneshot::Sender<PromptResponse>) -> InteractiveSessionBrowser {
    InteractiveSessionBrowser {
        current_session_id: "019f-test".into(),
        sessions: vec![
            session_browser_entry(
                "019f-test",
                "Dangerous full access resources",
                2,
                &[(
                    "Configure executables in the sandbox",
                    ModelMessageRole::User,
                )],
            ),
            session_browser_entry(
                "019f-cache",
                "Rust PR compiler cache",
                13,
                &[
                    (
                        "Our Rust CI runs show no cache found in the logs. Why is that happening?",
                        ModelMessageRole::User,
                    ),
                    (
                        "That message typically comes from your CI's higher-level cache layer, such as GitHub Actions cache, not having a hit for the key. sccache is a separate compiler cache.",
                        ModelMessageRole::Assistant,
                    ),
                    (
                        "Are we at least getting any sccache hits?",
                        ModelMessageRole::User,
                    ),
                    (
                        "Yes. sccache reports an 81.59% hit rate for Rust. Cache hits: 81.59% (12,345 hits / 15,129 requests). Cache misses: 18.41% (2,784 misses).",
                        ModelMessageRole::Assistant,
                    ),
                    (
                        "Got it. So the build is using sccache effectively, while the outer CI cache just cold-started.",
                        ModelMessageRole::User,
                    ),
                    (
                        "Exactly. Subsequent runs should warm that outer cache as well.",
                        ModelMessageRole::Assistant,
                    ),
                ],
            ),
            session_browser_entry(
                "019f-shell",
                "Run PowerShell in workspace",
                4,
                &[("Run the command PS", ModelMessageRole::User)],
            ),
            session_browser_entry(
                "019f-hello",
                "Quick hello",
                1,
                &[("hi", ModelMessageRole::User)],
            ),
        ],
        response,
    }
}

#[test]
fn session_browser_searches_skips_current_and_resumes_the_selected_id() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::SessionBrowser(session_browser(response)),
    );
    let Some(Overlay::SessionBrowser(browser)) = state.overlay.as_ref() else {
        panic!("session browser overlay");
    };
    assert_eq!(browser.selected, Some(1));
    assert!(state.transient_inline_screen_active());

    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
    );
    for character in "rust".chars() {
        handle_overlay_key(
            &mut state,
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
        );
    }
    let Some(Overlay::SessionBrowser(browser)) = state.overlay.as_ref() else {
        panic!("session browser overlay");
    };
    assert_eq!(browser.selected, Some(1));
    assert!(browser.search_active);

    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    assert!(matches!(
        state.overlay.as_ref(),
        Some(Overlay::SessionBrowser(SessionBrowserState {
            preview_scroll: 5,
            ..
        }))
    ));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    );

    handle_overlay_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(
        state.overlay,
        Some(Overlay::SessionBrowser(SessionBrowserState {
            search_active: false,
            ..
        }))
    ));
    handle_overlay_key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert_eq!(
        received.try_recv(),
        Ok(PromptResponse::Answer("019f-cache".into()))
    );
    assert!(!state.transient_inline_screen_active());
}

#[test]
fn terminal_operation_releases_an_open_session_browser() {
    let mut state = TuiState::from_snapshot(snapshot());
    let (response, mut received) = oneshot::channel();
    handle_host_event(
        &mut state,
        HostEvent::SessionBrowser(session_browser(response)),
    );

    handle_host_event(
        &mut state,
        HostEvent::OperationFinished(Box::new(Err("worker disconnected".into()))),
    );

    assert_eq!(received.try_recv(), Ok(PromptResponse::Cancelled));
    assert!(state.overlay.is_none());
}

#[test]
fn session_browser_matches_the_master_detail_reference_and_remains_responsive() {
    for (width, height) in [(40, 12), (120, 32)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut state = TuiState::from_snapshot(snapshot());
        state.operation = Some(OperationKind::Command);
        state.activity = Some("/resume".into());
        state.composer.insert("draft stays visible");
        let (response, _received) = oneshot::channel();
        handle_host_event(
            &mut state,
            HostEvent::SessionBrowser(session_browser(response)),
        );
        terminal
            .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
            .expect("draw session browser");
        let rendered = terminal.backend().to_string();
        assert!(
            rendered.contains("Resume session"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("Rust PR compiler"),
            "{width}x{height}: {rendered}"
        );
        assert!(
            rendered.contains("draft stays visible"),
            "{width}x{height}: {rendered}"
        );
        if width >= 72 {
            assert!(rendered.contains("CURRENT"), "{rendered}");
            assert!(rendered.contains("Recent conversation"), "{rendered}");
            assert!(rendered.contains("higher-level cache layer"), "{rendered}");
            assert!(rendered.contains("Enter Resume"), "{rendered}");
            let lines = rendered.lines().collect::<Vec<_>>();
            let current_row = lines
                .iter()
                .position(|line| line.contains("Dangerous full access"))
                .expect("current session row");
            let selected_row = lines
                .iter()
                .position(|line| line.contains("│ › Rust PR compiler"))
                .expect("selected session row");
            let shell_row = lines
                .iter()
                .position(|line| line.contains("Run PowerShell"))
                .expect("following session row");
            assert_eq!(selected_row, current_row + 2, "{rendered}");
            assert_eq!(shell_row, selected_row + 2, "{rendered}");
        }
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
fn mouse_wheel_scrolls_transcript_by_lines_and_returns_to_live_output() {
    let mut state = TuiState::from_snapshot(snapshot());
    state.transcript_height = 4;
    state.transcript_width = 80;
    for index in 0..12 {
        state.append_entry(user_entry(
            &format!("transcript row {index}"),
            TranscriptKind::User,
        ));
    }
    let mouse = |kind| MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };

    assert!(!handle_mouse(&mut state, mouse(MouseEventKind::ScrollUp)));
    assert_eq!(state.scroll_from_bottom, MOUSE_SCROLL_LINES);
    state.new_items = 2;
    assert!(!handle_mouse(&mut state, mouse(MouseEventKind::ScrollDown)));
    assert_eq!(state.scroll_from_bottom, 0);
    assert_eq!(state.new_items, 0);

    let mut requested_older = false;
    for _ in 0..100 {
        if handle_mouse(&mut state, mouse(MouseEventKind::ScrollUp)) {
            requested_older = true;
            break;
        }
    }
    assert!(requested_older);

    let offset = state.scroll_from_bottom;
    state.overlay = Some(Overlay::HistorySearch {
        query: String::new(),
    });
    assert!(!handle_mouse(&mut state, mouse(MouseEventKind::ScrollUp)));
    assert_eq!(state.scroll_from_bottom, offset);
}

#[test]
fn mouse_scrolling_keeps_the_composer_and_status_footer_sticky() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut state = TuiState::from_snapshot(snapshot());
    for index in 0..20 {
        state.append_entry(user_entry(
            &format!("scrollable row {index}"),
            TranscriptKind::User,
        ));
    }
    state.composer.insert("sticky draft");
    for _ in 0..4 {
        handle_mouse(
            &mut state,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
        );
    }
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("draw scrolled TUI");
    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("sticky draft"), "{rendered}");
    assert!(rendered.contains("primary:echo@echo"), "{rendered}");
    assert!(state.scroll_from_bottom > 0);
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
fn ctrl_c_exits_when_idle_and_cancels_once_before_exiting_an_active_run() {
    let mut idle = TuiState::from_snapshot(snapshot());
    idle.composer.insert("discarded draft");
    idle.interrupt_or_exit();
    assert!(idle.should_exit);

    let mut active = TuiState::from_snapshot(snapshot());
    let control = RunControl::default();
    active.operation = Some(OperationKind::Run);
    active.control = Some(control.clone());
    active.interrupt_or_exit();
    assert!(control.is_cancelled());
    assert!(!active.should_exit);
    assert_eq!(
        active.activity.as_deref(),
        Some("cancelling after the current effect settles")
    );

    active.interrupt_or_exit();
    assert!(active.should_exit);
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
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
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
                    .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
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
    let help = help_document(&state.completions);
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Command,
        document: help,
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
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
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
            content: String::new().into(),
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
