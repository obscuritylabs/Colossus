//! Keyboard navigation and released status for the Agents tab.

use super::*;
use crate::agents::{AgentAvailability, AgentNode, agent_status};
use crate::workspace_view::WorkspaceSessionRow;

#[derive(Clone, Copy)]
enum Attention {
    Approval,
    Input,
    Both,
}

impl Attention {
    fn label(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Self::Approval, false) => "Approval needed",
            (Self::Approval, true) => "Approval",
            (Self::Input, false) => "Input needed",
            (Self::Input, true) => "Input",
            (Self::Both, false) => "Approval and input",
            (Self::Both, true) => "Approval+input",
        }
    }
}

pub(crate) struct AgentGroup {
    id: String,
    title: String,
    status: WorkspaceSessionStatus,
    attention: Option<Attention>,
    availability: AgentAvailability,
    jobs: Vec<Arc<AgentNode>>,
}

impl AgentGroup {
    pub(crate) fn new(row: &WorkspaceSessionRow, state: &TuiState) -> Self {
        let prompts = match state.overlay.as_ref() {
            Some(Overlay::Prompt { request, .. }) => Some(request),
            _ => None,
        };
        let mut approval = false;
        let mut input = false;
        for prompt in prompts.iter().copied().chain(&state.pending_prompts) {
            if prompt.response.is_closed() {
                continue;
            }
            match prompt.kind {
                InteractivePromptKind::Approval => approval = true,
                InteractivePromptKind::UserInput
                | InteractivePromptKind::SandboxBoundaryAcknowledgement => input = true,
                InteractivePromptKind::Choice | InteractivePromptKind::ModelSelection => {}
            }
        }
        let attention = match (approval, input) {
            (true, true) => Some(Attention::Both),
            (true, false) => Some(Attention::Approval),
            (false, true) => Some(Attention::Input),
            (false, false) => None,
        };
        let status = if row.status == WorkspaceSessionStatus::Waiting && attention.is_none() {
            if state.is_busy() {
                WorkspaceSessionStatus::Running
            } else {
                WorkspaceSessionStatus::Ready
            }
        } else {
            row.status
        };
        let availability = if state.session_saved {
            state.agents.availability()
        } else {
            AgentAvailability::Current
        };
        Self {
            id: row.id.clone(),
            title: row.title.clone(),
            status,
            attention,
            availability,
            jobs: state.agents.ordered_nodes().into_iter().cloned().collect(),
        }
    }

    fn summary(&self, compact: bool) -> String {
        let label = match (self.availability, compact) {
            (AgentAvailability::Loading, false) => "Loading jobs",
            (AgentAvailability::Loading, true) => "Loading",
            (AgentAvailability::Current, _) if !self.jobs.is_empty() => {
                let active = self.jobs.iter().filter(|job| job.active()).count();
                return format!("{active}/{} active", self.jobs.len());
            }
            (AgentAvailability::Current, false) => "No child jobs",
            (AgentAvailability::Current, true) => "No jobs",
            (AgentAvailability::Stale, false) => "Jobs stale",
            (AgentAvailability::Stale, true) => "Stale",
            (AgentAvailability::Unavailable, false) => "Jobs unavailable",
            (AgentAvailability::Unavailable, true) => "Unavailable",
        };
        label.into()
    }
}

#[derive(PartialEq, Eq)]
struct Cursor {
    parent: String,
    child: Option<String>,
}

#[derive(Default)]
pub(crate) struct AgentsView {
    groups: Vec<AgentGroup>,
    expanded: BTreeSet<String>,
    selected: Option<Cursor>,
}

struct Row<'a> {
    group: &'a AgentGroup,
    job: Option<&'a AgentNode>,
}

impl Row<'_> {
    fn matches(&self, cursor: &Cursor) -> bool {
        self.group.id == cursor.parent
            && self.job.map(|job| job.id.as_str()) == cursor.child.as_deref()
    }

    fn cursor(&self) -> Cursor {
        Cursor {
            parent: self.group.id.clone(),
            child: self.job.map(|job| job.id.clone()),
        }
    }
}

impl AgentsView {
    pub(crate) fn clear_groups(&mut self) {
        self.groups.clear();
    }

    pub(crate) fn update(&mut self, groups: Vec<AgentGroup>) {
        self.groups = groups;
        self.expanded
            .retain(|id| self.groups.iter().any(|group| group.id == *id));
        if let Some(selected) = &self.selected {
            if self.rows().any(|row| row.matches(selected)) {
                return;
            }
            if self.groups.iter().any(|group| group.id == selected.parent) {
                self.select_parent(selected.parent.clone());
                return;
            }
        }
        self.selected = self.groups.first().map(|group| Cursor {
            parent: group.id.clone(),
            child: None,
        });
    }

    pub(crate) fn select_parent(&mut self, id: String) {
        if self.groups.iter().any(|group| group.id == id) {
            self.selected = Some(Cursor {
                parent: id,
                child: None,
            });
        }
    }

    fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        self.groups.iter().flat_map(|group| {
            let expanded = self.expanded.contains(&group.id);
            std::iter::once(Row { group, job: None }).chain(
                group.jobs.iter().filter(move |_| expanded).map(|job| Row {
                    group,
                    job: Some(job),
                }),
            )
        })
    }

    pub(super) fn move_selection(&mut self, offset: isize) {
        let rows = self.rows().collect::<Vec<_>>();
        let current = self.selected_index(&rows);
        let next = current
            .saturating_add_signed(offset)
            .min(rows.len().saturating_sub(1));
        self.selected = rows.get(next).map(Row::cursor);
    }

    fn selected_index(&self, rows: &[Row<'_>]) -> usize {
        self.selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| row.matches(selected)))
            .unwrap_or(0)
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Option<WorkspaceAction> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::PageUp => self.move_selection(-5),
            KeyCode::PageDown => self.move_selection(5),
            KeyCode::Enter => {
                return self
                    .selected
                    .as_ref()
                    .map(|cursor| WorkspaceAction::Switch(cursor.parent.clone()));
            }
            KeyCode::Right | KeyCode::Left | KeyCode::Char(' ') => {
                if let Some(cursor) = &mut self.selected {
                    match key.code {
                        KeyCode::Right => {
                            self.expanded.insert(cursor.parent.clone());
                        }
                        KeyCode::Left if cursor.child.is_some() => cursor.child = None,
                        KeyCode::Left => {
                            self.expanded.remove(&cursor.parent);
                        }
                        _ => {
                            if !self.expanded.remove(&cursor.parent) {
                                self.expanded.insert(cursor.parent.clone());
                            }
                            cursor.child = None;
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }
}

pub(super) fn draw(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    frame.render_widget(Clear, area);
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let muted = ratatui_style(palette.meta_style());
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(if inner.width >= 66 { 2 } else { 3 }),
    ])
    .split(inner);
    let count = state.workspace_manager.agents.groups.len();
    let unit = if count == 1 { "session" } else { "sessions" };
    let heading = format!("Agents · {count} {unit}");
    frame.render_widget(
        Paragraph::new(heading)
            .style(ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD)),
        sections[0],
    );
    super::render::draw_tabs(frame, state, &palette, sections[1]);
    let roster = &state.workspace_manager.agents;
    let rows = roster.rows().collect::<Vec<_>>();
    let list = sections[2];
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("No open sessions").style(muted), list);
    }
    let visible = usize::from(list.height);
    let current = roster.selected_index(&rows);
    let start = current
        .saturating_sub(visible / 2)
        .min(rows.len().saturating_sub(visible));
    for (index, row) in rows.iter().enumerate().skip(start).take(visible) {
        let hit = Rect::new(list.x, list.y + (index - start) as u16, list.width, 1);
        let selected = index == current;
        let line = draw_row(
            row,
            selected,
            roster.expanded.contains(&row.group.id),
            hit,
            &palette,
        );
        let mut row_widget = Paragraph::new(line);
        if selected {
            row_widget = row_widget.style(browser_selection_style(&palette));
        }
        frame.render_widget(row_widget, hit);
        if row.job.is_none() {
            state
                .workspace_hits
                .push((hit, WorkspaceAction::Select(row.group.id.clone())));
        }
    }
    render_browser_controls(
        frame,
        &palette,
        sections[3],
        &[
            ("Tab", "Views"),
            ("↑/↓", "Select"),
            ("Space", "Expand"),
            ("Enter", "Open"),
            ("Esc", "Back"),
        ],
    );
}

fn draw_row(
    row: &Row<'_>,
    selected: bool,
    expanded: bool,
    area: Rect,
    palette: &TerminalPalette,
) -> Line<'static> {
    let prefix = if selected { "› " } else { "  " };
    let (prefix, marker, label, status, tone) = if let Some(job) = row.job {
        let (marker, status, tone) = agent_status(job.status);
        let activity = job
            .updated_at
            .map(crate::session_browser::relative_activity)
            .unwrap_or_else(|| "Unavailable".into());
        (
            format!("{prefix}  "),
            marker,
            job.label.as_str(),
            format!("{status:<11} {activity:>11}"),
            tone,
        )
    } else {
        let (_, status, tone) = row.group.status.display();
        let metadata = |compact| {
            let attention = row
                .group
                .attention
                .map(|attention| attention.label(compact))
                .unwrap_or(status);
            format!("{} · {attention}", row.group.summary(compact))
        };
        let mut metadata_text = metadata(false);
        if UnicodeWidthStr::width(metadata_text.as_str())
            > usize::from(area.width.saturating_sub(14))
        {
            metadata_text = metadata(true);
        }
        (
            prefix.to_owned(),
            if expanded { "▾" } else { "▸" },
            row.group.title.as_str(),
            metadata_text,
            if row.group.attention.is_some() {
                PresentationTone::Warning
            } else {
                tone
            },
        )
    };
    let style = if selected {
        browser_selection_style(palette)
    } else {
        ratatui_style(palette.tone_style(tone))
    };
    crate::agents::agent_line(
        &prefix,
        marker,
        label,
        &status,
        usize::from(area.width),
        style,
        if selected {
            style
        } else {
            ratatui_style(palette.meta_style())
        },
    )
}
