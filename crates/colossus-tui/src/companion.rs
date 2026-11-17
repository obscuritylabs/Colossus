//! Read-only tail of a retained session. The workspace owns its lifetime and events.

use super::*;
use crate::workspace_view::WorkspaceSessionRow;

const MAX_RECENT_ENTRIES: usize = 24;
const MIN_HEIGHT: u16 = 8;

pub(super) struct CompanionView<'a> {
    pub(super) row: WorkspaceSessionRow,
    pub(super) state: &'a TuiState,
}

impl CompanionView<'_> {
    pub(super) fn fits(area: Rect) -> bool {
        area.width >= crate::agents::PANEL_WIDTH && area.height >= MIN_HEIGHT
    }

    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = TerminalPalette::for_preferences(&self.state.preferences);
        let meta = ratatui_style(palette.meta_style());
        let block = Block::bordered()
            .title(" Pinned · Read-only ")
            .border_style(meta);
        let body = block.inner(area);
        frame.render_widget(block, area);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(body);
        let width = usize::from(body.width);
        let id = sanitize_approval_field(&self.row.id.chars().take(120).collect::<String>());
        let (marker, status, tone) = self.row.status.display();
        frame.render_widget(
            Paragraph::new(truncate_width_with_ellipsis(&self.row.title, width))
                .style(meta.add_modifier(Modifier::BOLD)),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(truncate_width_with_ellipsis(&format!("ID {id}"), width)).style(meta),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(format!("{marker} Status: {status}"))
                .style(ratatui_style(palette.tone_style(tone))),
            rows[2],
        );
        let start = self
            .state
            .transcript
            .len()
            .saturating_sub(MAX_RECENT_ENTRIES);
        let lines = companion_transcript_lines(self.state, width, start);
        let first = lines.len().saturating_sub(usize::from(rows[3].height));
        if lines.is_empty() {
            frame.render_widget(Paragraph::new("No messages yet").style(meta), rows[3]);
        } else {
            // The transcript formatter already accounts for the pane width.
            // Paint each of the selected tail rows exactly once.
            frame.render_widget(
                Paragraph::new(lines.into_iter().skip(first).collect::<Vec<_>>()),
                rows[3],
            );
        }
        frame.render_widget(
            Paragraph::new("Live tail · F2 to unpin").style(meta),
            rows[4],
        );
    }
}
