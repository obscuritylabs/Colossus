use super::*;

pub(super) fn draw(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let manager = &state.workspace_manager;
    let areas = render_browser_shell(
        frame,
        &palette,
        area,
        44,
        BrowserHeader {
            title: "Sessions",
            count: format!(
                "{} {}",
                manager.filtered().len(),
                manager.tab.label().to_lowercase()
            ),
            search_hint: "Search title, message or ID",
            query: &manager.query,
            search_active: manager.searching,
        },
    );
    state.workspace_manager.preview_area = areas.preview;
    let action_height = if areas.list.height >= 8 {
        2
    } else if areas.list.height >= 6 {
        1
    } else {
        0
    };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(action_height),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(areas.list);
    draw_tabs(frame, state, &palette, rows[0]);
    let selected = state.workspace_manager.selected_entry();
    let mut actions = vec![
        ("[n] New", WorkspaceAction::New),
        ("[r] Refresh", WorkspaceAction::Browse),
    ];
    if let Some(entry) = selected {
        if matches!(
            entry.status,
            WorkspaceSessionStatus::Running | WorkspaceSessionStatus::Waiting
        ) {
            actions.push(("[s] Stop", WorkspaceAction::Stop(entry.summary.id.clone())));
        } else if state.workspace_manager.tab == SessionTab::Archived {
            actions.push((
                "[u] Restore",
                WorkspaceAction::Archive(entry.summary.id.clone(), false),
            ));
        } else {
            actions.push((
                "[a] Archive",
                WorkspaceAction::Archive(entry.summary.id.clone(), true),
            ));
        }
    }
    let mut x = rows[1].x;
    let mut y = rows[1].y;
    for (label, action) in actions {
        let width = label.len() as u16;
        if x + width > rows[1].right() {
            x = rows[1].x;
            y += 1;
        }
        if y >= rows[1].bottom() || width > rows[1].width {
            break;
        }
        let hit = Rect::new(x, y, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(ratatui_style(palette.user_style())),
            hit,
        );
        state.workspace_hits.push((hit, action));
        x += width + 2;
    }
    draw_list(frame, state, &palette, rows[2], rows[3]);
    if let Some(preview) = areas.preview {
        draw_preview(frame, state, &palette, preview);
    }
    render_browser_controls(
        frame,
        &palette,
        areas.controls,
        &[
            ("Tab", "Views"),
            ("/", "Search"),
            ("↑/↓", "Select"),
            (
                if state.workspace_manager.tab == SessionTab::Archived {
                    "u"
                } else {
                    "Enter"
                },
                if state.workspace_manager.tab == SessionTab::Archived {
                    "Restore"
                } else {
                    "Open"
                },
            ),
            ("PgUp/Dn", "Preview"),
            ("Esc", "Back"),
        ],
    );
}

fn draw_tabs(frame: &mut Frame<'_>, state: &mut TuiState, palette: &TerminalPalette, area: Rect) {
    let mut x = area.x;
    for tab in [SessionTab::Open, SessionTab::History, SessionTab::Archived] {
        let label = format!(" {} ", tab.label());
        let width = label.len() as u16;
        if x + width > area.right() {
            break;
        }
        let hit = Rect::new(x, area.y, width, 1);
        let style = if tab == state.workspace_manager.tab {
            browser_selection_style(palette)
        } else {
            ratatui_style(palette.meta_style())
        };
        frame.render_widget(Paragraph::new(label).style(style), hit);
        state.workspace_hits.push((hit, WorkspaceAction::Tab(tab)));
        x += width;
    }
}

fn columns(area: Rect) -> [Rect; 3] {
    let widths = crate::session_browser::list_column_widths(area.width);
    let columns = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(widths.0),
        Constraint::Length(widths.1),
    ])
    .split(area);
    [columns[0], columns[1], columns[2]]
}

fn draw_list(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    palette: &TerminalPalette,
    heading: Rect,
    area: Rect,
) {
    let muted = ratatui_style(palette.meta_style());
    let headers = columns(heading);
    for (label, column) in ["Session", "Updated", "Msgs"]
        .into_iter()
        .zip(headers.iter())
    {
        frame.render_widget(
            Paragraph::new(label).style(muted.add_modifier(Modifier::BOLD)),
            *column,
        );
    }
    let manager = &state.workspace_manager;
    let choices = manager.filtered();
    if choices.is_empty() {
        frame.render_widget(Paragraph::new("No matching sessions").style(muted), area);
        return;
    }
    let visible = usize::from(area.height);
    let current = choices
        .iter()
        .position(|index| {
            manager.selected.as_deref() == Some(manager.entries[*index].summary.id.as_str())
        })
        .unwrap_or(0);
    let start = current
        .saturating_sub(visible / 2)
        .min(choices.len().saturating_sub(visible));
    for (offset, index) in choices.into_iter().skip(start).take(visible).enumerate() {
        let entry = &manager.entries[index];
        let selected = manager.selected.as_deref() == Some(entry.summary.id.as_str());
        let style = if selected {
            browser_selection_style(palette)
        } else {
            ratatui_style(palette.tone_style(entry.status.display().2))
        };
        let hit = Rect::new(area.x, area.y + offset as u16, area.width, 1);
        frame.render_widget(Block::default().style(style), hit);
        let cells = columns(hit);
        let title = format!(
            "{}{} {}",
            if selected { "› " } else { "  " },
            entry.status.display().0,
            session_browser_title(&entry.summary)
        );
        frame.render_widget(
            Paragraph::new(truncate_width_with_ellipsis(
                &title,
                usize::from(cells[0].width.saturating_sub(1)),
            ))
            .style(style),
            cells[0],
        );
        frame.render_widget(
            Paragraph::new(truncate_width_with_ellipsis(
                &crate::session_browser::relative_timestamp(&entry.summary.updated_at),
                usize::from(cells[1].width.saturating_sub(1)),
            ))
            .style(style),
            cells[1],
        );
        frame.render_widget(
            Paragraph::new(entry.summary.message_count.to_string())
                .alignment(Alignment::Right)
                .style(style),
            cells[2],
        );
        state
            .workspace_hits
            .push((hit, WorkspaceAction::Select(entry.summary.id.clone())));
    }
}

fn draw_preview(frame: &mut Frame<'_>, state: &TuiState, palette: &TerminalPalette, area: Rect) {
    let manager = &state.workspace_manager;
    let muted = ratatui_style(palette.meta_style());
    let Some(entry) = manager.selected_entry() else {
        frame.render_widget(
            Paragraph::new("Select a session to preview").style(muted),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::styled(
            session_browser_title(&entry.summary),
            ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            format!(
                "{}{} · {} messages",
                if entry.open { "Open · " } else { "" },
                entry.status.display().1,
                entry.summary.message_count
            ),
            muted,
        ),
        Line::styled(
            format!(
                "ID: {}",
                entry.summary.id.chars().take(8).collect::<String>()
            ),
            muted,
        ),
        Line::styled(
            format!(
                "Updated: {}",
                crate::session_browser::compact_timestamp(&entry.summary.updated_at)
            ),
            muted,
        ),
    ];
    if !entry.detail.is_empty() {
        lines.push(Line::styled(entry.detail.clone(), muted));
    }
    if !entry.archive_allowed && manager.tab != SessionTab::Archived {
        lines.push(Line::styled(
            "Archive: stop work and clear draft/queue first",
            ratatui_style(palette.warning_style()),
        ));
    }
    let preview = &manager.preview;
    if let Some(work) = &preview.work {
        lines.extend([
            Line::default(),
            Line::styled(
                format!(
                    "Work · {} tasks · {} goals · {} plans",
                    work.open_task_count, work.goal_count, work.plan_count
                ),
                ratatui_style(palette.user_style()),
            ),
        ]);
        for (title, status) in work.tasks.iter().take(3) {
            lines.push(Line::styled(
                format!("{:?} · {}", status, sanitize_approval_field(title)),
                muted,
            ));
        }
        for (title, status) in work.goals.iter().take(2) {
            lines.push(Line::styled(
                format!("{:?} · {}", status, sanitize_approval_field(title)),
                muted,
            ));
        }
    } else {
        lines.push(Line::styled(
            if preview.loading {
                "Work · loading…"
            } else {
                "Work details unavailable"
            },
            muted,
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled("Recent conversation", ratatui_style(palette.user_style())),
    ]);
    if preview.failed {
        lines.push(Line::styled(
            "Preview unavailable. Press r to retry.",
            muted,
        ));
    } else if preview.loading && preview.messages.is_empty() {
        lines.push(Line::styled("Loading preview…", muted));
    } else {
        let preview_entry = InteractiveSessionBrowserEntry {
            summary: entry.summary.clone(),
            recent_messages: preview.messages.clone(),
        };
        lines.extend(crate::session_browser::preview_lines(
            &preview_entry,
            palette,
            usize::from(area.width),
        ));
    }
    let maximum = lines
        .iter()
        .map(|line| {
            UnicodeWidthStr::width(line.to_string().as_str())
                .div_ceil(usize::from(area.width).max(1))
        })
        .sum::<usize>()
        .saturating_sub(usize::from(area.height));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((
            u16::try_from(preview.scroll.min(maximum)).unwrap_or(u16::MAX),
            0,
        )),
        area,
    );
}
