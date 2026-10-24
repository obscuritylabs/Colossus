use super::*;

pub(super) fn sidebar_snapshot(
    work: WorkStateSnapshot,
    memory_count: Option<(usize, bool)>,
    servers: Vec<(String, bool)>,
) -> colossus_tui::SidebarSnapshot {
    use colossus_contracts::TaskStatus;
    let mut tasks = work
        .tasks
        .into_iter()
        .filter(|task| !matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled))
        .collect::<Vec<_>>();
    tasks.sort_by_key(|task| match task.status {
        TaskStatus::InProgress => 0,
        TaskStatus::Blocked => 1,
        _ => 2,
    });
    colossus_tui::SidebarSnapshot {
        tasks: tasks
            .into_iter()
            .take(5)
            .map(|task| (task.title, task.status))
            .collect(),
        open_task_count: work.open_task_count,
        goal_count: work.current_goals.len(),
        goals: work
            .current_goals
            .into_iter()
            .take(3)
            .map(|goal| (goal.objective, goal.status))
            .collect(),
        memory_count,
        plan_count: work.actionable_plans.len(),
        mcp_servers: servers.into_iter().take(64).collect(),
    }
}

pub(super) fn mcp_servers(value: Value) -> Result<Vec<(String, bool)>, String> {
    value
        .as_array()
        .ok_or("invalid MCP server metadata")?
        .iter()
        .take(64)
        .map(|server| {
            Ok((
                server["name"]
                    .as_str()
                    .ok_or("missing MCP server name")?
                    .into(),
                server["available"]
                    .as_bool()
                    .ok_or("missing MCP server availability")?,
            ))
        })
        .collect()
}
