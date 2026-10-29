use super::*;

/// The host chooses an exact execution identity separately from idle restoration.
pub(super) enum FooterUsageSelection<'a> {
    Latest,
    Exact(&'a str),
    Unavailable,
}

impl<'a> FooterUsageSelection<'a> {
    pub(super) fn run_id<'b>(&'b self, latest: Option<&'b str>) -> Option<&'b str> {
        match self {
            Self::Latest => latest,
            Self::Exact(run_id) => Some(*run_id),
            Self::Unavailable => None,
        }
    }
}

fn exact_usage_selection(run_id: Option<&str>) -> FooterUsageSelection<'_> {
    run_id.filter(|run_id| !run_id.is_empty()).map_or(
        FooterUsageSelection::Unavailable,
        FooterUsageSelection::Exact,
    )
}

pub(super) fn goal_result_usage_selection<'a>(
    result: &'a colossus_contracts::GoalRunResult,
    session_id: &str,
) -> FooterUsageSelection<'a> {
    if result.goal.session_id != session_id {
        return FooterUsageSelection::Unavailable;
    }
    exact_usage_selection(result.iterations.last().map(|run| run.run_id.as_str()))
}

pub(super) fn goal_outcome_usage_selection<'a>(
    outcome: &'a GoalRunOutcome,
    session_id: &str,
) -> FooterUsageSelection<'a> {
    match outcome {
        GoalRunOutcome::Completed { result } => goal_result_usage_selection(result, session_id),
        GoalRunOutcome::Cancelled {
            result,
            cancellation,
        } => {
            if result.goal.session_id != session_id {
                return FooterUsageSelection::Unavailable;
            }
            match cancellation.as_deref() {
                Some(run) if run.session_id == session_id => {
                    exact_usage_selection(Some(run.run_id.as_str()))
                }
                Some(_) => FooterUsageSelection::Unavailable,
                None => goal_result_usage_selection(result, session_id),
            }
        }
        GoalRunOutcome::Failed { result, run_id, .. } => {
            if result.goal.session_id != session_id {
                return FooterUsageSelection::Unavailable;
            }
            match run_id.as_deref() {
                Some(run_id) => exact_usage_selection(Some(run_id)),
                None => goal_result_usage_selection(result, session_id),
            }
        }
    }
}

pub(super) fn plan_usage_selection(outcome: &PlanExecutionOutcome) -> FooterUsageSelection<'_> {
    match outcome {
        PlanExecutionOutcome::CancelledBeforeStart { .. } => FooterUsageSelection::Unavailable,
        PlanExecutionOutcome::Direct { terminal, .. } => {
            exact_usage_selection(Some(match terminal {
                ControlledAgentTerminal::Completed { result } => result.run_id.as_str(),
                ControlledAgentTerminal::Cancelled { result } => result.run_id.as_str(),
                ControlledAgentTerminal::Failed { run_id, .. } => run_id.as_str(),
            }))
        }
        PlanExecutionOutcome::Goal { plan, terminal } => {
            goal_outcome_usage_selection(terminal, &plan.session_id)
        }
    }
}

pub(super) fn footer_status_document(footer: FooterState) -> PresentationDocument {
    let mut document = document_from_json(
        &json!({
            "role": footer.role,
            "route": footer.route,
            "context": footer.context,
            "message_count": footer.message_count,
            "status": footer.status,
            "approval_mode": footer.approval_mode,
        }),
        Some("Status"),
    );
    document
        .blocks
        .extend(crate::output_token_usage_document(footer.usage.as_ref()).blocks);
    document
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::ProviderUsage;
    use colossus_presentation::TerminalDocumentRenderer;

    #[test]
    fn status_exposes_counts_without_run_query_metadata() {
        let report = RunTokenUsage {
            run_id: "PRIVATE_RUN_ID".into(),
            session_id: "PRIVATE_SESSION_ID".into(),
            through_version: 123_456,
            reports: 2,
            usage: Some(ProviderUsage {
                input_tokens: 10,
                output_tokens: 3,
                total_tokens: 15,
                cached_input_tokens: Some(4),
                reasoning_tokens: None,
            }),
        };
        let document = footer_status_document(FooterState {
            usage: Some(report),
            ..FooterState::default()
        });
        let text = TerminalDocumentRenderer::new(TerminalPreferences::default(), 80)
            .with_color(false)
            .render(&document);
        assert!(text.contains("15 tokens"), "{text}");
        for hidden in [
            "PRIVATE_RUN_ID",
            "PRIVATE_SESSION_ID",
            "through_version",
            "123456",
        ] {
            assert!(!text.contains(hidden), "{text}");
        }
    }

    #[test]
    fn execution_selection_never_substitutes_the_sessions_latest_run() {
        assert_eq!(
            FooterUsageSelection::Exact("execution").run_id(Some("unrelated")),
            Some("execution")
        );
        assert_eq!(
            FooterUsageSelection::Unavailable.run_id(Some("unrelated")),
            None
        );
        assert_eq!(
            FooterUsageSelection::Latest.run_id(Some("latest")),
            Some("latest")
        );
    }

    fn goal_result(runs: &[&str]) -> colossus_contracts::GoalRunResult {
        use colossus_contracts::{GoalIterationResult, GoalRecord, GoalRunResult, GoalStatus};
        GoalRunResult {
            goal: GoalRecord {
                id: "goal".into(),
                session_id: "expected-session".into(),
                objective: "objective".into(),
                source_plan_id: None,
                status: GoalStatus::Active,
                summary: String::new(),
                blocked_reason: String::new(),
                iteration_budget: 5,
                iterations_completed: u16::try_from(runs.len()).expect("fixture iteration count"),
                created_at: String::new(),
                updated_at: String::new(),
            },
            iterations: runs
                .iter()
                .enumerate()
                .map(|(index, run)| GoalIterationResult {
                    iteration: u16::try_from(index + 1).expect("fixture iteration number"),
                    run_id: (*run).into(),
                    output: String::new(),
                    event_count: 1,
                    elapsed_seconds: 0.0,
                })
                .collect(),
            iteration_budget_exhausted: false,
            elapsed_seconds: 0.0,
        }
    }

    #[test]
    fn goal_result_selects_only_the_final_exact_iteration() {
        let empty = goal_result(&[]);
        assert_eq!(
            goal_result_usage_selection(&empty, "expected-session")
                .run_id(Some("unrelated-latest")),
            None
        );
        let result = goal_result(&["first", "final"]);
        assert_eq!(
            goal_result_usage_selection(&result, "expected-session")
                .run_id(Some("unrelated-latest")),
            Some("final")
        );
        assert_eq!(
            goal_result_usage_selection(&result, "another-session")
                .run_id(Some("unrelated-latest")),
            None
        );
        let empty_identity = goal_result(&["first", ""]);
        assert_eq!(
            goal_result_usage_selection(&empty_identity, "expected-session")
                .run_id(Some("unrelated-latest")),
            None
        );
    }

    #[test]
    fn controlled_goal_selection_uses_the_exact_terminal_run_and_session() {
        let completed = GoalRunOutcome::Completed {
            result: goal_result(&["first", "final"]),
        };
        assert_eq!(
            goal_outcome_usage_selection(&completed, "expected-session").run_id(Some("unrelated")),
            Some("final")
        );
        let cancellation = AgentRunCancellation {
            run_id: "partial-cancelled".into(),
            session_id: "expected-session".into(),
            turn: 1,
            plan: None,
            event_count: 4,
            elapsed_seconds: 0.1,
        };
        let cancelled = GoalRunOutcome::Cancelled {
            result: goal_result(&["completed-before-cancel"]),
            cancellation: Some(Box::new(cancellation.clone())),
        };
        assert_eq!(
            goal_outcome_usage_selection(&cancelled, "expected-session").run_id(Some("unrelated")),
            Some("partial-cancelled")
        );
        let foreign_cancelled = GoalRunOutcome::Cancelled {
            result: goal_result(&["completed-before-cancel"]),
            cancellation: Some(Box::new(AgentRunCancellation {
                session_id: "foreign-session".into(),
                ..cancellation
            })),
        };
        assert_eq!(
            goal_outcome_usage_selection(&foreign_cancelled, "expected-session")
                .run_id(Some("unrelated")),
            None
        );
        for (run_id, expected) in [
            (Some("partial-failed"), Some("partial-failed")),
            (None, Some("completed-before-failure")),
            (Some(""), None),
        ] {
            let failed = GoalRunOutcome::Failed {
                result: goal_result(&["completed-before-failure"]),
                run_id: run_id.map(str::to_owned),
                message: "failure".into(),
                outcome_unknown: false,
            };
            assert_eq!(
                goal_outcome_usage_selection(&failed, "expected-session").run_id(Some("unrelated")),
                expected
            );
            assert_eq!(
                goal_outcome_usage_selection(&failed, "foreign-session").run_id(Some("unrelated")),
                None
            );
        }
    }
}
