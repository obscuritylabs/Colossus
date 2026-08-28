use serde::{Deserialize, Serialize};

/// One bounded child-agent record released for a selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionDelegate {
    /// Durable child-agent job identifier.
    pub job_id: String,
    /// Public parent run that requested the child.
    pub parent_run_id: String,
    /// Isolated child session identifier.
    pub child_session_id: String,
    /// Child execution identifier when execution started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_run_id: Option<String>,
    /// Bounded delegated objective.
    pub task: String,
    /// Configured model role.
    pub role: String,
    /// Released child lifecycle state.
    pub status: String,
    /// Bounded released terminal output.
    pub final_output: String,
    /// Bounded released terminal error.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC start timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// UTC terminal timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// One canonical session-scoped task released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionTask {
    /// Durable task identifier.
    pub id: String,
    /// Bounded task title.
    pub title: String,
    /// Bounded task description.
    pub description: String,
    /// Canonical task lifecycle state.
    pub status: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One canonical durable plan released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionPlan {
    /// Durable plan identifier.
    pub id: String,
    /// Bounded prompt that produced the plan.
    pub prompt: String,
    /// Canonical plan lifecycle state.
    pub status: String,
    /// Current plan revision.
    pub revision: u64,
    /// Bounded durable plan content.
    pub content: String,
    /// Number of canonical plan steps.
    pub step_count: usize,
    /// Run that executed the plan when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_run_id: Option<String>,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One bounded-autonomy goal released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionGoal {
    /// Durable goal identifier.
    pub id: String,
    /// Bounded goal objective.
    pub objective: String,
    /// Source plan identifier when the goal came from a plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plan_id: Option<String>,
    /// Canonical goal lifecycle state.
    pub status: String,
    /// Bounded released progress summary.
    pub summary: String,
    /// Bounded blocking reason when blocked.
    pub blocked_reason: String,
    /// Maximum bounded-autonomy iterations.
    pub iteration_budget: u16,
    /// Completed bounded-autonomy iterations.
    pub iterations_completed: u16,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One canonical key decision released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionDecision {
    /// Durable decision identifier.
    pub id: String,
    /// Owning goal identifier when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// Owning plan identifier when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Provenance category for the decision.
    pub source: String,
    /// Canonical decision lifecycle state.
    pub status: String,
    /// Canonical decision priority.
    pub priority: String,
    /// Bounded decision title.
    pub title: String,
    /// Bounded decision commitment.
    pub decision: String,
    /// Bounded decision intent.
    pub intent: String,
    /// Bounded applicability rule.
    pub applies_when: String,
    /// Bounded decision rationale.
    pub rationale: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
}

/// One policy-released memory visible to the selected session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionMemory {
    /// Durable memory identifier.
    pub id: String,
    /// Released scope category without unrestricted identifiers.
    pub scope: String,
    /// Bounded memory kind.
    pub kind: String,
    /// Canonical confidence in the inclusive range zero to one.
    pub confidence: f32,
    /// Bounded provenance label.
    pub source: String,
    /// Canonical memory lifecycle state.
    pub status: String,
    /// Bounded policy-released memory text.
    pub text: String,
    /// Bounded memory rationale.
    pub rationale: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC expiry timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Replacement memory identifier when superseded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// One immutable context snapshot released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionContextSnapshot {
    /// Durable snapshot identifier.
    pub id: String,
    /// First canonical message represented by the snapshot.
    pub source_start_sequence: u64,
    /// Last canonical message represented by the snapshot.
    pub source_end_sequence: u64,
    /// Bounded future-context summary.
    pub summary: String,
    /// Bounded durable requirements and facts.
    pub pinned_facts: Vec<String>,
    /// Bounded unfinished user requests.
    pub open_tasks: Vec<String>,
    /// Bounded workspace paths observed in released tool results.
    pub files_touched: Vec<String>,
    /// Bounded notable released tool outcomes.
    pub notable_tool_results: Vec<String>,
    /// Canonical compaction strategy.
    pub strategy: String,
    /// UTC creation timestamp.
    pub created_at: String,
}

/// One canonical research run released for the selected session map.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionResearchRun {
    /// Durable research-run identifier.
    pub id: String,
    /// Bounded research question.
    pub question: String,
    /// Canonical research depth.
    pub depth: String,
    /// Allowed source-kind categories.
    pub source_kinds: Vec<String>,
    /// Canonical research lifecycle state.
    pub status: String,
    /// Number of canonical search queries.
    pub query_count: usize,
    /// Number of canonical released sources.
    pub source_count: usize,
    /// Number of recorded research limitations.
    pub limitation_count: usize,
    /// Bounded released research report.
    pub report: String,
    /// Bounded terminal research error.
    pub error: String,
    /// UTC creation timestamp.
    pub created_at: String,
    /// UTC last-update timestamp.
    pub updated_at: String,
    /// UTC terminal timestamp when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// One canonical source record released for a selected research run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionResearchSource {
    /// Durable research-source identifier.
    pub id: String,
    /// Owning research-run identifier.
    pub run_id: String,
    /// Bounded citation label.
    pub label: String,
    /// Canonical source kind.
    pub kind: String,
    /// Bounded source title.
    pub title: String,
    /// Bounded released source locator.
    pub uri: String,
    /// Bounded source query.
    pub query: String,
    /// UTC creation timestamp.
    pub created_at: String,
}

/// Bounded canonical resources available to the selected Desktop session map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerSessionMap {
    /// Exact selected canonical session identifier.
    pub session_id: String,
    /// Bounded child-agent records.
    pub delegates: Vec<WorkerSessionDelegate>,
    /// Bounded canonical goals.
    pub goals: Vec<WorkerSessionGoal>,
    /// Bounded canonical tasks.
    pub tasks: Vec<WorkerSessionTask>,
    /// Bounded durable plans.
    pub plans: Vec<WorkerSessionPlan>,
    /// Bounded key decisions.
    pub decisions: Vec<WorkerSessionDecision>,
    /// Bounded policy-released memories.
    pub memories: Vec<WorkerSessionMemory>,
    /// Bounded immutable context snapshots.
    pub context_snapshots: Vec<WorkerSessionContextSnapshot>,
    /// Bounded canonical research runs.
    pub research_runs: Vec<WorkerSessionResearchRun>,
    /// Bounded released research sources.
    pub research_sources: Vec<WorkerSessionResearchSource>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_snapshot_uses_the_desktop_camel_case_contract() {
        let snapshot = WorkerSessionContextSnapshot {
            id: "snapshot-1".into(),
            source_start_sequence: 3,
            source_end_sequence: 17,
            summary: "Bounded summary".into(),
            pinned_facts: vec!["Pinned".into()],
            open_tasks: vec!["Finish".into()],
            files_touched: vec!["src/lib.rs".into()],
            notable_tool_results: vec!["Tests passed".into()],
            strategy: "hybrid_model".into(),
            created_at: "2026-08-27T23:00:00Z".into(),
        };

        assert_eq!(
            serde_json::to_value(snapshot).expect("snapshot JSON"),
            serde_json::json!({
                "id": "snapshot-1",
                "sourceStartSequence": 3,
                "sourceEndSequence": 17,
                "summary": "Bounded summary",
                "pinnedFacts": ["Pinned"],
                "openTasks": ["Finish"],
                "filesTouched": ["src/lib.rs"],
                "notableToolResults": ["Tests passed"],
                "strategy": "hybrid_model",
                "createdAt": "2026-08-27T23:00:00Z",
            })
        );
    }
}
