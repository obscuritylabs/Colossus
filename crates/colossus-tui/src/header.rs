use super::*;

pub(super) const HEADER_HEIGHT: u16 = 2;

pub(super) fn render_header(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    if area.height < HEADER_HEIGHT || area.width < 4 {
        return;
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let band = chrome_band_style(&palette);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(ratatui_style(palette.meta_style()).add_modifier(Modifier::DIM));
    frame.render_widget(block, area);
    let row = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(Block::default().style(band), row);
    let inner = Rect::new(row.x + 1, row.y, row.width - 2, 1);
    let session = sanitize_approval_field(&state.session_id)
        .chars()
        .take(8)
        .collect::<String>();
    let detail = if inner.width >= 76 {
        format!("session {session}  ·  v{}", env!("CARGO_PKG_VERSION"))
    } else {
        format!("{session} · v{}", env!("CARGO_PKG_VERSION"))
    };
    let detail_width = u16::try_from(UnicodeWidthStr::width(detail.as_str()))
        .unwrap_or(inner.width)
        .min(inner.width);
    let workspace_width = inner.width.saturating_sub(detail_width + 3);
    let workspace = truncate_width_with_ellipsis(
        &workspace_display(&state.workspace),
        usize::from(workspace_width),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(workspace, band.add_modifier(Modifier::BOLD))),
        Rect::new(inner.x, inner.y, workspace_width, 1),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(detail, band)),
        Rect::new(inner.right() - detail_width, inner.y, detail_width, 1),
    );
}
