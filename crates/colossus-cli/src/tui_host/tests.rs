use super::*;

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

#[test]
fn session_picker_choice_prioritizes_human_context_over_full_ids() {
    let choice = session_picker_choice(&session(
        "019f72e2-c116-7fa3-b668-5778378e114f",
        12,
        Some("How can we\nget   sccache working locally?"),
        Some("Build speed"),
    ));
    assert_eq!(
        choice,
        "Build speed · 12 msgs · 019f72e2 · 2026-07-18 01:41Z\nHow can we get sccache working locally?"
    );
    assert!(!choice.contains("c116-7fa3"));
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
