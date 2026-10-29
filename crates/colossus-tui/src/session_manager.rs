//! Pure presentation state for the unified session inventory.

use super::*;
use crate::workspace_view::{WorkspaceAction, WorkspaceSessionStatus};

mod render;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SessionTab {
    #[default]
    Open,
    History,
    Archived,
}

impl SessionTab {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::History => "History",
            Self::Archived => "Archived",
        }
    }

    fn next(self, reverse: bool) -> Self {
        match (self, reverse) {
            (Self::Open, false) | (Self::Archived, true) => Self::History,
            (Self::History, false) | (Self::Open, true) => Self::Archived,
            _ => Self::Open,
        }
    }
}

#[derive(Clone)]
pub(super) struct ManagerEntry {
    pub(super) summary: SessionSummary,
    pub(super) status: WorkspaceSessionStatus,
    pub(super) open: bool,
    pub(super) detail: String,
    pub(super) archive_allowed: bool,
}

#[derive(Default)]
pub(super) struct ManagerPreview {
    pub(super) id: Option<String>,
    pub(super) generation: u64,
    pub(super) messages: Vec<InteractiveSessionBrowserMessage>,
    pub(super) work: Option<SidebarSnapshot>,
    pub(super) loading: bool,
    pub(super) failed: bool,
    pub(super) scroll: usize,
}

#[derive(Default)]
pub(super) struct SessionManagerState {
    pub(super) tab: SessionTab,
    pub(super) query: String,
    pub(super) searching: bool,
    pub(super) entries: Vec<ManagerEntry>,
    pub(super) selected: Option<String>,
    pub(super) preview: ManagerPreview,
    pub(super) preview_area: Option<Rect>,
}

impl SessionManagerState {
    pub(super) fn filtered(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let summary = &entry.summary;
                (query.is_empty()
                    || session_browser_title(summary)
                        .to_lowercase()
                        .contains(&query)
                    || summary.id.to_lowercase().contains(&query)
                    || summary
                        .last_user_preview
                        .as_deref()
                        .is_some_and(|text| text.to_lowercase().contains(&query)))
                .then_some(index)
            })
            .collect()
    }

    pub(super) fn selected_entry(&self) -> Option<&ManagerEntry> {
        self.selected
            .as_ref()
            .and_then(|id| self.entries.iter().find(|entry| &entry.summary.id == id))
    }

    pub(super) fn select(&mut self, id: Option<String>) {
        if self.selected != id {
            self.selected = id.clone();
            let generation = self.preview.generation.wrapping_add(1);
            self.preview = ManagerPreview {
                id,
                generation,
                ..Default::default()
            };
        }
    }

    pub(super) fn reconcile(&mut self) {
        let indices = self.filtered();
        if !indices
            .iter()
            .any(|index| self.selected.as_deref() == Some(self.entries[*index].summary.id.as_str()))
        {
            self.select(
                indices
                    .first()
                    .map(|index| self.entries[*index].summary.id.clone()),
            );
        }
    }

    fn move_selection(&mut self, offset: isize) {
        let choices = self.filtered();
        if choices.is_empty() {
            self.select(None);
            return;
        }
        let current = choices
            .iter()
            .position(|index| {
                self.selected.as_deref() == Some(self.entries[*index].summary.id.as_str())
            })
            .unwrap_or(0);
        let next = current.saturating_add_signed(offset).min(choices.len() - 1);
        self.select(Some(self.entries[choices[next]].summary.id.clone()));
    }
}

/// Hosts normally supply metadata; retain a useful view for minimal TUI hosts too.
fn ensure_entries(state: &mut TuiState) {
    if state.workspace_manager.tab != SessionTab::Open
        || !state.workspace_manager.entries.is_empty()
        || state.workspace_sessions.is_empty()
    {
        return;
    }
    state.workspace_manager.entries = state
        .workspace_sessions
        .iter()
        .map(|row| ManagerEntry {
            summary: SessionSummary {
                id: row.id.clone(),
                title: Some(row.title.clone()),
                created_at: String::new(),
                updated_at: String::new(),
                message_count: 0,
                last_run_id: None,
                last_user_preview: None,
            },
            status: row.status,
            open: true,
            detail: String::new(),
            archive_allowed: !matches!(
                row.status,
                WorkspaceSessionStatus::Running | WorkspaceSessionStatus::Waiting
            ),
        })
        .collect();
    state.workspace_manager.select(
        state
            .workspace_manager
            .entries
            .get(state.workspace_selected)
            .map(|entry| entry.summary.id.clone()),
    );
}

pub(super) fn render(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    ensure_entries(state);
    render::draw(frame, state, area);
}

pub(super) fn handle_key(state: &mut TuiState, key: KeyEvent) -> bool {
    if !state.workspace_center {
        return false;
    }
    ensure_entries(state);
    let manager = &mut state.workspace_manager;
    if manager.searching {
        match key.code {
            KeyCode::Esc => manager.searching = false,
            KeyCode::Enter => {
                manager.searching = false;
                if manager.tab != SessionTab::Archived
                    && let Some(entry) = manager.selected_entry()
                {
                    state.workspace_action =
                        Some(WorkspaceAction::Switch(entry.summary.id.clone()));
                }
            }
            KeyCode::Up => manager.move_selection(-1),
            KeyCode::Down => manager.move_selection(1),
            KeyCode::Backspace => {
                manager.query.pop();
                manager.reconcile();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && manager.query.len() + character.len_utf8() <= 256 =>
            {
                manager.query.push(character);
                manager.reconcile();
            }
            _ => {}
        }
        return true;
    }
    match key.code {
        KeyCode::Esc => state.workspace_action = Some(WorkspaceAction::CloseCenter),
        KeyCode::Char('/') => manager.searching = true,
        KeyCode::Tab | KeyCode::BackTab => {
            state.workspace_action = Some(WorkspaceAction::Tab(
                manager.tab.next(key.code == KeyCode::BackTab),
            ))
        }
        KeyCode::Up => manager.move_selection(-1),
        KeyCode::Down => manager.move_selection(1),
        KeyCode::Home | KeyCode::End => {
            let choices = manager.filtered();
            let index = if key.code == KeyCode::Home {
                choices.first()
            } else {
                choices.last()
            };
            manager.select(index.map(|index| manager.entries[*index].summary.id.clone()));
        }
        KeyCode::PageUp => manager.preview.scroll = manager.preview.scroll.saturating_sub(5),
        KeyCode::PageDown => manager.preview.scroll = manager.preview.scroll.saturating_add(5),
        KeyCode::Enter => {
            if manager.tab != SessionTab::Archived
                && let Some(entry) = manager.selected_entry()
            {
                state.workspace_action = Some(WorkspaceAction::Switch(entry.summary.id.clone()));
            }
        }
        KeyCode::Char('n') if key.modifiers.is_empty() => {
            state.workspace_action = Some(WorkspaceAction::New)
        }
        KeyCode::Char('r') if key.modifiers.is_empty() => {
            state.workspace_action = Some(WorkspaceAction::Browse)
        }
        KeyCode::Char('s') if key.modifiers.is_empty() => {
            if let Some(entry) = manager.selected_entry()
                && matches!(
                    entry.status,
                    WorkspaceSessionStatus::Running | WorkspaceSessionStatus::Waiting
                )
            {
                state.workspace_action = Some(WorkspaceAction::Stop(entry.summary.id.clone()));
            }
        }
        KeyCode::Char('a') if key.modifiers.is_empty() && manager.tab != SessionTab::Archived => {
            if let Some(entry) = manager.selected_entry() {
                state.workspace_action =
                    Some(WorkspaceAction::Archive(entry.summary.id.clone(), true));
            }
        }
        KeyCode::Char('u') if key.modifiers.is_empty() && manager.tab == SessionTab::Archived => {
            if let Some(entry) = manager.selected_entry() {
                state.workspace_action =
                    Some(WorkspaceAction::Archive(entry.summary.id.clone(), false));
            }
        }
        _ => {}
    }
    true
}

pub(super) fn scroll(state: &mut TuiState, mouse: MouseEvent) {
    let delta = if mouse.kind == MouseEventKind::ScrollUp {
        -(MOUSE_SCROLL_LINES as isize)
    } else {
        MOUSE_SCROLL_LINES as isize
    };
    if state
        .workspace_manager
        .preview_area
        .is_some_and(|area| area.contains(Position::new(mouse.column, mouse.row)))
    {
        state.workspace_manager.preview.scroll = state
            .workspace_manager
            .preview
            .scroll
            .saturating_add_signed(delta);
    } else {
        state.workspace_manager.move_selection(delta);
    }
}
