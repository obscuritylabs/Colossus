//! Workspace session navigation; runtime controls are handled by the workspace host.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceSessionRow {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) status: WorkspaceSessionStatus,
    pub(super) unread: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceSessionStatus {
    Ready,
    Running,
    Waiting,
    Complete,
    Failed,
    Stopped,
}

impl WorkspaceSessionStatus {
    pub(super) fn display(self) -> (&'static str, &'static str, PresentationTone) {
        match self {
            Self::Ready => ("○", "ready", PresentationTone::Neutral),
            Self::Running => ("●", "working", PresentationTone::Tool),
            Self::Waiting => ("!", "decision", PresentationTone::Warning),
            Self::Complete => ("✓", "done", PresentationTone::Success),
            Self::Failed => ("!", "failed", PresentationTone::Error),
            Self::Stopped => ("×", "stopped", PresentationTone::Warning),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WorkspaceAction {
    Switch(String),
    New,
    Browse,
    Center,
    History,
    Tab(crate::session_manager::SessionTab),
    Select(String),
    Archive(String, bool),
    CloseCenter,
    Stop(String),
}

/// Keep workspace navigation visible below the independently scrolling agent details.
pub(super) fn sessions_height(state: &TuiState, height: u16) -> u16 {
    if state.workspace_sessions.is_empty() {
        return 0;
    }
    let desired = u16::try_from(state.workspace_sessions.len().saturating_add(4))
        .unwrap_or(u16::MAX)
        .clamp(6, 10);
    desired.min(height.saturating_sub(4))
}

pub(super) fn render_sessions(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let muted = ratatui_style(palette.meta_style());
    let block = Block::default()
        .borders(Borders::LEFT | Borders::TOP)
        .border_style(muted);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = inset_horizontal(inner);
    if inner.height == 0 {
        return;
    }
    let heading = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new("Workspace sessions").style(muted.add_modifier(Modifier::BOLD)),
        heading,
    );
    state
        .workspace_hits
        .push((heading, WorkspaceAction::Center));
    let controls_height = u16::from(inner.height >= 3);
    let list = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1 + controls_height),
    );
    render_rows(frame, state, list, false, &palette);
    if controls_height > 0 {
        let controls = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
        render_actions(
            frame,
            state,
            controls,
            &palette,
            &[
                ("+ New", WorkspaceAction::New),
                ("F2 All", WorkspaceAction::Center),
            ],
        );
    }
    state.workspace_hits.push((area, WorkspaceAction::Center));
}

pub(super) fn render_center(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    crate::session_manager::render(frame, state, area);
}

fn inset_horizontal(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(u16::from(area.width > 0)),
        area.y,
        area.width.saturating_sub(2),
        area.height,
    )
}

fn render_actions(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    area: Rect,
    palette: &TerminalPalette,
    actions: &[(&str, WorkspaceAction)],
) {
    let mut x = area.x;
    for (label, action) in actions {
        let width = u16::try_from(UnicodeWidthStr::width(*label)).unwrap_or(u16::MAX);
        if x.saturating_add(width) > area.right() {
            break;
        }
        let hit = Rect::new(x, area.y, width, 1);
        frame.render_widget(
            Paragraph::new(*label).style(ratatui_style(palette.user_style())),
            hit,
        );
        state.workspace_hits.push((hit, action.clone()));
        x = x.saturating_add(width + 2);
    }
}

fn render_rows(
    frame: &mut Frame<'_>,
    state: &mut TuiState,
    area: Rect,
    center: bool,
    palette: &TerminalPalette,
) {
    let count = state.workspace_sessions.len();
    state.workspace_selected = state.workspace_selected.min(count.saturating_sub(1));
    if area.height == 0 || area.width == 0 {
        return;
    }
    if count == 0 {
        frame.render_widget(
            Paragraph::new("No open sessions. Press n to start.")
                .style(ratatui_style(palette.meta_style())),
            area,
        );
        return;
    }
    let overflow = count > usize::from(area.height);
    let visible = usize::from(area.height.saturating_sub(u16::from(overflow))).max(1);
    let start = state
        .workspace_selected
        .saturating_sub(visible.saturating_sub(1))
        .min(count.saturating_sub(visible));
    for (offset, index) in (start..count).take(visible).enumerate() {
        let row = &state.workspace_sessions[index];
        let selected = center && state.workspace_selected == index;
        let active = row.id == state.session_id;
        let mut text = session_line(row, active, usize::from(area.width), palette);
        let style = if selected {
            browser_selection_style(palette)
        } else {
            Style::default()
        };
        if selected {
            for span in &mut text.spans {
                span.style = style;
            }
        }
        let hit = Rect::new(area.x, area.y + offset as u16, area.width, 1);
        let action = WorkspaceAction::Switch(row.id.clone());
        frame.render_widget(Paragraph::new(text).style(style), hit);
        state.workspace_hits.push((hit, action));
    }
    if overflow && area.height > 1 {
        frame.render_widget(
            Paragraph::new(format!(
                "{}-{}/{} · scroll",
                start + 1,
                (start + visible).min(count),
                count
            ))
            .style(ratatui_style(palette.meta_style())),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

fn session_line(
    row: &WorkspaceSessionRow,
    active: bool,
    width: usize,
    palette: &TerminalPalette,
) -> Line<'static> {
    let (symbol, status, tone) = row.status.display();
    let title = sanitize_approval_field(&row.title.chars().take(256).collect::<String>());
    let title = if title.trim().is_empty() {
        "Untitled session"
    } else {
        &title
    };
    let unread = if row.unread == 0 {
        String::new()
    } else {
        format!(" +{}", row.unread.min(999))
    };
    let status = format!("{status}{unread}");
    crate::agents::agent_line(
        if active { "› " } else { "  " },
        symbol,
        title,
        &status,
        width,
        ratatui_style(palette.tone_style(tone)),
        ratatui_style(palette.meta_style()),
    )
}

pub(super) fn handle_mouse(state: &mut TuiState, mouse: MouseEvent) -> bool {
    let hit = state
        .workspace_hits
        .iter()
        .find(|(area, _)| area.contains(Position::new(mouse.column, mouse.row)))
        .map(|(_, action)| action.clone());
    if let Some(action) = hit {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => state.workspace_action = Some(action),
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if state.workspace_center => {
                crate::session_manager::scroll(state, mouse)
            }
            MouseEventKind::ScrollDown => move_selection(state, MOUSE_SCROLL_LINES as isize),
            MouseEventKind::ScrollUp => move_selection(state, -(MOUSE_SCROLL_LINES as isize)),
            _ => return state.workspace_center,
        }
        return true;
    }
    if state.workspace_center {
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                crate::session_manager::scroll(state, mouse)
            }
            _ => {}
        }
    }
    state.workspace_center
}

pub(super) fn handle_key(state: &mut TuiState, key: KeyEvent) -> bool {
    crate::session_manager::handle_key(state, key)
}

fn move_selection(state: &mut TuiState, offset: isize) {
    state.workspace_selected = state
        .workspace_selected
        .saturating_add_signed(offset)
        .min(state.workspace_sessions.len().saturating_sub(1));
}

#[cfg(test)]
mod tests;
