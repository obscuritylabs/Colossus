use super::*;

/// Shared definition idempotency, trust invalidation, and reconstruction checks for every
/// workflow repository adapter.
pub fn assert_workflow_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn WorkflowRepository>,
{
    let repository = factory();
    let definition = WorkflowDefinition {
        api_version: "colossus.dev/v1alpha1".into(),
        kind: "Workflow".into(),
        metadata: WorkflowMetadata {
            name: "conformance".into(),
            version: "1.0.0".into(),
            description: "Shared workflow repository contract.".into(),
        },
        inputs: serde_json::json!({"type": "object"}),
        outputs: serde_json::json!({"type": "object"}),
        capabilities: Vec::new(),
        max_concurrency: 1,
        step_budget: 2,
        steps: vec![WorkflowStep::Emit {
            id: "emit".into(),
            value: serde_json::json!({"ok": true}),
        }],
        compensation: Vec::new(),
    };
    repository
        .register(&definition, "hash-one", "repository:test")
        .expect("register definition");
    repository
        .register(&definition, "hash-one", "repository:test")
        .expect("idempotent registration");
    assert_eq!(
        repository
            .definition("conformance", "1.0.0")
            .expect("definition"),
        Some((definition.clone(), "hash-one".into()))
    );
    repository
        .register(&definition, "hash-two", "repository:test-changed")
        .expect("definition change");
    let schedule = WorkflowSchedule {
        schedule_id: "conformance-daily".into(),
        workflow_name: "conformance".into(),
        workflow_version: "1.0.0".into(),
        workflow_hash: "hash-two".into(),
        inputs: serde_json::json!({}),
        cadence_seconds: 86_400,
        misfire_policy: WorkflowScheduleMisfirePolicy::FireOnce,
        enabled: true,
        starts_at: "2026-01-01T00:00:00Z".into(),
        next_fire_at: "2026-01-01T00:00:00Z".into(),
        last_scheduled_at: None,
        last_run_id: None,
        blocked_reason: None,
        created_at: "2025-12-31T00:00:00Z".into(),
        updated_at: "2025-12-31T00:00:00Z".into(),
    };
    repository
        .create_schedule(
            &schedule,
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("create schedule");
    assert!(
        repository
            .create_schedule(
                &schedule,
                Actor {
                    actor_type: ActorType::User,
                    id: "duplicate".into(),
                },
            )
            .is_err(),
        "schedule identifiers must be unique"
    );
    let disabled = repository
        .set_schedule_enabled(
            &schedule.schedule_id,
            false,
            "2026-01-01T01:00:00Z",
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("disable schedule");
    assert!(!disabled.enabled);
    let webhook = WorkflowWebhook {
        webhook_id: "conformance-hook".into(),
        workflow_name: "conformance".into(),
        workflow_version: "1.0.0".into(),
        workflow_hash: "hash-two".into(),
        secret_reference: "env:CONFORMANCE_WEBHOOK_SECRET".into(),
        enabled: true,
        replay_window_seconds: 300,
        max_body_bytes: 4096,
        blocked_reason: None,
        created_at: "2025-12-31T00:00:00Z".into(),
        updated_at: "2025-12-31T00:00:00Z".into(),
    };
    repository
        .create_webhook(
            &webhook,
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("create webhook");
    assert!(
        repository
            .create_webhook(
                &webhook,
                Actor {
                    actor_type: ActorType::User,
                    id: "duplicate".into(),
                },
            )
            .is_err(),
        "webhook identifiers must be unique"
    );
    let disabled_webhook = repository
        .set_webhook_enabled(
            &webhook.webhook_id,
            false,
            "2026-01-01T01:00:00Z",
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("disable webhook");
    assert!(!disabled_webhook.enabled);
    let subscription = WorkflowSubscription {
        subscription_id: "conformance-events".into(),
        workflow_name: "conformance".into(),
        workflow_version: "1.0.0".into(),
        workflow_hash: "hash-two".into(),
        event_type: "task.created.v1".into(),
        stream_prefix: Some("task:".into()),
        enabled: true,
        checkpoint: 17,
        last_event_id: None,
        last_run_id: None,
        blocked_reason: None,
        created_at: "2025-12-31T00:00:00Z".into(),
        updated_at: "2025-12-31T00:00:00Z".into(),
    };
    repository
        .create_subscription(
            &subscription,
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("create subscription");
    assert!(
        repository
            .create_subscription(
                &subscription,
                Actor {
                    actor_type: ActorType::User,
                    id: "duplicate".into(),
                },
            )
            .is_err(),
        "subscription identifiers must be unique"
    );
    let disabled_subscription = repository
        .set_subscription_enabled(
            &subscription.subscription_id,
            false,
            "2026-01-01T01:00:00Z",
            Actor {
                actor_type: ActorType::User,
                id: "conformance".into(),
            },
        )
        .expect("disable subscription");
    assert!(!disabled_subscription.enabled);
    let reopened = factory();
    assert_eq!(
        reopened
            .definition("conformance", "1.0.0")
            .expect("reconstructed definition"),
        Some((definition, "hash-two".into()))
    );
    assert!(reopened.run("missing-run").expect("missing run").is_none());
    assert!(reopened.runs(10).expect("empty runs").is_empty());
    assert_eq!(
        reopened
            .schedule(&schedule.schedule_id)
            .expect("reconstructed schedule"),
        Some(disabled.clone())
    );
    assert_eq!(
        reopened.schedules(10).expect("schedule list"),
        vec![disabled]
    );
    assert_eq!(
        reopened
            .webhook(&webhook.webhook_id)
            .expect("reconstructed webhook"),
        Some(disabled_webhook.clone())
    );
    assert_eq!(
        reopened.webhooks(10).expect("webhook list"),
        vec![disabled_webhook]
    );
    assert!(
        reopened
            .webhook_delivery(&webhook.webhook_id, "missing-delivery")
            .expect("missing webhook delivery")
            .is_none()
    );
    assert_eq!(
        reopened
            .subscription(&subscription.subscription_id)
            .expect("reconstructed subscription"),
        Some(disabled_subscription.clone())
    );
    assert_eq!(
        reopened.subscriptions(10).expect("subscription list"),
        vec![disabled_subscription]
    );
    assert!(
        reopened
            .subscription_delivery(&subscription.subscription_id, "missing-event")
            .expect("missing subscription delivery")
            .is_none()
    );
}
