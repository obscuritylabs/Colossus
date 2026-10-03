use super::*;

pub(super) struct BrowserHeader<'a> {
    pub(super) title: &'a str,
    pub(super) count: String,
    pub(super) search_hint: &'a str,
    pub(super) query: &'a str,
    pub(super) search_active: bool,
}

pub(super) struct BrowserAreas {
    pub(super) list: Rect,
    pub(super) preview: Option<Rect>,
    pub(super) controls: Rect,
}

/// A shared full-screen surface for reversible selection and preview workflows.
pub(super) fn render_browser_shell(
    frame: &mut Frame<'_>,
    palette: &TerminalPalette,
    area: Rect,
    list_percent: u16,
    header: BrowserHeader<'_>,
) -> BrowserAreas {
    frame.render_widget(Clear, area);
    let margin: u16 = if area.width >= 80 { 2 } else { 1 };
    let inner = Rect::new(
        area.x.saturating_add(margin),
        area.y.saturating_add(1),
        area.width.saturating_sub(margin.saturating_mul(2)),
        area.height.saturating_sub(1),
    );
    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(if area.width >= 72 { 2 } else { 3 }),
    ])
    .split(inner);
    let (left, preview) = if area.width >= 80 && area.height >= 16 {
        let panes = Layout::horizontal([
            Constraint::Percentage(list_percent),
            Constraint::Percentage(100 - list_percent),
        ])
        .split(rows[0]);
        let separator = Block::default()
            .borders(Borders::LEFT)
            .border_style(ratatui_style(palette.meta_style()).add_modifier(Modifier::DIM));
        frame.render_widget(separator, panes[1]);
        let right = Rect::new(
            panes[1].x.saturating_add(3),
            panes[1].y,
            panes[1].width.saturating_sub(3),
            panes[1].height,
        );
        frame.render_widget(
            Paragraph::new(Span::styled("View", ratatui_style(palette.meta_style()))),
            Rect::new(right.x, right.y, right.width, 1),
        );
        (
            Rect::new(
                panes[0].x,
                panes[0].y,
                panes[0].width.saturating_sub(2),
                panes[0].height,
            ),
            Some(Rect::new(
                right.x,
                right.y.saturating_add(2),
                right.width,
                right.height.saturating_sub(2),
            )),
        )
    } else {
        (rows[0], None)
    };
    let left_rows = Layout::vertical([Constraint::Length(4), Constraint::Min(1)]).split(left);
    render_browser_header(frame, palette, &header, left_rows[0]);
    BrowserAreas {
        list: left_rows[1],
        preview,
        controls: rows[1],
    }
}

fn render_browser_header(
    frame: &mut Frame<'_>,
    palette: &TerminalPalette,
    header: &BrowserHeader<'_>,
    area: Rect,
) {
    let title = format!("{} · {}", header.title, header.count);
    let title = if UnicodeWidthStr::width(title.as_str()) <= usize::from(area.width) {
        title
    } else {
        header.title.to_owned()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            title,
            ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
        )),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let query = sanitize_approval_field(header.query);
    let search = if query.is_empty() && !header.search_active {
        format!("/ {}", header.search_hint)
    } else {
        // Keep the editing end visible even when a pasted query exceeds the pane.
        let mut tail = Vec::new();
        let mut width = 0;
        for grapheme in query.graphemes(true).rev() {
            width += UnicodeWidthStr::width(grapheme);
            if width > usize::from(area.width.saturating_sub(3)) {
                break;
            }
            tail.push(grapheme);
        }
        tail.reverse();
        format!("/ {}", tail.concat())
    };
    let y = area.y.saturating_add(2);
    frame.render_widget(
        Paragraph::new(Span::styled(
            search.clone(),
            ratatui_style(if header.search_active {
                palette.user_style()
            } else {
                palette.meta_style()
            }),
        )),
        Rect::new(area.x, y, area.width, 1),
    );
    if header.search_active && area.width > 0 && area.height > 2 {
        frame.set_cursor_position(Position::new(
            area.x.saturating_add(
                u16::try_from(UnicodeWidthStr::width(search.as_str()))
                    .unwrap_or(u16::MAX)
                    .min(area.width - 1),
            ),
            y,
        ));
    }
}

pub(super) fn render_browser_controls(
    frame: &mut Frame<'_>,
    palette: &TerminalPalette,
    area: Rect,
    controls: &[(&str, &str)],
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(ratatui_style(palette.meta_style()).add_modifier(Modifier::DIM));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let key_style = ratatui_style(palette.warning_style()).add_modifier(Modifier::BOLD);
    let label_style = ratatui_style(palette.meta_style());
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut width = 0;
    for &(key, label) in controls {
        let next_width = UnicodeWidthStr::width(key) + UnicodeWidthStr::width(label) + 1;
        if width > 0 && width + 2 + next_width > usize::from(inner.width) {
            lines.push(Line::from(std::mem::take(&mut spans)));
            width = 0;
        }
        if width > 0 {
            spans.push(Span::raw("  "));
            width += 2;
        }
        spans.push(Span::styled(key.to_owned(), key_style));
        spans.push(Span::styled(format!(" {label}"), label_style));
        width += next_width;
    }
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn browser_selection_style(palette: &TerminalPalette) -> Style {
    let accent = ratatui_style(palette.user_style());
    let background = match accent.fg.unwrap_or(Color::LightGreen) {
        Color::Rgb(red, green, blue) => Color::Rgb(
            soften_channel(red),
            soften_channel(green),
            soften_channel(blue),
        ),
        Color::Green | Color::LightGreen => Color::Rgb(191, 255, 207),
        Color::Cyan | Color::LightCyan => Color::Rgb(207, 246, 255),
        _ => Color::Gray,
    };
    Style::default()
        .fg(Color::Black)
        .bg(background)
        .add_modifier(Modifier::BOLD)
}

const fn soften_channel(value: u8) -> u8 {
    ((value as u16 * 28 + 255 * 72) / 100) as u8
}
