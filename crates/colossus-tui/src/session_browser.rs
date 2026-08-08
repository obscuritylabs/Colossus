use super::*;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SESSION_LIST_ENTRY_HEIGHT: u16 = 2;

pub(super) struct SessionBrowserState {
    pub(super) request: InteractiveSessionBrowser,
    pub(super) query: String,
    pub(super) search_active: bool,
    pub(super) selected: Option<usize>,
    pub(super) preview_scroll: usize,
}

impl SessionBrowserState {
    pub(super) fn new(request: InteractiveSessionBrowser) -> Self {
        let selected = request
            .sessions
            .iter()
            .position(|entry| entry.summary.id != request.current_session_id);
        Self {
            request,
            query: String::new(),
            search_active: false,
            selected,
            preview_scroll: 0,
        }
    }

    pub(super) fn filtered_indices(&self) -> Vec<usize> {
        self.request
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| self.matches(entry).then_some(index))
            .collect()
    }

    pub(super) fn selectable_indices(&self) -> Vec<usize> {
        self.filtered_indices()
            .into_iter()
            .filter(|index| {
                self.request.sessions[*index].summary.id != self.request.current_session_id
            })
            .collect()
    }

    pub(super) fn selected_entry(&self) -> Option<&InteractiveSessionBrowserEntry> {
        self.selected
            .and_then(|index| self.request.sessions.get(index))
    }

    pub(super) fn move_selection(&mut self, offset: isize) {
        let choices = self.selectable_indices();
        if choices.is_empty() {
            self.selected = None;
            self.preview_scroll = 0;
            return;
        }
        let current = self
            .selected
            .and_then(|selected| choices.iter().position(|index| *index == selected))
            .unwrap_or(0);
        let next = if offset < 0 {
            current
                .checked_sub(offset.unsigned_abs())
                .unwrap_or(choices.len() - 1)
        } else {
            (current + offset as usize) % choices.len()
        };
        self.selected = Some(choices[next]);
        self.preview_scroll = 0;
    }

    pub(super) fn select_boundary(&mut self, last: bool) {
        let choices = self.selectable_indices();
        self.selected = if last {
            choices.last().copied()
        } else {
            choices.first().copied()
        };
        self.preview_scroll = 0;
    }

    pub(super) fn reconcile_selection(&mut self) {
        let choices = self.selectable_indices();
        if !self
            .selected
            .is_some_and(|selected| choices.contains(&selected))
        {
            self.selected = choices.first().copied();
            self.preview_scroll = 0;
        }
    }

    fn matches(&self, entry: &InteractiveSessionBrowserEntry) -> bool {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let summary = &entry.summary;
        session_browser_title(summary)
            .to_lowercase()
            .contains(&query)
            || summary.id.to_lowercase().contains(&query)
            || summary
                .last_user_preview
                .as_deref()
                .is_some_and(|preview| preview.to_lowercase().contains(&query))
            || entry
                .recent_messages
                .iter()
                .any(|message| message.content.to_lowercase().contains(&query))
    }
}

pub(super) fn session_browser_title(summary: &SessionSummary) -> String {
    let title = clean_session_browser_text(summary.title.as_deref().unwrap_or_default());
    if !title.is_empty() && !title.eq_ignore_ascii_case("untitled") {
        return title;
    }
    summary
        .last_user_preview
        .as_deref()
        .map(clean_session_browser_text)
        .filter(|preview| !preview.is_empty())
        .unwrap_or_else(|| "Untitled session".into())
}

pub(super) fn render_session_browser(
    frame: &mut Frame<'_>,
    state: &TuiState,
    browser: &SessionBrowserState,
    area: Rect,
) {
    let canvas = session_browser_canvas_rect(state, area);
    frame.render_widget(Clear, canvas);
    let area = session_browser_rect(state, area);
    if area.width < 72 || area.height < 14 {
        render_compact_session_browser(frame, state, browser, area);
        return;
    }

    let palette = TerminalPalette::for_preferences(&state.preferences);
    let outer = Block::default().borders(Borders::ALL);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);
    render_session_browser_header(frame, browser, &palette, rows[0]);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(rows[1]);
    render_session_list(frame, browser, &palette, panes[0]);
    render_session_preview(frame, browser, &palette, panes[1]);
    render_session_browser_controls(frame, browser, &palette, rows[2]);
}

fn session_browser_rect(state: &TuiState, area: Rect) -> Rect {
    let canvas = session_browser_canvas_rect(state, area);
    let available_height = canvas.height;
    let horizontal_margin = if area.width >= 120 {
        4
    } else if area.width >= 80 {
        2
    } else if area.width >= 48 {
        1
    } else {
        0
    };
    let top_margin = u16::from(available_height >= 12);
    let bottom_margin = if available_height >= 32 {
        4
    } else if available_height >= 18 {
        2
    } else {
        u16::from(available_height >= 9)
    };
    Rect::new(
        canvas.x.saturating_add(horizontal_margin),
        canvas.y.saturating_add(top_margin),
        canvas
            .width
            .saturating_sub(horizontal_margin.saturating_mul(2)),
        available_height
            .saturating_sub(top_margin)
            .saturating_sub(bottom_margin),
    )
}

fn session_browser_canvas_rect(state: &TuiState, area: Rect) -> Rect {
    let reserved = composer_height(state, area.width)
        .saturating_add(u16::from(state.operation.is_some()))
        .saturating_add(1);
    Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.saturating_sub(reserved),
    )
}

fn render_compact_session_browser(
    frame: &mut Frame<'_>,
    state: &TuiState,
    browser: &SessionBrowserState,
    area: Rect,
) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let count = browser.filtered_indices().len();
    let title = if browser.search_active || !browser.query.is_empty() {
        format!(" Resume session · /{} · {count} ", browser.query)
    } else {
        format!(" Resume session · {count} ")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            ratatui_style(palette.assistant_style()),
        ))
        .title_bottom(Span::styled(
            " ↑/↓ select · / search · Enter resume · Esc cancel ",
            ratatui_style(palette.warning_style()),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let indices = browser.filtered_indices();
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No matching sessions",
                ratatui_style(palette.meta_style()),
            )),
            inner,
        );
        return;
    }
    let focus = browser
        .selected
        .and_then(|selected| indices.iter().position(|index| *index == selected))
        .unwrap_or(0);
    let visible = usize::from(inner.height).max(1);
    let start = focus
        .saturating_sub(visible / 2)
        .min(indices.len().saturating_sub(visible));
    let lines = indices
        .iter()
        .skip(start)
        .take(visible)
        .map(|index| {
            let entry = &browser.request.sessions[*index];
            let current = entry.summary.id == browser.request.current_session_id;
            let selected = browser.selected == Some(*index);
            let marker = if selected { "›" } else { " " };
            let prefix = if current { "CURRENT " } else { "" };
            let content = format!("{marker} {prefix}{}", session_browser_title(&entry.summary));
            let style = if selected {
                selected_session_style(&palette)
            } else if current {
                ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(
                truncate_width_with_ellipsis(&content, usize::from(inner.width)),
                style,
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_session_browser_header(
    frame: &mut Frame<'_>,
    browser: &SessionBrowserState,
    palette: &TerminalPalette,
    area: Rect,
) {
    frame.render_widget(Block::default().borders(Borders::BOTTOM), area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Percentage(38),
            Constraint::Length(18),
            Constraint::Min(0),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Resume session",
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Left),
        inset_vertically(columns[0]),
    );
    let search_text = if browser.search_active {
        format!("/ {}_", browser.query)
    } else if browser.query.is_empty() {
        "/ Search sessions".into()
    } else {
        format!("/ {}", browser.query)
    };
    let search_style = if browser.search_active {
        palette.user_style()
    } else {
        palette.meta_style()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(search_text, ratatui_style(search_style)))
            .block(Block::default().borders(Borders::ALL)),
        columns[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {} sessions", browser.filtered_indices().len()),
            ratatui_style(palette.user_style()),
        )),
        inset_vertically(columns[2]),
    );
}

fn render_session_list(
    frame: &mut Frame<'_>,
    browser: &SessionBrowserState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 || inner.width == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);
    render_session_list_header(frame, palette, rows[0]);
    let indices = browser.filtered_indices();
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No matching sessions",
                ratatui_style(palette.meta_style()),
            )),
            rows[1],
        );
        return;
    }
    let visible = usize::from(rows[1].height / SESSION_LIST_ENTRY_HEIGHT).max(1);
    let focus = browser
        .selected
        .and_then(|selected| indices.iter().position(|index| *index == selected))
        .unwrap_or(0);
    let start = focus
        .saturating_sub(visible / 2)
        .min(indices.len().saturating_sub(visible));
    for (row, index) in indices.iter().skip(start).take(visible).enumerate() {
        let row_offset = u16::try_from(row)
            .unwrap_or(u16::MAX)
            .saturating_mul(SESSION_LIST_ENTRY_HEIGHT);
        let row_area = Rect::new(
            rows[1].x,
            rows[1].y.saturating_add(row_offset),
            rows[1].width,
            SESSION_LIST_ENTRY_HEIGHT.min(rows[1].height.saturating_sub(row_offset)),
        );
        render_session_list_entry(
            frame,
            &browser.request.sessions[*index],
            browser.request.sessions[*index].summary.id == browser.request.current_session_id,
            browser.selected == Some(*index),
            palette,
            row_area,
        );
    }
}

fn render_session_list_header(frame: &mut Frame<'_>, palette: &TerminalPalette, area: Rect) {
    let block = Block::default().borders(Borders::BOTTOM);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let widths = list_column_widths(inner.width);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(widths.0),
            Constraint::Length(widths.1),
        ])
        .split(inner);
    let style = ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD);
    frame.render_widget(Paragraph::new(Span::styled("  Session", style)), columns[0]);
    frame.render_widget(Paragraph::new(Span::styled("Updated", style)), columns[1]);
    frame.render_widget(Paragraph::new(Span::styled("Msgs", style)), columns[2]);
}

fn render_session_list_entry(
    frame: &mut Frame<'_>,
    entry: &InteractiveSessionBrowserEntry,
    current: bool,
    selected: bool,
    palette: &TerminalPalette,
    area: Rect,
) {
    if area.height == 0 {
        return;
    }
    let style = if selected {
        selected_session_style(palette)
    } else {
        Style::default()
    };
    frame.render_widget(Block::default().style(style), area);
    let widths = list_column_widths(area.width);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(widths.0),
            Constraint::Length(widths.1),
        ])
        .split(Rect::new(area.x, area.y, area.width, 1));
    let marker = if selected { "›" } else { " " };
    let title = session_browser_title(&entry.summary);
    let prefix = if current { "CURRENT " } else { "" };
    let title_style = if selected {
        style
    } else if current {
        ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width_with_ellipsis(
                &format!(" {marker} {prefix}{title}"),
                usize::from(columns[0].width),
            ),
            title_style,
        ))
        .style(style),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            relative_timestamp(&entry.summary.updated_at),
            if selected {
                style
            } else {
                ratatui_style(palette.meta_style())
            },
        ))
        .style(style),
        columns[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            entry.summary.message_count.to_string(),
            title_style,
        ))
        .alignment(Alignment::Right)
        .style(style),
        columns[2],
    );
    if area.height > 1 {
        let id = entry.summary.id.chars().take(8).collect::<String>();
        let detail = if current {
            format!("    {id}  (active session)")
        } else {
            format!("    {id}")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_width(&detail, usize::from(area.width)),
                if selected {
                    style
                } else {
                    ratatui_style(palette.meta_style())
                },
            ))
            .style(style),
            Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
    }
}

fn render_session_preview(
    frame: &mut Frame<'_>,
    browser: &SessionBrowserState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let padding = if area.width >= 48 { 3 } else { 1 };
    let inner = Rect::new(
        area.x.saturating_add(padding),
        area.y.saturating_add(1),
        area.width.saturating_sub(padding.saturating_mul(2)),
        area.height.saturating_sub(2),
    );
    let Some(entry) = browser.selected_entry() else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "Choose another session to preview it",
                ratatui_style(palette.meta_style()),
            )),
            inner,
        );
        return;
    };
    if inner.height < 5 || inner.width == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width_with_ellipsis(
                &session_browser_title(&entry.summary),
                usize::from(rows[0].width),
            ),
            ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    let short_id = entry.summary.id.chars().take(8).collect::<String>();
    let metadata = format!(
        "ID: {short_id}   Last updated: {} · {}",
        relative_timestamp(&entry.summary.updated_at),
        compact_timestamp(&entry.summary.updated_at),
    );
    let metadata_block = Block::default().borders(Borders::BOTTOM);
    let metadata_inner = metadata_block.inner(rows[1]);
    frame.render_widget(metadata_block, rows[1]);
    let metadata_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(13)])
        .split(metadata_inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width_with_ellipsis(&metadata, usize::from(metadata_columns[0].width)),
            ratatui_style(palette.meta_style()),
        )),
        metadata_columns[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "{} message{}",
                entry.summary.message_count,
                if entry.summary.message_count == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            ratatui_style(palette.user_style()),
        ))
        .alignment(Alignment::Right),
        metadata_columns[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            "Recent conversation",
            ratatui_style(palette.user_style()),
        )),
        rows[2],
    );
    let lines = preview_lines(entry, palette, usize::from(rows[3].width));
    let visible = usize::from(rows[3].height);
    let maximum_scroll = lines.len().saturating_sub(visible);
    let scroll = browser.preview_scroll.min(maximum_scroll);
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        rows[3],
    );
    if scroll > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("⌃", ratatui_style(palette.meta_style()))),
            Rect::new(rows[3].right().saturating_sub(1), rows[3].y, 1, 1),
        );
    }
    if scroll < maximum_scroll && rows[3].height > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("⌄", ratatui_style(palette.meta_style()))),
            Rect::new(
                rows[3].right().saturating_sub(1),
                rows[3].bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn preview_lines(
    entry: &InteractiveSessionBrowserEntry,
    palette: &TerminalPalette,
    width: usize,
) -> Vec<Line<'static>> {
    if entry.recent_messages.is_empty() {
        return vec![Line::from(Span::styled(
            "No recent conversation preview is available.",
            ratatui_style(palette.meta_style()),
        ))];
    }
    let label_width = 11.min(width.saturating_sub(1));
    let content_width = width.saturating_sub(label_width).max(1);
    let mut lines = Vec::new();
    for message in &entry.recent_messages {
        let (label, style) = match message.role {
            ModelMessageRole::User => ("USER", palette.user_style()),
            ModelMessageRole::Assistant => ("ASSISTANT", palette.assistant_style()),
            ModelMessageRole::System => ("SYSTEM", palette.meta_style()),
            ModelMessageRole::Tool => ("TOOL", palette.tool_style()),
        };
        let content = clean_session_browser_text(&message.content);
        for (index, content) in wrap_approval_value(&content, content_width)
            .into_iter()
            .enumerate()
        {
            let gutter = if index == 0 {
                format!("{label:<label_width$}")
            } else {
                " ".repeat(label_width)
            };
            lines.push(Line::from(vec![
                Span::styled(gutter, ratatui_style(style).add_modifier(Modifier::BOLD)),
                Span::raw(content),
            ]));
        }
        lines.push(Line::default());
    }
    lines
}

fn render_session_browser_controls(
    frame: &mut Frame<'_>,
    browser: &SessionBrowserState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let inner = Block::default().borders(Borders::TOP).inner(area);
    frame.render_widget(Block::default().borders(Borders::TOP), area);
    let key_style = ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD);
    let label_style = ratatui_style(palette.meta_style());
    let mut spans = vec![
        Span::styled(" ↑/↓", key_style),
        Span::styled(" Select   ", label_style),
        Span::styled("/", key_style),
        Span::styled(" Search   ", label_style),
        Span::styled("PgUp/PgDn", key_style),
        Span::styled(" Preview   ", label_style),
        Span::styled("Enter", key_style),
        Span::styled(" Resume   ", label_style),
        Span::styled("Esc", key_style),
        Span::styled(" Cancel", label_style),
    ];
    if browser.selected.is_none() {
        spans = vec![Span::styled(
            " No other matching session can be resumed · / Search · Esc Cancel",
            key_style,
        )];
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn selected_session_style(palette: &TerminalPalette) -> Style {
    let accent = ratatui_style(palette.user_style());
    Style::default()
        .fg(Color::Black)
        .bg(softened_selection_color(
            accent.fg.unwrap_or(Color::LightGreen),
        ))
        .add_modifier(Modifier::BOLD)
}

fn softened_selection_color(color: Color) -> Color {
    match color {
        Color::Rgb(red, green, blue) => Color::Rgb(
            soften_channel(red),
            soften_channel(green),
            soften_channel(blue),
        ),
        Color::Green | Color::LightGreen => Color::Rgb(191, 255, 207),
        Color::Cyan | Color::LightCyan => Color::Rgb(207, 246, 255),
        _ => Color::Gray,
    }
}

const fn soften_channel(value: u8) -> u8 {
    ((value as u16 * 28 + 255 * 72) / 100) as u8
}

fn list_column_widths(width: u16) -> (u16, u16) {
    if width >= 48 { (13, 5) } else { (9, 4) }
}

fn inset_vertically(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    )
}

fn compact_timestamp(value: &str) -> String {
    value
        .get(..16)
        .map(|timestamp| format!("{}Z", timestamp.replace('T', " ")))
        .unwrap_or_else(|| value.to_owned())
}

fn clean_session_browser_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '\u{200b}'
                        | '\u{200c}'
                        | '\u{200d}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'..='\u{206f}'
                        | '\u{feff}'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn relative_timestamp(value: &str) -> String {
    let Ok(timestamp) = OffsetDateTime::parse(value, &Rfc3339) else {
        return compact_timestamp(value);
    };
    let seconds = (OffsetDateTime::now_utc() - timestamp).whole_seconds();
    match seconds {
        ..0 => compact_timestamp(value),
        0..=59 => "just now".into(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=86_399 => format!("{} hr ago", seconds / 3_600),
        86_400..=604_799 => format!("{} days ago", seconds / 86_400),
        _ => compact_timestamp(value),
    }
}
