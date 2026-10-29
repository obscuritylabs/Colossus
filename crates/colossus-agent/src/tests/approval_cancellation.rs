use super::*;

struct PendingApprovalTools {
    started: Arc<tokio::sync::Notify>,
    closed: Arc<tokio::sync::Notify>,
    calls: AtomicUsize,
    outcome_unknown: bool,
}

#[async_trait]
impl ToolExecutor for PendingApprovalTools {
    async fn execute(
        &self,
        _call: ToolCall,
        _context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.started.notify_one();
        self.closed.notified().await;
        if self.outcome_unknown {
            Err(ToolError::OutcomeUnknown(
                "effect termination was not confirmed".into(),
            ))
        } else {
            Err(ToolError::Denied("operator declined the approval".into()))
        }
    }
}

async fn approval_closure_case(stop: bool, outcome_unknown: bool) {
    let provider = Arc::new(ScriptedProvider::new(vec![
        turn(vec![
            ProviderEvent::ToolCallRequested {
                call_id: "approval-wait".into(),
                name: "echo".into(),
                arguments: json!({"text": "first"}),
            },
            ProviderEvent::ToolCallRequested {
                call_id: "not-started".into(),
                name: "echo".into(),
                arguments: json!({"text": "second"}),
            },
        ]),
        turn(vec![ProviderEvent::FinalOutput {
            text: "resumed safely".into(),
        }]),
    ]));
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let sessions = Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
    let tools = Arc::new(PendingApprovalTools {
        started: Arc::new(tokio::sync::Notify::new()),
        closed: Arc::new(tokio::sync::Notify::new()),
        calls: AtomicUsize::new(0),
        outcome_unknown,
    });
    let service = Arc::new(AgentService::new(
        Arc::clone(&journal),
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        Arc::new(StaticToolRegistry::builtins(&["echo".into()]).expect("catalog")),
        Arc::clone(&tools) as Arc<dyn ToolExecutor>,
        Arc::clone(&sessions) as Arc<dyn SessionRepository>,
    ));
    let control = RunControl::default();
    let run = {
        let service = Arc::clone(&service);
        let control = control.clone();
        tokio::spawn(async move {
            let mut observer = RecordingRunObserver::default();
            let outcome = service
                .run_in_session_with_skills_stream_controlled(
                    "primary",
                    "test",
                    "await one approval",
                    2,
                    None,
                    &[],
                    &mut observer,
                    &control,
                )
                .await;
            (outcome, observer)
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), tools.started.notified())
        .await
        .expect("approval pending");
    if stop {
        control.cancel();
    }
    tools.closed.notify_one();
    let (outcome, observer) = tokio::time::timeout(std::time::Duration::from_secs(2), run)
        .await
        .expect("bounded approval settlement")
        .expect("run task");
    assert_eq!(
        tools.calls.load(Ordering::Acquire),
        1,
        "later tools never start"
    );
    let summary = sessions.list_sessions(1).expect("session").remove(0);
    assert!(
        sessions
            .pending_tool_turn(&summary.id)
            .expect("pending tool turn")
            .is_none()
    );
    let messages = sessions
        .list_messages(&summary.id)
        .expect("paired messages");
    validate_model_transcript(
        &messages
            .iter()
            .map(|message| message.message.clone())
            .collect::<Vec<_>>(),
    )
    .expect("valid settled transcript");
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.message.role == ModelMessageRole::Tool)
            .count(),
        2
    );
    let run_events = journal
        .read_stream(&format!(
            "run:{}",
            summary.last_run_id.as_ref().expect("run id")
        ))
        .expect("canonical run events");
    let cancelled = stop && !outcome_unknown;
    assert_eq!(
        matches!(outcome, Ok(AgentRunOutcome::Cancelled { .. })),
        cancelled
    );
    assert_eq!(
        run_events
            .iter()
            .filter(|event| event.event_type == "run.cancelled.v1")
            .count(),
        usize::from(cancelled)
    );
    assert_eq!(
        run_events
            .iter()
            .filter(|event| event.event_type == "error.v1")
            .count(),
        usize::from(!cancelled)
    );
    assert_eq!(
        observer
            .events
            .iter()
            .filter(|event| matches!(
                event.event,
                RunEvent::Phase {
                    phase: RunPhase::Cancelled,
                    ..
                }
            ))
            .count(),
        usize::from(cancelled)
    );
    assert_eq!(
        observer
            .events
            .iter()
            .filter(|event| matches!(event.event, RunEvent::Error { .. }))
            .count(),
        usize::from(!cancelled)
    );
    if cancelled {
        assert_eq!(
            run_events
                .iter()
                .filter(|event| event.event_type == "tool.call.cancelled.v1")
                .count(),
            1
        );
        assert_eq!(
            run_events.last().expect("terminal event").event_type,
            "run.cancelled.v1"
        );
        assert!(run_events.iter().any(|event| {
            event.event_type == "tool.call.completed.v1"
                && journal
                    .decrypt_payload(event)
                    .expect("tool completion payload")["call_id"]
                    == "approval-wait"
        }));
        assert!(
            messages
                .iter()
                .any(|message| message.message.role == ModelMessageRole::Tool
                    && message.message.tool_call_id.as_deref() == Some("approval-wait")
                    && message.message.content.contains("tool.denied")),
            "retain the actual denied observation"
        );
    } else if outcome_unknown {
        assert!(matches!(
            outcome,
            Err(AgentError::Tool(ToolError::OutcomeUnknown(_)))
        ));
        assert!(run_events.iter().any(|event| {
            event.event_type == "tool.call.completed.v1"
                && journal
                    .decrypt_payload(event)
                    .expect("tool completion payload")["outcome_certainty"]
                    == "unknown"
        }));
    } else {
        assert!(matches!(
            outcome,
            Err(AgentError::Tool(ToolError::Denied(_)))
        ));
    }
    let resumed = service
        .run_in_session(
            "primary",
            "test",
            "resume after settled approval",
            1,
            Some(&summary.id),
        )
        .await
        .expect("same-session continuation");
    assert_eq!(resumed.output, "resumed safely");
    assert_eq!(provider.requests.lock().expect("requests").len(), 2);
}

#[tokio::test]
async fn stop_while_approval_closes_commits_one_cancellation_and_a_resumable_tool_turn() {
    approval_closure_case(true, false).await;
}

#[tokio::test]
async fn declined_approval_without_stop_remains_a_terminal_denial() {
    approval_closure_case(false, false).await;
}

#[tokio::test]
async fn stop_never_masks_a_tool_outcome_unknown_failure() {
    approval_closure_case(true, true).await;
}
