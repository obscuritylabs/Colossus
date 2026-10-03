use super::*;

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
        "Ctrl+D sends"
    } else if state.is_busy() {
        "Enter queues"
    } else {
        "Enter sends"
    };
    let title = if state.plan_review_decision_active() {
        " Message · paused for plan review ".into()
    } else if state.plan_execution_decision_active() {
        " Message · paused for plan execution ".into()
    } else if let Some(kind) = state.docked_decision_kind() {
        let decision = match kind {
            InteractivePromptKind::Approval => "approval",
            InteractivePromptKind::SandboxBoundaryAcknowledgement => "boundary acknowledgement",
            InteractivePromptKind::UserInput | InteractivePromptKind::Choice => "decision",
        };
        format!(" Message · paused for {decision} ")
    } else {
        match state.mode {
            InteractiveMode::Execute if state.welcome_visible => {
                format!(" Execute · {action} ")
            }
            InteractiveMode::Execute => format!(" Message · {action} "),
            InteractiveMode::Research => format!(" Research · {action} "),
            InteractiveMode::Plan if state.selected_plan.is_none() => {
                format!(" Plan · new draft · {action} ")
            }
            InteractiveMode::Plan => {
                let plan = state
                    .selected_plan
                    .as_ref()
                    .expect("selected plan checked above");
                if plan.status == PlanStatus::Approved {
                    format!(" Plan {} · approved ", short_plan_id(&plan.id))
                } else {
                    format!(
                        " Plan {} · draft r{} · {action} ",
                        short_plan_id(&plan.id),
                        plan.revision
                    )
                }
            }
        }
    };
    let title: String = title;
    let inner_width = composer_inner_width(area.width);
    let hint = composer_hint(state, &layout, visible_rows, inner_width);
    let surface = chrome_band_style(&palette).bg(Color::Rgb(35, 39, 44));
    let hint_style = ratatui_style(palette.meta_style()).remove_modifier(Modifier::DIM);
    let composer_block = Block::default()
        .style(surface)
        .borders(Borders::ALL)
        .border_style(hint_style)
        .title(Span::styled(
            truncate_width_with_ellipsis(&title, inner_width),
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(hint, hint_style));
    frame.render_widget(
        Paragraph::new(text).style(surface).block(composer_block),
        area,
    );
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

fn composer_hint(
    state: &TuiState,
    layout: &ComposerLayout,
    visible_rows: usize,
    width: usize,
) -> String {
    if state.docked_decision_active() {
        return truncate_width_with_ellipsis(" Draft preserved · respond above ", width);
    }
    let mut hints = Vec::new();
    if state.preferences.multiline {
        hints.push("Enter newline".to_owned());
    } else {
        hints.push(
            if width >= 64 {
                "Shift+Enter / Ctrl+J newline"
            } else {
                "Ctrl+J newline"
            }
            .to_owned(),
        );
    }
    let draft_rows = layout.cursor_stops.last().map_or(1, |stop| stop.row + 1);
    if draft_rows > visible_rows {
        hints.insert(0, format!("row {}/{}", layout.cursor_row + 1, draft_rows));
    }
    if !state.sticky_skills.is_empty() {
        hints.insert(
            0,
            format!(
                "Skills: {}",
                sanitize_approval_field(&state.sticky_skills.join(", "))
            ),
        );
    } else if let Some(plan) = &state.selected_plan {
        match plan.status {
            PlanStatus::Draft => hints.push("/plan approve".to_owned()),
            PlanStatus::Approved => hints.insert(0, "/plan execute".to_owned()),
            _ => {}
        }
    }
    hints.extend([
        "/ commands".to_owned(),
        "@ skills".to_owned(),
        "Ctrl+R history".to_owned(),
    ]);
    let mut hint = String::new();
    for segment in hints {
        let candidate = if hint.is_empty() {
            format!(" {segment} ")
        } else {
            format!("{}· {segment} ", hint)
        };
        if UnicodeWidthStr::width(candidate.as_str()) <= width {
            hint = candidate;
        } else if hint.is_empty() {
            hint = truncate_width_with_ellipsis(&candidate, width);
        }
    }
    hint
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

pub(super) fn composer_height(state: &TuiState, width: u16) -> u16 {
    let layout = composer_layout(
        &state.composer.draft,
        "",
        state.composer.cursor,
        composer_inner_width(width),
    );
    let preview_rows = if state.pending_images.is_empty() {
        0
    } else {
        6
    };
    u16::try_from(layout.lines.len().clamp(1, 6) + preview_rows + 2).unwrap_or(14)
}

pub(super) fn composer_inner_width(width: u16) -> usize {
    usize::from(width.saturating_sub(2)).max(1)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ComposerVisualLine {
    pub(super) draft: String,
    pub(super) ghost: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ComposerLayout {
    pub(super) lines: Vec<ComposerVisualLine>,
    pub(super) cursor_row: usize,
    pub(super) cursor_column: usize,
    pub(super) cursor_stops: Vec<ComposerCursorStop>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ComposerCursorStop {
    pub(super) byte: usize,
    pub(super) row: usize,
    pub(super) column: usize,
}

pub(super) fn composer_layout(
    draft: &str,
    ghost: &str,
    cursor: usize,
    width: usize,
) -> ComposerLayout {
    debug_assert!(cursor <= draft.len() && draft.is_char_boundary(cursor));
    let width = width.max(1);
    let mut lines = vec![ComposerVisualLine::default()];
    let mut row = 0;
    let mut column = 0;
    let mut pending_wrap = false;
    let mut cursor_position = None;
    let mut cursor_stops = Vec::new();

    for (value, offset, is_ghost) in [(draft, 0, false), (ghost, draft.len(), true)] {
        for (index, grapheme) in value.grapheme_indices(true) {
            let grapheme_width = UnicodeWidthStr::width(grapheme);
            let wrapped_before_grapheme = if pending_wrap {
                lines.push(ComposerVisualLine::default());
                row += 1;
                column = 0;
                pending_wrap = false;
                true
            } else if grapheme != "\n" && column + grapheme_width > width {
                lines.push(ComposerVisualLine::default());
                row += 1;
                column = 0;
                true
            } else {
                false
            };
            let grapheme_start = offset + index;
            if !is_ghost {
                cursor_stops.push(ComposerCursorStop {
                    byte: grapheme_start,
                    row,
                    column,
                });
            }
            let cursor_inside_grapheme =
                !is_ghost && grapheme_start < cursor && cursor < grapheme_start + grapheme.len();

            if cursor_position.is_none() && grapheme_start == cursor {
                cursor_position = Some((row, column));
            }

            if grapheme == "\n" {
                if !wrapped_before_grapheme {
                    lines.push(ComposerVisualLine::default());
                    row += 1;
                    column = 0;
                }
                continue;
            }

            let line = lines.last_mut().expect("composer has at least one line");
            if is_ghost {
                line.ghost.push_str(grapheme);
            } else {
                line.draft.push_str(grapheme);
            }
            column += grapheme_width;
            pending_wrap = column >= width;
            if cursor_position.is_none() && cursor_inside_grapheme {
                cursor_position = Some(if pending_wrap {
                    (row + 1, 0)
                } else {
                    (row, column)
                });
            }
        }
        if !is_ghost {
            cursor_stops.push(ComposerCursorStop {
                byte: draft.len(),
                row: row + usize::from(pending_wrap),
                column: if pending_wrap { 0 } else { column },
            });
        }
    }

    if pending_wrap {
        lines.push(ComposerVisualLine::default());
        row += 1;
        column = 0;
    }
    let (cursor_row, cursor_column) = cursor_position.unwrap_or((row, column));
    ComposerLayout {
        lines,
        cursor_row,
        cursor_column,
        cursor_stops,
    }
}
