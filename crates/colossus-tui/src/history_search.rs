use super::*;

pub(super) struct HistorySearchState {
    pub(super) query: String,
    pub(super) selected: Option<usize>,
    pub(super) preview_scroll: usize,
    preview_max_scroll: usize,
    matches: Vec<usize>,
    normalized_history: Option<Vec<String>>,
}

impl HistorySearchState {
    pub(super) fn new(history: &[String]) -> Self {
        Self {
            query: String::new(),
            selected: history.len().checked_sub(1),
            preview_scroll: 0,
            preview_max_scroll: 0,
            matches: (0..history.len()).rev().collect(),
            normalized_history: None,
        }
    }

    pub(super) fn filtered_indices(&self) -> &[usize] {
        &self.matches
    }

    pub(super) fn reconcile_selection(&mut self, history: &[String]) {
        self.matches.clear();
        if self.query.is_empty() {
            self.matches.extend((0..history.len()).rev());
        } else {
            // History stays fixed while this picker is open. Normalize lazily once,
            // and filter only when input changes rather than on every redraw.
            let entries = self
                .normalized_history
                .get_or_insert_with(|| history.iter().map(|entry| entry.to_lowercase()).collect());
            let query = self.query.to_lowercase();
            self.matches.extend(
                entries
                    .iter()
                    .enumerate()
                    .rev()
                    .filter_map(|(index, entry)| entry.contains(&query).then_some(index)),
            );
        }
        if !self
            .selected
            .is_some_and(|selected| self.matches.contains(&selected))
        {
            self.selected = self.matches.first().copied();
        }
        self.reset_preview();
    }

    pub(super) fn move_selection(&mut self, offset: isize) {
        if self.matches.is_empty() {
            self.selected = None;
            self.reset_preview();
            return;
        }
        let current = self
            .selected
            .and_then(|selected| self.matches.iter().position(|index| *index == selected))
            .unwrap_or(0);
        let next = if offset < 0 {
            current
                .checked_sub(offset.unsigned_abs())
                .unwrap_or(self.matches.len() - 1)
        } else {
            (current + offset as usize) % self.matches.len()
        };
        self.selected = Some(self.matches[next]);
        self.reset_preview();
    }

    pub(super) fn select_boundary(&mut self, last: bool) {
        self.selected = if last {
            self.matches.last().copied()
        } else {
            self.matches.first().copied()
        };
        self.reset_preview();
    }

    pub(super) fn scroll_preview(&mut self, down: bool) {
        self.preview_scroll = if down {
            self.preview_scroll
                .saturating_add(5)
                .min(self.preview_max_scroll)
        } else {
            self.preview_scroll.saturating_sub(5)
        };
    }

    fn reset_preview(&mut self) {
        self.preview_scroll = 0;
        self.preview_max_scroll = 0;
    }
}

pub(super) fn render_history_search(
    frame: &mut Frame<'_>,
    history: &[String],
    preferences: &TerminalPreferences,
    search: &mut HistorySearchState,
    area: Rect,
) {
    let palette = TerminalPalette::for_preferences(preferences);
    let indices = search.filtered_indices();
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
        let prompt = history[*index]
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
        if let Some(prompt) = search.selected.and_then(|index| history.get(index)) {
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
            search.preview_max_scroll = lines.len().saturating_sub(usize::from(rows[1].height));
            search.preview_scroll = search.preview_scroll.min(search.preview_max_scroll);
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((u16::try_from(search.preview_scroll).unwrap_or(u16::MAX), 0)),
                rows[1],
            );
        }
        controls.insert(1, ("PgUp/Dn", "View"));
    } else {
        search.reset_preview();
    }
    render_browser_controls(frame, &palette, areas.controls, &controls);
}
