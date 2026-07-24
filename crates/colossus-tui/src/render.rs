use super::*;

pub(super) fn render(frame: &mut Frame<'_>, state: &mut TuiState) {
    let area = frame.area();
    if area.width < MINIMUM_TERMINAL_WIDTH || area.height < MINIMUM_TERMINAL_HEIGHT {
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

    let composer_height = composer_height(state, area.width);
    let activity_height = u16::from(state.operation.is_some());
    let completion_height =
        completion_menu_height(state, area.height, composer_height, activity_height);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(activity_height),
            Constraint::Length(completion_height),
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
    render_transcript(frame, state, rows[0]);
    if activity_height > 0 {
        render_activity(frame, state, rows[1]);
    }
    if completion_height > 0 {
        render_completion_menu(frame, state, rows[2]);
    }
    render_composer(frame, state, rows[3]);
    render_footer(frame, state, rows[4]);
    if state.overlay.is_some() {
        render_overlay(frame, state, area);
    }
}

pub(super) fn render_transcript(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
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
    let lines = transcript_lines(state, width);
    let visible = usize::from(transcript_area.height);
    let live_top = lines.len().saturating_sub(visible);
    let top = live_top.saturating_sub(state.scroll_from_bottom);
    let paragraph = Paragraph::new(lines)
        .scroll((u16::try_from(top).unwrap_or(u16::MAX), 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, transcript_area);
}

pub(super) fn transcript_lines<'a>(state: &'a TuiState, width: usize) -> Vec<Line<'a>> {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let mut lines = Vec::new();
    let mut visible_entries = 0_usize;
    for entry in &state.transcript {
        if entry.document.is_empty() {
            continue;
        }
        if visible_entries > 0
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
        let rendered =
            StyledDocumentRenderer::for_transcript(state.preferences.clone(), content_width)
                .render(&entry.document);
        lines.extend(rendered.into_iter().map(|mut line| {
            if show_label && !line.spans.is_empty() {
                line.spans.insert(
                    0,
                    colossus_presentation::StyledSpan {
                        content: "  ".into(),
                        style: palette.meta_style(),
                    },
                );
            }
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        }));
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
    let menu_width = area.width.saturating_sub(2).min(80);
    let menu_area = Rect::new(area.x.saturating_add(1), area.y, menu_width, area.height);
    let content_width = usize::from(menu_width.saturating_sub(5)).max(1);
    let lines = candidates
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .map(|(index, candidate)| {
            let is_selected = index == selected;
            let marker = if is_selected { "› " } else { "  " };
            let style = if is_selected {
                ratatui_style(candidate_style)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                ratatui_style(candidate_style)
            };
            Line::from(Span::styled(
                format!("{marker}{}", truncate_width(candidate, content_width)),
                style,
            ))
        })
        .collect::<Vec<_>>();
    let label = match context.kind {
        CompletionKind::Command => "Commands",
        CompletionKind::Skill => "Skills",
    };
    frame.render_widget(Clear, menu_area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(format!(
            " {label} · {} matches · ↑/↓ select · Tab accept ",
            candidates.len()
        ))),
        menu_area,
    );
}

pub(super) fn render_composer(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let inner_width = usize::from(area.width.saturating_sub(4)).max(1);
    let (before, after) = state.composer.draft.split_at(state.composer.cursor);
    let ghost = state.ghost_text().unwrap_or("");
    let mut text = Vec::new();
    let logical_lines = format!("{before}{after}{ghost}")
        .split('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // Render the common single-line case with a distinct ghost span. Multiline retains
    // newlines and uses the real terminal cursor for exact position.
    if !state.composer.draft.contains('\n') {
        let mut ghost_style = palette.meta_style();
        ghost_style.dim = true;
        text.push(Line::from(vec![
            Span::raw(before.to_owned()),
            Span::raw(after.to_owned()),
            Span::styled(ghost.to_owned(), ratatui_style(ghost_style)),
        ]));
    } else {
        text.extend(logical_lines.into_iter().map(Line::from));
    }
    let title = if state.preferences.multiline {
        " Message · Ctrl/Alt+Enter sends "
    } else {
        " Message · Enter sends "
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
    let (cursor_row, cursor_column) = composer_cursor_position(before, inner_width);
    let x = area
        .x
        .saturating_add(1)
        .saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX));
    let y = area
        .y
        .saturating_add(1)
        .saturating_add(u16::try_from(cursor_row).unwrap_or(u16::MAX));
    if x < area.right().saturating_sub(1) && y < area.bottom().saturating_sub(1) {
        frame.set_cursor_position((x, y));
    }
}

pub(super) fn render_footer(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let width = usize::from(area.width);
    let short_session = state.session_id.chars().take(8).collect::<String>();
    let mut segments = vec![format!(" Colossus {short_session}")];
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
    let mut footer = segments.join(" · ");
    if UnicodeWidthStr::width(footer.as_str()) > width {
        footer = truncate_width(&footer, width);
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    frame.render_widget(
        Paragraph::new(Span::styled(footer, ratatui_style(palette.meta_style()))),
        area,
    );
}

pub(super) fn render_overlay(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
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
