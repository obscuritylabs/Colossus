use super::*;
use colossus_contracts::WorkflowControlOperation;

#[tokio::test]
async fn schedule_reviews_persist_replay_and_require_the_exact_owner_response() {
    for content in [
        serde_json::json!({"operation":"create_schedule","schedule_id":"weekly", "workflow_id":"", "expected_hash":"", "inputs":{}, "cadence_seconds":0, "calendar":{"timezone":"America/New_York","time":"09:00","weekdays":[1]}, "task":{"name":"Briefing","instructions":"Summarize procurement news.","tools":[]}, "starts_at":"2026-10-05T13:00:00Z","misfire_policy":"fire_once","enabled":true,"idempotency_key":"weekly-v1"}),
        serde_json::json!({"operation":"set_schedule_enabled","schedule_id":"weekly","enabled":false,"etag":"a".repeat(64)}),
    ] {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn RunRepository> =
            Arc::new(EventSourcedRunRepository::new(journal.clone()));
        let owner = caller("app:schedule-owner", "schedule-owner");
        let create = request("schedule-approval-v1", "Schedule a briefing.");
        let new_run =
            NewRun::from_request("review-run", "review-session", "primary", &create).unwrap();
        let run = repository
            .create_run(&owner, &create, &new_run)
            .unwrap()
            .value;
        let writer = Arc::new(RunWriter::new(
            repository.clone(),
            Arc::new(crate::feed::RunFeeds::default()),
            owner.clone(),
            &run,
        ));
        writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .unwrap();
        let router = Arc::new(
            InteractionRouter::new(Arc::new(DenyApproval), None)
                .with_timeout(Duration::from_secs(2)),
        );
        let operation: WorkflowControlOperation = serde_json::from_value(content.clone()).unwrap();
        let effect = effect_request(
            owner.actor(),
            operation.action(),
            operation.resource(),
            content,
        );
        let decision = PolicyDecision {
            decision_id: "schedule-decision".into(),
            policy_revision: "test-v1".into(),
            outcome: DecisionOutcome::RequireApproval,
            reason: "Persistent schedule requires review".into(),
            obligations: PolicyObligations::default(),
        };
        let waiter = {
            let router = router.clone();
            let writer = writer.clone();
            tokio::spawn(async move {
                router
                    .scope(writer, async {
                        router
                            .request_approval(&effect, &"b".repeat(64), &decision, None)
                            .await
                    })
                    .await
            })
        };
        let pending = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let run = repository.get_run(&owner, "review-run").unwrap().unwrap();
                if let Some(pending) = run.pending_interaction {
                    break pending;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("schedule approval must be published");
        assert_eq!(pending.action.as_deref(), Some("workflow.schedule.control"));
        assert_eq!(pending.resource.as_deref(), Some("persistent schedule"));
        assert!(pending.prompt.contains("weekly"));
        assert!(
            pending.prompt.contains("Summarize procurement news.")
                || pending.prompt.contains("Reviewed canonical revision")
        );
        let replay = EventSourcedRunRepository::new(journal);
        assert_eq!(
            replay
                .get_run(&owner, "review-run")
                .unwrap()
                .unwrap()
                .pending_interaction,
            Some(pending.clone())
        );
        assert!(
            replay
                .get_run(&caller("app:other", "other"), "review-run")
                .unwrap()
                .is_none()
        );
        let run = repository.get_run(&owner, "review-run").unwrap().unwrap();
        let resolved = writer
            .respond_interaction(
                &owner,
                &pending.id,
                &run.etag,
                &IdempotencyKey::new("answer-v1").unwrap(),
                InteractionResponse::Approval {
                    approved: true,
                    request_hash: pending.request_hash.unwrap(),
                },
            )
            .unwrap();
        assert!(router.deliver("review-run", &resolved));
        let proof = waiter.await.unwrap().unwrap().expect("approved proof");
        assert_eq!(proof.request_hash, "b".repeat(64));
        assert_eq!(proof.approved_by, "app:schedule-owner");
    }
}
