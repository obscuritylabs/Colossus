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

pub(super) fn plan_usage_selection(outcome: &PlanExecutionOutcome) -> FooterUsageSelection<'_> {
    let run_id = match outcome {
        PlanExecutionOutcome::CancelledBeforeStart { .. } => None,
        PlanExecutionOutcome::Direct { terminal, .. } => match terminal {
            ControlledAgentTerminal::Completed { result } => Some(result.run_id.as_str()),
            ControlledAgentTerminal::Cancelled { result } => Some(result.run_id.as_str()),
            ControlledAgentTerminal::Failed { run_id, .. } => Some(run_id.as_str()),
        },
        PlanExecutionOutcome::Goal { terminal, .. } => match terminal {
            GoalRunOutcome::Completed { result } => {
                result.iterations.last().map(|run| run.run_id.as_str())
            }
            GoalRunOutcome::Cancelled {
                result,
                cancellation,
            } => cancellation
                .as_deref()
                .map(|run| run.run_id.as_str())
                .or_else(|| result.iterations.last().map(|run| run.run_id.as_str())),
            GoalRunOutcome::Failed { result, run_id, .. } => run_id
                .as_deref()
                .or_else(|| result.iterations.last().map(|run| run.run_id.as_str())),
        },
    };
    run_id.map_or(
        FooterUsageSelection::Unavailable,
        FooterUsageSelection::Exact,
    )
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
}
