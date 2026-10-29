use super::*;
use colossus_presentation::{
    ToolActivityPhase, ToolActivityPresentation, tool_activity_presentation,
};

const MAX_TOOL_CONTEXTS: usize = 64;
const MAX_CONTEXT_ID_BYTES: usize = 256;
type ContextKey = (String, String, String);

#[derive(Default)]
pub(super) struct ToolActivityContexts {
    entries: BTreeMap<ContextKey, ToolActivityPresentation>,
}

pub(super) struct ContextUpdate {
    key: ContextKey,
    started: bool,
    cancelled: bool,
    duration: Option<f64>,
}

impl ToolActivityContexts {
    pub(super) fn prepare(envelope: &RunEventEnvelope) -> Option<ContextUpdate> {
        let (call_id, started, cancelled, duration) = match &envelope.event {
            RunEvent::ToolStarted { call, .. } => (&call.call_id, true, false, None),
            RunEvent::ToolCompleted {
                result,
                duration_seconds,
                ..
            } => (&result.call_id, false, false, Some(*duration_seconds)),
            RunEvent::ToolCancelled { call, .. } => (&call.call_id, false, true, None),
            _ => return None,
        };
        if [&envelope.run_id, &envelope.session_id, call_id]
            .iter()
            .any(|id| id.len() > MAX_CONTEXT_ID_BYTES)
        {
            return None;
        }
        Some(ContextUpdate {
            key: (
                envelope.run_id.clone(),
                envelope.session_id.clone(),
                call_id.clone(),
            ),
            started,
            cancelled,
            duration,
        })
    }

    pub(super) fn apply(&mut self, source: Option<ContextUpdate>, update: &mut RunUpdateKind) {
        let Some(source) = source else {
            return;
        };
        let RunUpdateKind::ToolActivity { activity } = update else {
            return;
        };
        let mut display = presentation(activity);
        if source.started {
            if display.context.is_some()
                && (self.entries.len() < MAX_TOOL_CONTEXTS
                    || self.entries.contains_key(&source.key))
            {
                self.entries.insert(source.key, display.clone());
            }
        } else if let Some(previous) = self.entries.remove(&source.key)
            && !source.cancelled
            && previous.tool_name == display.tool_name
        {
            if previous.context.is_some() {
                display.context = previous.context;
            }
            let current = std::mem::take(&mut display.facts);
            for (label, value) in previous.facts.into_iter().chain(current) {
                if !display.facts.iter().any(|(known, _)| known == &label) {
                    display.add_fact(&label, value);
                }
            }
        }
        if let Some(duration) = source.duration {
            display.add_fact("Duration", format!("{duration:.2}s"));
        }
        activity.summary = display.summary();
    }
}

pub(super) fn presentation(activity: &ToolActivity) -> ToolActivityPresentation {
    let phase = match activity.state {
        ToolActivityState::Requested => ToolActivityPhase::Requested,
        ToolActivityState::WaitingApproval => ToolActivityPhase::WaitingApproval,
        ToolActivityState::Started => ToolActivityPhase::Started,
        ToolActivityState::Completed => ToolActivityPhase::Completed,
        ToolActivityState::Cancelled => ToolActivityPhase::Cancelled,
        ToolActivityState::Failed => ToolActivityPhase::Failed,
        ToolActivityState::OutcomeUnknown => ToolActivityPhase::OutcomeUnknown,
    };
    // Use only the existing public input/preview projection. Raw command arguments,
    // denied output and private native errors never enter the shared presenter.
    let input = activity
        .input
        .as_deref()
        .filter(|input| input.len() <= colossus_presentation::TOOL_ACTIVITY_DETAIL_BYTES)
        .and_then(|input| serde_json::from_str(input).ok());
    tool_activity_presentation(
        &activity.tool_name,
        phase,
        input.as_ref(),
        activity.preview.as_deref(),
    )
}

/// Normalize only the summary after the existing public history release projection.
pub(super) fn historical_summary(activity: &SessionActivity) -> Option<String> {
    if activity.kind != SessionActivityKind::Tool {
        return None;
    }
    let phase = match activity.status {
        Some(SessionActivityStatus::Requested) => ToolActivityPhase::Requested,
        Some(SessionActivityStatus::Running) => ToolActivityPhase::Started,
        Some(SessionActivityStatus::Waiting) => ToolActivityPhase::WaitingApproval,
        Some(SessionActivityStatus::Completed) => ToolActivityPhase::Completed,
        Some(SessionActivityStatus::Failed) => ToolActivityPhase::Failed,
        Some(SessionActivityStatus::Cancelled) => ToolActivityPhase::Cancelled,
        Some(SessionActivityStatus::OutcomeUnknown) => ToolActivityPhase::OutcomeUnknown,
        None => ToolActivityPhase::Recorded,
    };
    let input = activity
        .input
        .as_ref()
        .filter(|_| {
            matches!(
                phase,
                ToolActivityPhase::Started
                    | ToolActivityPhase::Completed
                    | ToolActivityPhase::Failed
            )
        })
        .filter(|input| input.value.len() <= colossus_presentation::TOOL_ACTIVITY_DETAIL_BYTES)
        .and_then(|input| serde_json::from_str(&input.value).ok());
    let output = activity
        .result
        .as_ref()
        .filter(|_| {
            matches!(
                phase,
                ToolActivityPhase::Completed
                    | ToolActivityPhase::Failed
                    | ToolActivityPhase::Recorded
            )
        })
        .map(|output| output.value.as_str());
    let mut display = tool_activity_presentation(&activity.title, phase, input.as_ref(), output);
    if let Some(milliseconds) = activity.duration_ms {
        display.add_fact("Duration", format!("{:.2}s", milliseconds as f64 / 1000.0));
    }
    Some(display.summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(contexts: &mut ToolActivityContexts, run: &str, event: RunEvent) -> ToolActivity {
        let envelope = RunEventEnvelope {
            schema_version: 1,
            run_id: run.into(),
            session_id: format!("session-{run}"),
            event,
        };
        let source = ToolActivityContexts::prepare(&envelope);
        let mut update = public_event(envelope.event);
        contexts.apply(source, &mut update);
        let RunUpdateKind::ToolActivity { activity } = update else {
            panic!("tool activity")
        };
        activity
    }
    fn start(path: &str) -> RunEvent {
        RunEvent::ToolStarted {
            turn: 1,
            call: ToolCall {
                call_id: "same-call".into(),
                name: "filesystem.read".into(),
                arguments: json!({"path":path,"line_start":4,"line_end":8}),
            },
            elapsed_seconds: 0.0,
        }
    }
    fn complete(name: &str, output: &str) -> RunEvent {
        RunEvent::ToolCompleted {
            turn: 1,
            result: ToolResult {
                call_id: "same-call".into(),
                name: name.into(),
                output: output.into(),
                exit_code: 0,
                images: Vec::new(),
            },
            duration_seconds: 0.2,
            elapsed_seconds: 0.3,
        }
    }
    #[test]
    fn completed_human_rows_keep_only_bounded_matching_run_context() {
        let mut contexts = ToolActivityContexts::default();
        project(&mut contexts, "alpha", start("alpha.rs"));
        project(&mut contexts, "beta", start("beta.rs"));
        let beta = project(
            &mut contexts,
            "beta",
            complete("filesystem.read", "{\"bytes\":42}"),
        );
        assert_eq!(
            beta.summary,
            "Read file · beta.rs · Lines: 4–8 · Duration: 0.20s · completed"
        );
        let alpha = project(
            &mut contexts,
            "alpha",
            complete("filesystem.read", "raw text"),
        );
        assert!(alpha.summary.contains("alpha.rs"));
        assert!(!alpha.summary.contains("beta.rs"));
        assert!(contexts.entries.is_empty());
    }
    #[test]
    fn canonical_repository_summary_matches_native_presenter_without_raw_body() {
        let output=json!({"root":"src","file_count":200,"truncated":true,"files":[{"path":"PRIVATE_BODY"}]}).to_string();
        let activity = project(
            &mut ToolActivityContexts::default(),
            "run",
            complete("repo.map", &output),
        );
        let mut expected = tool_activity_presentation(
            "repo.map",
            ToolActivityPhase::Completed,
            None,
            Some(&output),
        );
        expected.add_fact("Duration", "0.20s");
        assert_eq!(activity.summary, expected.summary());
        assert!(!activity.summary.contains("PRIVATE_BODY"));
    }
    #[test]
    fn requested_cancelled_and_failed_shell_rows_keep_the_existing_release_fences() {
        let mut contexts = ToolActivityContexts::default();
        let requested = project(
            &mut contexts,
            "run",
            RunEvent::Provider {
                event: ProviderEvent::ToolCallRequested {
                    call_id: "same-call".into(),
                    name: "filesystem.read".into(),
                    arguments: json!({"path":"PRIVATE_PATH"}),
                },
            },
        );
        assert!(!requested.summary.contains("PRIVATE"));
        let cancelled = project(
            &mut contexts,
            "run",
            RunEvent::ToolCancelled {
                turn: 1,
                call: ToolCall {
                    call_id: "same-call".into(),
                    name: "mcp.call".into(),
                    arguments: json!({"server":"PRIVATE_SERVER","tool":"PRIVATE_TOOL"}),
                },
                elapsed_seconds: 0.0,
            },
        );
        assert!(!cancelled.summary.contains("PRIVATE"));
        let mut failure = complete(
            "shell.run",
            "{\"error\":{\"code\":\"tool.denied\",\"message\":\"PRIVATE_COMMAND\"}}",
        );
        if let RunEvent::ToolCompleted { result, .. } = &mut failure {
            result.exit_code = 1;
        }
        let failure = project(&mut contexts, "run", failure);
        assert!(!failure.summary.contains("PRIVATE"));
        let preview = failure
            .preview
            .expect("existing categorical denial projection");
        assert!(!preview.contains("PRIVATE"));
        let display: serde_json::Value = serde_json::from_str(&preview).expect("categorical JSON");
        assert_eq!(
            display
                .pointer("/error/code")
                .and_then(serde_json::Value::as_str),
            Some("tool.denied")
        );
    }
    #[test]
    fn released_start_context_wins_over_result_supplied_identity() {
        let mut contexts = ToolActivityContexts::default();
        project(
            &mut contexts,
            "run",
            RunEvent::ToolStarted {
                turn: 1,
                call: ToolCall {
                    call_id: "same-call".into(),
                    name: "mcp.call".into(),
                    arguments: json!({"server":"Trusted","tool":"list"}),
                },
                elapsed_seconds: 0.0,
            },
        );
        let result = project(
            &mut contexts,
            "run",
            complete("mcp.call", r#"{"server":"Spoofed","tool":"wrong"}"#),
        );
        assert!(result.summary.contains("Trusted · list"));
        assert!(!result.summary.contains("Spoofed"));
    }
    #[test]
    fn context_memory_is_capped_and_completion_releases_its_slot() {
        let mut contexts = ToolActivityContexts::default();
        for index in 0..MAX_TOOL_CONTEXTS + 8 {
            project(&mut contexts, &format!("run-{index}"), start("file.rs"));
        }
        assert_eq!(contexts.entries.len(), MAX_TOOL_CONTEXTS);
        project(&mut contexts, "run-0", complete("filesystem.read", "text"));
        assert_eq!(contexts.entries.len(), MAX_TOOL_CONTEXTS - 1);
        project(&mut contexts, "another", start("another.rs"));
        assert_eq!(contexts.entries.len(), MAX_TOOL_CONTEXTS);
    }
    fn historical(name: &str, status: &str, input: &str, output: &str) -> ProjectedSessionActivity {
        serde_json::from_value(json!({"activity_id":"activity","session_id":"session","run_id":"run","turn":1,"lane":"tools","kind":"tool","title":name,"summary":"legacy raw tool summary","actor":"tool","status":status,"started_at":"2026-09-09T00:00:00Z","completed_at":null,"duration_ms":200,"input":{"format":"json","value":input},"result":{"format":"text","value":output},"attributes":{},"source_event_types":[],"first_sequence":1,"last_sequence":2})).expect("projected history")
    }
    #[test]
    fn historical_tool_rows_share_human_metadata_without_changing_identity_or_released_content() {
        let output=json!({"root":"src","file_count":200,"truncated":true,"files":[{"path":"BODY_MARKER"}]}).to_string();
        let source = historical("repo.map", "completed", r#"{"path":"src"}"#, &output);
        let activity = public_activity(&source).expect("public history");
        assert_eq!(activity.title, "repo.map");
        assert_eq!(activity.status, Some(SessionActivityStatus::Completed));
        let mut expected = tool_activity_presentation(
            "repo.map",
            ToolActivityPhase::Completed,
            Some(&json!({"path":"src"})),
            Some(&output),
        );
        expected.add_fact("Duration", "0.20s");
        assert_eq!(activity.summary, expected.summary());
        assert!(!activity.summary.contains("BODY_MARKER"));
        assert_eq!(activity.result.expect("released result").value, output);
        assert_eq!(source.summary, "legacy raw tool summary");
    }
    #[test]
    fn historical_summaries_keep_unstarted_shell_and_computer_disclosure_fences() {
        for status in ["requested", "cancelled", "waiting"] {
            let activity = public_activity(&historical(
                "mcp.call",
                status,
                r#"{"server":"PRIVATE_SERVER","tool":"PRIVATE_TOOL"}"#,
                "PRIVATE_BODY",
            ))
            .expect("history");
            assert!(!activity.summary.contains("PRIVATE"));
        }
        let activity = public_activity(&historical(
            "shell.run",
            "failed",
            r#"{"command":"PRIVATE_COMMAND"}"#,
            r#"{"error":{"code":"tool.denied","message":"PRIVATE_REASON"}}"#,
        ))
        .expect("shell history");
        assert_eq!(activity.input, None);
        assert!(!activity.summary.contains("PRIVATE"));
        assert!(
            !activity
                .result
                .expect("categorical denial")
                .value
                .contains("PRIVATE")
        );
        let activity=public_activity(&historical("computer.click","failed","{}",r#"{"error":{"type":"execution_error","message":"PRIVATE_NATIVE","recoverable":false}}"#)).expect("computer history");
        assert!(!activity.summary.contains("PRIVATE"));
        assert!(
            !activity
                .result
                .expect("categorical failure")
                .value
                .contains("PRIVATE")
        );
        let long = public_activity(&historical(
            "repo.map",
            "completed",
            &json!({"path":"é".repeat(1000)}).to_string(),
            "{}",
        ))
        .expect("long history");
        assert!(long.summary.chars().count() <= colossus_presentation::TOOL_ACTIVITY_SUMMARY_CHARS);
        assert!(long.summary.ends_with(" · completed"));
    }
}
