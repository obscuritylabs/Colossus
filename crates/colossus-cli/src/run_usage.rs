use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ObservedRunIdentity {
    session_id: String,
    run_id: String,
}

/// Retain only exact identities from released events, without changing delivery.
pub(super) struct UsageTrackingObserver<'a> {
    inner: &'a mut dyn RunEventObserver,
    expected_session: Option<String>,
    identity: Option<ObservedRunIdentity>,
    valid: bool,
}

impl<'a> UsageTrackingObserver<'a> {
    pub(super) fn new(inner: &'a mut dyn RunEventObserver, expected_session: Option<&str>) -> Self {
        Self {
            inner,
            expected_session: expected_session.map(str::to_owned),
            identity: None,
            valid: true,
        }
    }

    pub(super) fn into_identity(self) -> Option<ObservedRunIdentity> {
        self.identity
    }
}

#[async_trait]
impl RunEventObserver for UsageTrackingObserver<'_> {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if self.valid {
            let session = self
                .expected_session
                .get_or_insert_with(|| envelope.session_id.clone());
            if envelope.schema_version == 1
                && !envelope.run_id.is_empty()
                && !envelope.session_id.is_empty()
                && *session == envelope.session_id
            {
                self.identity = Some(ObservedRunIdentity {
                    session_id: envelope.session_id.clone(),
                    run_id: envelope.run_id.clone(),
                });
            } else {
                self.identity = None;
                self.valid = false;
            }
        }
        self.inner.observe(envelope).await
    }
}

#[derive(Clone, Copy)]
pub(super) enum RunUsageScope {
    Run,
    LastGoalIteration,
}

/// Select the direct run represented by an ordinary response, or the final
/// completed iteration of a Goal response. This never infers session totals.
fn response_run_identity(value: &Value) -> Option<(&str, &str)> {
    if let (Some(session_id), Some(run_id)) = (
        value.get("session_id").and_then(Value::as_str),
        value.get("run_id").and_then(Value::as_str),
    ) {
        return Some((session_id, run_id));
    }
    let session_id = value.get("goal")?.get("session_id")?.as_str()?;
    let run_id = value
        .get("iterations")?
        .as_array()?
        .last()?
        .get("run_id")?
        .as_str()?;
    Some((session_id, run_id))
}

pub(super) fn embedded_run_usage_for_output(
    runtime: &Runtime,
    value: &Value,
) -> Option<RunTokenUsage> {
    if !human_output(io::stdout().is_terminal()) {
        return None;
    }
    let (session_id, run_id) = response_run_identity(value)?;
    runtime.run_token_usage(session_id, run_id).ok()
}

pub(super) async fn print_worker_run_response(
    client: &WorkerClient,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn Error>> {
    let value = serde_json::to_value(value)?;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&value);
    }
    let response = value
        .get("output")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("iterations")?
                .as_array()?
                .last()?
                .get("output")?
                .as_str()
        })
        .or_else(|| value.get("goal")?.get("summary")?.as_str());
    let Some(response) = response else {
        return print_json(&value);
    };
    let usage = if let Some((session_id, run_id)) = response_run_identity(&value) {
        // An older worker can reject this optional read. Keep the successful run
        // output and show unknown counts; never rerun work or change placement.
        worker_run_usage_for_session(client, session_id, run_id).await
    } else {
        None
    };
    println!(
        "{}",
        render_run_output(
            &value,
            response,
            usage.as_ref(),
            output_mode(),
            terminal,
            terminal_width(),
            terminal_preferences(),
        )?
    );
    Ok(())
}

pub(super) async fn worker_run_usage_for_session(
    client: &WorkerClient,
    session_id: &str,
    run_id: &str,
) -> Option<RunTokenUsage> {
    client
        .call(WorkerOperation::RunTokenUsage {
            session_id: session_id.into(),
            run_id: run_id.into(),
        })
        .await
        .ok()
        .and_then(|value| serde_json::from_value::<RunTokenUsage>(value).ok())
        .filter(|usage| usage.session_id == session_id && usage.run_id == run_id)
}

pub(super) fn print_embedded_observed_usage(
    runtime: &Runtime,
    identity: Option<&ObservedRunIdentity>,
    scope: RunUsageScope,
    target: StreamTarget,
) {
    if !human_output(io::stdout().is_terminal()) {
        return;
    }
    let usage = identity.and_then(|identity| {
        runtime
            .run_token_usage(&identity.session_id, &identity.run_id)
            .ok()
    });
    print_observed_usage(usage.as_ref(), scope, target);
}

pub(super) async fn print_worker_observed_usage(
    client: &WorkerClient,
    identity: Option<&ObservedRunIdentity>,
    scope: RunUsageScope,
    target: StreamTarget,
) {
    if !human_output(io::stdout().is_terminal()) {
        return;
    }
    let usage = if let Some(identity) = identity {
        worker_run_usage_for_session(client, &identity.session_id, &identity.run_id).await
    } else {
        None
    };
    print_observed_usage(usage.as_ref(), scope, target);
}

fn print_observed_usage(usage: Option<&RunTokenUsage>, scope: RunUsageScope, target: StreamTarget) {
    let mut document = PresentationDocument::default();
    if matches!(scope, RunUsageScope::LastGoalIteration) {
        document.push(PresentationBlock::Markdown(
            "Token usage below is for the last observed goal iteration.".into(),
        ));
    }
    document
        .blocks
        .extend(output_token_usage_document(usage).blocks);
    match target {
        StreamTarget::Stdout => print_terminal_document(
            &document,
            &terminal_preferences(),
            io::stdout().is_terminal(),
        ),
        StreamTarget::Stderr => {
            // This is released accounting metadata only. A failed metadata write
            // must never replace the original run error or trigger execution.
            let _ = write_stderr_document(&document);
        }
    }
}

pub(super) fn output_token_usage_document(usage: Option<&RunTokenUsage>) -> PresentationDocument {
    usage.map_or_else(
        || {
            PresentationDocument::from_block(PresentationBlock::KeyValue(vec![(
                "Run token usage".into(),
                "Unavailable".into(),
            )]))
        },
        run_token_usage_document,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_follows_the_response_run_or_last_goal_iteration() {
        let direct = json!({
            "run_id": "run-a", "session_id": "session-a", "output": "complete",
        });
        assert_eq!(response_run_identity(&direct), Some(("session-a", "run-a")));
        let goal = json!({
            "goal": {"session_id": "session-b"},
            "iterations": [
                {"run_id": "first", "output": "first output"},
                {"run_id": "last", "output": "last output"},
            ],
        });
        assert_eq!(response_run_identity(&goal), Some(("session-b", "last")));
        let empty_goal = json!({"goal": {"session_id": "session-b"}, "iterations": []});
        assert_eq!(response_run_identity(&empty_goal), None);
        assert_eq!(response_run_identity(&json!({"output": "unbound"})), None);
    }
    fn reported_usage() -> RunTokenUsage {
        RunTokenUsage {
            run_id: "private-run-identifier".into(),
            session_id: "private-session-identifier".into(),
            through_version: 12,
            reports: 2,
            usage: Some(colossus_contracts::ProviderUsage {
                input_tokens: 1_000,
                output_tokens: 250,
                total_tokens: 1_499,
                cached_input_tokens: Some(300),
                reasoning_tokens: None,
            }),
        }
    }

    #[test]
    fn human_token_summary_does_not_change_automation_json() {
        let value = json!({
            "run_id": "private-run-identifier",
            "session_id": "private-session-identifier",
            "output": "answer",
        });
        let usage = reported_usage();
        for (mode, terminal) in [(OutputMode::Auto, true), (OutputMode::Human, false)] {
            let human = render_run_output(
                &value,
                "answer",
                Some(&usage),
                mode,
                terminal,
                80,
                TerminalPreferences::default(),
            )
            .unwrap();
            for expected in ["answer", "1,499", "1,000", "250", "300"] {
                assert!(human.contains(expected), "{human}");
            }
            assert!(human.to_lowercase().contains("run usage"), "{human}");
            assert!(!human.contains("private-run-identifier"), "{human}");
            assert!(!human.contains("private-session-identifier"), "{human}");
            assert!(!human.to_lowercase().contains("reasoning"), "{human}");
        }
        for (mode, terminal) in [
            (OutputMode::Auto, false),
            (OutputMode::Json, false),
            (OutputMode::Json, true),
        ] {
            let machine = render_run_output(
                &value,
                "answer",
                Some(&usage),
                mode,
                terminal,
                80,
                TerminalPreferences::default(),
            )
            .unwrap();
            assert_eq!(serde_json::from_str::<Value>(&machine).unwrap(), value);
        }
    }

    #[test]
    fn unreported_and_unavailable_counts_are_not_zero() {
        let value = json!({"output": "answer"});
        let mut unreported = reported_usage();
        unreported.reports = 0;
        unreported.usage = None;
        let human = render_run_output(
            &value,
            "answer",
            Some(&unreported),
            OutputMode::Human,
            false,
            80,
            TerminalPreferences::default(),
        )
        .unwrap();
        assert!(human.to_lowercase().contains("not reported"), "{human}");
        assert!(!human.contains("0 tokens"), "{human}");
        let unavailable = render_run_output(
            &value,
            "answer",
            None,
            OutputMode::Human,
            false,
            80,
            TerminalPreferences::default(),
        )
        .unwrap();
        assert!(unavailable.contains("Unavailable"), "{unavailable}");
        assert!(!unavailable.contains("0 tokens"), "{unavailable}");
    }

    #[test]
    fn goal_summary_identifies_the_last_iteration_scope() {
        let value = json!({
            "goal": {"session_id": "session"},
            "iterations": [{"run_id": "first"}, {"run_id": "last"}],
        });
        let human = render_run_output(
            &value,
            "done",
            Some(&reported_usage()),
            OutputMode::Human,
            false,
            80,
            TerminalPreferences::default(),
        )
        .unwrap();
        assert!(human.contains("last goal iteration"), "{human}");
    }
    #[derive(Default)]
    struct RecordingObserver(Vec<RunEventEnvelope>);

    #[async_trait]
    impl RunEventObserver for RecordingObserver {
        async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
            self.0.push(envelope);
            Ok(())
        }
    }

    fn observed_event(run_id: &str, session_id: &str) -> RunEventEnvelope {
        RunEventEnvelope {
            schema_version: 1,
            run_id: run_id.into(),
            session_id: session_id.into(),
            event: RunEvent::Phase {
                phase: colossus_contracts::RunPhase::Preparing,
                turn: Some(1),
                action: None,
                elapsed_seconds: 0.0,
            },
        }
    }

    #[tokio::test]
    async fn usage_observer_forwards_events_and_tracks_the_last_exact_goal_iteration() {
        let mut downstream = RecordingObserver::default();
        let mut observer = UsageTrackingObserver::new(&mut downstream, Some("goal-session"));
        let first = observed_event("first-iteration", "goal-session");
        let last = observed_event("last-iteration", "goal-session");
        observer
            .observe(first.clone())
            .await
            .expect("first iteration");
        observer
            .observe(last.clone())
            .await
            .expect("last iteration");
        assert_eq!(
            observer.into_identity(),
            Some(ObservedRunIdentity {
                session_id: "goal-session".into(),
                run_id: "last-iteration".into(),
            })
        );
        assert_eq!(downstream.0, [first, last]);
    }

    #[tokio::test]
    async fn usage_observer_keeps_an_exact_cancelled_run_binding() {
        let mut downstream = RecordingObserver::default();
        let mut observer = UsageTrackingObserver::new(&mut downstream, None);
        let mut cancelled = observed_event("cancelled-run", "exact-session");
        cancelled.event = RunEvent::Phase {
            phase: colossus_contracts::RunPhase::Cancelled,
            turn: Some(2),
            action: None,
            elapsed_seconds: 1.0,
        };
        observer
            .observe(cancelled.clone())
            .await
            .expect("cancellation");
        assert_eq!(
            observer.into_identity(),
            Some(ObservedRunIdentity {
                session_id: "exact-session".into(),
                run_id: "cancelled-run".into(),
            })
        );
        assert_eq!(downstream.0, [cancelled]);
    }

    #[tokio::test]
    async fn malformed_or_cross_session_events_withhold_metadata_without_affecting_delivery() {
        for invalid in [
            observed_event("other-run", "other-session"),
            observed_event("", "expected-session"),
            RunEventEnvelope {
                schema_version: 2,
                ..observed_event("run", "expected-session")
            },
        ] {
            let mut downstream = RecordingObserver::default();
            let mut observer =
                UsageTrackingObserver::new(&mut downstream, Some("expected-session"));
            let valid = observed_event("valid-run", "expected-session");
            observer.observe(valid.clone()).await.expect("valid event");
            observer
                .observe(invalid.clone())
                .await
                .expect("unchanged delivery");
            observer
                .observe(valid.clone())
                .await
                .expect("later delivery");
            assert_eq!(observer.into_identity(), None);
            assert_eq!(downstream.0, [valid.clone(), invalid, valid]);
        }
    }
}
