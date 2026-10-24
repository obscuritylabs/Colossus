//! Metadata-only work status and observed MCP activity for the right rail.

use super::*;
use crate::agents::{MIN_CHAT_WIDTH, PANEL_WIDTH};
use colossus_contracts::{GoalStatus, TaskStatus};

const MAX_MCP_SERVERS: usize = 64;

#[derive(Clone, Copy)]
enum McpActivity {
    Using,
    Used,
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(super) struct SidebarState {
    snapshot: Option<SidebarSnapshot>,
    stale: bool,
    activity: BTreeMap<String, McpActivity>,
    calls: BTreeMap<String, String>,
}

impl SidebarState {
    pub(super) fn loaded(&self) -> bool {
        self.snapshot.is_some() || self.stale
    }

    pub(super) fn refresh(&mut self, result: Result<Option<SidebarSnapshot>, String>) {
        let Ok(Some(mut snapshot)) = result else {
            self.stale = result.is_err();
            return;
        };
        self.stale = false;
        snapshot.tasks.truncate(5);
        snapshot.goals.truncate(3);
        snapshot.mcp_servers.truncate(MAX_MCP_SERVERS);
        for (name, _) in &mut snapshot.mcp_servers {
            *name = name.chars().take(256).collect();
        }
        for (label, _) in &mut snapshot.tasks {
            *label = safe_label(label);
        }
        for (label, _) in &mut snapshot.goals {
            *label = safe_label(label);
        }
        snapshot
            .mcp_servers
            .sort_by(|left, right| left.0.cmp(&right.0));
        self.activity
            .retain(|server, _| snapshot.mcp_servers.iter().any(|(name, _)| name == server));
        self.snapshot = Some(snapshot);
    }

    pub(super) fn observe(&mut self, event: &RunEvent) {
        match event {
            RunEvent::ToolStarted { call, .. }
                if matches!(call.name.as_str(), "mcp.call" | "mcp.tools") =>
            {
                if let Some(server) = call
                    .arguments
                    .get("server")
                    .and_then(|value| value.as_str())
                    && !server.is_empty()
                    && server.len() <= 1024
                    && call.call_id.len() <= 256
                    && self.calls.len() < MAX_MCP_SERVERS
                    && (self.activity.contains_key(server) || self.activity.len() < MAX_MCP_SERVERS)
                {
                    self.calls.insert(call.call_id.clone(), server.into());
                    self.activity.insert(server.into(), McpActivity::Using);
                }
            }
            RunEvent::ToolCompleted { result, .. } => {
                if let Some(server) = self.calls.remove(&result.call_id) {
                    let activity = if self.calls.values().any(|active| active == &server) {
                        McpActivity::Using
                    } else if result.exit_code == 0 {
                        McpActivity::Used
                    } else {
                        McpActivity::Failed
                    };
                    self.activity.insert(server, activity);
                }
            }
            RunEvent::ToolCancelled { call, .. } => {
                if let Some(server) = self.calls.remove(&call.call_id) {
                    self.activity.insert(server, McpActivity::Cancelled);
                }
            }
            RunEvent::Error { .. } => self.finish_calls(),
            _ => {}
        }
    }

    pub(super) fn finish_calls(&mut self) {
        for (_, server) in std::mem::take(&mut self.calls) {
            self.activity.insert(server, McpActivity::Failed);
        }
    }

    pub(super) fn lines(&self, width: usize, palette: TerminalPalette) -> Vec<Line<'static>> {
        let Some(snapshot) = &self.snapshot else {
            return if self.stale {
                vec![Line::styled(
                    "Status unavailable",
                    ratatui_style(palette.meta_style()),
                )]
            } else {
                Vec::new()
            };
        };
        let muted = ratatui_style(palette.meta_style());
        let heading = |text: &str| {
            Line::from(Span::styled(
                text.to_owned(),
                muted.add_modifier(Modifier::BOLD),
            ))
        };
        let memory = snapshot.memory_count.map_or("—".into(), |(count, capped)| {
            format!("{count}{}", if capped { "+" } else { "" })
        });
        let mut lines = vec![
            heading(if self.stale {
                "Status · stale"
            } else {
                "Status"
            }),
            Line::styled(
                format!(
                    "Tasks {} · Goals {}",
                    snapshot.open_task_count, snapshot.goal_count
                ),
                muted,
            ),
            Line::styled(
                format!("Memory {memory} · Plans {}", snapshot.plan_count),
                muted,
            ),
        ];
        let row = |symbol, name: &str, status, tone| {
            crate::agents::agent_line(
                "",
                symbol,
                name,
                status,
                width,
                ratatui_style(palette.tone_style(tone)),
                muted,
            )
        };
        if !snapshot.tasks.is_empty() {
            lines.extend([Line::default(), heading("Open tasks")]);
            for (name, status) in &snapshot.tasks {
                let (symbol, label, tone) = match status {
                    TaskStatus::Pending => ("○", "pending", PresentationTone::Neutral),
                    TaskStatus::InProgress => ("●", "working", PresentationTone::Tool),
                    TaskStatus::Blocked => ("!", "blocked", PresentationTone::Warning),
                    TaskStatus::Completed | TaskStatus::Cancelled => continue,
                };
                lines.push(row(symbol, name, label, tone));
            }
            if snapshot.open_task_count > snapshot.tasks.len() {
                lines.push(Line::styled(
                    format!(
                        "… +{} · /tasks",
                        snapshot.open_task_count - snapshot.tasks.len()
                    ),
                    muted,
                ));
            }
        }
        if !snapshot.goals.is_empty() {
            lines.extend([Line::default(), heading("Goals")]);
            for (name, status) in &snapshot.goals {
                let (symbol, label, tone) = match status {
                    GoalStatus::Active => ("●", "active", PresentationTone::Tool),
                    GoalStatus::Blocked => ("!", "blocked", PresentationTone::Warning),
                    GoalStatus::Complete => continue,
                };
                lines.push(row(symbol, name, label, tone));
            }
            if snapshot.goal_count > snapshot.goals.len() {
                lines.push(Line::styled(
                    format!("… +{} · /goals", snapshot.goal_count - snapshot.goals.len()),
                    muted,
                ));
            }
        }
        lines.extend([
            Line::default(),
            heading(&format!("MCP · {} configured", snapshot.mcp_servers.len())),
        ]);
        for (server, available) in &snapshot.mcp_servers {
            let (symbol, label, tone) = if !available {
                ("×", "unavailable", PresentationTone::Warning)
            } else {
                match self.activity.get(server) {
                    Some(McpActivity::Using) => ("◉", "in use", PresentationTone::Tool),
                    Some(McpActivity::Used) => ("●", "used", PresentationTone::Success),
                    Some(McpActivity::Failed) => ("!", "error", PresentationTone::Error),
                    Some(McpActivity::Cancelled) => ("×", "cancelled", PresentationTone::Warning),
                    None => ("○", "configured", PresentationTone::Neutral),
                }
            };
            lines.push(row(symbol, &safe_label(server), label, tone));
        }
        lines.push(Line::default());
        lines
    }
}

fn safe_label(label: &str) -> String {
    sanitize_approval_field(&label.chars().take(256).collect::<String>())
}

impl TuiState {
    pub(super) fn sidebar_width(&self, width: u16) -> u16 {
        if self.sidebar.loaded() && width >= MIN_CHAT_WIDTH + PANEL_WIDTH {
            PANEL_WIDTH
        } else {
            self.agents.width(width)
        }
    }

    pub(super) fn sidebar_inline_height(&self, width: u16) -> u16 {
        if self.sidebar.loaded() && self.sidebar_width(width) > 0 {
            12
        } else {
            self.agents.inline_height(width)
        }
    }
}

#[cfg(test)]
mod tests;
