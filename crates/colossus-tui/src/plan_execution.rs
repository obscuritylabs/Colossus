use super::*;
use crate::contract::DEFAULT_GOAL_ITERATIONS;

pub(super) fn plan_execution_dock_height(
    state: &TuiState,
    total_height: u16,
    composer_height: u16,
    activity_height: u16,
) -> u16 {
    if !state.plan_execution_decision_active() {
        return 0;
    }
    let available = total_height
        .saturating_sub(MINIMUM_APPROVAL_TRANSCRIPT_ROWS)
        .saturating_sub(activity_height)
        .saturating_sub(composer_height)
        .saturating_sub(1);
    if available < MIN_PLAN_EXECUTION_DOCK_ROWS {
        return 0;
    }
    available.min(MAX_PLAN_EXECUTION_DOCK_ROWS)
}

pub(super) fn render_plan_execution_dock(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let Some(Overlay::PlanExecutionChoice { plan, selected }) = state.overlay.as_ref() else {
        return;
    };
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let title = selected.map_or_else(
        || {
            format!(
                " Execute plan {} · choose strategy ",
                short_plan_id(&plan.id)
            )
        },
        |selected| {
            format!(
                " Execute plan {} · strategy {}/2 ",
                short_plan_id(&plan.id),
                selected + 1
            )
        },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, ratatui_style(palette.warning_style())));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    render_plan_context(frame, plan, &palette, rows[0]);
    render_strategy(
        frame,
        "D",
        "Direct",
        "One model run; ordinary effect policy, approval, sandbox, and audit controls remain active.",
        *selected == Some(0),
        &palette,
        rows[1],
    );
    render_strategy(
        frame,
        "G",
        "Goal Mode",
        &format!(
            "Up to {DEFAULT_GOAL_ITERATIONS} autonomous iterations; the same effect controls apply to every iteration."
        ),
        *selected == Some(1),
        &palette,
        rows[2],
    );
    let decision = selected.map_or_else(
        || "No strategy selected".into(),
        |selected| {
            format!(
                "Selected: {} · Enter starts execution",
                if selected == 0 { "Direct" } else { "Goal Mode" }
            )
        },
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width(&decision, usize::from(rows[3].width)),
            ratatui_style(palette.warning_style()),
        )),
        rows[3],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width(
                "Esc cancel · ↑/↓ choose · D/G select · Enter confirm",
                usize::from(rows[4].width),
            ),
            ratatui_style(palette.warning_style()),
        )),
        rows[4],
    );
}

fn render_plan_context(
    frame: &mut Frame<'_>,
    plan: &PlanRecord,
    palette: &TerminalPalette,
    area: Rect,
) {
    let mutating = plan
        .steps
        .iter()
        .filter(|step| step.requires_mutation)
        .count();
    let summary = format!(
        "Revision r{} · {} step{} · {} mutating",
        plan.revision,
        plan.steps.len(),
        if plan.steps.len() == 1 { "" } else { "s" },
        mutating,
    );
    let prompt = sanitize_approval_field(&plan.prompt);
    let prompt_width = usize::from(area.width.saturating_sub(6)).max(1);
    let prompt = wrap_approval_value(&prompt, prompt_width)
        .into_iter()
        .next()
        .unwrap_or_else(|| "Untitled plan".into());
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Plan  ",
                ratatui_style(palette.meta_style()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_width(&prompt, prompt_width),
                ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Scope ",
                ratatui_style(palette.meta_style()).add_modifier(Modifier::BOLD),
            ),
            Span::styled(summary, ratatui_style(palette.meta_style())),
        ]),
    ];
    if area.height >= 3 {
        lines.push(Line::from(Span::styled(
            "Choose how the approved revision should be consumed. No strategy is preselected.",
            ratatui_style(palette.meta_style()),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

#[allow(clippy::too_many_arguments)]
fn render_strategy(
    frame: &mut Frame<'_>,
    shortcut: &str,
    label: &str,
    description: &str,
    selected: bool,
    palette: &TerminalPalette,
    area: Rect,
) {
    if area.height == 0 {
        return;
    }
    let style = filled_approval_control_style(
        if selected {
            palette.warning_style()
        } else {
            palette.meta_style()
        },
        selected,
    );
    let marker = if selected { "›" } else { " " };
    let title = format!("{marker} [{shortcut}] {label}");
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_width(&title, usize::from(area.width)),
            style.add_modifier(Modifier::BOLD),
        )),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.height > 1 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_width(description, usize::from(area.width.saturating_sub(4))),
                ratatui_style(palette.meta_style()),
            )),
            Rect::new(
                area.x.saturating_add(4),
                area.y.saturating_add(1),
                area.width.saturating_sub(4),
                1,
            ),
        );
    }
}
