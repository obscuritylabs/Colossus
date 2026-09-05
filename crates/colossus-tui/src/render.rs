use super::*;

const OBSCURITY_LABS_RED: Color = Color::Rgb(213, 16, 48);
const OBSCURITY_LABS_SEGMENT: &str = "OL //  ";
const OBSCURITY_LABS_FOOTER_SEGMENT: &str = " OL // COLOSSUS ";
const WIDE_WELCOME_MIN_WIDTH: usize = 96;
const WIDE_WELCOME_MIN_HEIGHT: usize = 22;

pub(super) fn render(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    transcript_start: usize,
    screen_mode: ScreenMode,
) {
    let area = frame.area();
    let minimum_height = match screen_mode {
        ScreenMode::Alternate => MINIMUM_TERMINAL_HEIGHT,
        ScreenMode::Inline => MINIMUM_INLINE_VIEWPORT_HEIGHT,
    };
    if area.width < MINIMUM_TERMINAL_WIDTH || area.height < minimum_height {
        let notice = Paragraph::new(format!(
            "Colossus needs at least {MINIMUM_TERMINAL_WIDTH}x{MINIMUM_TERMINAL_HEIGHT}. Current: {}x{}.\nYour draft and transcript are preserved.",
            area.width, area.height
        ))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("Resize terminal"))
        .wrap(Wrap { trim: true });
        frame.render_widget(notice, area);
        return;
    }

    prepare_image_previews(state, area);

    let composer_height = composer_height(state, area.width);
    let activity_height = u16::from(state.operation.is_some());
    let approval_height =
        approval_dock_height(state, area.height, composer_height, activity_height);
    let plan_execution_height =
        plan_execution_dock_height(state, area.height, composer_height, activity_height);
    let decision_height = approval_height.max(plan_execution_height);
    let completion_height = if state.docked_decision_active() {
        0
    } else {
        completion_menu_height(
            state,
            area.width,
            area.height,
            composer_height,
            activity_height,
        )
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(if screen_mode == ScreenMode::Inline {
                0
            } else {
                3
            }),
            Constraint::Length(activity_height),
            Constraint::Length(completion_height),
            Constraint::Length(decision_height),
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(area);
    state.transcript_height = usize::from(
        rows[0]
            .height
            .saturating_sub(u16::from(state.new_items > 0)),
    );
    state.transcript_width = usize::from(rows[0].width).max(20);
    // Use the whole viewport for the welcome breakpoint. Inline sizing uses
    // that same height, so transient completion cannot reflow the start screen.
    render_transcript(
        frame,
        state,
        rows[0],
        transcript_start,
        usize::from(area.height),
    );
    if activity_height > 0 {
        render_activity(frame, state, rows[1]);
    }
    if completion_height > 0 {
        render_completion_menu(frame, state, rows[2]);
    }
    if decision_height > 0 {
        if approval_height > 0 {
            render_approval_dock(frame, state, rows[3]);
        } else {
            render_plan_execution_dock(frame, state, rows[3]);
        }
    }
    render_composer(frame, state, rows[4]);
    render_footer(frame, state, rows[5]);
    if state.overlay.is_some() && decision_height == 0 {
        render_overlay(frame, state, area);
    }
}

fn prepare_image_previews(state: &mut TuiState, area: Rect) {
    let transcript_size = Size::new(
        area.width.saturating_mul(2).saturating_div(3).clamp(1, 80),
        area.height.saturating_div(2).clamp(1, 24),
    );
    state.preview_transcript_size = transcript_size;
    let transcript_images = state
        .transcript
        .iter()
        .flat_map(|entry| entry.document.blocks.iter())
        .filter_map(|block| match block {
            PresentationBlock::Image(image) => Some(image.sha256.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for digest in transcript_images {
        state.preview_cache.prepare(&digest, transcript_size);
    }
    let pending = state
        .pending_images
        .iter()
        .take(3)
        .map(|image| image.sha256.clone())
        .collect::<Vec<_>>();
    for digest in pending {
        state.preview_cache.prepare(&digest, Size::new(18, 5));
    }
}

pub(super) fn render_transcript(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    area: Rect,
    transcript_start: usize,
    viewport_height: usize,
) {
    let (badge_area, transcript_area) = if state.new_items > 0 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        (Some(rows[0]), rows[1])
    } else {
        (None, area)
    };
    if let Some(badge_area) = badge_area {
        let palette = TerminalPalette::for_preferences(&state.preferences);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" ↑ {} new · End returns live ", state.new_items),
                ratatui_style(palette.warning_style()),
            ))
            .alignment(Alignment::Right),
            badge_area,
        );
    }
    let width = usize::from(transcript_area.width).max(20);
    let mut lines = if state.welcome_visible {
        welcome_lines(state, width, viewport_height)
    } else {
        Vec::new()
    };
    let welcome_line_count = lines.len();
    let (mut transcript, mut image_placements) = transcript_lines_range_with_images(
        state,
        width,
        transcript_start.min(state.transcript.len()),
        state.transcript.len(),
        !lines.is_empty(),
    );
    for placement in &mut image_placements {
        placement.line = placement.line.saturating_add(welcome_line_count);
    }
    lines.append(&mut transcript);
    let visible = usize::from(transcript_area.height);
    let live_top = lines.len().saturating_sub(visible);
    let top = live_top.saturating_sub(state.scroll_from_bottom);
    let paragraph = Paragraph::new(lines)
        .scroll((u16::try_from(top).unwrap_or(u16::MAX), 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, transcript_area);
    if state.preview_cache.native_graphics() {
        let bottom = top.saturating_add(visible);
        for placement in image_placements {
            let image_bottom = placement
                .line
                .saturating_add(usize::from(state.preview_transcript_size.height));
            if placement.line < top || image_bottom > bottom {
                continue;
            }
            let area = Rect::new(
                transcript_area
                    .x
                    .saturating_add(u16::try_from(placement.indent).unwrap_or(u16::MAX)),
                transcript_area.y.saturating_add(
                    u16::try_from(placement.line.saturating_sub(top)).unwrap_or(u16::MAX),
                ),
                state.preview_transcript_size.width,
                state.preview_transcript_size.height,
            );
            if area.right() <= transcript_area.right() && area.bottom() <= transcript_area.bottom()
            {
                state.preview_cache.render_native(
                    frame,
                    &placement.digest,
                    state.preview_transcript_size,
                    area,
                );
            }
        }
    }
}

pub(super) fn desired_inline_viewport_height(
    state: &TuiState,
    width: u16,
    screen_height: u16,
    transcript_start: usize,
) -> u16 {
    if screen_height == 0 {
        return 0;
    }
    if width < MINIMUM_TERMINAL_WIDTH {
        return MINIMUM_TERMINAL_HEIGHT.min(screen_height);
    }
    if state.overlay.is_some() {
        return screen_height;
    }

    let composer_height = composer_height(state, width);
    let activity_height = u16::from(state.operation.is_some());
    // Inline completion is rendered on a transient alternate screen. The main
    // screen viewport must therefore remain independent of completion chrome.
    let completion_height = 0;
    let chrome_height = activity_height
        .saturating_add(completion_height)
        .saturating_add(composer_height)
        .saturating_add(1);
    let mut transcript_line_count = transcript_lines_range(
        state,
        usize::from(width).max(20),
        transcript_start.min(state.transcript.len()),
        state.transcript.len(),
        false,
    )
    .len();
    if state.welcome_visible {
        if transcript_line_count > 0
            && state.preferences.transcript_density
                == colossus_contracts::TranscriptDensity::Comfortable
        {
            transcript_line_count = transcript_line_count.saturating_add(1);
        }
        transcript_line_count = transcript_line_count.saturating_add(
            welcome_lines(
                state,
                usize::from(width).max(20),
                usize::from(screen_height),
            )
            .len(),
        );
    }
    let transcript_height = u16::try_from(transcript_line_count).unwrap_or(u16::MAX);

    chrome_height
        .saturating_add(transcript_height)
        .min(screen_height)
}

pub(super) fn welcome_lines(state: &TuiState, width: usize, height: usize) -> Vec<Line<'static>> {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let brand = obscurity_labs_brand_style(&state.preferences);
    let accent = ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD);
    let primary = ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD);
    let secondary = ratatui_style(palette.meta_style());
    let muted = ratatui_style(palette.meta_style()).add_modifier(Modifier::DIM);
    let ready =
        ratatui_style(palette.tone_style(PresentationTone::Success)).add_modifier(Modifier::BOLD);
    let warning = ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD);
    let security_style = if state.security_posture.is_hardened() {
        ready
    } else {
        warning
    };
    let security_state = if state.security_posture.is_hardened() {
        "hardened".to_owned()
    } else {
        format!(
            "{} warning{}",
            state.security_posture.finding_count(),
            if state.security_posture.finding_count() == 1 {
                ""
            } else {
                "s"
            }
        )
    };
    let workspace = welcome_workspace(&state.workspace);
    let route = sanitize_approval_field(
        &state
            .footer
            .route
            .replace(" via ", " · ")
            .replace('@', " · "),
    );
    let sandbox_profile = sanitize_approval_field(&state.sandbox_profile);
    let approval_mode = sanitize_approval_field(&state.footer.approval_mode);
    let readiness = sanitize_approval_field(&state.footer.status);
    let session_state = format!("durable · {}", readiness.to_lowercase());

    if !wide_welcome_layout(width, height) {
        return vec![
            Line::from(vec![
                Span::styled(OBSCURITY_LABS_SEGMENT, brand),
                Span::styled("COLOSSUS", accent),
            ]),
            Line::from(Span::styled(
                truncate_width("─".repeat(width).as_str(), width),
                ratatui_style(palette.assistant_style()),
            )),
            Line::from(Span::styled("What do you want to work on?", primary)),
            Line::from(Span::styled(truncate_width(&workspace, width), secondary)),
            Line::from(vec![
                Span::styled("› ", accent),
                Span::styled("Build or change something", accent),
            ]),
            Line::from(Span::styled("  /plan · /resume · /tools", secondary)),
            Line::from(Span::styled(
                truncate_width(
                    &format!(
                        "{} · {} · approval {}",
                        route, sandbox_profile, approval_mode
                    ),
                    width,
                ),
                muted,
            )),
            Line::from(vec![
                Span::styled("SESSION  ", accent),
                Span::styled(session_state, ready),
                Span::styled(" · ", secondary),
                Span::styled(security_state, security_style),
            ]),
        ];
    }

    let label_style = muted;
    let value_style = ratatui_style(palette.user_style());
    let section_style = value_style.add_modifier(Modifier::BOLD);
    vec![
        Line::from(vec![
            Span::styled(OBSCURITY_LABS_SEGMENT, brand),
            Span::styled("COLOSSUS", accent),
            Span::styled("    OBSCURITY LABS", muted),
        ]),
        Line::default(),
        Line::from(Span::styled("What do you want to work on?", primary)),
        Line::from(Span::styled(truncate_width(&workspace, width), secondary)),
        Line::default(),
        welcome_action_line(
            ">   ",
            "Build or change something",
            None,
            width,
            section_style,
            accent,
            section_style,
        ),
        welcome_action_line(
            "    ",
            "Plan it first",
            Some("/plan"),
            width,
            secondary,
            secondary,
            section_style,
        ),
        welcome_action_line(
            "    ",
            "Resume recent work",
            Some("/resume"),
            width,
            secondary,
            secondary,
            section_style,
        ),
        welcome_action_line(
            "    ",
            "Inspect capabilities",
            Some("/tools"),
            width,
            secondary,
            secondary,
            section_style,
        ),
        Line::default(),
        welcome_status_row(
            "RUNTIME",
            &route,
            value_style,
            Some(("SANDBOX", &sandbox_profile, value_style)),
            width,
            label_style,
        ),
        welcome_status_row(
            "APPROVAL",
            &approval_mode,
            value_style,
            Some(("SESSION", &session_state, ready)),
            width,
            label_style,
        ),
        welcome_status_row(
            "SECURITY",
            &security_state,
            security_style,
            None,
            width,
            label_style,
        ),
    ]
}

fn wide_welcome_layout(width: usize, height: usize) -> bool {
    width >= WIDE_WELCOME_MIN_WIDTH && height >= WIDE_WELCOME_MIN_HEIGHT
}

fn welcome_action_line(
    prefix: &str,
    label: &str,
    shortcut: Option<&str>,
    width: usize,
    prefix_style: Style,
    text_style: Style,
    shortcut_style: Style,
) -> Line<'static> {
    let left = format!("{prefix}{label}");
    let shortcut_column = width.min(48);
    let gap = shortcut.map_or(0, |shortcut| {
        shortcut_column
            .saturating_sub(UnicodeWidthStr::width(left.as_str()))
            .max(1)
            .min(
                width.saturating_sub(
                    UnicodeWidthStr::width(left.as_str())
                        .saturating_add(UnicodeWidthStr::width(shortcut)),
                ),
            )
    });
    let mut spans = vec![
        Span::styled(prefix.to_owned(), prefix_style),
        Span::styled(label.to_owned(), text_style),
    ];
    if let Some(shortcut) = shortcut {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(shortcut.to_owned(), shortcut_style));
    }
    Line::from(spans)
}

fn welcome_status_row(
    left_label: &str,
    left_value: &str,
    left_value_style: Style,
    right: Option<(&str, &str, Style)>,
    width: usize,
    label_style: Style,
) -> Line<'static> {
    let label_width = 10;
    let left_label = format!("// {left_label:<label_width$}");
    let mut spans = vec![
        Span::styled(left_label.clone(), label_style),
        Span::styled(left_value.to_owned(), left_value_style),
    ];
    if let Some((right_label, right_value, right_value_style)) = right {
        let left_width = UnicodeWidthStr::width(left_label.as_str())
            .saturating_add(UnicodeWidthStr::width(left_value));
        let right_column = width.min(48);
        let gap = right_column.saturating_sub(left_width).max(3);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(
            format!("// {right_label:<label_width$}"),
            label_style,
        ));
        spans.push(Span::styled(right_value.to_owned(), right_value_style));
    }
    Line::from(spans)
}

fn welcome_workspace(workspace: &str) -> String {
    let user_home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    let display = user_home
        .as_deref()
        .and_then(|user_home| workspace.strip_prefix(user_home))
        .map_or_else(|| workspace.to_owned(), |relative| format!("~{relative}"));
    format!("{} · new session", sanitize_approval_field(&display))
}

pub(super) fn transcript_lines<'a>(state: &'a TuiState, width: usize) -> Vec<Line<'a>> {
    transcript_lines_range(state, width, 0, state.transcript.len(), false)
}

pub(super) fn transcript_lines_range<'a>(
    state: &'a TuiState,
    width: usize,
    start: usize,
    end: usize,
    leading_separator: bool,
) -> Vec<Line<'a>> {
    transcript_lines_range_with_images(state, width, start, end, leading_separator).0
}

struct TranscriptImagePlacement {
    line: usize,
    indent: usize,
    digest: String,
}

fn transcript_lines_range_with_images<'a>(
    state: &'a TuiState,
    width: usize,
    start: usize,
    end: usize,
    leading_separator: bool,
) -> (Vec<Line<'a>>, Vec<TranscriptImagePlacement>) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let mut lines = Vec::new();
    let mut image_placements = Vec::new();
    let mut visible_entries = 0_usize;
    let start = start.min(state.transcript.len());
    let end = end.min(state.transcript.len());
    for (offset, entry) in state.transcript[start..end].iter().enumerate() {
        let entry_index = start.saturating_add(offset);
        if entry.document.is_empty() {
            continue;
        }
        if (visible_entries > 0 || leading_separator)
            && state.preferences.transcript_density
                == colossus_contracts::TranscriptDensity::Comfortable
        {
            lines.push(Line::default());
        }
        visible_entries += 1;
        let (marker, label) = match entry.kind {
            TranscriptKind::User => ("›", "You"),
            TranscriptKind::Assistant => ("●", "Colossus"),
            TranscriptKind::Tool => ("◆", "Tool"),
            TranscriptKind::Command => ("›", "Command"),
            TranscriptKind::Error => ("!", "Error"),
        };
        let label_style = match entry.kind {
            TranscriptKind::User => palette.user_style(),
            TranscriptKind::Command => palette.meta_style(),
            TranscriptKind::Assistant => palette.assistant_style(),
            TranscriptKind::Tool => palette.tool_style(),
            TranscriptKind::Error => palette.error_style(),
        };
        let has_semantic_heading = entry
            .document
            .blocks
            .first()
            .is_some_and(|block| matches!(block, PresentationBlock::Card { .. }));
        let show_label = state.preferences.transcript_density
            == colossus_contracts::TranscriptDensity::Comfortable
            && !(has_semantic_heading
                && matches!(
                    entry.kind,
                    TranscriptKind::Tool | TranscriptKind::Command | TranscriptKind::Error
                ));
        if show_label {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    ratatui_style(label_style).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    label,
                    ratatui_style(label_style).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        let content_width = width.saturating_sub(if show_label { 2 } else { 0 }).max(1);
        let (rendered, images) = if state.security_posture_entry == Some(entry_index) {
            (security_posture_lines(state, content_width), Vec::new())
        } else {
            transcript_document_lines(state, &entry.document, content_width)
        };
        let rendered_start = lines.len();
        image_placements.extend(images.into_iter().map(|(line, digest)| {
            TranscriptImagePlacement {
                line: rendered_start.saturating_add(line),
                indent: usize::from(show_label) * 2,
                digest,
            }
        }));
        lines.extend(rendered.into_iter().map(|mut line| {
            if show_label && !line.spans.is_empty() {
                line.spans
                    .insert(0, Span::styled("  ", ratatui_style(palette.meta_style())));
            }
            line
        }));
    }
    (lines, image_placements)
}

fn transcript_document_lines(
    state: &TuiState,
    document: &PresentationDocument,
    width: usize,
) -> (Vec<Line<'static>>, Vec<(usize, String)>) {
    let renderer = StyledDocumentRenderer::for_transcript(state.preferences.clone(), width);
    let mut lines = Vec::new();
    let mut images = Vec::new();
    for block in &document.blocks {
        if let PresentationBlock::Image(image) = block
            && let Some(preview) = state
                .preview_cache
                .lines(&image.sha256, state.preview_transcript_size)
        {
            images.push((lines.len(), image.sha256.clone()));
            lines.extend(preview.iter().cloned());
        }
        lines.extend(
            renderer
                .render(&PresentationDocument::from_block(block.clone()))
                .into_iter()
                .map(|line| {
                    Line::from(
                        line.spans
                            .into_iter()
                            .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                            .collect::<Vec<_>>(),
                    )
                }),
        );
    }
    (lines, images)
}

pub(super) fn security_posture_lines(state: &TuiState, width: usize) -> Vec<Line<'static>> {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let warning = ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD);
    let mut primary_style = palette.assistant_style();
    primary_style.bold = true;
    primary_style.dim = false;
    let primary = ratatui_style(primary_style);
    let mut recommendation_style = palette.meta_style();
    recommendation_style.dim = true;
    let recommendation = ratatui_style(recommendation_style);
    let count = state.security_posture.finding_count();
    let mut lines = vec![Line::from(vec![
        Span::styled("! ", warning),
        Span::styled(
            format!(
                "Security posture · {count} warning{}",
                if count == 1 { "" } else { "s" }
            ),
            warning,
        ),
    ])];

    for finding in &state.security_posture.findings {
        let summary = sanitize_approval_field(&finding.summary);
        let summary_prefix = "  • ";
        let summary_continuation = "    ";
        let summary_width = width
            .saturating_sub(UnicodeWidthStr::width(summary_prefix))
            .max(1);
        for (index, segment) in wrap_approval_value(&summary, summary_width)
            .into_iter()
            .enumerate()
        {
            lines.push(Line::from(vec![
                Span::styled(
                    if index == 0 {
                        summary_prefix
                    } else {
                        summary_continuation
                    },
                    warning,
                ),
                Span::styled(segment, primary),
            ]));
        }

        let remediation = sanitize_approval_field(match finding.code.as_str() {
            "sandbox.danger_full_access" => {
                "Use an isolating native, windows_job, or oci boundary."
            }
            _ => &finding.remediation,
        });
        let recommendation_prefix = "    Recommendation: ";
        let recommendation_continuation = "                    ";
        let recommendation_width = width
            .saturating_sub(UnicodeWidthStr::width(recommendation_prefix))
            .max(1);
        for (index, segment) in wrap_approval_value(&remediation, recommendation_width)
            .into_iter()
            .enumerate()
        {
            lines.push(Line::from(vec![
                Span::styled(
                    if index == 0 {
                        recommendation_prefix
                    } else {
                        recommendation_continuation
                    },
                    recommendation,
                ),
                Span::styled(segment, recommendation),
            ]));
        }
    }
    if state.welcome_visible {
        lines.push(Line::default());
    }
    lines
}

pub(super) fn render_activity(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let elapsed = state
        .started_at
        .map_or(0.0, |started| started.elapsed().as_secs_f64());
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let frame_text = palette.activity_frame(elapsed, false);
    let activity = state.activity.as_deref().unwrap_or("working");
    let queued = if state.queue.is_empty() {
        String::new()
    } else {
        format!(" · {} queued", state.queue.len())
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {frame_text} "),
                ratatui_style(palette.activity_style()),
            ),
            Span::styled(
                format!("{activity} · {elapsed:.1}s{queued}"),
                ratatui_style(palette.activity_style()),
            ),
        ])),
        area,
    );
}

pub(super) fn render_completion_menu(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let candidates = state.completion_menu_candidates();
    let Some(context) = state.structured_completion_context() else {
        return;
    };
    if candidates.is_empty() || area.height < 3 {
        return;
    }
    let selected = state.composer.completion_index.unwrap_or(0) % candidates.len();
    let visible_rows = usize::from(area.height.saturating_sub(2));
    let first = selected
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(candidates.len().saturating_sub(visible_rows));
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let candidate_style = match context.kind {
        CompletionKind::Command => palette.assistant_style(),
        CompletionKind::Skill => palette.tool_style(),
    };
    let visible_candidates = candidates
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .collect::<Vec<_>>();
    let command_width = visible_candidates
        .iter()
        .map(|(_, candidate)| UnicodeWidthStr::width(**candidate))
        .max()
        .unwrap_or(1)
        .min(MAX_COMMAND_COLUMN_WIDTH);
    let description_width = if context.kind == CompletionKind::Command {
        visible_candidates
            .iter()
            .filter_map(|(_, candidate)| command_description(candidate))
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
            .min(MAX_DESCRIPTION_COLUMN_WIDTH)
    } else {
        0
    };
    let label = match context.kind {
        CompletionKind::Command => "Commands",
        CompletionKind::Skill => "Skills",
    };
    let title = format!(" {label} · {} ", candidates.len());
    let controls = " ↑/↓ select · Tab accept ";
    let desired_content_width =
        2 + command_width + usize::from(description_width > 0) * (2 + description_width);
    let desired_width = (desired_content_width + 2)
        .max(UnicodeWidthStr::width(title.as_str()) + 2)
        .max(UnicodeWidthStr::width(controls) + 2);
    let minimum_width = if area.width >= ROOMY_COMPLETION_MENU_WIDTH
        && area.height >= u16::try_from(ROOMY_COMPLETION_MENU_ROWS + 2).unwrap_or(u16::MAX)
    {
        ROOMY_COMPLETION_MENU_WIDTH
    } else {
        MIN_COMPLETION_MENU_WIDTH
    };
    let menu_width = u16::try_from(desired_width)
        .unwrap_or(u16::MAX)
        .max(minimum_width.min(area.width))
        .min(area.width);
    let menu_area = Rect::new(area.x, area.y, menu_width, area.height);
    let content_width = usize::from(menu_width.saturating_sub(2)).max(1);
    let available_after_marker = content_width.saturating_sub(2);
    let show_descriptions = description_width > 0
        && available_after_marker >= command_width + 2 + MIN_DESCRIPTION_COLUMN_WIDTH;
    let command_column_width = if show_descriptions {
        command_width.min(available_after_marker.saturating_sub(2 + MIN_DESCRIPTION_COLUMN_WIDTH))
    } else {
        command_width.min(available_after_marker)
    };
    let description_column_width = available_after_marker
        .saturating_sub(command_column_width)
        .saturating_sub(2);
    let description_style = ratatui_style(palette.meta_style()).add_modifier(Modifier::DIM);
    let lines = visible_candidates
        .into_iter()
        .map(|(index, candidate)| {
            let is_selected = index == selected;
            let marker = if is_selected { "› " } else { "  " };
            let style = if is_selected {
                ratatui_style(candidate_style).add_modifier(Modifier::BOLD)
            } else {
                ratatui_style(candidate_style)
            };
            let command = truncate_width(candidate, command_column_width);
            let mut spans = vec![
                Span::styled(marker, style),
                Span::styled(format!("{command:<command_column_width$}"), style),
            ];
            if show_descriptions {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate_width(
                        command_description(candidate).unwrap_or_default(),
                        description_column_width,
                    ),
                    description_style,
                ));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Clear, menu_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_bottom(Span::styled(controls, ratatui_style(palette.meta_style()))),
        ),
        menu_area,
    );
}

pub(super) fn render_composer(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let ghost = if state.welcome_visible && state.composer.draft.is_empty() {
        "Implement {feature}"
    } else {
        state.ghost_text().unwrap_or("")
    };
    let layout = composer_layout(
        &state.composer.draft,
        ghost,
        state.composer.cursor,
        composer_inner_width(area.width),
    );
    let mut text = pending_thumbnail_lines(state);
    let preview_rows = text.len();
    let visible_rows = usize::from(area.height.saturating_sub(2)).saturating_sub(preview_rows);
    let first_visible_row = layout
        .cursor_row
        .saturating_sub(visible_rows.saturating_sub(1));
    let mut ghost_style = palette.meta_style();
    ghost_style.dim = true;
    text.extend(
        layout
            .lines
            .iter()
            .skip(first_visible_row)
            .take(visible_rows)
            .map(|line| {
                Line::from(vec![
                    Span::raw(line.draft.clone()),
                    Span::styled(line.ghost.clone(), ratatui_style(ghost_style)),
                ])
            }),
    );
    let action = if state.preferences.multiline {
        "Ctrl/Alt+Enter sends"
    } else {
        "Enter sends"
    };
    let title = if state.plan_review_decision_active() {
        " Message · paused for plan review · draft preserved ".into()
    } else if state.plan_execution_decision_active() {
        " Message · paused for plan execution · draft preserved ".into()
    } else if let Some(kind) = state.docked_decision_kind() {
        let decision = match kind {
            InteractivePromptKind::Approval => "approval",
            InteractivePromptKind::SandboxBoundaryAcknowledgement => "boundary acknowledgement",
            InteractivePromptKind::UserInput | InteractivePromptKind::Choice => "decision",
        };
        format!(" Message · paused for {decision} · draft preserved ")
    } else {
        match state.mode {
            InteractiveMode::Execute if state.welcome_visible => {
                format!(" Execute · {action} ")
            }
            InteractiveMode::Execute => format!(" Message · {action} "),
            InteractiveMode::Research => format!(" Research · sourced question · {action} "),
            InteractiveMode::Plan if state.selected_plan.is_none() => {
                format!(" Plan · new draft · {action} ")
            }
            InteractiveMode::Plan => {
                let plan = state
                    .selected_plan
                    .as_ref()
                    .expect("selected plan checked above");
                if plan.status == PlanStatus::Approved {
                    format!(
                        " Plan {} · approved · use /plan execute ",
                        short_plan_id(&plan.id)
                    )
                } else {
                    format!(
                        " Plan {} · draft r{} · message refines · {action} · /plan approve ",
                        short_plan_id(&plan.id),
                        plan.revision
                    )
                }
            }
        }
    };
    let mut composer_block = Block::default().borders(Borders::ALL).title(title);
    if !state.sticky_skills.is_empty() {
        composer_block = composer_block.title_bottom(format!(
            " Skills: {} · /plugin remove ID ",
            state.sticky_skills.join(", ")
        ));
    }
    frame.render_widget(Paragraph::new(text).block(composer_block), area);
    if state.preview_cache.native_graphics() {
        let pending = state
            .pending_images
            .iter()
            .take(3)
            .map(|image| image.sha256.clone())
            .collect::<Vec<_>>();
        for (index, digest) in pending.into_iter().enumerate() {
            let thumbnail_area = Rect::new(
                area.x
                    .saturating_add(1)
                    .saturating_add(u16::try_from(index).unwrap_or(u16::MAX).saturating_mul(19)),
                area.y.saturating_add(1),
                18,
                5,
            );
            if thumbnail_area.right() <= area.right().saturating_sub(1)
                && thumbnail_area.bottom() <= area.bottom().saturating_sub(1)
            {
                state
                    .preview_cache
                    .render_native(frame, &digest, Size::new(18, 5), thumbnail_area);
            }
        }
    }
    let cursor_row = layout.cursor_row.saturating_sub(first_visible_row);
    let x = area
        .x
        .saturating_add(1)
        .saturating_add(u16::try_from(layout.cursor_column).unwrap_or(u16::MAX));
    let y = area
        .y
        .saturating_add(1)
        .saturating_add(u16::try_from(preview_rows).unwrap_or(u16::MAX))
        .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX));
    if state.overlay.is_none()
        && x < area.right().saturating_sub(1)
        && y < area.bottom().saturating_sub(1)
    {
        frame.set_cursor_position((x, y));
    }
}

fn pending_thumbnail_lines(state: &TuiState) -> Vec<Line<'static>> {
    if state.pending_images.is_empty() {
        return Vec::new();
    }
    let visible = state.pending_images.iter().take(3).collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(6);
    for row in 0..5 {
        let mut spans = Vec::new();
        for (index, image) in visible.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            if let Some(preview) = state.preview_cache.lines(&image.sha256, Size::new(18, 5)) {
                if let Some(line) = preview.get(row) {
                    spans.extend(line.spans.clone());
                }
            } else {
                spans.push(Span::raw(if row == 2 {
                    "   loading image  "
                } else {
                    "                  "
                }));
            }
        }
        lines.push(Line::from(spans));
    }
    let mut labels = visible
        .iter()
        .map(|image| truncate_width(&image.file_name, 18))
        .collect::<Vec<_>>()
        .join(" ");
    let more = state.pending_images.len().saturating_sub(3);
    if more > 0 {
        labels.push_str(&format!("  +{more} more"));
    }
    lines.push(Line::from(labels));
    lines
}

pub(super) fn render_footer(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let width = usize::from(area.width);
    let short_session = state.session_id.chars().take(8).collect::<String>();
    if state.welcome_visible {
        render_branded_footer(frame, state, area, &welcome_workspace(&state.workspace));
        return;
    }

    let mut segments = vec![short_session, format!("mode={}", state.mode.as_str())];
    if width >= 72
        && let Some(plan) = state.selected_plan.as_ref()
    {
        segments.push(format!(
            "plan={}:r{}:{}",
            short_plan_id(&plan.id),
            plan.revision,
            plan_status_label(plan.status)
        ));
    }
    if width >= 60 {
        segments.push(format!("{}:{}", state.footer.role, state.footer.route));
    }
    if width >= 90
        && let Some((used, maximum)) = state.footer.context
    {
        segments.push(format!("ctx={used}/{maximum}"));
    }
    if width >= 110 {
        segments.push(format!("msgs={}", state.footer.message_count));
        segments.push(format!("approval={}", state.footer.approval_mode));
    }
    segments.push(format!("status={}", state.footer.status));
    render_branded_footer(frame, state, area, &segments.join(" · "));
}

fn render_branded_footer(frame: &mut Frame<'_>, state: &TuiState, area: Rect, footer: &str) {
    let width = usize::from(area.width);
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let surface = filled_approval_control_style(palette.meta_style(), false);
    let brand_width = UnicodeWidthStr::width(OBSCURITY_LABS_FOOTER_SEGMENT);
    if state.security_posture.is_hardened() {
        let footer = truncate_width(footer, width.saturating_sub(brand_width));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    OBSCURITY_LABS_FOOTER_SEGMENT,
                    obscurity_labs_footer_style(&state.preferences),
                ),
                Span::styled(footer, ratatui_style(palette.meta_style())),
            ]))
            .style(surface),
            area,
        );
    } else {
        let badge = format!(" ⚠ Security: {} ", state.security_posture.finding_count());
        let remaining = width
            .saturating_sub(UnicodeWidthStr::width(badge.as_str()))
            .saturating_sub(brand_width);
        let footer = truncate_width(footer, remaining);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    OBSCURITY_LABS_FOOTER_SEGMENT,
                    obscurity_labs_footer_style(&state.preferences),
                ),
                Span::styled(
                    badge,
                    filled_approval_control_style(palette.warning_style(), true),
                ),
                Span::styled(footer, ratatui_style(palette.meta_style())),
            ]))
            .style(surface),
            area,
        );
    }
}

fn obscurity_labs_brand_style(preferences: &TerminalPreferences) -> Style {
    let style = Style::default().add_modifier(Modifier::BOLD);
    if preferences.theme == colossus_contracts::ThemeName::Mono {
        style
    } else {
        style.fg(OBSCURITY_LABS_RED)
    }
}

fn obscurity_labs_footer_style(preferences: &TerminalPreferences) -> Style {
    ratatui_style(TerminalPalette::for_preferences(preferences).meta_style())
        .add_modifier(Modifier::BOLD)
}

pub(super) fn render_overlay(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    if state.docked_decision_kind().is_some() {
        render_approval_dock(frame, state, area);
        return;
    }
    if state.plan_decision_active() {
        render_plan_execution_dock(frame, state, area);
        return;
    }
    if let Some(Overlay::SessionBrowser(browser)) = state.overlay.as_ref() {
        render_session_browser(frame, state, browser, area);
        return;
    }
    if let Some(Overlay::ThemePicker(picker)) = state.overlay.as_ref() {
        render_theme_picker(frame, state, picker, area);
        return;
    }
    let overlay_area = match state.overlay.as_ref() {
        Some(Overlay::Prompt { request, .. })
            if request.document.is_empty() && !request.choices.is_empty() =>
        {
            picker_rect(area, &request.choices)
        }
        _ => centered_rect(80, 60, area),
    };
    frame.render_widget(Clear, overlay_area);
    let (title, mut lines) = match state.overlay.as_ref() {
        Some(Overlay::Prompt {
            request,
            input,
            selected,
            ..
        }) => {
            let inner_width = usize::from(overlay_area.width.saturating_sub(2)).max(1);
            let inner_height = usize::from(overlay_area.height.saturating_sub(2)).max(1);
            let lines = prompt_lines(
                request,
                input,
                *selected,
                &state.preferences,
                inner_width,
                inner_height,
            );
            let title = selected.map_or_else(
                || request.title.clone(),
                |selected| {
                    format!(
                        "{} · {}/{}",
                        request.title,
                        selected + 1,
                        request.choices.len()
                    )
                },
            );
            (title, lines)
        }
        Some(Overlay::HistorySearch { query }) => (
            "History search".into(),
            vec![
                Line::from(format!("> {query}")),
                Line::default(),
                Line::from(
                    state
                        .history
                        .iter()
                        .rev()
                        .find(|entry| entry.contains(query.as_str()))
                        .map_or("No match", String::as_str)
                        .to_owned(),
                ),
            ],
        ),
        Some(Overlay::QueuePaused) => (
            "Queued turns paused".into(),
            vec![
                Line::from("The prior run failed or was cancelled."),
                Line::from(format!("{} queued turn(s) remain.", state.queue.len())),
                Line::default(),
                Line::from("Enter/R: resume queue · C: clear queue · Esc: keep paused"),
            ],
        ),
        Some(
            Overlay::SessionBrowser(_)
            | Overlay::ThemePicker(_)
            | Overlay::PlanReviewChoice { .. }
            | Overlay::PlanExecutionChoice { .. },
        ) => return,
        None => return,
    };
    if lines.is_empty() {
        lines.push(Line::default());
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(Style::default().bg(Color::Reset)),
            )
            .wrap(Wrap { trim: false }),
        overlay_area,
    );
}

pub(super) fn approval_dock_height(
    state: &TuiState,
    total_height: u16,
    composer_height: u16,
    activity_height: u16,
) -> u16 {
    if state.docked_decision_kind().is_none() {
        return 0;
    }
    let available = total_height
        .saturating_sub(MINIMUM_APPROVAL_TRANSCRIPT_ROWS)
        .saturating_sub(activity_height)
        .saturating_sub(composer_height)
        .saturating_sub(1);
    if available < MIN_APPROVAL_DOCK_ROWS {
        return 0;
    }
    available.min(MAX_APPROVAL_DOCK_ROWS)
}

fn render_approval_dock(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let Some(Overlay::Prompt {
        request,
        selected,
        approval_section,
        document_scroll,
        ..
    }) = state.overlay.as_ref()
    else {
        return;
    };
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let title = selected.map_or_else(
        || format!(" {} · {} ", request.title, approval_section.label()),
        |index| {
            format!(
                " {} · {} · {}/{} ",
                request.title,
                approval_section.label(),
                index + 1,
                request.choices.len()
            )
        },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, ratatui_style(palette.warning_style())));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let controls = inner.height.min(3);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(u16::from(controls >= 3)),
            Constraint::Length(u16::from(controls >= 2)),
            Constraint::Length(u16::from(controls >= 1)),
        ])
        .split(inner);
    let document_lines = approval_section_lines(
        &request.document,
        request.kind,
        *approval_section,
        &state.preferences,
        &palette,
        usize::from(rows[0].width).max(1),
    );
    let visible = usize::from(rows[0].height);
    let document_line_count = document_lines.len();
    let maximum_scroll = document_line_count.saturating_sub(visible);
    let scroll = (*document_scroll).min(maximum_scroll);
    frame.render_widget(
        Paragraph::new(document_lines)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    if rows[1].height > 0 {
        frame.render_widget(
            Paragraph::new(approval_section_line(
                *approval_section,
                &palette,
                rows[1].width,
            )),
            rows[1],
        );
    }
    if rows[2].height > 0 {
        frame.render_widget(
            Paragraph::new(approval_choice_line(
                request,
                *selected,
                &palette,
                rows[2].width,
            )),
            rows[2],
        );
    }
    if rows[3].height > 0 {
        let dismissal = if request.kind == InteractivePromptKind::SandboxBoundaryAcknowledgement {
            "Esc keep blocked"
        } else {
            "Esc deny"
        };
        let mut hint = format!("{dismissal} · ↑/↓ choose · Enter confirm · Tab sections");
        if maximum_scroll > 0 {
            let first = scroll.saturating_add(1);
            let last = scroll.saturating_add(visible).min(document_line_count);
            hint.push_str(&format!(
                " · PgUp/PgDn details {first}-{last}/{}",
                document_line_count
            ));
        }
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_width(&hint, usize::from(rows[3].width)),
                ratatui_style(palette.warning_style()),
            )),
            rows[3],
        );
    }
}

fn approval_section_lines(
    document: &PresentationDocument,
    kind: InteractivePromptKind,
    section: ApprovalSection,
    preferences: &TerminalPreferences,
    palette: &TerminalPalette,
    width: usize,
) -> Vec<Line<'static>> {
    if section == ApprovalSection::Summary && kind == InteractivePromptKind::Approval {
        let mut entries = Vec::new();
        collect_approval_summary_entries(&document.blocks, &mut entries);
        if !entries.is_empty() {
            return compact_approval_summary_lines(&entries, palette, width);
        }
    }
    styled_document_lines(
        &approval_section_document(document, kind, section),
        preferences,
        width,
    )
}

fn collect_approval_summary_entries(
    blocks: &[PresentationBlock],
    entries: &mut Vec<(String, String)>,
) {
    for block in blocks {
        match block {
            PresentationBlock::KeyValue(values) => entries.extend(values.iter().cloned()),
            PresentationBlock::Card { body, .. } => {
                collect_approval_summary_entries(body, entries);
            }
            _ => {}
        }
    }
}

pub(super) fn compact_approval_summary_lines(
    entries: &[(String, String)],
    palette: &TerminalPalette,
    width: usize,
) -> Vec<Line<'static>> {
    let label_width = entries
        .iter()
        .map(|(label, _)| UnicodeWidthStr::width(label.as_str()))
        .max()
        .unwrap_or(1)
        .min((width / 3).clamp(8, 18));
    let value_width = width.saturating_sub(label_width.saturating_add(2)).max(1);
    let mut label_style = palette.meta_style();
    label_style.bold = true;
    label_style.dim = false;
    let mut value_style = palette.meta_style();
    value_style.dim = false;

    let mut lines = Vec::new();
    for (label, value) in entries {
        let label = sanitize_approval_field(label);
        let label = truncate_width_with_ellipsis(&label, label_width);
        let padding = label_width.saturating_sub(UnicodeWidthStr::width(label.as_str()));
        let value = sanitize_approval_field(value);
        for (index, segment) in wrap_approval_value(&value, value_width)
            .into_iter()
            .enumerate()
        {
            let gutter = if index == 0 {
                format!("{label}{}", " ".repeat(padding))
            } else {
                " ".repeat(label_width)
            };
            lines.push(Line::from(vec![
                Span::styled(gutter, ratatui_style(label_style)),
                Span::raw("  "),
                Span::styled(segment, ratatui_style(value_style)),
            ]));
        }
    }
    lines
}

/// Wraps a sanitized approval value to `width` display columns without
/// discarding any character, so authorization-relevant detail stays legible and
/// scrollable instead of being replaced by an ellipsis.
pub(super) fn wrap_approval_value(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut break_at = None;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if !current.is_empty() && current_width + character_width > width {
            match break_at.filter(|index| *index < current.len()) {
                Some(index) => {
                    let remainder = current.split_off(index);
                    lines.push(std::mem::take(&mut current));
                    current_width = UnicodeWidthStr::width(remainder.as_str());
                    current = remainder;
                }
                None => {
                    lines.push(std::mem::take(&mut current));
                    current_width = 0;
                }
            }
            break_at = None;
        }
        current.push(character);
        current_width += character_width;
        if character == ' ' {
            break_at = Some(current.len());
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

pub(super) fn sanitize_approval_field(value: &str) -> String {
    value
        .chars()
        .filter_map(|character| match character {
            '\n' | '\r' | '\t' => Some(' '),
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' => None,
            character if character.is_control() => None,
            character => Some(character),
        })
        .take(1024 * 1024)
        .collect()
}

pub(super) fn truncate_width_with_ellipsis(value: &str, maximum: usize) -> String {
    if UnicodeWidthStr::width(value) <= maximum {
        return value.to_owned();
    }
    if maximum == 0 {
        return String::new();
    }
    let mut truncated = truncate_width(value, maximum.saturating_sub(1));
    truncated.push('…');
    truncated
}

fn styled_document_lines(
    document: &PresentationDocument,
    preferences: &TerminalPreferences,
    width: usize,
) -> Vec<Line<'static>> {
    StyledDocumentRenderer::for_transcript(preferences.clone(), width)
        .render(document)
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

pub(super) fn approval_section_document(
    document: &PresentationDocument,
    kind: InteractivePromptKind,
    section: ApprovalSection,
) -> PresentationDocument {
    match section {
        ApprovalSection::Summary => PresentationDocument {
            blocks: approval_summary_blocks(&document.blocks),
        },
        ApprovalSection::Request => {
            let mut blocks = Vec::new();
            let mut scope = Vec::new();
            collect_approval_summary_entries(&document.blocks, &mut scope);
            if !scope.is_empty() {
                blocks.push(PresentationBlock::KeyValue(
                    scope
                        .into_iter()
                        .map(|(label, value)| {
                            (
                                sanitize_approval_field(&label),
                                sanitize_approval_field(&value),
                            )
                        })
                        .collect(),
                ));
            }
            collect_approval_request_blocks(&document.blocks, &mut blocks);
            if blocks.is_empty() {
                blocks.push(PresentationBlock::Text(
                    "No additional prepared-request body was released.".into(),
                ));
            }
            PresentationDocument { blocks }
        }
        ApprovalSection::Protections
            if kind == InteractivePromptKind::SandboxBoundaryAcknowledgement =>
        {
            PresentationDocument::from_block(PresentationBlock::KeyValue(vec![
                (
                    "Fail closed".into(),
                    "Process execution stays blocked unless this acknowledgement is confirmed."
                        .into(),
                ),
                (
                    "Scope".into(),
                    "This acknowledgement covers only this TUI session and is retained only by the current Colossus process."
                        .into(),
                ),
                (
                    "Policy".into(),
                    "Effect policy and separate approval obligations remain active.".into(),
                ),
                (
                    "Isolation".into(),
                    "Acknowledgement does not add filesystem or network isolation.".into(),
                ),
                (
                    "Audit".into(),
                    "The selected boundary mode and acknowledgement remain auditable.".into(),
                ),
            ]))
        }
        ApprovalSection::Protections => {
            PresentationDocument::from_block(PresentationBlock::KeyValue(vec![
                (
                    "Exact scope".into(),
                    "The approval proof is bound to this prepared request.".into(),
                ),
                (
                    "One use".into(),
                    "Replay against another request is rejected.".into(),
                ),
                (
                    "Policy re-check".into(),
                    "Approval satisfies an obligation; it cannot reverse a denial.".into(),
                ),
                (
                    "Containment".into(),
                    "Permit, sandbox, output quarantine, release policy, and audit remain active."
                        .into(),
                ),
            ]))
        }
    }
}

fn approval_summary_blocks(blocks: &[PresentationBlock]) -> Vec<PresentationBlock> {
    let mut summary = Vec::new();
    for block in blocks {
        match block {
            PresentationBlock::Code { .. } | PresentationBlock::Diff(_) => {}
            PresentationBlock::Card { body, .. } => {
                summary.extend(approval_summary_blocks(body));
            }
            block => summary.push(block.clone()),
        }
    }
    summary
}

fn collect_approval_request_blocks(
    blocks: &[PresentationBlock],
    request: &mut Vec<PresentationBlock>,
) {
    for block in blocks {
        match block {
            PresentationBlock::Code { .. } | PresentationBlock::Diff(_) => {
                request.push(block.clone());
            }
            PresentationBlock::Card { body, .. } => {
                collect_approval_request_blocks(body, request);
            }
            _ => {}
        }
    }
}

pub(super) fn approval_section_line(
    selected: ApprovalSection,
    palette: &TerminalPalette,
    width: u16,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (section, shortcut)) in [
        (ApprovalSection::Summary, "S"),
        (ApprovalSection::Request, "R"),
        (ApprovalSection::Protections, "P"),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            spans.push(Span::raw(if width < 56 { " " } else { "  " }));
        }
        let style = if section == selected {
            filled_approval_control_style(palette.warning_style(), true)
        } else {
            filled_approval_control_style(palette.meta_style(), false)
        };
        let label = if width < 56 {
            match section {
                ApprovalSection::Summary => "Summary",
                ApprovalSection::Request => "Request",
                ApprovalSection::Protections => "Protect",
            }
        } else {
            section.label()
        };
        let content = if width < 56 {
            format!("[{shortcut}] {label}")
        } else {
            format!(" [{shortcut}] {label} ")
        };
        spans.push(Span::styled(
            truncate_width(&content, usize::from(width)),
            style,
        ));
    }
    Line::from(spans)
}

fn approval_choice_line(
    request: &InteractivePrompt,
    selected: Option<usize>,
    palette: &TerminalPalette,
    width: u16,
) -> Line<'static> {
    let mut spans = Vec::new();
    if width >= 72 {
        spans.push(Span::styled(
            "Decision  ",
            ratatui_style(palette.meta_style()).add_modifier(Modifier::BOLD),
        ));
    }
    for (index, choice) in request.choices.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let shortcut = match (request.kind, index, choice.as_str()) {
            (InteractivePromptKind::SandboxBoundaryAcknowledgement, 0, _) => "A",
            (InteractivePromptKind::SandboxBoundaryAcknowledgement, 1, _) => "D",
            (_, _, "Allow once") => "A",
            (_, _, "Deny") => "D",
            _ => " ",
        };
        let style = if selected == Some(index) {
            filled_approval_control_style(palette.warning_style(), true)
        } else {
            filled_approval_control_style(palette.meta_style(), false)
        };
        let content = if width < 56 {
            format!("[{shortcut}] {choice}")
        } else {
            format!(" [{shortcut}] {choice} ")
        };
        spans.push(Span::styled(
            truncate_width(&content, usize::from(width)),
            style,
        ));
    }
    if selected.is_none() {
        let prompt = if width >= 72 {
            "  No decision selected"
        } else {
            " · Select one"
        };
        spans.push(Span::styled(
            prompt,
            ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

pub(super) fn filled_approval_control_style(base: ThemeTextStyle, selected: bool) -> Style {
    let mut style = ratatui_style(base);
    let Some(accent) = base.foreground else {
        style = style.add_modifier(Modifier::REVERSED);
        if !selected {
            style = style.add_modifier(Modifier::DIM);
        }
        return style;
    };

    let accent_color = Color::Rgb(accent.red, accent.green, accent.blue);
    if selected {
        style
            .fg(contrasting_terminal_color(accent))
            .bg(accent_color)
            .add_modifier(Modifier::BOLD)
    } else {
        style.fg(accent_color).bg(Color::Rgb(
            subdued_approval_channel(accent.red),
            subdued_approval_channel(accent.green),
            subdued_approval_channel(accent.blue),
        ))
    }
}

const fn subdued_approval_channel(value: u8) -> u8 {
    (value / 10) * 3 + ((value % 10) * 3) / 10
}

fn contrasting_terminal_color(background: colossus_contracts::ThemeColor) -> Color {
    let luminance = u32::from(background.red) * 299
        + u32::from(background.green) * 587
        + u32::from(background.blue) * 114;
    if luminance >= 150_000 {
        Color::Black
    } else {
        Color::White
    }
}

pub(super) fn picker_rect(area: Rect, choices: &[String]) -> Rect {
    let width = if area.width <= 60 {
        area.width
    } else {
        area.width.saturating_sub(8).min(96)
    };
    let choice_rows = choices
        .iter()
        .map(|choice| choice.lines().count().max(1))
        .sum::<usize>();
    let desired_height = u16::try_from(choice_rows.min(14).saturating_add(3))
        .unwrap_or(u16::MAX)
        .min(area.height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(desired_height) / 2),
        width,
        desired_height,
    )
}

pub(super) fn prompt_lines(
    request: &InteractivePrompt,
    input: &str,
    selected: Option<usize>,
    preferences: &TerminalPreferences,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let palette = TerminalPalette::for_preferences(preferences);
    let document = StyledDocumentRenderer::new(preferences.clone(), width)
        .render(&request.document)
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let choice_rows = request
        .choices
        .iter()
        .map(|choice| choice.lines().count().max(1))
        .sum::<usize>();
    let footer_rows = usize::from(height > 0);
    let minimum_document_rows = if document.is_empty() {
        0
    } else {
        document.len().min(3)
    };
    let choice_budget = choice_rows.min(
        height
            .saturating_sub(footer_rows)
            .saturating_sub(minimum_document_rows),
    );
    let document_budget = height
        .saturating_sub(footer_rows)
        .saturating_sub(choice_budget);
    let mut lines = document
        .into_iter()
        .take(document_budget)
        .collect::<Vec<_>>();

    if choice_budget > 0 {
        let focus = selected
            .unwrap_or(0)
            .min(request.choices.len().saturating_sub(1));
        let (start, end) = visible_choice_range(&request.choices, focus, choice_budget);
        let mut remaining = choice_budget;
        for (index, choice) in request
            .choices
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            let is_selected = selected == Some(index);
            let marker = if is_selected { "› " } else { "  " };
            let prefix = format!("{marker}{}. ", index + 1);
            let continuation = " ".repeat(prefix.chars().count());
            for (line_index, content) in choice.lines().enumerate() {
                if remaining == 0 {
                    break;
                }
                let prefix = if line_index == 0 {
                    prefix.as_str()
                } else {
                    continuation.as_str()
                };
                let available = width.saturating_sub(UnicodeWidthStr::width(prefix));
                let base_style = if line_index == 0 {
                    palette.assistant_style()
                } else {
                    palette.meta_style()
                };
                let style = if is_selected {
                    ratatui_style(base_style)
                        .add_modifier(Modifier::BOLD)
                        .add_modifier(Modifier::REVERSED)
                } else {
                    ratatui_style(base_style)
                };
                lines.push(Line::from(Span::styled(
                    format!("{prefix}{}", truncate_width(content, available)),
                    style,
                )));
                remaining -= 1;
            }
        }
    }

    if footer_rows > 0 {
        let hint = if !input.is_empty() {
            format!("Choice: {input} · Enter submit · Esc cancel")
        } else if selected.is_some() && !request.choices.is_empty() {
            "↑/↓ move · Enter select · Esc cancel".into()
        } else if !request.choices.is_empty() {
            "↑/↓ move · type a number · Esc cancel".into()
        } else if request.allow_free_form {
            "Type an answer · Enter submit · Esc cancel".into()
        } else {
            "Enter submit · Esc cancel".into()
        };
        lines.push(Line::from(Span::styled(
            truncate_width(&hint, width),
            ratatui_style(palette.warning_style()),
        )));
    }
    lines
}

pub(super) fn visible_choice_range(
    choices: &[String],
    selected: usize,
    row_budget: usize,
) -> (usize, usize) {
    if choices.is_empty() || row_budget == 0 {
        return (0, 0);
    }
    let selected = selected.min(choices.len() - 1);
    let row_count = |choice: &String| choice.lines().count().max(1);
    let mut start = 0;
    let mut used = choices[..=selected].iter().map(row_count).sum::<usize>();
    while start < selected && used > row_budget {
        used = used.saturating_sub(row_count(&choices[start]));
        start += 1;
    }
    let mut end = selected + 1;
    while end < choices.len() {
        let next = row_count(&choices[end]);
        if used.saturating_add(next) > row_budget {
            break;
        }
        used += next;
        end += 1;
    }
    while start > 0 {
        let previous = row_count(&choices[start - 1]);
        if used.saturating_add(previous) > row_budget {
            break;
        }
        start -= 1;
        used += previous;
    }
    (start, end)
}
