use super::*;

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
