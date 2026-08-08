use super::*;

pub(super) struct ThemePickerState {
    pub(super) request: InteractiveThemePicker,
    pub(super) original_preferences: TerminalPreferences,
    pub(super) query: String,
    pub(super) search_active: bool,
    pub(super) selected: Option<usize>,
}

impl ThemePickerState {
    pub(super) fn new(
        request: InteractiveThemePicker,
        original_preferences: TerminalPreferences,
    ) -> Self {
        let selected = request
            .themes
            .iter()
            .position(|theme| theme.name == request.current_theme)
            .or_else(|| (!request.themes.is_empty()).then_some(0));
        Self {
            request,
            original_preferences,
            query: String::new(),
            search_active: false,
            selected,
        }
    }

    pub(super) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        self.request
            .themes
            .iter()
            .enumerate()
            .filter_map(|(index, theme)| {
                (query.is_empty() || theme.name.to_lowercase().contains(&query)).then_some(index)
            })
            .collect()
    }

    pub(super) fn selected_entry(&self) -> Option<&InteractiveThemePickerEntry> {
        self.selected
            .and_then(|index| self.request.themes.get(index))
    }

    pub(super) fn move_selection(&mut self, offset: isize) {
        let choices = self.filtered_indices();
        if choices.is_empty() {
            self.selected = None;
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
    }

    pub(super) fn select_boundary(&mut self, last: bool) {
        let choices = self.filtered_indices();
        self.selected = if last {
            choices.last().copied()
        } else {
            choices.first().copied()
        };
    }

    pub(super) fn reconcile_selection(&mut self) {
        let choices = self.filtered_indices();
        if !self
            .selected
            .is_some_and(|selected| choices.contains(&selected))
        {
            self.selected = choices.first().copied();
        }
    }
}

pub(super) fn render_theme_picker(
    frame: &mut Frame<'_>,
    state: &TuiState,
    picker: &ThemePickerState,
    area: Rect,
) {
    let canvas = theme_picker_canvas_rect(state, area);
    frame.render_widget(Clear, canvas);
    let area = theme_picker_rect(state, area);
    if area.width < 72 || area.height < 16 {
        render_compact_theme_picker(frame, picker, area);
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
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(inner);
    render_theme_header(frame, picker, &palette, rows[0]);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(rows[1]);
    render_theme_list(frame, picker, &palette, panes[0]);
    render_theme_preview(frame, picker, panes[1]);
    render_theme_controls(frame, picker, &palette, rows[2]);
}

fn theme_picker_rect(state: &TuiState, area: Rect) -> Rect {
    let canvas = theme_picker_canvas_rect(state, area);
    let horizontal_margin = if area.width >= 120 {
        4
    } else if area.width >= 80 {
        2
    } else {
        0
    };
    let top_margin = u16::from(canvas.height >= 12);
    let bottom_margin = if canvas.height >= 30 {
        3
    } else if canvas.height >= 18 {
        2
    } else {
        0
    };
    Rect::new(
        canvas.x.saturating_add(horizontal_margin),
        canvas.y.saturating_add(top_margin),
        canvas
            .width
            .saturating_sub(horizontal_margin.saturating_mul(2)),
        canvas
            .height
            .saturating_sub(top_margin)
            .saturating_sub(bottom_margin),
    )
}

fn theme_picker_canvas_rect(state: &TuiState, area: Rect) -> Rect {
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

fn render_theme_header(
    frame: &mut Frame<'_>,
    picker: &ThemePickerState,
    palette: &TerminalPalette,
    area: Rect,
) {
    frame.render_widget(Block::default().borders(Borders::BOTTOM), area);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Percentage(38),
            Constraint::Length(16),
            Constraint::Min(0),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Choose theme",
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        )),
        inset(columns[0]),
    );
    let search = if picker.search_active {
        format!("/ {}_", picker.query)
    } else if picker.query.is_empty() {
        "/ Search themes".into()
    } else {
        format!("/ {}", picker.query)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            search,
            ratatui_style(if picker.search_active {
                palette.user_style()
            } else {
                palette.meta_style()
            }),
        ))
        .block(Block::default().borders(Borders::ALL)),
        columns[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {} themes", picker.filtered_indices().len()),
            ratatui_style(palette.user_style()),
        )),
        inset(columns[2]),
    );
}

fn render_theme_list(
    frame: &mut Frame<'_>,
    picker: &ThemePickerState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "  Theme",
            ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD),
        ))
        .block(Block::default().borders(Borders::BOTTOM)),
        rows[0],
    );
    let indices = picker.filtered_indices();
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No matching themes",
                ratatui_style(palette.meta_style()),
            )),
            rows[1],
        );
        return;
    }
    let visible = usize::from(rows[1].height).max(1);
    let focus = picker
        .selected
        .and_then(|selected| indices.iter().position(|index| *index == selected))
        .unwrap_or(0);
    let start = focus
        .saturating_sub(visible / 2)
        .min(indices.len().saturating_sub(visible));
    for (row, index) in indices.iter().skip(start).take(visible).enumerate() {
        let entry = &picker.request.themes[*index];
        let selected = picker.selected == Some(*index);
        let current = entry.name == picker.request.current_theme;
        let style = if selected {
            selected_theme_style(palette)
        } else if current {
            ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD)
        } else {
            ratatui_style(palette.meta_style())
        };
        let marker = if selected { "›" } else { " " };
        let current = if current { "  CURRENT" } else { "" };
        let content = truncate_width(
            &format!(" {marker} {}{current}", entry.name),
            usize::from(rows[1].width),
        );
        frame.render_widget(
            Paragraph::new(Span::styled(content, style)).style(style),
            Rect::new(
                rows[1].x,
                rows[1]
                    .y
                    .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                rows[1].width,
                1,
            ),
        );
    }
}

fn render_theme_preview(frame: &mut Frame<'_>, picker: &ThemePickerState, area: Rect) {
    let Some(entry) = picker.selected_entry() else {
        frame.render_widget(
            Paragraph::new("Choose a theme to preview it"),
            inset_horizontally(area, 3),
        );
        return;
    };
    let palette = TerminalPalette::for_preferences(&entry.preferences);
    let inner = Rect::new(
        area.x.saturating_add(3),
        area.y.saturating_add(1),
        area.width.saturating_sub(6),
        area.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("{} preview", entry.name),
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    let kind = if colossus_contracts::ThemeName::parse(&entry.name).is_some() {
        "Built-in"
    } else {
        "Custom"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("{kind} · preview only until Enter"),
            ratatui_style(palette.meta_style()),
        ))
        .block(Block::default().borders(Borders::BOTTOM)),
        rows[1],
    );
    let sample = vec![
        Line::from(vec![
            Span::styled("● COLOSSUS   ", ratatui_style(palette.assistant_style())),
            Span::raw("I can inspect the workspace and explain the change."),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled("◆ TOOL       ", ratatui_style(palette.tool_style())),
            Span::raw("filesystem.search · completed"),
        ]),
        Line::from(Span::styled(
            "✓ Success     The focused checks passed.",
            ratatui_style(palette.tone_style(PresentationTone::Success)),
        )),
        Line::from(Span::styled(
            "⚠ Warning     Approval is required before this effect.",
            ratatui_style(palette.warning_style()),
        )),
        Line::from(Span::styled(
            "! Error       Provider response was unavailable.",
            ratatui_style(palette.error_style()),
        )),
        Line::from(Span::styled(
            "Metadata      ctx=12k/128k · status=ready",
            ratatui_style(palette.meta_style()),
        )),
    ];
    frame.render_widget(Paragraph::new(sample).wrap(Wrap { trim: false }), rows[2]);
    frame.render_widget(
        Paragraph::new("Draft remains unchanged while you preview").block(
            Block::default().borders(Borders::ALL).title(Span::styled(
                " Message · Enter sends ",
                ratatui_style(palette.meta_style()),
            )),
        ),
        rows[3],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Colossus · mode=execute · status=ready",
            ratatui_style(palette.meta_style()),
        )),
        rows[4],
    );
}

fn render_theme_controls(
    frame: &mut Frame<'_>,
    picker: &ThemePickerState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let inner = Block::default().borders(Borders::TOP).inner(area);
    frame.render_widget(Block::default().borders(Borders::TOP), area);
    let key = ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD);
    let label = ratatui_style(palette.meta_style());
    let spans = if picker.selected.is_some() {
        vec![
            Span::styled(" ↑/↓", key),
            Span::styled(" Select   ", label),
            Span::styled("/", key),
            Span::styled(" Search   ", label),
            Span::styled("Enter", key),
            Span::styled(" Apply   ", label),
            Span::styled("Esc", key),
            Span::styled(" Cancel and restore", label),
        ]
    } else {
        vec![Span::styled(
            " No matching theme · edit search or Esc cancel",
            key,
        )]
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_compact_theme_picker(frame: &mut Frame<'_>, picker: &ThemePickerState, area: Rect) {
    let preferences = picker
        .selected_entry()
        .map(|entry| &entry.preferences)
        .unwrap_or(&picker.original_preferences);
    let palette = TerminalPalette::for_preferences(preferences);
    let title = picker.selected_entry().map_or_else(
        || " Choose theme ".into(),
        |entry| format!(" Choose theme · {} ", entry.name),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            ratatui_style(palette.assistant_style()),
        ))
        .title_bottom(Span::styled(
            " ↑/↓ select · Enter apply · Esc restore ",
            ratatui_style(palette.warning_style()),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(entry) = picker.selected_entry() else {
        frame.render_widget(Paragraph::new("No matching themes"), inner);
        return;
    };
    let lines = vec![
        Line::from(Span::styled(
            format!("› {}", entry.name),
            ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            "● Assistant preview",
            ratatui_style(palette.assistant_style()),
        )),
        Line::from(Span::styled(
            "◆ Tool preview",
            ratatui_style(palette.tool_style()),
        )),
        Line::from(Span::styled(
            "⚠ Warning preview",
            ratatui_style(palette.warning_style()),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn selected_theme_style(palette: &TerminalPalette) -> Style {
    let accent = ratatui_style(palette.user_style());
    Style::default()
        .fg(Color::Black)
        .bg(match accent.fg.unwrap_or(Color::LightGreen) {
            Color::Rgb(red, green, blue) => Color::Rgb(
                soften_channel(red),
                soften_channel(green),
                soften_channel(blue),
            ),
            Color::Green | Color::LightGreen => Color::Rgb(191, 255, 207),
            Color::Cyan | Color::LightCyan => Color::Rgb(207, 246, 255),
            _ => Color::Gray,
        })
        .add_modifier(Modifier::BOLD)
}

const fn soften_channel(value: u8) -> u8 {
    ((value as u16 * 28 + 255 * 72) / 100) as u8
}

fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    )
}

fn inset_horizontally(area: Rect, amount: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(amount),
        area.y,
        area.width.saturating_sub(amount.saturating_mul(2)),
        area.height,
    )
}
