use super::*;

pub(super) struct HistorySearchState {
    pub(super) query: String,
    pub(super) selected: Option<usize>,
    pub(super) preview_scroll: usize,
}

impl HistorySearchState {
    pub(super) fn new(history: &[String]) -> Self {
        Self {
            query: String::new(),
            selected: history.len().checked_sub(1),
            preview_scroll: 0,
        }
    }

    pub(super) fn filtered_indices(&self, history: &[String]) -> Vec<usize> {
        let query = self.query.to_lowercase();
        history
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, entry)| entry.to_lowercase().contains(&query).then_some(index))
            .collect()
    }

    pub(super) fn reconcile_selection(&mut self, history: &[String]) {
        let choices = self.filtered_indices(history);
        if !self
            .selected
            .is_some_and(|selected| choices.contains(&selected))
        {
            self.selected = choices.first().copied();
        }
        self.preview_scroll = 0;
    }

    pub(super) fn move_selection(&mut self, history: &[String], offset: isize) {
        let choices = self.filtered_indices(history);
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
        self.preview_scroll = 0;
    }

    pub(super) fn select_boundary(&mut self, history: &[String], last: bool) {
        let choices = self.filtered_indices(history);
        self.selected = if last {
            choices.last().copied()
        } else {
            choices.first().copied()
        };
        self.preview_scroll = 0;
    }
}

pub(super) fn render_history_search(
    frame: &mut Frame<'_>,
    state: &TuiState,
    search: &HistorySearchState,
    area: Rect,
) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let indices = search.filtered_indices(&state.history);
    let areas = render_browser_shell(
        frame,
        &palette,
        area,
        44,
        BrowserHeader {
            title: "Prompt history",
            count: format!("{} prompts", indices.len()),
            search_hint: "Search prompts",
            query: &search.query,
            search_active: true,
        },
    );
    let visible = usize::from(areas.list.height);
    let focus = search
        .selected
        .and_then(|selected| indices.iter().position(|index| *index == selected))
        .unwrap_or(0);
    let start = focus
        .saturating_sub(visible / 2)
        .min(indices.len().saturating_sub(visible));
    if indices.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "No matching prompts",
                ratatui_style(palette.meta_style()),
            )),
            areas.list,
        );
    }
    for (row, index) in indices.iter().skip(start).take(visible).enumerate() {
        let selected = search.selected == Some(*index);
        let style = if selected {
            browser_selection_style(&palette)
        } else {
            Style::default()
        };
        let prompt = state.history[*index]
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        let title = format!(
            "{} {}",
            if selected { "›" } else { " " },
            sanitize_approval_field(prompt)
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_width_with_ellipsis(&title, usize::from(areas.list.width)),
                style,
            ))
            .style(style),
            Rect::new(
                areas.list.x,
                areas
                    .list
                    .y
                    .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                areas.list.width,
                1,
            ),
        );
    }
    let mut controls = vec![
        ("↑/↓", "Select"),
        ("Enter", "Use prompt"),
        ("Esc", "Cancel"),
    ];
    if let Some(preview) = areas.preview {
        if let Some(prompt) = search.selected.and_then(|index| state.history.get(index)) {
            let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(preview);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "Enter places this prompt in your draft",
                    ratatui_style(palette.meta_style()),
                ))
                .wrap(Wrap { trim: false }),
                rows[0],
            );
            let lines = prompt
                .split('\n')
                .flat_map(|line| {
                    wrap_approval_value(&sanitize_approval_field(line), usize::from(rows[1].width))
                })
                .map(Line::from)
                .collect::<Vec<_>>();
            let scroll = search
                .preview_scroll
                .min(lines.len().saturating_sub(usize::from(rows[1].height)));
            frame.render_widget(
                Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
                rows[1],
            );
        }
        controls.insert(1, ("PgUp/Dn", "View"));
    }
    render_browser_controls(frame, &palette, areas.controls, &controls);
}
