use super::*;
use colossus_contracts::{CustomTheme, ThemeName, ThemeSpinner};

fn canvas() -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 180, 60));
    for cell in &mut buffer.content {
        cell.set_char('#');
    }
    buffer
}

fn frame(animation: &mut StartupAnimation, area: Rect, now: Instant) -> Buffer {
    let mut buffer = canvas();
    animation.render(&mut buffer, area, &TerminalPreferences::default(), now);
    buffer
}

fn row_text(buffer: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buffer.cell(Position::new(x, y)).unwrap().symbol())
        .collect()
}

fn text(buffer: &Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| row_text(buffer, area, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn visible_cells(buffer: &Buffer, area: Rect) -> usize {
    area.positions()
        .filter(|position| {
            buffer
                .cell(*position)
                .is_some_and(|cell| !cell.symbol().trim().is_empty())
        })
        .count()
}

fn mono_custom_theme() -> CustomTheme {
    let style = TerminalPalette::for_preferences(&TerminalPreferences::default()).meta_style();
    CustomTheme {
        schema_version: 1,
        name: "custom_mono".into(),
        source_hash: "a".repeat(64),
        base: ThemeName::Mono,
        title: "Custom Mono".into(),
        caret: ">".into(),
        continuation: ":".into(),
        prompt_left: style.foreground,
        prompt_right: style.foreground,
        indicator: style.foreground,
        continuation_color: style.foreground,
        assistant: style,
        activity: style,
        thinking: style,
        tool: style,
        success: style,
        warning: style,
        error: style,
        meta: style,
        spinner: ThemeSpinner::Line,
    }
}

#[test]
fn input_skips_ignore_initial_size_reports_and_passive_motion() {
    let mut size = Some((140, 40));
    assert!(!interrupts(&Event::Resize(140, 40), &mut size));
    assert!(interrupts(&Event::Resize(136, 38), &mut size));
    assert!(!interrupts(&Event::Resize(136, 38), &mut size));
    let mut unknown = None;
    assert!(!interrupts(&Event::Resize(140, 40), &mut unknown));
    assert_eq!(unknown, Some((140, 40)));
    assert!(interrupts(
        &Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        &mut size
    ));
    assert!(interrupts(&Event::Paste("draft".into()), &mut size));
    for (kind, expected) in [
        (MouseEventKind::Moved, false),
        (MouseEventKind::Up(MouseButton::Left), false),
        (MouseEventKind::Down(MouseButton::Left), true),
        (MouseEventKind::ScrollDown, true),
    ] {
        assert_eq!(
            interrupts(
                &Event::Mouse(MouseEvent {
                    kind,
                    column: 10,
                    row: 10,
                    modifiers: KeyModifiers::NONE
                }),
                &mut size
            ),
            expected
        );
    }
}

#[test]
fn eligibility_is_process_local_and_respects_accessible_color_modes() {
    let mut preferences = TerminalPreferences::default();
    for setting in [None, Some("on"), Some("true"), Some("")] {
        assert!(enabled(ScreenMode::Alternate, &preferences, false, setting));
    }
    for setting in ["off", " OFF ", "false", " FALSE ", "0", " 0 "] {
        assert!(!enabled(
            ScreenMode::Alternate,
            &preferences,
            false,
            Some(setting)
        ));
    }
    assert!(!enabled(ScreenMode::Inline, &preferences, false, None));
    assert!(!enabled(ScreenMode::Alternate, &preferences, true, None));
    preferences.theme = ThemeName::Mono;
    assert!(!enabled(ScreenMode::Alternate, &preferences, false, None));
    preferences.theme = ThemeName::Default;
    preferences.custom_theme = Some(mono_custom_theme());
    assert!(!enabled(ScreenMode::Alternate, &preferences, false, None));
}

#[test]
fn splash_is_centered_and_stacked_at_supported_sizes_with_a_static_identity_footer() {
    let now = Instant::now();
    for (width, height) in [(40, 24), (80, 32), (140, 40), (141, 41)] {
        let area = Rect::new(3, 2, width, height);
        let mut animation = StartupAnimation::new();
        animation.prepare(now);
        let rendered = frame(&mut animation, area, now + Duration::from_millis(1_050));
        let content = content_area(area);
        assert!(row_text(&rendered, content, content.y + 11).contains("OBSCURITY LABS"));
        assert!(row_text(&rendered, content, content.y + 17).contains("COLOSSUS"));
        assert!(row_text(&rendered, content, content.y + 18).contains("AI agent runtime"));
        assert!(row_text(&rendered, content, content.y + 21).contains(HINT));
        assert!(row_text(&rendered, area, area.bottom() - 1).contains(IDENTITY));
        for y in [
            content.y + 11,
            content.y + 13,
            content.y + 17,
            content.y + 21,
        ] {
            let xs = (area.x..area.right())
                .filter(|x| {
                    !rendered
                        .cell(Position::new(*x, y))
                        .unwrap()
                        .symbol()
                        .trim()
                        .is_empty()
                })
                .collect::<Vec<_>>();
            let center = u32::from(*xs.first().unwrap()) + u32::from(*xs.last().unwrap());
            let desired = u32::from(area.x) + u32::from(area.right()) - 1;
            assert!(
                center.abs_diff(desired) <= 1,
                "row {y} is not centered: {}",
                row_text(&rendered, area, y)
            );
        }
    }
}

#[test]
fn reveal_spin_and_assembly_finish_on_the_exact_settled_front_face() {
    let now = Instant::now();
    let area = Rect::new(3, 2, 140, 40);
    let content = content_area(area);
    let mut animation = StartupAnimation::new();
    assert!(animation.started.is_none());
    let zero = frame(&mut animation, area, now);
    assert_eq!(visible_cells(&zero, labs_area(content)), 0);
    assert_eq!(visible_cells(&zero, colossus_area(content)), 0);
    assert!(text(&zero, area).contains(HINT));
    assert!(text(&zero, area).contains(IDENTITY));
    let edge = frame(&mut animation, area, now + Duration::from_millis(263));
    assert!(row_text(&edge, content, content.y + 11).contains("OBSCURITY LABS"));
    assert_eq!(visible_cells(&edge, colossus_area(content)), 0);
    let assembling = frame(&mut animation, area, now + Duration::from_millis(700));
    assert!(visible_cells(&assembling, colossus_area(content)) > 0);
    let settled = frame(&mut animation, area, now + Duration::from_millis(1_050));
    assert_ne!(edge, settled);
    assert!(
        visible_cells(&assembling, colossus_area(content))
            < visible_cells(&settled, colossus_area(content))
    );
    assert_eq!(
        settled,
        frame(&mut animation, area, now + Duration::from_millis(1_450))
    );
    assert!(!animation.prepare(now + INTRO_DURATION));
    assert_eq!(
        settled,
        frame(&mut animation, area, now + Duration::from_secs(30))
    );
}

#[test]
fn rendering_is_deterministic_at_zero_time_and_never_writes_outside_the_splash() {
    let now = Instant::now();
    let area = Rect::new(5, 3, 140, 40);
    let mut animation = StartupAnimation::new();
    let zero = frame(&mut animation, area, now);
    assert_eq!(zero, frame(&mut animation, area, now));
    let mut other = StartupAnimation::new();
    assert_eq!(zero, frame(&mut other, area, now));
    let baseline = canvas();
    let footer = row_text(&zero, area, area.bottom() - 1);
    for elapsed in [0, 125, 263, 525, 788, 1_050, 1_450] {
        let rendered = frame(&mut animation, area, now + Duration::from_millis(elapsed));
        for position in baseline.area.positions() {
            if !area.contains(position) {
                assert_eq!(rendered.cell(position), baseline.cell(position));
            }
        }
        assert!(text(&rendered, area).contains(HINT));
        assert_eq!(row_text(&rendered, area, area.bottom() - 1), footer);
        assert_eq!(
            rendered,
            frame(&mut other, area, now + Duration::from_millis(elapsed))
        );
    }
}

#[test]
fn supported_palettes_survive_a_settled_splash_without_invented_ink() {
    let now = Instant::now();
    let area = Rect::new(3, 2, 140, 40);
    for theme in [
        ThemeName::Default,
        ThemeName::Mono,
        ThemeName::HighContrast,
        ThemeName::Carrot,
        ThemeName::Hacker,
    ] {
        let preferences = TerminalPreferences {
            theme,
            ..Default::default()
        };
        let palette = TerminalPalette::for_preferences(&preferences);
        let allowed = [
            Color::Reset,
            ratatui_style(palette.user_style())
                .fg
                .unwrap_or(Color::Reset),
            ratatui_style(palette.assistant_style())
                .fg
                .unwrap_or(Color::Reset),
            ratatui_style(palette.meta_style())
                .fg
                .unwrap_or(Color::Reset),
        ];
        let mut animation = StartupAnimation::new();
        animation.prepare(now);
        let mut buffer = canvas();
        assert!(!animation.render(&mut buffer, area, &preferences, now + INTRO_DURATION));
        assert!(
            area.positions()
                .all(|position| allowed.contains(&buffer.cell(position).unwrap().fg))
        );
    }
}

#[test]
fn compact_areas_and_wall_clock_expiry_do_not_require_extra_ticks() {
    let now = Instant::now();
    let mut animation = StartupAnimation::new();
    let mut buffer = canvas();
    assert!(!animation.render(
        &mut buffer,
        Rect::new(5, 3, 20, 8),
        &TerminalPreferences::default(),
        now
    ));
    let mut animation = StartupAnimation::new();
    assert!(animation.prepare(now));
    assert!(!animation.prepare(now + INTRO_DURATION));
    assert!(!animation.render(
        &mut buffer,
        Rect::new(5, 3, 140, 40),
        &TerminalPreferences::default(),
        now + INTRO_DURATION
    ));
    assert!(text(&buffer, Rect::new(5, 3, 140, 40)).contains(IDENTITY));
}
