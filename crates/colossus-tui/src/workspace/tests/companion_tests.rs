use super::*;
use ratatui::backend::TestBackend;

fn draw(workspace: &mut Workspace, width: u16) -> (String, Vec<String>) {
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).expect("terminal");
    let (active, companion) = workspace.draw_parts();
    terminal
        .draw(|frame| render_with_companion(frame, active, 0, ScreenMode::Alternate, companion))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let left = buffer
        .content
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .take(usize::from(width.saturating_sub(36)))
                .map(Cell::symbol)
                .collect::<String>()
        })
        .collect();
    (terminal.backend().to_string(), left)
}

fn press(workspace: &mut Workspace, code: KeyCode) {
    assert!(key(workspace, KeyEvent::new(code, KeyModifiers::NONE)));
    workspace.process_action(host());
}

fn send(workspace: &mut Workspace, id: &str, event: HostEvent) {
    workspace.receive(
        WorkspaceEvent::Session {
            id: id.into(),
            event: Box::new(event),
        },
        host(),
    );
}

#[tokio::test]
async fn companion_preserves_active_draft_and_rendered_scroll_and_never_opens_background_decisions()
{
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    workspace.composer.insert("Alpha private unsent draft");
    open_snapshot(&mut workspace, "beta");
    let alpha = workspace.background.get_mut("alpha").unwrap();
    alpha.operation = Some(OperationKind::Run);
    let alpha_control = RunControl::default();
    alpha.control = Some(alpha_control.clone());
    alpha.append_plain(TranscriptKind::Assistant, "Old Alpha history");
    for _ in 0..24 {
        alpha.append_plain(TranscriptKind::Assistant, "Recent Alpha history");
    }
    for index in 0..40 {
        workspace.append_plain(
            TranscriptKind::Assistant,
            &format!("Beta history {index:02}"),
        );
    }
    workspace.composer.insert("Beta draft in progress");
    draw(&mut workspace, 120);
    workspace.scroll_up_lines(20);
    let (before, left_before) = draw(&mut workspace, 120);
    assert!(!before.contains("Beta history 39"));

    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    assert!(draw(&mut workspace, 120).0.contains("[p] Pin"));
    press(&mut workspace, KeyCode::Char('p'));
    send(
        &mut workspace,
        "alpha",
        delta("alpha", "Alpha streaming tail"),
    );
    let (prompt, mut reply) = approval("ALPHA_PRIVATE_APPROVAL");
    send(&mut workspace, "alpha", prompt);
    let (pinned, pinned_left) = draw(&mut workspace, 120);
    assert_eq!(left_before, pinned_left);
    assert!(pinned.contains("Pinned · Read-only"));
    assert!(pinned.contains("ID alpha"));
    assert!(pinned.contains("Alpha streaming tail"));
    assert!(pinned.contains("Status: decision"));
    assert!(!pinned.contains("Old Alpha history"));
    assert!(!pinned.contains("Alpha private unsent draft"));
    assert!(!pinned.contains("ALPHA_PRIVATE_APPROVAL"));
    assert!(workspace.overlay.is_none());
    assert!(reply.try_recv().is_err());

    let area = workspace.companion_area.expect("wide companion");
    assert!(!crate::app::handle_mouse(
        &mut workspace,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: area.x + 1,
            row: area.y + 1,
            modifiers: KeyModifiers::NONE,
        }
    ));
    assert_eq!(workspace.scroll_from_bottom, 20);
    assert!(!draw(&mut workspace, 80).0.contains("Pinned · Read-only"));
    assert!(workspace.companion_area.is_none());
    assert_eq!(draw(&mut workspace, 120).1, left_before);
    assert_eq!(workspace.draft(), "Beta draft in progress");
    assert_eq!(workspace.session_id, "beta");
    assert!(!alpha_control.is_cancelled());

    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    assert!(draw(&mut workspace, 120).0.contains("[p] Unpin"));
    press(&mut workspace, KeyCode::Char('p'));
    assert!(workspace.companion_id.is_none());
    assert_eq!(draw(&mut workspace, 120).1, left_before);
    assert!(reply.try_recv().is_err());
}

#[tokio::test]
async fn companion_selection_clears_on_focus_or_archival_and_ignores_stale_other_session_events() {
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    open_snapshot(&mut workspace, "beta");
    workspace.workspace_action = Some(WorkspaceAction::ToggleCompanion("missing".into()));
    workspace.process_action(host());
    assert!(workspace.companion_id.is_none());
    workspace.workspace_action = Some(WorkspaceAction::ToggleCompanion("beta".into()));
    workspace.process_action(host());
    assert!(workspace.companion_id.is_none());

    workspace.workspace_action = Some(WorkspaceAction::ToggleCompanion("alpha".into()));
    workspace.process_action(host());
    workspace.switch("alpha");
    assert!(workspace.companion_id.is_none(), "pin must stay inactive");
    workspace.switch("beta");
    workspace.workspace_action = Some(WorkspaceAction::ToggleCompanion("alpha".into()));
    workspace.process_action(host());
    workspace.receive(
        WorkspaceEvent::Archived {
            id: "alpha".into(),
            archived: true,
            result: Box::new(Ok(None)),
        },
        host(),
    );
    assert!(workspace.companion_id.is_none());
    assert!(!workspace.background.contains_key("alpha"));

    open_snapshot(&mut workspace, "gamma");
    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    press(&mut workspace, KeyCode::Char('p'));
    send(
        &mut workspace,
        "alpha",
        delta("alpha", "Archived stale turn"),
    );
    send(
        &mut workspace,
        "beta",
        delta("alpha", "Mismatched stale turn"),
    );
    send(
        &mut workspace,
        "beta",
        delta("beta", "Beta visible live tail"),
    );
    let (screen, _) = draw(&mut workspace, 120);
    assert!(screen.contains("ID beta"));
    assert!(screen.contains("Beta visible live tail"));
    assert!(!screen.contains("stale turn"));
}

#[tokio::test]
async fn companion_keeps_latest_delta_visible_below_recent_image_metadata() {
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    let image = ModelImageReference {
        artifact_id: "wide-preview".into(),
        file_name: "wide-image.png".into(),
        media_type: "image/png".into(),
        size_bytes: 128,
        sha256: "a".repeat(64),
        width_pixels: 128,
        height_pixels: 64,
        detail: colossus_contracts::ModelImageDetail::Auto,
    };
    // Keep a native preview variant from when Alpha occupied the wide main
    // transcript. Its reserved cells are wider than the companion's text area.
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    workspace.preview_cache.set_picker(picker);
    let preview_size = Size::new(64, 20);
    workspace.preview_transcript_size = preview_size;
    workspace.preview_cache.insert(
        image.sha256.clone(),
        image::DynamicImage::new_rgba8(128, 64),
    );
    workspace.preview_cache.prepare(&image.sha256, preview_size);
    workspace.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::User,
        document: PresentationDocument::from_block(PresentationBlock::Image(image)),
        temporary: false,
    });
    assert!(
        transcript_lines(&workspace, 34)
            .iter()
            .any(|line| line.width() > 34)
    );

    open_snapshot(&mut workspace, "beta");
    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    press(&mut workspace, KeyCode::Char('p'));
    send(
        &mut workspace,
        "alpha",
        delta("alpha", "Newest streamed text"),
    );
    let (screen, _) = draw(&mut workspace, 100);
    assert!(screen.contains("wide-image.png"), "{screen}");
    assert!(screen.contains("Newest streamed text"), "{screen}");
}

#[tokio::test]
async fn open_tab_can_unpin_a_session_while_its_archive_update_is_pending() {
    let (mut workspace, _) = Workspace::new(
        ScreenMode::Alternate,
        TuiState::from_snapshot(source("alpha")),
    );
    open_snapshot(&mut workspace, "beta");
    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    press(&mut workspace, KeyCode::Char('p'));
    workspace
        .background
        .get_mut("alpha")
        .unwrap()
        .workspace_archiving = true;

    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    assert!(draw(&mut workspace, 120).0.contains("[p] Unpin"));
    press(&mut workspace, KeyCode::Tab);
    assert!(!draw(&mut workspace, 120).0.contains("[p] Unpin"));
    press(&mut workspace, KeyCode::BackTab);
    press(&mut workspace, KeyCode::Home);
    press(&mut workspace, KeyCode::Char('p'));
    assert!(workspace.companion_id.is_none());

    press(&mut workspace, KeyCode::F(2));
    press(&mut workspace, KeyCode::Home);
    assert!(!draw(&mut workspace, 120).0.contains("[p] Pin"));
    press(&mut workspace, KeyCode::Char('p'));
    assert!(workspace.companion_id.is_none());
}
