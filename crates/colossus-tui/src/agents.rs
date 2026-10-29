use super::*;

pub(super) const PANEL_WIDTH: u16 = 36;
pub(super) const MIN_CHAT_WIDTH: u16 = 64;
const MIN_PANEL_WIDTH: u16 = 28;
const MAX_AGENTS: usize = 100;

pub(super) fn panel_width(terminal_width: u16) -> u16 {
    let available = terminal_width.saturating_sub(MIN_CHAT_WIDTH);
    if available < MIN_PANEL_WIDTH {
        0
    } else {
        available.min(PANEL_WIDTH)
    }
}

/// Retain display metadata only; released child results stay in the transcript.
struct AgentNode {
    id: String,
    label: String,
    status: SubagentStatus,
    created_at: String,
    updated_at: String,
    live_revision: u64,
}

impl AgentNode {
    fn from_job(job: &SubagentJob, live_revision: u64) -> Self {
        let label = if job.task.trim().is_empty() {
            &job.role
        } else {
            &job.task
        };
        Self {
            id: job.id.clone(),
            label: sanitize_approval_field(&label.chars().take(256).collect::<String>()),
            status: job.status,
            created_at: job.created_at.clone(),
            updated_at: job.updated_at.clone(),
            live_revision,
        }
    }

    fn active(&self) -> bool {
        matches!(
            self.status,
            SubagentStatus::Queued | SubagentStatus::Running
        )
    }
}

#[derive(Default)]
pub(super) struct AgentTree {
    nodes: BTreeMap<String, AgentNode>,
    revision: u64,
    scroll: usize,
    pub(super) area: Option<Rect>,
    stale: bool,
}

impl AgentTree {
    pub(super) fn update(&mut self, job: &SubagentJob) {
        self.revision = self.revision.saturating_add(1);
        self.nodes
            .insert(job.id.clone(), AgentNode::from_job(job, self.revision));
        self.trim();
    }

    pub(super) fn refresh(
        &mut self,
        session_id: &str,
        revision: u64,
        result: Result<Vec<SubagentJob>, String>,
    ) {
        let Ok(jobs) = result else {
            self.stale = true;
            return;
        };
        self.stale = false;
        for job in jobs.into_iter().take(MAX_AGENTS) {
            if job.session_id != session_id {
                continue;
            }
            let live_revision = self.nodes.get(&job.id).map_or(0, |node| node.live_revision);
            // A slow snapshot cannot regress lifecycle events received during its read.
            if live_revision <= revision {
                self.nodes
                    .insert(job.id.clone(), AgentNode::from_job(&job, live_revision));
            }
        }
        self.trim();
    }

    fn trim(&mut self) {
        while self.nodes.len() > MAX_AGENTS {
            let oldest = self
                .nodes
                .values()
                .min_by_key(|node| (node.active(), &node.updated_at, &node.id))
                .map(|node| node.id.clone());
            if let Some(id) = oldest {
                self.nodes.remove(&id);
            }
        }
    }

    pub(super) fn width(&self, width: u16) -> u16 {
        if !self.nodes.is_empty() {
            panel_width(width)
        } else {
            0
        }
    }

    pub(super) fn inline_height(&self, width: u16) -> u16 {
        if self.width(width) == 0 {
            return 0;
        }
        u16::try_from(self.nodes.len().saturating_add(2))
            .unwrap_or(u16::MAX)
            .min(12)
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        if !self
            .area
            .is_some_and(|area| area.contains(Position::new(mouse.column, mouse.row)))
        {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(MOUSE_SCROLL_LINES)
            }
            MouseEventKind::ScrollDown => {
                self.scroll = self.scroll.saturating_add(MOUSE_SCROLL_LINES)
            }
            _ => return false,
        }
        true
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> bool {
        let Some(area) = self.area else {
            return false;
        };
        if !key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        let page = usize::from(area.height.saturating_sub(3)).max(1);
        match key.code {
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(page),
            _ => return false,
        }
        true
    }
}

/// Own at most one bounded read; cancel it on session changes and terminal shutdown.
#[derive(Default)]
pub(super) struct AgentRefresh {
    session_id: String,
    session_epoch: u64,
    last_read: Option<Instant>,
    last_status_read: Option<Instant>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for AgentRefresh {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl AgentRefresh {
    pub(super) fn schedule(
        &mut self,
        state: &TuiState,
        host: Arc<dyn InteractiveHost>,
        events: mpsc::Sender<HostEvent>,
    ) {
        if self.session_id != state.session_id || self.session_epoch != state.transcript_epoch {
            if let Some(task) = self.task.take() {
                task.abort();
            }
            self.session_id = state.session_id.clone();
            self.session_epoch = state.transcript_epoch;
            self.last_read = None;
            self.last_status_read = None;
        }
        let interval = if state.agents.nodes.values().any(AgentNode::active) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(5)
        };
        if self.task.as_ref().is_some_and(|task| !task.is_finished())
            || self.last_read.is_some_and(|last| last.elapsed() < interval)
        {
            return;
        }
        self.last_read = Some(Instant::now());
        let status_due = self
            .last_status_read
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(5));
        if status_due {
            self.last_status_read = Some(Instant::now());
        }
        let session_id = self.session_id.clone();
        let revision = state.agents.revision;
        let session_epoch = self.session_epoch;
        self.task = Some(tokio::spawn(async move {
            let (result, status) = tokio::join!(
                async {
                    tokio::time::timeout(Duration::from_secs(5), host.subagents(&session_id))
                        .await
                        .unwrap_or_else(|_| Err("agent refresh timed out".into()))
                },
                async {
                    if status_due {
                        Some(
                            tokio::time::timeout(
                                Duration::from_secs(5),
                                host.sidebar_status(&session_id),
                            )
                            .await
                            .unwrap_or_else(|_| Err("status refresh timed out".into())),
                        )
                    } else {
                        None
                    }
                },
            );
            if let Some(result) = status {
                let _ = events
                    .send(HostEvent::Sidebar {
                        session_id: session_id.clone(),
                        session_epoch,
                        result,
                    })
                    .await;
            }
            let _ = events
                .send(HostEvent::Subagents {
                    session_id,
                    revision,
                    session_epoch,
                    result,
                })
                .await;
        }));
    }
}

pub(super) fn render_agent_tree(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let sessions_height = crate::workspace_view::sessions_height(state, area.height);
    let sections =
        Layout::vertical([Constraint::Min(0), Constraint::Length(sessions_height)]).split(area);
    state.agents.area = Some(sections[0]);
    render_current_agents(frame, state, sections[0]);
    crate::workspace_view::render_sessions(frame, state, sections[1]);
}

fn render_current_agents(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let muted = ratatui_style(palette.meta_style());
    let block = Block::default().borders(Borders::LEFT).border_style(muted);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let width = usize::from(inner.width);
    let mut nodes = state.agents.nodes.values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        right
            .active()
            .cmp(&left.active())
            .then_with(|| {
                if left.active() {
                    left.created_at.cmp(&right.created_at)
                } else {
                    right.updated_at.cmp(&left.updated_at)
                }
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    let active = nodes.iter().filter(|node| node.active()).count();
    let heading = if state.agents.stale {
        "Agents · stale".into()
    } else {
        format!("Agents · {active} active")
    };
    let root_status = if matches!(state.overlay, Some(Overlay::Prompt { .. })) {
        "waiting"
    } else if state.is_busy() {
        "working"
    } else {
        "ready"
    };
    let mut lines = state.sidebar.lines(width, palette);
    let details = lines.split_off(lines.len().min(3));
    lines.extend([
        Line::from(Span::styled(heading, muted.add_modifier(Modifier::BOLD))),
        agent_line(
            "",
            "●",
            &sanitize_approval_field(&state.footer.role),
            root_status,
            width,
            ratatui_style(palette.assistant_style()),
            muted,
        ),
    ]);
    for (index, node) in nodes.iter().enumerate() {
        let (symbol, label, tone) = match node.status {
            SubagentStatus::Queued => ("○", "queued", PresentationTone::Neutral),
            SubagentStatus::Running => ("●", "working", PresentationTone::Tool),
            SubagentStatus::Completed => ("✓", "done", PresentationTone::Success),
            SubagentStatus::Failed => ("!", "failed", PresentationTone::Error),
            SubagentStatus::Cancelled => ("×", "cancelled", PresentationTone::Warning),
            SubagentStatus::Interrupted => ("!", "interrupted", PresentationTone::Warning),
        };
        let branch = if index + 1 == nodes.len() {
            "└─ "
        } else {
            "├─ "
        };
        lines.push(agent_line(
            branch,
            symbol,
            &node.label,
            label,
            width,
            ratatui_style(palette.tone_style(tone)),
            muted,
        ));
    }
    lines.extend(details);
    let overflow = lines.len() > usize::from(inner.height);
    let visible = usize::from(inner.height.saturating_sub(u16::from(overflow)));
    state.agents.scroll = state.agents.scroll.min(lines.len().saturating_sub(visible));
    let total = lines.len();
    let mut visible_lines = lines
        .into_iter()
        .skip(state.agents.scroll)
        .take(visible)
        .collect::<Vec<_>>();
    if overflow {
        visible_lines.push(Line::from(Span::styled(
            format!("{}/{} · Alt+PgUp/PgDn", state.agents.scroll + 1, total),
            muted,
        )));
    }
    frame.render_widget(Paragraph::new(visible_lines), inner);
}

pub(super) fn agent_line(
    prefix: &str,
    symbol: &str,
    name: &str,
    status: &str,
    width: usize,
    style: Style,
    muted: Style,
) -> Line<'static> {
    let available = width.saturating_sub(UnicodeWidthStr::width(prefix) + 2 + status.len() + 1);
    let name = truncate_width_with_ellipsis(name, available);
    let gap = " ".repeat(available.saturating_sub(UnicodeWidthStr::width(name.as_str())) + 1);
    Line::from(vec![
        Span::styled(prefix.to_owned(), muted),
        Span::styled(format!("{symbol} "), style),
        Span::styled(name, style),
        Span::raw(gap),
        Span::styled(status.to_owned(), style),
    ])
}

#[cfg(test)]
mod tests;
