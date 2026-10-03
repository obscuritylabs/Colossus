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
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let areas = render_browser_shell(
        frame,
        &palette,
        area,
        34,
        BrowserHeader {
            title: "Choose theme",
            count: format!("{} themes", picker.filtered_indices().len()),
            search_hint: "Search themes",
            query: &picker.query,
            search_active: picker.search_active,
        },
    );
    render_theme_list(frame, picker, &palette, areas.list);
    if let Some(preview) = areas.preview {
        render_theme_preview(frame, picker, preview);
    }
    render_browser_controls(
        frame,
        &palette,
        areas.controls,
        &[
            ("↑/↓", "Select"),
            ("/", "Search"),
            ("Enter", "Apply"),
            ("Esc", "Cancel and restore"),
        ],
    );
}

fn render_theme_list(
    frame: &mut Frame<'_>,
    picker: &ThemePickerState,
    palette: &TerminalPalette,
    area: Rect,
) {
    let inner = area;
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
        )),
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
            browser_selection_style(palette)
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
        frame.render_widget(Paragraph::new("Choose a theme to preview it"), area);
        return;
    };
    let palette = TerminalPalette::for_preferences(&entry.preferences);
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
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
        )),
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
