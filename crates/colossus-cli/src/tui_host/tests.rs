use super::*;
use colossus_contracts::SessionMessage;

fn plan(id: &str, session_id: &str, status: PlanStatus, revision: u64) -> PlanRecord {
    PlanRecord {
        id: id.into(),
        session_id: session_id.into(),
        prompt: "Implement Plan Mode".into(),
        status,
        revision,
        content: "# Plan".into(),
        steps: Vec::new(),
        created_at: "2026-07-29T00:00:00Z".into(),
        updated_at: "2026-07-29T00:00:00Z".into(),
        approved_at: (status == PlanStatus::Approved).then(|| "2026-07-29T00:01:00Z".into()),
        executed_run_id: (status == PlanStatus::Executed).then(|| "run-1".into()),
    }
}

#[test]
fn provider_doctor_commands_accept_at_most_one_optional_profile() {
    assert_eq!(doctor_profile("doctor", "models"), Ok(None));
    assert_eq!(doctor_profile("doctor local", "models"), Ok(Some("local")));
    assert!(doctor_profile("doctor local extra", "models").is_err());
    assert!(doctor_profile("status", "provider").is_err());
}

#[test]
fn model_diagnostics_use_a_readable_route_and_named_check_table() {
    let document = model_diagnostics_document(&json!({
        "ready": false,
        "route": {
            "role": "primary",
            "profile": "codex",
            "model_profile": "codex",
            "provider_profile": "codex-provider",
            "provider": "openai_codex",
            "model": "gpt-5.6-sol",
            "limits": {
                "contextWindowTokens": 128000,
                "maxOutputTokens": 16000,
                "safetyMarginTokens": 12800,
                "inputBudgetTokens": 99200
            },
            "capabilities": {
                "toolCalls": true,
                "streaming": true
            },
            "reasoning_effort": "xhigh"
        },
        "checks": [
            {
                "name": "metadata",
                "status": "pass",
                "detail": "Explicit limits and capabilities are valid."
            },
            {
                "name": "generation",
                "status": "fail",
                "detail": "provider endpoint returned HTTP 400"
            }
        ]
    }))
    .expect("model diagnostics document");

    let [PresentationBlock::Card { title, tone, body }] = document.blocks.as_slice() else {
        panic!("expected one diagnostics card");
    };
    assert_eq!(title, "Model diagnostics");
    assert_eq!(*tone, PresentationTone::Error);
    let PresentationBlock::KeyValue(details) = &body[0] else {
        panic!("expected route details");
    };
    assert!(details.contains(&("Status".into(), "Not ready".into())));
    assert!(details.contains(&("Model".into(), "gpt-5.6-sol".into())));
    assert!(details.contains(&("Reasoning".into(), "xhigh".into())));
    assert!(details.contains(&(
        "Tokens".into(),
        "128,000 context · 99,200 input budget · 16,000 max output".into()
    )));
    let PresentationBlock::Table(checks) = &body[1] else {
        panic!("expected check table");
    };
    assert_eq!(checks.headers, ["Check", "Status", "Detail"]);
    assert_eq!(
        checks.rows[1],
        ["generation", "Fail", "provider endpoint returned HTTP 400"]
    );
}

#[test]
fn terminal_plan_selection_is_session_scoped_and_actionable() {
    let selected = selectable_plan(
        current_session_plan(
            Some(plan("plan-1", "session-1", PlanStatus::Draft, 1)),
            "plan-1",
            "session-1",
        )
        .expect("current-session plan"),
    )
    .expect("actionable plan");
    assert_eq!(selected.id, "plan-1");

    assert!(
        current_session_plan(
            Some(plan("plan-2", "session-2", PlanStatus::Draft, 1)),
            "plan-2",
            "session-1",
        )
        .is_err()
    );
    assert!(selectable_plan(plan("plan-3", "session-1", PlanStatus::Executed, 3)).is_err());
}

#[test]
fn plan_execution_mapping_distinguishes_pre_and_post_consumption() {
    let approved = plan("plan-1", "session-1", PlanStatus::Approved, 2);
    let cancelled = host_plan_execution_result(
        PlanExecutionOutcome::CancelledBeforeStart {
            plan: approved.clone(),
        },
        FooterState::default(),
    )
    .expect("cancelled mapping");
    assert_eq!(
        cancelled.outcome,
        HostPlanExecutionOutcome::CancelledBeforeStart
    );
    assert!(matches!(
        cancelled.plan_selection,
        PlanSelectionUpdate::Set(plan) if *plan == approved
    ));

    let failed = host_plan_execution_result(
        PlanExecutionOutcome::Direct {
            plan: plan("plan-1", "session-1", PlanStatus::Executed, 3),
            terminal: ControlledAgentTerminal::Failed {
                run_id: "run-1".into(),
                message: "bounded failure".into(),
                outcome_unknown: false,
            },
        },
        FooterState::default(),
    )
    .expect("failed mapping");
    assert_eq!(
        failed.outcome,
        HostPlanExecutionOutcome::FailedAfterConsumption("bounded failure".into())
    );
    assert!(matches!(failed.plan_selection, PlanSelectionUpdate::Clear));
}

#[test]
fn worker_setup_cancellation_returns_typed_run_cancellation() {
    let outcome =
        worker::worker_run_outcome(Err(WorkerError::Cancelled), "session-1").expect("cancellation");

    let AgentRunOutcome::Cancelled { result } = outcome else {
        panic!("expected typed cancellation");
    };
    assert!(uuid::Uuid::parse_str(&result.run_id).is_ok());
    assert_eq!(result.session_id, "session-1");
    assert_eq!(result.turn, 1);
    assert!(result.plan.is_none());
    assert_eq!(result.event_count, 0);
    assert_eq!(result.elapsed_seconds, 0.0);
}

#[test]
fn worker_setup_cancellation_returns_typed_pre_consumption_plan_outcome() {
    let approved = plan("plan-1", "session-1", PlanStatus::Approved, 2);
    let outcome = worker::worker_plan_execution_outcome(Err(WorkerError::Cancelled), &approved)
        .expect("cancellation");
    let result = host_plan_execution_result(outcome, FooterState::default()).expect("host mapping");

    assert_eq!(
        result.outcome,
        HostPlanExecutionOutcome::CancelledBeforeStart
    );
    assert_eq!(result.plan, approved);
    assert!(matches!(
        result.plan_selection,
        PlanSelectionUpdate::Set(plan) if *plan == approved
    ));
}

#[test]
fn plan_execution_errors_reconcile_durable_consumption_or_fail_unknown() {
    let approved = plan("plan-1", "session-1", PlanStatus::Approved, 2);
    approved_plan_at_revision(Some(approved.clone()), "plan-1", "session-1", 2)
        .expect("selected approved revision");

    let before = host_plan_execution_failure(
        approved.clone(),
        Ok(Some(approved.clone())),
        "policy denied".into(),
        FooterState::default(),
    );
    assert_eq!(
        before.outcome,
        HostPlanExecutionOutcome::FailedBeforeConsumption("policy denied".into())
    );
    assert!(matches!(
        before.plan_selection,
        PlanSelectionUpdate::Set(plan) if *plan == approved
    ));

    let executed = plan("plan-1", "session-1", PlanStatus::Executed, 3);
    let after = host_plan_execution_failure(
        approved.clone(),
        Ok(Some(executed.clone())),
        "connection closed".into(),
        FooterState::default(),
    );
    assert_eq!(
        after.outcome,
        HostPlanExecutionOutcome::ConsumedOutcomeUnknown("connection closed".into())
    );
    assert_eq!(after.plan, executed);
    assert!(matches!(after.plan_selection, PlanSelectionUpdate::Clear));

    let unknown = host_plan_execution_failure(
        approved.clone(),
        Err("worker unavailable".into()),
        "connection closed".into(),
        FooterState::default(),
    );
    assert_eq!(
        unknown.outcome,
        HostPlanExecutionOutcome::OutcomeUnknown("connection closed".into())
    );
    assert_eq!(unknown.plan, approved);
    assert!(matches!(unknown.plan_selection, PlanSelectionUpdate::Clear));
}

#[test]
fn plan_lifecycle_errors_apply_readback_before_pausing_the_queue() {
    let draft = plan("plan-1", "session-1", PlanStatus::Draft, 2);
    let approved = plan("plan-1", "session-1", PlanStatus::Approved, 3);
    let committed = host_plan_lifecycle_failure(
        draft.clone(),
        Ok(Some(approved.clone())),
        PlanStatus::Approved,
        "connection closed".into(),
    );
    assert!(!committed.continue_queue);
    assert!(matches!(
        committed.plan_selection,
        PlanSelectionUpdate::Set(plan) if *plan == approved
    ));

    let unchanged = host_plan_lifecycle_failure(
        draft.clone(),
        Ok(Some(draft.clone())),
        PlanStatus::Approved,
        "policy denied".into(),
    );
    assert!(!unchanged.continue_queue);
    assert!(matches!(
        unchanged.plan_selection,
        PlanSelectionUpdate::Set(plan) if *plan == draft
    ));

    let unknown = host_plan_lifecycle_failure(
        draft,
        Err("worker unavailable".into()),
        PlanStatus::Discarded,
        "connection closed".into(),
    );
    assert!(!unknown.continue_queue);
    assert!(matches!(unknown.plan_selection, PlanSelectionUpdate::Clear));
}

fn session(
    id: &str,
    message_count: u64,
    preview: Option<&str>,
    title: Option<&str>,
) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        title: title.map(str::to_owned),
        created_at: "2026-07-18T01:00:00Z".into(),
        updated_at: "2026-07-18T01:41:50.425459Z".into(),
        message_count,
        last_run_id: None,
        last_user_preview: preview.map(str::to_owned),
    }
}

fn conversation_message(sequence: u64) -> SessionMessage {
    SessionMessage {
        session_id: "019f72e2-c116-7fa3-b668-5778378e114f".into(),
        run_id: "run".into(),
        sequence,
        message: colossus_contracts::ModelMessage {
            role: if sequence.is_multiple_of(2) {
                ModelMessageRole::User
            } else {
                ModelMessageRole::Assistant
            },
            content: format!("Message {sequence}\nwith   normalized spacing"),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        created_at: "2026-07-18T01:41:50Z".into(),
    }
}

fn tool_message(sequence: u64) -> SessionMessage {
    SessionMessage {
        session_id: "019f72e2-c116-7fa3-b668-5778378e114f".into(),
        run_id: "run".into(),
        sequence,
        message: colossus_contracts::ModelMessage {
            role: ModelMessageRole::Tool,
            content: format!("tool result {sequence}"),
            tool_call_id: Some(format!("call-{sequence}")),
            tool_calls: Vec::new(),
        },
        created_at: "2026-07-18T01:41:50Z".into(),
    }
}

/// Page `messages` (chronological) backward the way the runtime store does.
fn message_page(messages: &[SessionMessage], before_sequence: Option<u64>) -> SessionMessagePage {
    let upper = before_sequence.unwrap_or(u64::MAX);
    let mut page = messages
        .iter()
        .rev()
        .filter(|message| message.sequence < upper)
        .take(SESSION_BROWSER_PAGE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    page.reverse();
    let before_sequence = page.first().map(|message| message.sequence);
    let has_more = before_sequence.is_some_and(|first| {
        messages
            .iter()
            .any(|message| message.sequence < first && message.sequence < upper)
    });
    SessionMessagePage {
        messages: page,
        before_sequence,
        has_more,
    }
}

fn collected_preview(messages: &[SessionMessage]) -> Vec<InteractiveSessionBrowserMessage> {
    let mut collector = SessionPreviewCollector::new();
    while collector.wants_older_page() {
        collector.absorb(message_page(messages, collector.before_sequence()));
    }
    collector.finish()
}

#[test]
fn session_browser_entry_keeps_the_latest_visible_conversation_bounded() {
    let messages = (0..10).map(conversation_message).collect::<Vec<_>>();
    let entry = session_browser_entry(
        session(
            "019f72e2-c116-7fa3-b668-5778378e114f",
            12,
            Some("How can we get sccache working locally?"),
            Some("Build speed"),
        ),
        collected_preview(&messages),
    );
    assert_eq!(entry.recent_messages.len(), 8);
    assert_eq!(
        entry
            .recent_messages
            .first()
            .map(|message| message.content.as_str()),
        Some("Message 2 with normalized spacing")
    );
    assert_eq!(
        entry
            .recent_messages
            .last()
            .map(|message| message.content.as_str()),
        Some("Message 9 with normalized spacing")
    );
    assert_eq!(compact_text("Safe\n text\u{200b}\u{1b}", 100), "Safe text");
}

#[test]
fn session_preview_pages_backward_past_tool_heavy_history() {
    // Two prompts trailed by far more tool records than one page can hold.
    let mut messages = vec![conversation_message(0), conversation_message(1)];
    messages.extend((2..(2 + (SESSION_BROWSER_PAGE_LIMIT as u64 * 3))).map(tool_message));
    let preview = collected_preview(&messages);
    assert_eq!(
        preview
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Message 0 with normalized spacing",
            "Message 1 with normalized spacing",
        ]
    );
}

#[test]
fn session_preview_stops_after_the_bounded_backward_page_budget() {
    let messages = (0..(SESSION_BROWSER_PAGE_LIMIT as u64
        * (SESSION_BROWSER_PREVIEW_PAGES as u64 + 2)))
        .map(tool_message)
        .collect::<Vec<_>>();
    let mut collector = SessionPreviewCollector::new();
    let mut pages = 0_usize;
    while collector.wants_older_page() {
        collector.absorb(message_page(&messages, collector.before_sequence()));
        pages += 1;
    }
    assert_eq!(pages, SESSION_BROWSER_PREVIEW_PAGES);
    assert!(collector.finish().is_empty());
}

#[test]
fn resume_picker_excludes_empty_sessions_before_applying_the_limit() {
    let sessions = resumable_sessions(
        vec![
            session("empty", 0, None, None),
            session("first", 2, Some("first"), None),
            session("second", 3, Some("second"), None),
        ],
        1,
    );
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "first");
}

#[tokio::test]
async fn theme_browser_releases_exact_reversible_preview_preferences() {
    let themes = ThemeLibrary::default();
    let preferences = TerminalPreferences::default();
    let (sender, mut events) = mpsc::channel(1);
    let browser = tokio::spawn(async move {
        browse_themes(&sender, &themes, &preferences)
            .await
            .expect("browse themes")
    });

    let HostEvent::ThemePicker(request) = events.recv().await.expect("theme picker") else {
        panic!("expected a theme picker event");
    };
    assert_eq!(request.current_theme, "default");
    assert_eq!(request.themes.len(), 5);
    let hacker = request
        .themes
        .iter()
        .find(|theme| theme.name == "hacker")
        .expect("hacker preview");
    assert_eq!(hacker.preferences.theme_name(), "hacker");
    request
        .response
        .send(PromptResponse::Answer("hacker".into()))
        .expect("select hacker");
    assert_eq!(browser.await.expect("browser task"), Some("hacker".into()));
}

#[tokio::test]
async fn embedded_tui_receives_automatic_approval_as_a_non_blocking_notice() {
    let router = Arc::new(TuiPromptRouter::default());
    let (sender, mut events) = mpsc::channel(1);
    router.install(Some(sender));
    let provider = TuiApprovalProvider {
        router,
        risk_auto: true,
    };

    provider
        .automatic_approval_granted(AutomaticApprovalNotice {
            action: "network.http".into(),
            resource: "https://example.test/resource".into(),
            risk_level: colossus_contracts::RiskLevel::Low,
            reason: "bodyless GET to an exact configured origin".into(),
        })
        .await;

    let HostEvent::Notice(document) = events.recv().await.expect("notice") else {
        panic!("expected a non-blocking notice");
    };
    assert!(matches!(
        document.blocks.first(),
        Some(PresentationBlock::Card { title, .. }) if title == "Automatic approval review"
    ));
}

#[tokio::test]
async fn embedded_tui_drops_automatic_approval_when_notice_queue_is_full() {
    let router = Arc::new(TuiPromptRouter::default());
    let (sender, mut events) = mpsc::channel(1);
    sender
        .try_send(HostEvent::Notice(PresentationDocument::new()))
        .expect("fill notice queue");
    router.install(Some(sender));
    let provider = TuiApprovalProvider {
        router,
        risk_auto: true,
    };

    tokio::time::timeout(
        Duration::from_millis(100),
        provider.automatic_approval_granted(AutomaticApprovalNotice {
            action: "web.search".into(),
            resource: "configured search provider".into(),
            risk_level: colossus_contracts::RiskLevel::Low,
            reason: "read-only configured search".into(),
        }),
    )
    .await
    .expect("a best-effort notice must not wait for queue capacity");

    assert!(matches!(events.recv().await, Some(HostEvent::Notice(_))));
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn embedded_tui_receives_risk_review_failure_before_manual_approval() {
    let router = Arc::new(TuiPromptRouter::default());
    let (sender, mut events) = mpsc::channel(1);
    router.install(Some(sender));
    let provider = TuiApprovalProvider {
        router,
        risk_auto: true,
    };

    provider
        .risk_review_fallback(RiskReviewFallbackNotice {
            action: "web.search".into(),
            resource: "http://127.0.0.1:8888/search".into(),
            failure: colossus_contracts::RiskReviewFailure::InvalidAssessment,
            reason: "The risk evaluator response failed strict validation, so manual approval is required."
                .into(),
        })
        .await;

    let HostEvent::Notice(document) = events.recv().await.expect("notice") else {
        panic!("expected a non-blocking notice");
    };
    assert!(matches!(
        document.blocks.first(),
        Some(PresentationBlock::Card { title, .. })
            if title == "Automatic approval review failed"
    ));
}

#[tokio::test]
async fn embedded_tui_manual_prompt_explains_risk_auto_ineligibility() {
    let router = Arc::new(TuiPromptRouter::default());
    let (sender, mut events) = mpsc::channel(1);
    router.install(Some(sender));
    let provider = TuiApprovalProvider {
        router,
        risk_auto: true,
    };
    let mut request = colossus_policy::effect_request(
        colossus_policy::system_actor("tui-test"),
        "mcp.call",
        "http://127.0.0.1:3001/mcp",
        json!({"operation": {"kind": "call_tool"}}),
    );
    request.risk.reason =
        Some("Risk-auto review requires supported, request-bound MCP discovery metadata.".into());
    let decision = PolicyDecision {
        decision_id: "decision-test".into(),
        policy_revision: "test-v1".into(),
        outcome: colossus_contracts::DecisionOutcome::RequireApproval,
        reason: "explicit operator approval required".into(),
        obligations: colossus_contracts::PolicyObligations::default(),
    };
    let approval = tokio::spawn(async move {
        provider
            .request_approval(&request, "request-hash", &decision)
            .await
    });

    let HostEvent::Prompt(prompt) = events.recv().await.expect("prompt") else {
        panic!("expected an approval prompt");
    };
    assert_eq!(prompt.kind, InteractivePromptKind::Approval);
    let [PresentationBlock::Card { body, .. }] = prompt.document.blocks.as_slice() else {
        panic!("expected one approval card");
    };
    let Some(PresentationBlock::KeyValue(details)) = body.first() else {
        panic!("expected approval details");
    };
    assert!(details.iter().any(|(label, value)| {
        label == "Risk review" && value.contains("request-bound MCP discovery metadata")
    }));
    assert!(
        details
            .iter()
            .any(|(label, value)| { label == "Requested by" && value.contains("System service") })
    );
    prompt
        .response
        .send(PromptResponse::Answer("Deny".into()))
        .expect("deny response");
    assert!(
        approval
            .await
            .expect("approval task")
            .expect("result")
            .is_none()
    );
}

#[tokio::test]
async fn worker_tui_projects_approvals_into_the_typed_dock_document() {
    let (sender, mut events) = mpsc::channel(1);
    let handler = worker::TuiWorkerPromptHandler { sender };
    let prompt = WorkerPrompt {
        prompt_id: "worker-approval".into(),
        kind: WorkerPromptKind::Approval,
        title: "Approval required".into(),
        question: "explicit operator approval required".into(),
        choices: vec!["Allow once".into(), "Deny".into()],
        allow_free_form: false,
        details: json!({
            "actor": {"actor_type": "model", "id": "primary"},
            "action": "mcp.call",
            "resource": "http://127.0.0.1:18000/mcp",
            "reason": "explicit operator approval required",
            "risk": {
                "status": "unavailable",
                "level": null,
                "reason": "The configured risk evaluator was unavailable."
            },
            "content": {"operation": {"server": "splunk", "tool": "index_info"}}
        }),
    };
    let task = tokio::spawn(async move { handler.prompt(prompt).await });

    let HostEvent::Prompt(prompt) = events.recv().await.expect("approval prompt") else {
        panic!("expected worker approval prompt");
    };
    assert_eq!(prompt.kind, InteractivePromptKind::Approval);
    let [PresentationBlock::Card { body, .. }] = prompt.document.blocks.as_slice() else {
        panic!("expected approval card");
    };
    let [
        PresentationBlock::KeyValue(details),
        PresentationBlock::Code { content, .. },
    ] = body.as_slice()
    else {
        panic!("expected dock summary and exact request");
    };
    assert!(
        details
            .iter()
            .any(|(label, value)| { label == "Requested by" && value == "model · primary" })
    );
    assert!(
        details
            .iter()
            .any(|(label, value)| { label == "Risk review" && value.starts_with("not assessed:") })
    );
    assert!(content.contains("index_info"));
    prompt
        .response
        .send(PromptResponse::Answer("Deny".into()))
        .expect("deny response");
    assert_eq!(
        task.await.expect("prompt task").expect("prompt result"),
        Some("Deny".into())
    );
}

#[tokio::test]
async fn worker_tui_accepts_only_the_canonical_boundary_prompt() {
    let (sender, mut events) = mpsc::channel(1);
    let handler = worker::TuiWorkerPromptHandler { sender };
    let prompt = WorkerPrompt {
        prompt_id: "worker-boundary".into(),
        kind: WorkerPromptKind::SandboxBoundaryAcknowledgement,
        title: "External sandbox boundary".into(),
        question: "Acknowledge the external boundary?".into(),
        choices: vec![
            "Acknowledge the external boundary".into(),
            "Keep process execution blocked".into(),
        ],
        allow_free_form: false,
        details: json!({"mode": "external"}),
    };
    let task = tokio::spawn(async move { handler.prompt(prompt).await });

    let HostEvent::Prompt(prompt) = events.recv().await.expect("boundary prompt") else {
        panic!("expected worker boundary prompt");
    };
    assert_eq!(
        prompt.kind,
        InteractivePromptKind::SandboxBoundaryAcknowledgement
    );
    assert!(matches!(
        prompt.document.blocks.as_slice(),
        [PresentationBlock::Card {
            tone: PresentationTone::Warning,
            ..
        }]
    ));
    prompt
        .response
        .send(PromptResponse::Answer(
            "Acknowledge the external boundary".into(),
        ))
        .expect("acknowledgement response");
    assert_eq!(
        task.await.expect("prompt task").expect("prompt result"),
        Some("Acknowledge the external boundary".into())
    );

    let (sender, mut events) = mpsc::channel(1);
    let handler = worker::TuiWorkerPromptHandler { sender };
    let error = handler
        .prompt(WorkerPrompt {
            prompt_id: "worker-boundary-tampered".into(),
            kind: WorkerPromptKind::SandboxBoundaryAcknowledgement,
            title: "External sandbox boundary".into(),
            question: "Acknowledge the external boundary?".into(),
            choices: vec!["Always allow without prompting".into()],
            allow_free_form: false,
            details: json!({"mode": "external"}),
        })
        .await
        .expect_err("noncanonical boundary prompt");
    assert!(matches!(error, WorkerError::Protocol(_)));
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn worker_tui_drops_approval_review_notice_when_event_queue_is_full() {
    let (sender, mut events) = mpsc::channel(1);
    sender
        .try_send(HostEvent::Notice(PresentationDocument::new()))
        .expect("fill event queue");
    let handler = worker::TuiWorkerPromptHandler { sender };

    tokio::time::timeout(
        Duration::from_millis(100),
        handler.notice(ApprovalReviewNotice::AutomaticApproval {
            notice: AutomaticApprovalNotice {
                action: "network.http".into(),
                resource: "https://example.test/resource".into(),
                risk_level: colossus_contracts::RiskLevel::Low,
                reason: "bodyless GET to an exact configured origin".into(),
            },
        }),
    )
    .await
    .expect("a worker notice must not wait for queue capacity")
    .expect("dropping a best-effort notice must not fail the run");

    assert!(matches!(events.recv().await, Some(HostEvent::Notice(_))));
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}
