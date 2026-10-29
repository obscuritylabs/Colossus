//! Read-only, lazily formatted tool inspection; raw results stay in transcript sources.
use super::*;

#[derive(Clone)]
pub(super) struct ToolHit {
    pub area: Rect,
    index: usize,
    session: String,
    epoch: u64,
    count: usize,
}

impl ToolHit {
    pub fn new(area: Rect, index: usize, state: &TuiState) -> Self {
        Self {
            area,
            index,
            session: state.session_id.clone(),
            epoch: state.transcript_epoch,
            count: state.transcript.len(),
        }
    }
    fn current(&self, state: &TuiState) -> bool {
        self.session == state.session_id
            && self.epoch == state.transcript_epoch
            && self.count == state.transcript.len()
    }
}

pub(super) struct ToolDetailsState {
    index: usize,
    session: String,
    epoch: u64,
    document: PresentationDocument,
    lines: Vec<Line<'static>>,
    width: usize,
    preferences: TerminalPreferences,
    scroll: usize,
    height: usize,
    close: Rect,
}

fn inspectable(state: &TuiState, index: usize) -> bool {
    state
        .transcript_sources
        .get(index)
        .and_then(Option::as_ref)
        .is_some_and(TranscriptRenderSource::inspectable)
}

pub(super) fn open(state: &mut TuiState, index: usize) {
    if !inspectable(state, index)
        || state.workspace_center
        || state.selecting_text
        || state
            .overlay
            .as_ref()
            .is_some_and(|overlay| !matches!(overlay, Overlay::ToolDetails(_)))
    {
        return;
    }
    let mut preferences = state.preferences.clone();
    preferences.events_mode = colossus_contracts::EventDisplayMode::Verbose;
    let Some(document) = state.transcript_sources[index]
        .as_ref()
        .and_then(|source| source.render(&preferences))
    else {
        return;
    };
    state.hide_completion();
    state.text_selection.clear();
    state.tool_click = None;
    state.overlay = Some(Overlay::ToolDetails(ToolDetailsState {
        index,
        session: state.session_id.clone(),
        epoch: state.transcript_epoch,
        document,
        lines: Vec::new(),
        width: 0,
        preferences: state.preferences.clone(),
        scroll: 0,
        height: 0,
        close: Rect::default(),
    }));
}

pub(super) fn key(state: &mut TuiState, key: KeyEvent) {
    let Some(Overlay::ToolDetails(details)) = state.overlay.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::F(3) => {
            state.overlay = None;
            state.toggle_event_details();
        }
        KeyCode::Up => details.scroll = details.scroll.saturating_sub(1),
        KeyCode::Down => details.scroll = details.scroll.saturating_add(1),
        KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(details.height.max(1)),
        KeyCode::PageDown | KeyCode::Char(' ') => {
            details.scroll = details.scroll.saturating_add(details.height.max(1))
        }
        KeyCode::Home => details.scroll = 0,
        KeyCode::End => details.scroll = usize::MAX,
        KeyCode::Left | KeyCode::Right => {
            let index = details.index;
            let next = if key.code == KeyCode::Left {
                (0..index).rev().find(|&index| inspectable(state, index))
            } else {
                (index.saturating_add(1)..state.transcript.len())
                    .find(|&index| inspectable(state, index))
            };
            if let Some(next) = next {
                open(state, next);
            }
        }
        _ => {}
    }
}

/// Track a stationary click, leaving drag-to-copy with the selection controller.
pub(super) fn mouse(state: &mut TuiState, mouse: MouseEvent) -> bool {
    let position = Position::new(mouse.column, mouse.row);
    if let Some(Overlay::ToolDetails(details)) = state.overlay.as_mut() {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if details.close.contains(position) => {
                state.text_selection.clear();
                state.overlay = None;
                return true;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                state.text_selection.clear();
                details.scroll = if mouse.kind == MouseEventKind::ScrollUp {
                    details.scroll.saturating_sub(MOUSE_SCROLL_LINES)
                } else {
                    details.scroll.saturating_add(MOUSE_SCROLL_LINES)
                };
                return true;
            }
            _ => return false,
        }
    }
    if state.overlay.is_some() || state.workspace_center || state.selecting_text {
        state.tool_click = None;
        return false;
    }
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            state.tool_click = state
                .tool_hits
                .iter()
                .find(|hit| hit.current(state) && hit.area.contains(position))
                .cloned()
                .map(|hit| (hit, position));
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some((hit, anchor)) = state.tool_click.take()
                && anchor == position
                && hit.current(state)
                && hit.area.contains(position)
                && state
                    .tool_hits
                    .iter()
                    .any(|visible| visible.index == hit.index && visible.area == hit.area)
            {
                open(state, hit.index);
                return true;
            }
        }
        MouseEventKind::Drag(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            state.tool_click = None;
        }
        _ => {}
    }
    false
}

pub(super) fn render(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let Some(Overlay::ToolDetails(details)) = state.overlay.as_mut() else {
        return;
    };
    if details.session != state.session_id || details.epoch != state.transcript_epoch {
        state.overlay = None;
        return;
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(" Tool details").style(ratatui_style(palette.tool_style())),
        rows[0],
    );
    details.close = Rect::new(area.right().saturating_sub(9), area.y, 9.min(area.width), 1);
    frame.render_widget(
        Paragraph::new("[ Close ]").style(ratatui_style(palette.meta_style())),
        details.close,
    );
    let content = rows[1].inner(ratatui::layout::Margin::new(1, 1));
    let width = usize::from(content.width).max(1);
    if width != details.width || details.preferences != state.preferences {
        let mut detail_preferences = state.preferences.clone();
        detail_preferences.events_mode = colossus_contracts::EventDisplayMode::Verbose;
        details.lines =
            crate::render::styled_document_lines(&details.document, &detail_preferences, width);
        details.width = width;
        details.preferences = state.preferences.clone();
    }
    details.height = usize::from(content.height);
    details.scroll = details
        .scroll
        .min(details.lines.len().saturating_sub(details.height));
    let lines = details
        .lines
        .iter()
        .skip(details.scroll)
        .take(details.height)
        .cloned()
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), content);
    state.text_selection.regions.push(content);
    let footer = format!(
        " Esc close · ←/→ event · ↑/↓ scroll · {}/{}",
        details.scroll.saturating_add(1),
        details.lines.len().max(1)
    );
    frame.render_widget(
        Paragraph::new(crate::render::truncate_width_with_ellipsis(
            &footer,
            usize::from(area.width),
        ))
        .style(ratatui_style(palette.meta_style())),
        rows[2],
    );
}
