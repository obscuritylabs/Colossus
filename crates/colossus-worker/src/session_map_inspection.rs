use super::*;
use colossus_worker_protocol::{
    WorkerSessionDecision, WorkerSessionDelegate, WorkerSessionGoal, WorkerSessionMap,
    WorkerSessionMemory, WorkerSessionPlan, WorkerSessionResearchRun, WorkerSessionResearchSource,
    WorkerSessionTask,
};

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_RECORDS_PER_FAMILY: usize = 32;
const MAX_TITLE_BYTES: usize = 512;
const MAX_DETAIL_BYTES: usize = 8 * 1024;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024;
// The authenticated control frame base64-encodes its payload. Keeping the raw map
// below 2 MiB leaves ample room beneath the 4 MiB frame ceiling for that expansion,
// the signed envelope, and protocol metadata.
const MAX_SESSION_MAP_JSON_BYTES: usize = 2 * 1024 * 1024;
const TRUNCATION_MARKER: &str = "\n… truncated by Colossus Desktop";

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let content_limit = limit.saturating_sub(TRUNCATION_MARKER.len());
    let mut end = content_limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], TRUNCATION_MARKER)
}

fn task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn plan_status(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Draft => "draft",
        PlanStatus::Approved => "approved",
        PlanStatus::Executed => "executed",
        PlanStatus::Discarded => "discarded",
    }
}

fn goal_status(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Complete => "complete",
        GoalStatus::Blocked => "blocked",
    }
}

fn delegate_status(status: SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Queued => "queued",
        SubagentStatus::Running => "running",
        SubagentStatus::Completed => "completed",
        SubagentStatus::Failed => "failed",
        SubagentStatus::Cancelled => "cancelled",
        SubagentStatus::Interrupted => "interrupted",
    }
}

fn decision_status(status: DecisionStatus) -> &'static str {
    match status {
        DecisionStatus::Active => "active",
        DecisionStatus::Archived => "archived",
        DecisionStatus::Superseded => "superseded",
    }
}

fn decision_source(source: colossus_contracts::DecisionSource) -> &'static str {
    match source {
        colossus_contracts::DecisionSource::User => "user",
        colossus_contracts::DecisionSource::Agent => "agent",
    }
}

fn decision_priority(priority: DecisionPriority) -> &'static str {
    match priority {
        DecisionPriority::Critical => "critical",
        DecisionPriority::High => "high",
        DecisionPriority::Normal => "normal",
    }
}

fn memory_status(status: MemoryStatus) -> &'static str {
    match status {
        MemoryStatus::Active => "active",
        MemoryStatus::Archived => "archived",
        MemoryStatus::Superseded => "superseded",
    }
}

fn memory_scope(scope: &MemoryScope) -> &'static str {
    match scope {
        MemoryScope::Global => "global",
        MemoryScope::Repository(_) => "repository",
        MemoryScope::Session(_) => "session",
    }
}

fn memory_visible(scope: &MemoryScope, repository_id: &str, session_id: &str) -> bool {
    match scope {
        MemoryScope::Global => true,
        MemoryScope::Repository(id) => id == repository_id,
        MemoryScope::Session(id) => id == session_id,
    }
}

fn research_status(status: ResearchStatus) -> &'static str {
    match status {
        ResearchStatus::Running => "running",
        ResearchStatus::Completed => "completed",
        ResearchStatus::Failed => "failed",
        ResearchStatus::Interrupted => "interrupted",
    }
}

fn research_depth(depth: ResearchDepth) -> &'static str {
    match depth {
        ResearchDepth::Quick => "quick",
        ResearchDepth::Standard => "standard",
        ResearchDepth::Deep => "deep",
    }
}

fn source_kind(kind: ResearchSourceKind) -> &'static str {
    match kind {
        ResearchSourceKind::Repo => "repo",
        ResearchSourceKind::Web => "web",
        ResearchSourceKind::Mcp => "mcp",
    }
}

fn bound_session_map_payload(mut map: WorkerSessionMap) -> Result<WorkerSessionMap, WorkerError> {
    loop {
        let encoded = serde_json::to_vec(&map)
            .map_err(|_| WorkerError::Protocol("session map could not be encoded".into()))?;
        if encoded.len() <= MAX_SESSION_MAP_JSON_BYTES {
            return Ok(map);
        }

        let mut removed = false;
        macro_rules! pop_extra {
            ($records:expr) => {
                if $records.len() > 1 {
                    $records.pop();
                    removed = true;
                }
            };
        }
        pop_extra!(map.delegates);
        pop_extra!(map.goals);
        pop_extra!(map.tasks);
        pop_extra!(map.plans);
        pop_extra!(map.decisions);
        pop_extra!(map.memories);
        pop_extra!(map.research_runs);
        pop_extra!(map.research_sources);
        if !removed {
            return Err(WorkerError::Protocol(
                "session map exceeds its aggregate release budget".into(),
            ));
        }
    }
}

pub(super) async fn inspect_session_map(
    runtime: &Runtime,
    session_id: &str,
) -> Result<WorkerSessionMap, WorkerError> {
    if session_id.is_empty() || session_id.len() > MAX_IDENTIFIER_BYTES {
        return Err(WorkerError::Protocol(
            "session map identifier is invalid".into(),
        ));
    }
    runtime
        .get_session(session_id)?
        .ok_or_else(|| WorkerError::Protocol("session map was not found".into()))?;

    let work = runtime.work_repository();
    let delegates = work
        .list_subagents(Some(session_id), None, MAX_RECORDS_PER_FAMILY)?
        .into_iter()
        .map(|job| WorkerSessionDelegate {
            job_id: job.id,
            parent_run_id: job.parent_run_id,
            child_session_id: job.child_session_id,
            child_run_id: job.child_run_id,
            task: bounded_text(&job.task, MAX_DETAIL_BYTES),
            role: bounded_text(&job.role, MAX_TITLE_BYTES),
            status: delegate_status(job.status).into(),
            final_output: bounded_text(&job.final_output, MAX_DOCUMENT_BYTES),
            error: bounded_text(&job.error, MAX_DETAIL_BYTES),
            created_at: job.created_at,
            updated_at: job.updated_at,
            started_at: job.started_at,
            completed_at: job.completed_at,
        })
        .collect();
    let tasks = work
        .list_tasks(Some(session_id), None, MAX_RECORDS_PER_FAMILY)?
        .into_iter()
        .map(|task| WorkerSessionTask {
            id: task.id,
            title: bounded_text(&task.title, MAX_TITLE_BYTES),
            description: bounded_text(&task.description, MAX_DETAIL_BYTES),
            status: task_status(task.status).into(),
            created_at: task.created_at,
            updated_at: task.updated_at,
        })
        .collect();
    let plans = runtime
        .list_plans(Some(session_id), None, MAX_RECORDS_PER_FAMILY)?
        .into_iter()
        .map(|plan| WorkerSessionPlan {
            id: plan.id,
            prompt: bounded_text(&plan.prompt, MAX_DETAIL_BYTES),
            status: plan_status(plan.status).into(),
            revision: plan.revision,
            content: bounded_text(&plan.content, MAX_DOCUMENT_BYTES),
            step_count: plan.steps.len(),
            executed_run_id: plan.executed_run_id,
            created_at: plan.created_at,
            updated_at: plan.updated_at,
        })
        .collect();
    let goals = runtime
        .list_goals(Some(session_id), None, MAX_RECORDS_PER_FAMILY)?
        .into_iter()
        .map(|goal| WorkerSessionGoal {
            id: goal.id,
            objective: bounded_text(&goal.objective, MAX_DETAIL_BYTES),
            source_plan_id: goal.source_plan_id,
            status: goal_status(goal.status).into(),
            summary: bounded_text(&goal.summary, MAX_DETAIL_BYTES),
            blocked_reason: bounded_text(&goal.blocked_reason, MAX_DETAIL_BYTES),
            iteration_budget: goal.iteration_budget,
            iterations_completed: goal.iterations_completed,
            created_at: goal.created_at,
            updated_at: goal.updated_at,
        })
        .collect();
    let decisions = runtime
        .list_decisions(Some(session_id), None, MAX_RECORDS_PER_FAMILY)?
        .into_iter()
        .map(|decision| WorkerSessionDecision {
            id: decision.id,
            goal_id: decision.goal_id,
            plan_id: decision.plan_id,
            source: decision_source(decision.source).into(),
            status: decision_status(decision.status).into(),
            priority: decision_priority(decision.priority).into(),
            title: bounded_text(&decision.title, MAX_TITLE_BYTES),
            decision: bounded_text(&decision.decision, MAX_DETAIL_BYTES),
            intent: bounded_text(&decision.intent, MAX_DETAIL_BYTES),
            applies_when: bounded_text(&decision.applies_when, MAX_DETAIL_BYTES),
            rationale: bounded_text(&decision.rationale, MAX_DETAIL_BYTES),
            created_at: decision.created_at,
            updated_at: decision.updated_at,
        })
        .collect();
    let repository_id = runtime.repository_id();
    let memories = runtime
        .list_memories(Some(MemoryStatus::Active), MAX_RECORDS_PER_FAMILY * 4)
        .await?
        .into_iter()
        .filter(|record| memory_visible(&record.scope, repository_id, session_id))
        .take(MAX_RECORDS_PER_FAMILY)
        .map(|memory| WorkerSessionMemory {
            id: memory.id,
            scope: memory_scope(&memory.scope).into(),
            kind: bounded_text(&memory.kind, MAX_TITLE_BYTES),
            confidence: memory.confidence,
            source: bounded_text(&memory.source, MAX_TITLE_BYTES),
            status: memory_status(memory.status).into(),
            text: bounded_text(&memory.text, MAX_DETAIL_BYTES),
            rationale: bounded_text(&memory.rationale, MAX_DETAIL_BYTES),
            created_at: memory.created_at,
            updated_at: memory.updated_at,
            expires_at: memory.expires_at,
            superseded_by: memory.superseded_by,
        })
        .collect();

    let canonical_research =
        runtime.list_research_runs(Some(session_id), MAX_RECORDS_PER_FAMILY)?;
    let mut research_sources = Vec::new();
    let mut research_runs = Vec::with_capacity(canonical_research.len());
    for run in canonical_research {
        let sources = runtime.research_sources(&run.id)?;
        let source_count = sources.len();
        research_sources.extend(
            sources
                .into_iter()
                .take(MAX_RECORDS_PER_FAMILY)
                .map(|source| WorkerSessionResearchSource {
                    id: source.id,
                    run_id: source.run_id,
                    label: bounded_text(&source.label, MAX_TITLE_BYTES),
                    kind: source_kind(source.kind).into(),
                    title: bounded_text(&source.title, MAX_TITLE_BYTES),
                    uri: bounded_text(&source.uri, MAX_DETAIL_BYTES),
                    query: bounded_text(&source.query, MAX_DETAIL_BYTES),
                    created_at: source.created_at,
                }),
        );
        research_runs.push(WorkerSessionResearchRun {
            id: run.id,
            question: bounded_text(&run.question, MAX_DETAIL_BYTES),
            depth: research_depth(run.depth).into(),
            source_kinds: run
                .source_kinds
                .into_iter()
                .map(|kind| source_kind(kind).into())
                .collect(),
            status: research_status(run.status).into(),
            query_count: run.queries.len(),
            source_count,
            limitation_count: run.limitations.len(),
            report: bounded_text(&run.report, MAX_DOCUMENT_BYTES),
            error: bounded_text(&run.error, MAX_DETAIL_BYTES),
            created_at: run.created_at,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
        });
    }
    research_sources.truncate(MAX_RECORDS_PER_FAMILY);

    bound_session_map_payload(WorkerSessionMap {
        session_id: session_id.into(),
        delegates,
        goals,
        tasks,
        plans,
        decisions,
        memories,
        research_runs,
        research_sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_map_memory_visibility_rejects_other_repositories_and_sessions() {
        assert!(memory_visible(&MemoryScope::Global, "repo-a", "session-a"));
        assert!(memory_visible(
            &MemoryScope::Repository("repo-a".into()),
            "repo-a",
            "session-a",
        ));
        assert!(!memory_visible(
            &MemoryScope::Repository("repo-b".into()),
            "repo-a",
            "session-a",
        ));
        assert!(memory_visible(
            &MemoryScope::Session("session-a".into()),
            "repo-a",
            "session-a",
        ));
        assert!(!memory_visible(
            &MemoryScope::Session("session-b".into()),
            "repo-a",
            "session-a",
        ));
    }

    #[test]
    fn session_map_text_is_utf8_safe_and_bounded() {
        let bounded = bounded_text(&"界".repeat(64), 64);
        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(bounded.len() <= 64);
        assert!(bounded.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn session_map_enforces_an_aggregate_frame_safe_budget() {
        let detail = "d".repeat(MAX_DETAIL_BYTES);
        let document = "r".repeat(MAX_DOCUMENT_BYTES);
        let delegates = (0..MAX_RECORDS_PER_FAMILY)
            .map(|index| WorkerSessionDelegate {
                job_id: format!("job-{index}"),
                parent_run_id: "parent".into(),
                child_session_id: format!("child-{index}"),
                child_run_id: Some(format!("run-{index}")),
                task: detail.clone(),
                role: "role".into(),
                status: "completed".into(),
                final_output: document.clone(),
                error: detail.clone(),
                created_at: "2026-08-17T00:00:00Z".into(),
                updated_at: "2026-08-17T00:00:01Z".into(),
                started_at: None,
                completed_at: None,
            })
            .collect();
        let research_runs = (0..MAX_RECORDS_PER_FAMILY)
            .map(|index| WorkerSessionResearchRun {
                id: format!("research-{index}"),
                question: detail.clone(),
                depth: "deep".into(),
                source_kinds: Vec::new(),
                status: "completed".into(),
                query_count: 1,
                source_count: 1,
                limitation_count: 0,
                report: document.clone(),
                error: detail.clone(),
                created_at: "2026-08-17T00:00:00Z".into(),
                updated_at: "2026-08-17T00:00:01Z".into(),
                completed_at: None,
            })
            .collect();
        let oversized = WorkerSessionMap {
            session_id: "session".into(),
            delegates,
            goals: Vec::new(),
            tasks: Vec::new(),
            plans: Vec::new(),
            decisions: Vec::new(),
            memories: Vec::new(),
            research_runs,
            research_sources: Vec::new(),
        };
        assert!(
            serde_json::to_vec(&oversized).expect("map JSON").len() > MAX_SESSION_MAP_JSON_BYTES
        );

        let bounded = bound_session_map_payload(oversized).expect("bounded session map");
        assert!(
            serde_json::to_vec(&bounded).expect("bounded JSON").len() <= MAX_SESSION_MAP_JSON_BYTES
        );
        assert!(!bounded.delegates.is_empty());
        assert!(!bounded.research_runs.is_empty());
    }
}
