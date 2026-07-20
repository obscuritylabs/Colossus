use super::{
    Condition, DenyWorkflowEffects, EventSourcedWorkflowRepository, MAX_CONDITION_BYTES,
    MAX_CONDITION_DEPTH, MAX_CONDITION_TOKENS, WorkflowEffect, WorkflowEffectRunner, WorkflowError,
    WorkflowService, parse_schedule_time, validate_definition,
};
use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
    ProjectionWorkItem, SignedCheckpoint, WorkflowScheduleDispatchStatus,
    WorkflowScheduleMisfirePolicy, WorkflowStatus, WorkflowSubscriptionDispatchStatus,
    WorkflowTriggerKind,
};
use colossus_journal_redb::{Ed25519CheckpointSigner, RedbEventJournal};
use colossus_ports::{
    EventJournal, KeyProvider, StoreError, VerificationReport, WorkflowRepository,
};
use colossus_testkit::{InMemoryEventJournal, assert_workflow_repository_conformance};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
use tempfile::tempdir;

const SIMPLE: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: smoke
  version: 1.0.0
  description: Offline smoke workflow
inputs:
  type: object
  required: [message]
  properties:
    message: { type: string }
outputs:
  type: object
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: emit
    id: result
    value: { ok: true }
"#;

const WEBHOOK_WORKFLOW: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: webhook-smoke
  version: 1.0.0
  description: Authenticated webhook smoke workflow
inputs:
  type: object
  additionalProperties: false
  required: [body, delivery_id, headers, timestamp]
  properties:
    body: { type: object }
    delivery_id: { type: string }
    headers: { type: object }
    timestamp: { type: string }
outputs:
  type: object
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: emit
    id: result
    value: { ok: true }
"#;

const SUBSCRIPTION_WORKFLOW: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: subscription-smoke
  version: 1.0.0
  description: Repository event subscription smoke workflow
inputs:
  type: object
  additionalProperties: false
  required: [event, idempotency_key, subscription_id]
  properties:
    subscription_id: { type: string }
    idempotency_key: { type: string }
    event:
      type: object
      required: [event_id, global_sequence, stream_id, stream_version, classification, event_type, actor, context, occurred_at, payload]
      properties:
        payload:
          type: object
          required: [title]
          properties:
            title: { type: string }
outputs:
  type: object
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: emit
    id: result
    value: { ok: true }
"#;

fn webhook_signature(timestamp: &str, delivery_id: &str, body: &[u8], secret: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC secret");
    mac.update(timestamp.as_bytes());
    mac.update(b"\n");
    mac.update(delivery_id.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[test]
fn event_sourced_workflow_repository_passes_shared_conformance() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    assert_workflow_repository_conformance(|| {
        Box::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)))
    });
}

#[tokio::test]
async fn fire_once_schedule_reconstructs_time_and_queues_exactly_once() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        Arc::new(DenyWorkflowEffects),
    );
    service
        .register_definition(SIMPLE, "schedule-test")
        .expect("register workflow");
    service
        .create_schedule_at(
            "every-minute",
            "smoke",
            "1.0.0",
            json!({"message": "scheduled"}),
            60,
            WorkflowScheduleMisfirePolicy::FireOnce,
            true,
            Some("2026-01-01T12:00:00Z"),
            parse_schedule_time("2026-01-01T11:59:00Z", "test clock").expect("test clock"),
        )
        .expect("create schedule");

    let dispatches = service
        .tick_schedules_at("2026-01-01T12:03:10Z")
        .expect("tick schedule");
    assert_eq!(dispatches.len(), 1);
    let dispatch = &dispatches[0];
    assert_eq!(dispatch.status, WorkflowScheduleDispatchStatus::Queued);
    assert_eq!(
        dispatch.scheduled_at.as_deref(),
        Some("2026-01-01T12:03:00Z")
    );
    assert_eq!(dispatch.next_fire_at, "2026-01-01T12:04:00Z");
    assert_eq!(dispatch.missed_occurrences, 3);
    let run_id = dispatch.run_id.clone().expect("scheduled run id");
    assert!(
        service
            .tick_schedules_at("2026-01-01T12:03:10Z")
            .expect("repeat tick")
            .is_empty(),
        "the same clock value must not queue the occurrence twice"
    );
    let queued = service.get_run(&run_id).expect("queued run");
    assert_eq!(queued.status, WorkflowStatus::Queued);
    assert_eq!(queued.trigger_kind, Some(WorkflowTriggerKind::Schedule));
    assert_eq!(queued.trigger_id.as_deref(), Some("every-minute"));
    assert_eq!(
        queued.trigger_occurrence.as_deref(),
        Some("2026-01-01T12:03:00Z")
    );
    let completed = service.drain().await.expect("drain scheduled run");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].status, WorkflowStatus::Completed);

    let reopened = WorkflowService::new(
        Arc::clone(&journal),
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal))),
        Arc::new(DenyWorkflowEffects),
    );
    let schedule = reopened.get_schedule("every-minute").expect("schedule");
    assert_eq!(schedule.next_fire_at, "2026-01-01T12:04:00Z");
    assert_eq!(schedule.last_run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(
        reopened.get_run(&run_id).expect("reopened run").status,
        WorkflowStatus::Completed
    );

    let events = journal.read_global(1, usize::MAX).expect("events");
    let fired = events
        .iter()
        .position(|event| event.event_type == "workflow.schedule.fired.v1")
        .expect("fired event");
    assert_eq!(
        events.get(fired + 1).map(|event| event.event_type.as_str()),
        Some("workflow.run.queued.v1"),
        "the schedule transition and queued run must be adjacent in one journal batch"
    );
}

#[tokio::test]
async fn authenticated_webhook_is_policy_gated_and_atomically_queues_once() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    service
        .register_definition(WEBHOOK_WORKFLOW, "webhook-test")
        .expect("register workflow");
    let webhook = service
        .create_webhook(
            "github-main",
            "webhook-smoke",
            "1.0.0",
            "env:COLOSSUS_WEBHOOK_SECRET",
            300,
            4096,
            true,
        )
        .expect("create webhook");
    assert_eq!(
        service.get_webhook("github-main").expect("stored webhook"),
        webhook
    );

    let secret = b"this-secret-is-at-least-thirty-two-bytes";
    let timestamp = "2026-07-16T12:00:00Z";
    let delivery_id = "delivery-0001";
    let body = br#"{"event":"push"}"#;
    let signature = webhook_signature(timestamp, delivery_id, body, secret);
    let received = parse_schedule_time("2026-07-16T12:02:00Z", "test clock").expect("test clock");
    let dispatch = service
        .ingest_webhook_at(
            "github-main",
            delivery_id,
            timestamp,
            &signature,
            BTreeMap::from([("content-type".into(), "application/json".into())]),
            body,
            secret,
            received,
        )
        .await
        .expect("ingest webhook");
    assert_eq!(dispatch.run.status, WorkflowStatus::Queued);
    assert_eq!(
        dispatch.run.trigger_kind,
        Some(WorkflowTriggerKind::Webhook)
    );
    assert_eq!(dispatch.run.trigger_id.as_deref(), Some("github-main"));
    assert_eq!(
        dispatch.run.trigger_occurrence.as_deref(),
        Some(delivery_id)
    );
    assert_eq!(dispatch.delivery.run_id, dispatch.run.run_id);
    assert_eq!(
        repository
            .webhook_delivery("github-main", delivery_id)
            .expect("delivery lookup")
            .expect("delivery"),
        dispatch.delivery
    );

    let calls = effects.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].action, "workflow.webhook.ingest");
    assert_eq!(calls[0].credential_references.len(), 1);
    assert_eq!(
        calls[0].credential_references[0].reference,
        "env:COLOSSUS_WEBHOOK_SECRET"
    );
    assert!(
        !calls[0]
            .content
            .to_string()
            .contains(&String::from_utf8_lossy(secret).to_string())
    );
    assert!(!calls[0].content.to_string().contains(&signature));

    let replay = service
        .ingest_webhook_at(
            "github-main",
            delivery_id,
            timestamp,
            &signature,
            BTreeMap::new(),
            body,
            secret,
            received,
        )
        .await;
    assert!(matches!(replay, Err(WorkflowError::InvalidTransition(_))));
    assert_eq!(service.list_runs(10).expect("runs").len(), 1);
    assert_eq!(effects.calls().len(), 1);

    let events = journal.read_global(1, usize::MAX).expect("events");
    let accepted = events
        .iter()
        .position(|event| event.event_type == "workflow.webhook.delivery.accepted.v1")
        .expect("accepted event");
    assert_eq!(
        events
            .get(accepted + 1)
            .map(|event| event.event_type.as_str()),
        Some("workflow.run.queued.v1")
    );
}

#[tokio::test]
async fn webhook_rejects_bad_auth_and_stale_delivery_before_policy() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects.clone());
    service
        .register_definition(WEBHOOK_WORKFLOW, "webhook-test")
        .expect("register workflow");
    service
        .create_webhook(
            "incoming",
            "webhook-smoke",
            "1.0.0",
            "env:COLOSSUS_WEBHOOK_SECRET",
            60,
            32,
            true,
        )
        .expect("create webhook");
    let secret = b"this-secret-is-at-least-thirty-two-bytes";
    let received = parse_schedule_time("2026-07-16T12:02:00Z", "test clock").expect("test clock");
    let bad = service
        .ingest_webhook_at(
            "incoming",
            "bad-auth",
            "2026-07-16T12:02:00Z",
            &format!("sha256={}", "0".repeat(64)),
            BTreeMap::new(),
            br#"{}"#,
            secret,
            received,
        )
        .await;
    assert!(matches!(bad, Err(WorkflowError::InvalidTransition(_))));

    let stale_timestamp = "2026-07-16T11:59:00Z";
    let stale = service
        .ingest_webhook_at(
            "incoming",
            "stale",
            stale_timestamp,
            &webhook_signature(stale_timestamp, "stale", br#"{}"#, secret),
            BTreeMap::new(),
            br#"{}"#,
            secret,
            received,
        )
        .await;
    assert!(matches!(stale, Err(WorkflowError::InvalidTransition(_))));

    let oversized = service
        .ingest_webhook_at(
            "incoming",
            "oversized",
            "2026-07-16T12:02:00Z",
            &format!("sha256={}", "0".repeat(64)),
            BTreeMap::new(),
            &[b'x'; 33],
            secret,
            received,
        )
        .await;
    assert!(matches!(
        oversized,
        Err(WorkflowError::InvalidDefinition(_))
    ));
    assert!(effects.calls().is_empty());
}

#[tokio::test]
async fn authenticated_delivery_blocks_webhook_after_definition_trust_changes() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects.clone());
    service
        .register_definition(WEBHOOK_WORKFLOW, "webhook-test")
        .expect("register workflow");
    service
        .create_webhook(
            "trust-change",
            "webhook-smoke",
            "1.0.0",
            "env:COLOSSUS_WEBHOOK_SECRET",
            300,
            4096,
            true,
        )
        .expect("create webhook");
    service
        .register_definition(
            &WEBHOOK_WORKFLOW.replace("value: { ok: true }", "value: { ok: false }"),
            "webhook-test-changed",
        )
        .expect("change workflow definition");
    let secret = b"this-secret-is-at-least-thirty-two-bytes";
    let timestamp = "2026-07-16T12:00:00Z";
    let result = service
        .ingest_webhook_at(
            "trust-change",
            "delivery-after-change",
            timestamp,
            &webhook_signature(timestamp, "delivery-after-change", br#"{}"#, secret),
            BTreeMap::new(),
            br#"{}"#,
            secret,
            parse_schedule_time(timestamp, "test clock").expect("test clock"),
        )
        .await;
    assert!(matches!(result, Err(WorkflowError::InvalidTransition(_))));
    let webhook = service.get_webhook("trust-change").expect("webhook");
    assert!(!webhook.enabled);
    assert!(webhook.blocked_reason.is_some());
    assert!(effects.calls().is_empty());
}

#[tokio::test]
async fn repository_subscription_is_policy_gated_restartable_and_duplicate_safe() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    service
        .register_definition(SUBSCRIPTION_WORKFLOW, "subscription-test")
        .expect("register workflow");
    service
        .create_subscription(
            "task-events",
            "subscription-smoke",
            "1.0.0",
            "task.created.v1",
            Some("task:"),
            true,
            Some(0),
        )
        .expect("create subscription");
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "memory:not-a-task-stream".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"title": "filtered by stream prefix"}),
        })
        .expect("append wrong-stream event");
    let source = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:alpha".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext {
                correlation_id: "source-correlation".into(),
                ..ExecutionContext::default()
            },
            payload: json!({"title": "Review durable delivery"}),
        })
        .expect("append source event");

    effects.fail("workflow.subscription.dispatch", 1);
    let deferred = service
        .tick_subscriptions_now()
        .await
        .expect("defer refused dispatch");
    assert_eq!(deferred.len(), 1);
    assert_eq!(
        deferred[0].status,
        WorkflowSubscriptionDispatchStatus::Deferred
    );
    assert_eq!(
        deferred[0].source_event_id.as_deref(),
        Some(source.event_id.as_str())
    );
    let pending = service
        .get_subscription("task-events")
        .expect("pending subscription");
    assert!(pending.enabled);
    assert_eq!(pending.checkpoint, 0);
    assert!(service.list_runs(10).expect("pending runs").is_empty());

    let dispatches = service
        .tick_subscriptions_now()
        .await
        .expect("tick subscriptions");
    assert_eq!(dispatches.len(), 1);
    let dispatch = &dispatches[0];
    assert_eq!(dispatch.status, WorkflowSubscriptionDispatchStatus::Queued);
    assert_eq!(dispatch.checkpoint, source.global_sequence);
    assert_eq!(
        dispatch.source_event_id.as_deref(),
        Some(source.event_id.as_str())
    );
    let run_id = dispatch.run_id.clone().expect("subscription run");
    let run = service.get_run(&run_id).expect("queued run");
    assert_eq!(run.status, WorkflowStatus::Queued);
    assert_eq!(run.trigger_kind, Some(WorkflowTriggerKind::Subscription));
    assert_eq!(run.trigger_id.as_deref(), Some("task-events"));
    assert_eq!(
        run.trigger_occurrence.as_deref(),
        Some(source.event_id.as_str())
    );
    assert_eq!(
        run.inputs["event"]["payload"]["title"],
        "Review durable delivery"
    );
    assert_eq!(
        repository
            .subscription_delivery("task-events", &source.event_id)
            .expect("delivery lookup")
            .expect("delivery")
            .run_id,
        run_id
    );
    let calls = effects.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].action, "workflow.subscription.dispatch");
    assert_eq!(
        calls[1].content["event"]["payload"]["title"],
        "Review durable delivery"
    );

    let events = journal.read_global(1, usize::MAX).expect("events");
    let delivered = events
        .iter()
        .position(|event| event.event_type == "workflow.subscription.delivered.v1")
        .expect("subscription transition");
    assert_eq!(
        events
            .get(delivered + 1)
            .map(|event| event.event_type.as_str()),
        Some("workflow.subscription.delivery.accepted.v1")
    );
    assert_eq!(
        events
            .get(delivered + 2)
            .map(|event| event.event_type.as_str()),
        Some("workflow.run.queued.v1")
    );

    let mut rewound = service
        .get_subscription("task-events")
        .expect("subscription");
    rewound.checkpoint = 0;
    rewound.last_event_id = None;
    rewound.last_run_id = None;
    let stream = super::subscription_stream("task-events");
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: stream.clone(),
            expected_stream_version: u64::try_from(
                journal
                    .read_stream(&stream)
                    .expect("subscription stream")
                    .len(),
            )
            .expect("stream version"),
            classification: EventClassification::Workflow,
            event_type: "workflow.subscription.test_rewound.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "at-least-once-test".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"record": rewound}),
        })
        .expect("simulate stale consumer checkpoint");
    let duplicate = service
        .tick_subscriptions_now()
        .await
        .expect("redeliver source event");
    assert_eq!(duplicate.len(), 1);
    assert_eq!(
        duplicate[0].status,
        WorkflowSubscriptionDispatchStatus::Duplicate
    );
    assert_eq!(duplicate[0].run_id.as_deref(), Some(run_id.as_str()));
    assert_eq!(service.list_runs(10).expect("runs").len(), 1);
    assert_eq!(
        effects.calls().len(),
        2,
        "duplicate bypasses policy and queueing"
    );

    let reopened = WorkflowService::new(
        Arc::clone(&journal),
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal))),
        Arc::new(RecordingEffects::default()),
    );
    assert!(
        reopened
            .tick_subscriptions_now()
            .await
            .expect("restart tick")
            .is_empty()
    );
    let completed = reopened.drain().await.expect("drain subscription run");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn subscription_checkpoints_unmatched_events_and_resumes_after_reopen() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    service
        .register_definition(SUBSCRIPTION_WORKFLOW, "subscription-test")
        .expect("register workflow");
    let historical = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:historical".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"title": "must not replay by default"}),
        })
        .expect("append historical event");
    service
        .create_subscription(
            "future-tasks",
            "subscription-smoke",
            "1.0.0",
            "task.created.v1",
            None,
            true,
            None,
        )
        .expect("create subscription");
    assert_eq!(
        service
            .get_subscription("future-tasks")
            .expect("default checkpoint")
            .checkpoint,
        historical.global_sequence
    );
    let unmatched = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "memory:alpha".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "memory.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"text": "not a task"}),
        })
        .expect("append unmatched event");
    let checkpoint = service
        .tick_subscriptions_now()
        .await
        .expect("checkpoint unmatched event");
    assert_eq!(checkpoint.len(), 1);
    assert_eq!(
        checkpoint[0].status,
        WorkflowSubscriptionDispatchStatus::Checkpointed
    );
    assert_eq!(checkpoint[0].checkpoint, unmatched.global_sequence);
    assert!(effects.calls().is_empty());

    let reopened = WorkflowService::new(
        Arc::clone(&journal),
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal))),
        Arc::new(RecordingEffects::default()),
    );
    assert_eq!(
        reopened
            .get_subscription("future-tasks")
            .expect("reopened subscription")
            .checkpoint,
        unmatched.global_sequence
    );
    let source = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:beta".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"title": "deliver after restart"}),
        })
        .expect("append matching event");
    let dispatch = reopened
        .tick_subscriptions_now()
        .await
        .expect("dispatch after restart");
    assert_eq!(dispatch.len(), 1);
    assert_eq!(
        dispatch[0].status,
        WorkflowSubscriptionDispatchStatus::Queued
    );
    assert_eq!(dispatch[0].checkpoint, source.global_sequence);
}

#[tokio::test]
async fn subscription_schema_and_trust_failures_block_without_consuming_source() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    service
        .register_definition(SUBSCRIPTION_WORKFLOW, "subscription-test")
        .expect("register workflow");
    for invalid_event_type in [
        "task.created",
        "task.created.v",
        "task.created.V1",
        "workflow.run.queued.v1",
    ] {
        assert!(
            service
                .create_subscription(
                    "invalid-source",
                    "subscription-smoke",
                    "1.0.0",
                    invalid_event_type,
                    None,
                    true,
                    Some(0),
                )
                .is_err(),
            "invalid source event type {invalid_event_type} must be rejected"
        );
    }

    service
        .create_subscription(
            "schema-block",
            "subscription-smoke",
            "1.0.0",
            "task.created.v1",
            None,
            true,
            Some(0),
        )
        .expect("create schema subscription");
    let invalid_source = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:invalid-payload".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"not_title": true}),
        })
        .expect("append schema-invalid source");
    let schema_blocked = service
        .tick_subscriptions_now()
        .await
        .expect("evaluate schema-invalid source");
    assert_eq!(schema_blocked.len(), 1);
    assert_eq!(
        schema_blocked[0].status,
        WorkflowSubscriptionDispatchStatus::Blocked
    );
    assert_eq!(
        schema_blocked[0].source_event_id.as_deref(),
        Some(invalid_source.event_id.as_str())
    );
    let schema_subscription = service
        .get_subscription("schema-block")
        .expect("schema-blocked subscription");
    assert!(!schema_subscription.enabled);
    assert_eq!(schema_subscription.checkpoint, 0);

    service
        .create_subscription(
            "trust-block",
            "subscription-smoke",
            "1.0.0",
            "task.created.v1",
            None,
            true,
            None,
        )
        .expect("create trust subscription");
    let trust_checkpoint = service
        .get_subscription("trust-block")
        .expect("trust subscription")
        .checkpoint;
    service
        .register_definition(
            &SUBSCRIPTION_WORKFLOW.replace("value: { ok: true }", "value: { ok: false }"),
            "subscription-test-changed",
        )
        .expect("change pinned workflow");
    let trusted_shape_source = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:changed-definition".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"title": "definition no longer trusted"}),
        })
        .expect("append trust-invalid source");
    let trust_blocked = service
        .tick_subscriptions_now()
        .await
        .expect("evaluate trust-invalid source");
    assert_eq!(trust_blocked.len(), 1);
    assert_eq!(
        trust_blocked[0].status,
        WorkflowSubscriptionDispatchStatus::Blocked
    );
    assert_eq!(
        trust_blocked[0].source_event_id.as_deref(),
        Some(trusted_shape_source.event_id.as_str())
    );
    let trust_subscription = service
        .get_subscription("trust-block")
        .expect("trust-blocked subscription");
    assert!(!trust_subscription.enabled);
    assert_eq!(trust_subscription.checkpoint, trust_checkpoint);
    assert!(effects.calls().is_empty());
}

#[tokio::test]
async fn deferred_subscription_does_not_starve_later_subscriptions() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(Arc::clone(&journal), repository, effects.clone());
    service
        .register_definition(SUBSCRIPTION_WORKFLOW, "subscription-test")
        .expect("register workflow");
    for subscription_id in ["a-deferred", "z-ready"] {
        service
            .create_subscription(
                subscription_id,
                "subscription-smoke",
                "1.0.0",
                "task.created.v1",
                None,
                true,
                Some(0),
            )
            .expect("create subscription");
    }
    let source = journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "task:shared".into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "task.created.v1".into(),
            actor: Actor {
                actor_type: ActorType::User,
                id: "tester".into(),
            },
            context: ExecutionContext::default(),
            payload: json!({"title": "continue after a deferred dispatch"}),
        })
        .expect("append source event");
    effects.fail("workflow.subscription.dispatch", 1);

    let outcomes = service
        .tick_subscriptions_now()
        .await
        .expect("evaluate both subscriptions");
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].subscription_id, "a-deferred");
    assert_eq!(
        outcomes[0].status,
        WorkflowSubscriptionDispatchStatus::Deferred
    );
    assert_eq!(outcomes[0].checkpoint, 0);
    assert_eq!(outcomes[1].subscription_id, "z-ready");
    assert_eq!(
        outcomes[1].status,
        WorkflowSubscriptionDispatchStatus::Queued
    );
    assert_eq!(outcomes[1].checkpoint, source.global_sequence);
    assert_eq!(service.list_runs(10).expect("queued runs").len(), 1);
    assert_eq!(effects.calls().len(), 2);
}

#[test]
fn skip_schedule_drops_backlog_but_fires_the_next_single_occurrence() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        repository,
        Arc::new(DenyWorkflowEffects),
    );
    service
        .register_definition(SIMPLE, "schedule-test")
        .expect("register workflow");
    service
        .create_schedule_at(
            "skip-backlog",
            "smoke",
            "1.0.0",
            json!({"message": "scheduled"}),
            60,
            WorkflowScheduleMisfirePolicy::Skip,
            true,
            Some("2026-01-01T12:00:00Z"),
            parse_schedule_time("2026-01-01T11:59:00Z", "test clock").expect("test clock"),
        )
        .expect("create schedule");

    let skipped = service
        .tick_schedules_at("2026-01-01T12:03:10Z")
        .expect("skip backlog");
    assert_eq!(skipped[0].status, WorkflowScheduleDispatchStatus::Skipped);
    assert_eq!(skipped[0].missed_occurrences, 4);
    assert!(skipped[0].run_id.is_none());
    assert!(service.list_runs(10).expect("runs").is_empty());
    assert_eq!(
        service
            .get_schedule("skip-backlog")
            .expect("schedule")
            .next_fire_at,
        "2026-01-01T12:04:00Z"
    );

    let due = service
        .tick_schedules_at("2026-01-01T12:04:30Z")
        .expect("single due occurrence");
    assert_eq!(due[0].status, WorkflowScheduleDispatchStatus::Queued);
    assert_eq!(due[0].missed_occurrences, 0);
    assert_eq!(due[0].scheduled_at.as_deref(), Some("2026-01-01T12:04:00Z"));
}

#[test]
fn changed_definition_blocks_and_disables_due_schedule() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        repository,
        Arc::new(DenyWorkflowEffects),
    );
    service
        .register_definition(SIMPLE, "schedule-test")
        .expect("register workflow");
    service
        .create_schedule_at(
            "trust-pinned",
            "smoke",
            "1.0.0",
            json!({"message": "scheduled"}),
            60,
            WorkflowScheduleMisfirePolicy::FireOnce,
            true,
            Some("2026-01-01T12:00:00Z"),
            parse_schedule_time("2026-01-01T11:59:00Z", "test clock").expect("test clock"),
        )
        .expect("create schedule");
    service
        .register_definition(
            &SIMPLE.replace(
                "Offline smoke workflow",
                "Changed workflow invalidates schedule trust",
            ),
            "schedule-test-change",
        )
        .expect("change definition");

    let blocked = service
        .tick_schedules_at("2026-01-01T12:00:01Z")
        .expect("tick changed schedule");
    assert_eq!(blocked[0].status, WorkflowScheduleDispatchStatus::Blocked);
    assert!(blocked[0].run_id.is_none());
    let schedule = service.get_schedule("trust-pinned").expect("schedule");
    assert!(!schedule.enabled);
    assert_eq!(
        schedule.blocked_reason.as_deref(),
        Some("pinned workflow definition hash changed")
    );
    assert!(service.list_runs(10).expect("runs").is_empty());
    assert!(
        service.set_schedule_enabled("trust-pinned", true).is_err(),
        "an operator cannot re-enable a schedule whose pinned definition changed"
    );
}

#[test]
fn schedule_validation_rejects_unbounded_cadence_and_non_utc_start() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    service
        .register_definition(SIMPLE, "schedule-test")
        .expect("register workflow");
    let now = parse_schedule_time("2026-01-01T11:59:00Z", "test clock").expect("test clock");
    assert!(
        service
            .create_schedule_at(
                "too-fast",
                "smoke",
                "1.0.0",
                json!({"message": "scheduled"}),
                59,
                WorkflowScheduleMisfirePolicy::FireOnce,
                true,
                None,
                now,
            )
            .is_err()
    );
    assert!(
        service
            .create_schedule_at(
                "not-utc",
                "smoke",
                "1.0.0",
                json!({"message": "scheduled"}),
                60,
                WorkflowScheduleMisfirePolicy::FireOnce,
                true,
                Some("2026-01-01T12:00:00-05:00"),
                now,
            )
            .is_err()
    );
}

const WAITING: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: waiting
  version: 1.0.0
  description: Input wait workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: wait_for_input
    id: answer
    prompt: Supply an answer
    schema: { type: string }
  - type: emit
    id: done
    value: { ok: true }
"#;

const PARALLEL: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: parallel
  version: 1.0.0
  description: Bounded parallel workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 2
stepBudget: 4
steps:
  - type: parallel
    id: branches
    max_concurrency: 2
    branches:
      - [{ type: emit, id: left, value: 1 }]
      - [{ type: emit, id: right, value: 2 }]
"#;

#[derive(Default)]
struct RecordingEffects {
    calls: Mutex<Vec<WorkflowEffect>>,
    failures: Mutex<BTreeMap<String, usize>>,
}

impl RecordingEffects {
    fn fail(&self, action: &str, times: usize) {
        self.failures
            .lock()
            .expect("failures")
            .insert(action.into(), times);
    }

    fn calls(&self) -> Vec<WorkflowEffect> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl WorkflowEffectRunner for RecordingEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<serde_json::Value, WorkflowError> {
        self.calls.lock().expect("calls").push(effect.clone());
        let mut failures = self.failures.lock().expect("failures");
        if let Some(remaining) = failures.get_mut(&effect.action)
            && *remaining > 0
        {
            *remaining -= 1;
            return Err(WorkflowError::Effect(format!(
                "injected failure for {}",
                effect.action
            )));
        }
        Ok(json!({"action": effect.action, "compensation": effect.compensation}))
    }
}

struct FileKeyProvider {
    anchor: PathBuf,
}

impl KeyProvider for FileKeyProvider {
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError> {
        Ok(("workflow-process-kill-key".into(), [31_u8; 32]))
    }

    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError> {
        if key_id == "workflow-process-kill-key" {
            Ok([31_u8; 32])
        } else {
            Err(StoreError::KeyUnavailable(key_id.into()))
        }
    }

    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError> {
        fs::write(
            &self.anchor,
            serde_json::to_vec(&json!({"sequence": sequence, "hash": hash}))
                .map_err(|error| StoreError::Adapter(error.to_string()))?,
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }

    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError> {
        if !self.anchor.exists() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(&self.anchor).map_err(|error| StoreError::Adapter(error.to_string()))?,
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let sequence = value
            .get("sequence")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| StoreError::Verification("test anchor sequence is absent".into()))?;
        let hash = value
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| StoreError::Verification("test anchor hash is absent".into()))?;
        Ok(Some((sequence, hash.into())))
    }
}

struct KillAfterDurableMarkerEffects {
    marker: PathBuf,
    fail_primary: bool,
    pass_workflow_start: bool,
}

struct ReturnAfterDurableMarkerEffects {
    marker: PathBuf,
}

struct CrashAfterEventJournal {
    inner: Arc<dyn EventJournal>,
    event_type: &'static str,
}

impl CrashAfterEventJournal {
    fn terminate_if_target(&self, target: bool) {
        if target {
            std::process::abort();
        }
    }
}

impl EventJournal for CrashAfterEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        let target = event.event_type == self.event_type;
        let appended = self.inner.append(event)?;
        self.terminate_if_target(target);
        Ok(appended)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let target = events
            .iter()
            .any(|event| event.event_type == self.event_type);
        let appended = self.inner.append_batch(events)?;
        self.terminate_if_target(target);
        Ok(appended)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream(stream_id)
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_from(stream_id, after_version, limit)
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner
            .read_stream_backwards(stream_id, before_version, limit)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_global(from_sequence, limit)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from_sequence, limit)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.inner.head()
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<serde_json::Value, StoreError> {
        self.inner.decrypt_payload(event)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.inner.verify()
    }

    fn is_recovery_mode(&self) -> bool {
        self.inner.is_recovery_mode()
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.inner.checkpoint()
    }
}

#[async_trait]
impl WorkflowEffectRunner for KillAfterDurableMarkerEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<serde_json::Value, WorkflowError> {
        if self.pass_workflow_start && effect.action == "workflow.start" {
            return Ok(json!({"authorized": "workflow.start"}));
        }
        if self.fail_primary && !effect.compensation {
            return Err(WorkflowError::Effect(
                "known primary failure before compensation".into(),
            ));
        }
        let mut marker = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.marker)
            .expect("open durable external-effect marker");
        writeln!(marker, "{}:{}", effect.action, effect.attempt)
            .expect("write durable external-effect marker");
        marker.sync_all().expect("sync external-effect marker");
        std::process::abort();
    }
}

#[async_trait]
impl WorkflowEffectRunner for ReturnAfterDurableMarkerEffects {
    async fn run(&self, effect: WorkflowEffect) -> Result<serde_json::Value, WorkflowError> {
        let mut marker = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.marker)
            .expect("open durable external-effect marker");
        writeln!(marker, "{}:{}", effect.action, effect.attempt)
            .expect("write durable external-effect marker");
        marker.sync_all().expect("sync external-effect marker");
        Ok(json!({"authorized": effect.action}))
    }
}

fn process_kill_journal(root: &Path) -> Arc<dyn EventJournal> {
    Arc::new(
        RedbEventJournal::open(
            root.join("workflow.redb"),
            Arc::new(FileKeyProvider {
                anchor: root.join("workflow.anchor"),
            }),
            Arc::new(Ed25519CheckpointSigner::new(
                "workflow-process-kill-signing",
                [47_u8; 32],
            )),
        )
        .expect("open durable workflow journal"),
    )
}

fn process_kill_definition(mode: &str) -> String {
    if mode == "parallel" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable parallel-branch process-kill recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 8
steps:
  - type: parallel
    id: branches
    max_concurrency: 1
    branches:
      -
        - type: emit
          id: before
          value: { persisted: true }
      -
        - type: tool
          id: mutate
          tool: parallel.run
          arguments: {}
          idempotency: parallel-key
"#
        .into();
    }
    if mode == "nested-child" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable nested-child process-kill recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: workflow
    id: child-call
    workflow: process-kill-child
    version: 1.0.0
    inputs: {}
"#
        .into();
    }
    if mode == "subworkflow-link" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable linked-subworkflow process-kill recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: workflow
    id: child-call
    workflow: process-kill-child
    version: 1.0.0
    inputs: {}
"#
        .into();
    }
    if mode == "completed-step" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable completed-step process-kill recovery
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 4
steps:
  - type: emit
    id: durable
    value: { persisted: true }
"#
        .into();
    }
    if mode == "compensation" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable compensation process-kill recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: tool
    id: primary
    tool: primary.fail
    arguments: {}
    idempotency: null
compensation:
  - type: tool
    id: rollback
    tool: rollback.run
    arguments: {}
    idempotency: durable-rollback
"#
        .into();
    }
    format!(
        r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill
  version: 1.0.0
  description: Durable process-kill recovery
inputs: {{ type: object }}
outputs: {{ type: object }}
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: tool
    id: mutate
    tool: mutation.run
    arguments: {{}}
    idempotency: {}
"#,
        if mode == "idempotent" {
            "durable-key"
        } else {
            "null"
        }
    )
}

fn process_kill_child_definition(mode: &str) -> &'static str {
    if mode == "nested-child" {
        return r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill-child
  version: 1.0.0
  description: Durable nested effect child
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: tool
    id: nested-mutate
    tool: nested.run
    arguments: {}
    idempotency: nested-key
"#;
    }
    r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: process-kill-child
  version: 1.0.0
  description: Durable linked child
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 2
steps:
  - type: emit
    id: child-result
    value: { child: complete }
"#
}

async fn process_kill_child(root: &Path, marker: &Path, run_id: &str, mode: &str) {
    let durable_journal = process_kill_journal(root);
    let crash_event = match mode {
        "completed-step" => Some("workflow.step.completed.v1"),
        "subworkflow-link" => Some("workflow.subworkflow.linked.v1"),
        _ => None,
    };
    let journal: Arc<dyn EventJournal> = match crash_event {
        Some(event_type) => Arc::new(CrashAfterEventJournal {
            inner: durable_journal,
            event_type,
        }),
        None => durable_journal,
    };
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects: Arc<dyn WorkflowEffectRunner> = match mode {
        "completed-step" => Arc::new(DenyWorkflowEffects),
        "subworkflow-link" => Arc::new(ReturnAfterDurableMarkerEffects {
            marker: marker.into(),
        }),
        _ => Arc::new(KillAfterDurableMarkerEffects {
            marker: marker.into(),
            fail_primary: mode == "compensation",
            pass_workflow_start: mode == "nested-child",
        }),
    };
    let service = WorkflowService::new(journal, repository, effects);
    if matches!(mode, "subworkflow-link" | "nested-child") {
        service
            .register_definition(process_kill_child_definition(mode), "process-kill-test")
            .expect("register process-kill child workflow");
    }
    service
        .register_definition(&process_kill_definition(mode), "process-kill-test")
        .expect("register process-kill workflow");
    service
        .queue_run_with_lineage(
            run_id,
            "process-kill",
            "1.0.0",
            json!({}),
            None,
            None,
            None,
            1,
        )
        .expect("queue process-kill workflow");
    service
        .run_queued(run_id)
        .await
        .expect("effect runner must terminate this process");
    panic!("process-kill effect returned without terminating the child");
}

#[test]
fn strict_yaml_hashes_exact_content_and_rejects_code_fields() {
    let validated = validate_definition(SIMPLE).expect("valid");
    let with_space = validate_definition(&format!("{SIMPLE}\n")).expect("valid with space");
    assert_ne!(validated.content_hash, with_space.content_hash);
    let executable = SIMPLE.replace("value: { ok: true }", "shell: whoami");
    assert!(validate_definition(&executable).is_err());
}

#[test]
fn condition_grammar_is_non_executable_and_evaluates_json_pointers() {
    let condition =
        Condition::parse("exists(/inputs/name) && /inputs/count >= 2").expect("condition");
    assert!(condition.evaluate(&json!({"inputs":{"name":"a","count":2}})));
    assert!(Condition::parse("system(\"whoami\")").is_err());
}

#[test]
fn condition_grammar_rejects_unbounded_input_tokens_and_recursion() {
    assert!(Condition::parse(&" ".repeat(MAX_CONDITION_BYTES + 1)).is_err());
    assert!(Condition::parse(&")".repeat(MAX_CONDITION_TOKENS + 1)).is_err());
    let excessive_not = format!("{}exists(/a)", "!".repeat(MAX_CONDITION_DEPTH + 1));
    assert!(Condition::parse(&excessive_not).is_err());
    let excessive_parentheses = format!(
        "{}exists(/a){}",
        "(".repeat(MAX_CONDITION_DEPTH + 1),
        ")".repeat(MAX_CONDITION_DEPTH + 1)
    );
    assert!(Condition::parse(&excessive_parentheses).is_err());
    let excessive_boolean = std::iter::repeat_n("exists(/a)", MAX_CONDITION_DEPTH + 2)
        .collect::<Vec<_>>()
        .join(" && ");
    assert!(Condition::parse(&excessive_boolean).is_err());
}

#[tokio::test]
async fn event_sourced_run_completes_and_definition_change_invalidates_trust() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        Arc::new(DenyWorkflowEffects),
    );
    let registered = service
        .register_definition(SIMPLE, "repo:.colossus/workflows/smoke.yaml")
        .expect("register");
    let run = service
        .start_run("smoke", "1.0.0", json!({"message":"hello"}))
        .await
        .expect("run");
    assert_eq!(run.status, colossus_contracts::WorkflowStatus::Completed);
    assert_eq!(run.workflow_hash, registered.content_hash);

    service
        .register_definition(
            &SIMPLE.replace("Offline smoke workflow", "Changed workflow"),
            "repo:.colossus/workflows/smoke.yaml",
        )
        .expect("changed definition");
    let events = journal
        .read_stream("workflow-definition:smoke:1.0.0")
        .expect("events");
    assert_eq!(
        events.last().expect("last").event_type,
        "workflow.definition.changed.v1"
    );
}

#[tokio::test]
async fn input_completes_waiting_step_before_resume() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    service
        .register_definition(WAITING, "test")
        .expect("register");
    let waiting = service
        .start_run("waiting", "1.0.0", json!({}))
        .await
        .expect("start");
    assert_eq!(waiting.status, colossus_contracts::WorkflowStatus::Waiting);
    let completed = service
        .provide_input(&waiting.run_id, json!("accepted"))
        .await
        .expect("input");
    assert_eq!(
        completed.status,
        colossus_contracts::WorkflowStatus::Completed
    );
    assert_eq!(completed.completed_steps, 2);
}

#[tokio::test]
async fn nested_input_wait_resumes_without_repeating_the_completed_wait() {
    const NESTED_WAIT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: nested-wait
  version: 1.0.0
  description: Nested input wait
inputs:
  type: object
  required: [ask]
  properties: { ask: { type: boolean } }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 6
steps:
  - type: emit
    id: before
    value: { retained: true }
  - type: condition
    id: branch
    expression: /inputs/ask == true
    then:
      - type: wait_for_input
        id: nested-answer
        prompt: Supply nested input
        schema: { type: string }
      - type: emit
        id: nested-done
        value: { ok: true }
    otherwise:
      - type: emit
        id: skipped
        value: { ok: false }
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        repository,
        Arc::new(DenyWorkflowEffects),
    );
    service
        .register_definition(NESTED_WAIT, "test")
        .expect("register");
    let waiting = service
        .start_run("nested-wait", "1.0.0", json!({"ask": true}))
        .await
        .expect("start");
    assert_eq!(waiting.status, colossus_contracts::WorkflowStatus::Waiting);
    let completed = service
        .provide_input(&waiting.run_id, json!("accepted"))
        .await
        .expect("nested input");
    assert_eq!(
        completed.status,
        colossus_contracts::WorkflowStatus::Completed
    );
    let outputs = completed.outputs.expect("outputs");
    assert_eq!(outputs["before"], json!({"retained": true}));
    assert_eq!(outputs["nested-answer"], json!("accepted"));
    let events = journal
        .read_stream(&format!("workflow-run:{}", waiting.run_id))
        .expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "workflow.input.provided.v1")
            .count(),
        1
    );
}

#[tokio::test]
async fn foreach_inputs_resume_with_distinct_iteration_scopes() {
    const ITERATIVE: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: iterative-input
  version: 1.0.0
  description: Iteration scoped input
inputs:
  type: object
  required: [items]
  properties:
    items: { type: array, items: { type: string } }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 10
steps:
  - type: foreach
    id: each
    items: /inputs/items
    max_items: 2
    steps:
      - type: wait_for_input
        id: answer
        prompt: Answer for this item
        schema: { type: string }
      - type: emit
        id: done
        value: { ok: true }
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    service
        .register_definition(ITERATIVE, "test")
        .expect("register");
    let first_wait = service
        .start_run(
            "iterative-input",
            "1.0.0",
            json!({"items":["left", "right"]}),
        )
        .await
        .expect("start");
    assert_eq!(
        first_wait.waiting_execution_id.as_deref(),
        Some("each[0]/answer")
    );
    let second_wait = service
        .provide_input(&first_wait.run_id, json!("first"))
        .await
        .expect("first input");
    assert_eq!(
        second_wait.status,
        colossus_contracts::WorkflowStatus::Waiting
    );
    assert_eq!(
        second_wait.waiting_execution_id.as_deref(),
        Some("each[1]/answer")
    );
    let completed = service
        .provide_input(&first_wait.run_id, json!("second"))
        .await
        .expect("second input");
    assert_eq!(
        completed.status,
        colossus_contracts::WorkflowStatus::Completed
    );
    let iterations = completed.outputs.expect("outputs")["each"]
        .as_array()
        .expect("iterations")
        .clone();
    assert_eq!(iterations[0]["steps"]["answer"], json!("first"));
    assert_eq!(iterations[1]["steps"]["answer"], json!("second"));
}

#[tokio::test]
async fn foreach_effects_receive_distinct_execution_and_idempotency_identity() {
    const ITERATIVE: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: iterative-effects
  version: 1.0.0
  description: Iteration scoped effects
inputs:
  type: object
  required: [items]
  properties: { items: { type: array } }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: foreach
    id: each
    items: /inputs/items
    max_items: 2
    steps:
      - type: tool
        id: call
        tool: iterative.call
        arguments: {}
        idempotency: per-item
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects.clone());
    service
        .register_definition(ITERATIVE, "test")
        .expect("register");
    let run = service
        .start_run("iterative-effects", "1.0.0", json!({"items":[1, 2]}))
        .await
        .expect("run");
    assert_eq!(run.status, colossus_contracts::WorkflowStatus::Completed);
    let calls = effects.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].definition_step_id, "call");
    assert_eq!(calls[1].definition_step_id, "call");
    assert_eq!(calls[0].step_id, "each[0]/call");
    assert_eq!(calls[1].step_id, "each[1]/call");
    assert_ne!(calls[0].idempotency, calls[1].idempotency);
}

#[tokio::test]
async fn subworkflow_runs_are_hash_pinned_linked_and_completed_once() {
    const CHILD: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: child
  version: 1.0.0
  description: Child workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 2
steps:
  - type: emit
    id: child-result
    value: { child: true }
"#;
    const PARENT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: parent
  version: 1.0.0
  description: Parent workflow
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 3
steps:
  - type: workflow
    id: launch-child
    workflow: child
    version: 1.0.0
    inputs: {}
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects.clone());
    service.register_definition(CHILD, "test").expect("child");
    service.register_definition(PARENT, "test").expect("parent");
    let parent = service
        .start_run("parent", "1.0.0", json!({}))
        .await
        .expect("parent run");
    assert_eq!(parent.status, colossus_contracts::WorkflowStatus::Completed);
    let runs = service.list_runs(10).expect("runs");
    assert_eq!(runs.len(), 2);
    let child = runs
        .iter()
        .find(|run| run.parent_run_id.as_deref() == Some(&parent.run_id))
        .expect("linked child");
    assert_eq!(child.parent_step_id.as_deref(), Some("launch-child"));
    assert_eq!(child.call_depth, 2);
    assert_eq!(child.status, colossus_contracts::WorkflowStatus::Completed);
    assert_eq!(
        effects
            .calls()
            .iter()
            .filter(|call| call.action == "workflow.start")
            .count(),
        1
    );
    assert_eq!(
        parent.outputs.expect("parent outputs")["launch-child"]["outputs"]["child-result"],
        json!({"child": true})
    );
}

#[tokio::test]
async fn waiting_child_resumes_parent_without_duplicate_launch() {
    const CHILD: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: input-child
  version: 1.0.0
  description: Waiting child workflow
inputs: { type: object }
outputs: { type: object }
capabilities: []
maxConcurrency: 1
stepBudget: 3
steps:
  - type: wait_for_input
    id: child-input
    prompt: Child input required
    schema: { type: string }
  - type: emit
    id: child-done
    value: { done: true }
"#;
    const PARENT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: waiting-parent
  version: 1.0.0
  description: Parent waiting on child
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 4
steps:
  - type: workflow
    id: child-call
    workflow: input-child
    version: 1.0.0
    inputs: {}
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects.clone());
    service.register_definition(CHILD, "test").expect("child");
    service.register_definition(PARENT, "test").expect("parent");
    let waiting_parent = service
        .start_run("waiting-parent", "1.0.0", json!({}))
        .await
        .expect("parent run");
    assert_eq!(
        waiting_parent.status,
        colossus_contracts::WorkflowStatus::Waiting
    );
    let child_run_id = waiting_parent
        .waiting_child_run_id
        .clone()
        .expect("visible child id");
    let child = service.get_run(&child_run_id).expect("child run");
    assert_eq!(child.status, colossus_contracts::WorkflowStatus::Waiting);
    service
        .provide_input(&child_run_id, json!("ready"))
        .await
        .expect("child input");
    let completed_parent = service
        .resume_run(&waiting_parent.run_id)
        .await
        .expect("parent resume");
    assert_eq!(
        completed_parent.status,
        colossus_contracts::WorkflowStatus::Completed
    );
    assert_eq!(service.list_runs(10).expect("runs").len(), 2);
    assert_eq!(
        effects
            .calls()
            .iter()
            .filter(|call| call.action == "workflow.start")
            .count(),
        1
    );
    let second_parent = service
        .start_run("waiting-parent", "1.0.0", json!({}))
        .await
        .expect("second parent");
    let second_child_id = second_parent
        .waiting_child_run_id
        .clone()
        .expect("second child");
    let cancelled_parent = service
        .cancel_run(&second_parent.run_id)
        .expect("cancel parent");
    assert_eq!(
        cancelled_parent.status,
        colossus_contracts::WorkflowStatus::Cancelled
    );
    assert_eq!(
        service
            .get_run(&second_child_id)
            .expect("cancelled child")
            .status,
        colossus_contracts::WorkflowStatus::Cancelled
    );
}

#[tokio::test]
async fn restart_recreates_linked_child_with_the_original_run_id() {
    let child = SIMPLE
        .replace("name: smoke", "name: orphan-child")
        .replace("Offline smoke workflow", "Orphan child");
    let parent = SIMPLE
            .replace("name: smoke", "name: orphan-parent")
            .replace("Offline smoke workflow", "Orphan parent")
            .replace(
                "- type: emit\n    id: result\n    value: { ok: true }",
                "- type: workflow\n    id: child-call\n    workflow: orphan-child\n    version: 1.0.0\n    inputs: { message: child }",
            );
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(journal, repository, effects);
    service.register_definition(&child, "test").expect("child");
    service
        .register_definition(&parent, "test")
        .expect("parent");
    let queued = service
        .queue_run("orphan-parent", "1.0.0", json!({"message":"parent"}))
        .expect("queue");
    service
        .append_run_event(&queued.run_id, "workflow.run.started.v1", json!({}))
        .expect("claim");
    service
        .append_run_event(
            &queued.run_id,
            "workflow.step.started.v1",
            json!({"step_id":"child-call", "attempt":1}),
        )
        .expect("step start");
    let original_child_id = uuid::Uuid::now_v7().to_string();
    service
        .append_run_event(
            &queued.run_id,
            "workflow.subworkflow.linked.v1",
            json!({
                "step_id": "child-call",
                "child_run_id": original_child_id,
                "workflow_name": "orphan-child",
                "workflow_version": "1.0.0",
                "inputs": {"message":"child"},
                "call_depth": 2,
            }),
        )
        .expect("durable intent");
    service.recover_interrupted().expect("recover parent");
    let completed = service
        .resume_run(&queued.run_id)
        .await
        .expect("resume parent");
    assert_eq!(
        completed.status,
        colossus_contracts::WorkflowStatus::Completed
    );
    let recreated = service.get_run(&original_child_id).expect("same child id");
    assert_eq!(
        recreated.parent_run_id.as_deref(),
        Some(queued.run_id.as_str())
    );
    assert_eq!(service.list_runs(10).expect("runs").len(), 2);
}

#[tokio::test]
async fn child_terminal_failure_fails_the_parent_without_relaunch() {
    const CHILD: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: failing-child
  version: 1.0.0
  description: Failing child
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 2
steps:
  - type: tool
    id: fail
    tool: child.fail
    arguments: {}
    idempotency: null
"#;
    const PARENT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: failure-parent
  version: 1.0.0
  description: Failure parent
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 3
steps:
  - type: workflow
    id: child-call
    workflow: failing-child
    version: 1.0.0
    inputs: {}
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    effects.fail("child.fail", 1);
    let service = WorkflowService::new(journal, repository, effects.clone());
    service.register_definition(CHILD, "test").expect("child");
    service.register_definition(PARENT, "test").expect("parent");
    let parent = service
        .start_run("failure-parent", "1.0.0", json!({}))
        .await
        .expect("parent run");
    assert_eq!(parent.status, colossus_contracts::WorkflowStatus::Failed);
    let child = service
        .list_runs(10)
        .expect("runs")
        .into_iter()
        .find(|run| run.parent_run_id.as_deref() == Some(parent.run_id.as_str()))
        .expect("child");
    assert_eq!(child.status, colossus_contracts::WorkflowStatus::Failed);
    assert_eq!(
        effects
            .calls()
            .iter()
            .filter(|call| call.action == "workflow.start")
            .count(),
        1
    );
}

#[tokio::test]
async fn parallel_step_serializes_durable_events_without_losing_concurrency_bounds() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    service
        .register_definition(PARALLEL, "test")
        .expect("register");
    let run = service
        .start_run("parallel", "1.0.0", json!({}))
        .await
        .expect("parallel run");
    assert_eq!(run.status, colossus_contracts::WorkflowStatus::Completed);
}

#[test]
fn direct_recursive_workflow_is_rejected() {
    let recursive = SIMPLE.replace(
            "- type: emit\n    id: result\n    value: { ok: true }",
            "- type: workflow\n    id: recurse\n    workflow: smoke\n    version: 1.0.0\n    inputs: {}",
        );
    assert!(validate_definition(&recursive).is_err());
}

#[tokio::test]
async fn queued_runs_are_claimed_only_by_start_or_drain() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    service
        .register_definition(SIMPLE, "test")
        .expect("register");
    let queued = service
        .queue_run("smoke", "1.0.0", json!({"message":"queued"}))
        .expect("queue");
    assert_eq!(queued.status, colossus_contracts::WorkflowStatus::Queued);
    let drained = service.drain().await.expect("drain");
    assert_eq!(drained.len(), 1);
    assert_eq!(
        drained[0].status,
        colossus_contracts::WorkflowStatus::Completed
    );
    assert!(service.drain().await.expect("empty drain").is_empty());
}

#[test]
fn indirect_cycles_and_excessive_call_depth_are_rejected() {
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
    let definition = |name: &str, target: &str| {
        format!(
            "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: {name}\n  version: 1.0.0\n  description: graph node\ninputs: {{ type: object }}\noutputs: {{ type: object }}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 4\nsteps:\n  - type: workflow\n    id: call-{target}\n    workflow: {target}\n    version: 1.0.0\n    inputs: {{}}\n"
        )
    };
    service
        .register_definition(&definition("alpha", "beta"), "test")
        .expect("forward reference");
    let cycle = service
        .register_definition(&definition("beta", "alpha"), "test")
        .expect_err("indirect cycle");
    assert!(cycle.to_string().contains("cycle detected"));

    let leaf = SIMPLE
        .replace("name: smoke", "name: node16")
        .replace("Offline smoke workflow", "graph leaf");
    service.register_definition(&leaf, "test").expect("leaf");
    for index in (0..16).rev() {
        let name = format!("node{index}");
        let target = format!("node{}", index + 1);
        let result = service.register_definition(&definition(&name, &target), "test");
        if index == 0 {
            assert!(
                result
                    .expect_err("depth limit")
                    .to_string()
                    .contains("call depth exceeds")
            );
        } else {
            result.expect("bounded graph node");
        }
    }
}

#[tokio::test]
async fn idempotent_effects_retry_and_compensation_is_separately_dispatched() {
    const COMPENSATING: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: compensating
  version: 1.0.0
  description: Retry and compensate
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 8
steps:
  - type: tool
    id: primary
    tool: primary.fail
    arguments: { value: 1 }
    idempotency: primary-key
compensation:
  - type: tool
    id: rollback
    tool: rollback.run
    arguments: { value: 1 }
    idempotency: rollback-key
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    effects.fail("primary.fail", 2);
    let service = WorkflowService::new(journal, repository, effects.clone());
    service
        .register_definition(COMPENSATING, "test")
        .expect("register");
    let run = service
        .start_run("compensating", "1.0.0", json!({}))
        .await
        .expect("run");
    assert_eq!(run.status, colossus_contracts::WorkflowStatus::Failed);
    let calls = effects.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.action == "primary.fail")
            .count(),
        2
    );
    let rollback = calls
        .iter()
        .find(|call| call.action == "rollback.run")
        .expect("rollback call");
    assert!(rollback.compensation);
    assert_ne!(rollback.step_id, calls[0].step_id);
}

#[tokio::test]
async fn process_kill_after_schedule_batch_recovers_without_duplicate_run() {
    const CHILD_ENV: &str = "COLOSSUS_WORKFLOW_SCHEDULE_PROCESS_KILL_CHILD";
    const ROOT_ENV: &str = "COLOSSUS_WORKFLOW_SCHEDULE_PROCESS_KILL_ROOT";
    const SCHEDULE_ID: &str = "process-kill-schedule";
    const OCCURRENCE: &str = "2026-01-01T12:00:00Z";

    if std::env::var_os(CHILD_ENV).is_some() {
        let root =
            PathBuf::from(std::env::var_os(ROOT_ENV).expect("schedule process-kill child root"));
        let durable_journal = process_kill_journal(&root);
        let repository: Arc<dyn WorkflowRepository> = Arc::new(
            EventSourcedWorkflowRepository::new(Arc::clone(&durable_journal)),
        );
        let service = WorkflowService::new(
            Arc::clone(&durable_journal),
            repository,
            Arc::new(DenyWorkflowEffects),
        );
        service
            .register_definition(SIMPLE, "schedule-process-kill-test")
            .expect("register scheduled workflow");
        service
            .create_schedule_at(
                SCHEDULE_ID,
                "smoke",
                "1.0.0",
                json!({"message": "scheduled"}),
                60,
                WorkflowScheduleMisfirePolicy::FireOnce,
                true,
                Some(OCCURRENCE),
                parse_schedule_time("2026-01-01T11:59:00Z", "test clock").expect("test clock"),
            )
            .expect("create schedule");
        drop(service);

        let journal: Arc<dyn EventJournal> = Arc::new(CrashAfterEventJournal {
            inner: durable_journal,
            event_type: "workflow.schedule.fired.v1",
        });
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service = WorkflowService::new(journal, repository, Arc::new(DenyWorkflowEffects));
        service
            .tick_schedules_at("2026-01-01T12:00:01Z")
            .expect("schedule batch must terminate this process after commit");
        panic!("schedule process-kill child returned without terminating");
    }

    let directory = tempdir().expect("schedule process-kill directory");
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "tests::process_kill_after_schedule_batch_recovers_without_duplicate_run",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, directory.path())
        .output()
        .expect("spawn schedule process-kill child");
    assert!(
        !output.status.success(),
        "schedule process-kill child unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let journal = process_kill_journal(directory.path());
    journal
        .verify()
        .expect("verify journal after schedule process kill");
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        repository,
        Arc::new(DenyWorkflowEffects),
    );
    let schedule = service.get_schedule(SCHEDULE_ID).expect("reopen schedule");
    assert_eq!(schedule.next_fire_at, "2026-01-01T12:01:00Z");
    assert_eq!(schedule.last_scheduled_at.as_deref(), Some(OCCURRENCE));
    let run_id = schedule.last_run_id.clone().expect("queued scheduled run");
    let queued = service.get_run(&run_id).expect("reopen scheduled run");
    assert_eq!(queued.status, WorkflowStatus::Queued);
    assert_eq!(queued.trigger_kind, Some(WorkflowTriggerKind::Schedule));
    assert_eq!(queued.trigger_id.as_deref(), Some(SCHEDULE_ID));
    assert_eq!(queued.trigger_occurrence.as_deref(), Some(OCCURRENCE));
    assert!(
        service
            .tick_schedules_at("2026-01-01T12:00:01Z")
            .expect("repeat schedule tick")
            .is_empty(),
        "recovery must not queue the committed occurrence twice"
    );
    let completed = service.drain().await.expect("drain recovered schedule run");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].run_id, run_id);
    assert_eq!(completed[0].status, WorkflowStatus::Completed);
    assert_eq!(service.list_runs(10).expect("list runs").len(), 1);
    journal.verify().expect("verify recovered schedule journal");
    drop(service);
    drop(journal);

    let reopened = process_kill_journal(directory.path());
    reopened.verify().expect("verify reopened schedule journal");
    let repository = EventSourcedWorkflowRepository::new(reopened);
    assert_eq!(
        repository
            .schedule(SCHEDULE_ID)
            .expect("reopened schedule")
            .expect("schedule")
            .last_run_id
            .as_deref(),
        Some(run_id.as_str())
    );
    let runs = repository.runs(10).expect("reopened runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn process_kill_after_webhook_batch_recovers_without_duplicate_delivery() {
    const CHILD_ENV: &str = "COLOSSUS_WORKFLOW_WEBHOOK_PROCESS_KILL_CHILD";
    const ROOT_ENV: &str = "COLOSSUS_WORKFLOW_WEBHOOK_PROCESS_KILL_ROOT";
    const WEBHOOK_ID: &str = "process-kill-webhook";
    const DELIVERY_ID: &str = "process-kill-delivery";
    const TIMESTAMP: &str = "2026-07-16T12:00:00Z";
    const BODY: &[u8] = br#"{"event":"process-kill"}"#;
    const SECRET: &[u8] = b"process-kill-webhook-secret-at-least-32-bytes";

    if std::env::var_os(CHILD_ENV).is_some() {
        let root =
            PathBuf::from(std::env::var_os(ROOT_ENV).expect("webhook process-kill child root"));
        let durable_journal = process_kill_journal(&root);
        let repository: Arc<dyn WorkflowRepository> = Arc::new(
            EventSourcedWorkflowRepository::new(Arc::clone(&durable_journal)),
        );
        let service = WorkflowService::new(
            Arc::clone(&durable_journal),
            repository,
            Arc::new(RecordingEffects::default()),
        );
        service
            .register_definition(WEBHOOK_WORKFLOW, "webhook-process-kill-test")
            .expect("register webhook workflow");
        service
            .create_webhook(
                WEBHOOK_ID,
                "webhook-smoke",
                "1.0.0",
                "env:COLOSSUS_WEBHOOK_PROCESS_KILL_SECRET",
                300,
                4096,
                true,
            )
            .expect("create webhook");
        drop(service);

        let journal: Arc<dyn EventJournal> = Arc::new(CrashAfterEventJournal {
            inner: durable_journal,
            event_type: "workflow.webhook.delivery.accepted.v1",
        });
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service =
            WorkflowService::new(journal, repository, Arc::new(RecordingEffects::default()));
        service
            .ingest_webhook_at(
                WEBHOOK_ID,
                DELIVERY_ID,
                TIMESTAMP,
                &webhook_signature(TIMESTAMP, DELIVERY_ID, BODY, SECRET),
                BTreeMap::new(),
                BODY,
                SECRET,
                parse_schedule_time(TIMESTAMP, "test clock").expect("test clock"),
            )
            .await
            .expect("webhook batch must terminate this process after commit");
        panic!("webhook process-kill child returned without terminating");
    }

    let directory = tempdir().expect("webhook process-kill directory");
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "tests::process_kill_after_webhook_batch_recovers_without_duplicate_delivery",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, directory.path())
        .output()
        .expect("spawn webhook process-kill child");
    assert!(
        !output.status.success(),
        "webhook process-kill child unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let journal = process_kill_journal(directory.path());
    journal
        .verify()
        .expect("verify journal after webhook process kill");
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    let delivery = repository
        .webhook_delivery(WEBHOOK_ID, DELIVERY_ID)
        .expect("reopen webhook delivery")
        .expect("accepted webhook delivery");
    let queued = service
        .get_run(&delivery.run_id)
        .expect("reopen webhook run");
    assert_eq!(queued.status, WorkflowStatus::Queued);
    assert_eq!(queued.trigger_kind, Some(WorkflowTriggerKind::Webhook));
    assert_eq!(queued.trigger_id.as_deref(), Some(WEBHOOK_ID));
    assert_eq!(queued.trigger_occurrence.as_deref(), Some(DELIVERY_ID));

    let replay = service
        .ingest_webhook_at(
            WEBHOOK_ID,
            DELIVERY_ID,
            TIMESTAMP,
            &webhook_signature(TIMESTAMP, DELIVERY_ID, BODY, SECRET),
            BTreeMap::new(),
            BODY,
            SECRET,
            parse_schedule_time(TIMESTAMP, "test clock").expect("test clock"),
        )
        .await;
    assert!(matches!(replay, Err(WorkflowError::InvalidTransition(_))));
    assert!(effects.calls().is_empty());
    let completed = service.drain().await.expect("drain recovered webhook run");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].run_id, delivery.run_id);
    assert_eq!(completed[0].status, WorkflowStatus::Completed);
    assert_eq!(service.list_runs(10).expect("list runs").len(), 1);
    journal.verify().expect("verify recovered webhook journal");
    drop(service);
    drop(repository);
    drop(journal);

    let reopened = process_kill_journal(directory.path());
    reopened.verify().expect("verify reopened webhook journal");
    let repository = EventSourcedWorkflowRepository::new(reopened);
    let replayed_delivery = repository
        .webhook_delivery(WEBHOOK_ID, DELIVERY_ID)
        .expect("reopened delivery")
        .expect("delivery");
    assert_eq!(replayed_delivery, delivery);
    let runs = repository.runs(10).expect("reopened runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn process_kill_after_subscription_batch_recovers_without_duplicate_run() {
    const CHILD_ENV: &str = "COLOSSUS_WORKFLOW_SUBSCRIPTION_PROCESS_KILL_CHILD";
    const ROOT_ENV: &str = "COLOSSUS_WORKFLOW_SUBSCRIPTION_PROCESS_KILL_ROOT";
    const SUBSCRIPTION_ID: &str = "process-kill-subscription";

    if std::env::var_os(CHILD_ENV).is_some() {
        let root = PathBuf::from(
            std::env::var_os(ROOT_ENV).expect("subscription process-kill child root"),
        );
        let durable_journal = process_kill_journal(&root);
        let repository: Arc<dyn WorkflowRepository> = Arc::new(
            EventSourcedWorkflowRepository::new(Arc::clone(&durable_journal)),
        );
        let service = WorkflowService::new(
            Arc::clone(&durable_journal),
            repository,
            Arc::new(RecordingEffects::default()),
        );
        service
            .register_definition(SUBSCRIPTION_WORKFLOW, "subscription-process-kill-test")
            .expect("register subscription workflow");
        service
            .create_subscription(
                SUBSCRIPTION_ID,
                "subscription-smoke",
                "1.0.0",
                "task.created.v1",
                Some("task:"),
                true,
                Some(0),
            )
            .expect("create subscription");
        durable_journal
            .append(NewEvent {
                event_version: 1,
                stream_id: "task:process-kill".into(),
                expected_stream_version: 0,
                classification: EventClassification::Domain,
                event_type: "task.created.v1".into(),
                actor: Actor {
                    actor_type: ActorType::User,
                    id: "process-kill-test".into(),
                },
                context: ExecutionContext::default(),
                payload: json!({"title": "survive process loss"}),
            })
            .expect("append source event");
        drop(service);

        let journal: Arc<dyn EventJournal> = Arc::new(CrashAfterEventJournal {
            inner: durable_journal,
            event_type: "workflow.subscription.delivered.v1",
        });
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let service =
            WorkflowService::new(journal, repository, Arc::new(RecordingEffects::default()));
        service
            .tick_subscriptions_now()
            .await
            .expect("subscription batch must terminate this process after commit");
        panic!("subscription process-kill child returned without terminating");
    }

    let directory = tempdir().expect("subscription process-kill directory");
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args([
            "--exact",
            "tests::process_kill_after_subscription_batch_recovers_without_duplicate_run",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, directory.path())
        .output()
        .expect("spawn subscription process-kill child");
    assert!(
        !output.status.success(),
        "subscription process-kill child unexpectedly succeeded: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let journal = process_kill_journal(directory.path());
    journal
        .verify()
        .expect("verify journal after subscription process kill");
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let effects = Arc::new(RecordingEffects::default());
    let service = WorkflowService::new(
        Arc::clone(&journal),
        Arc::clone(&repository),
        effects.clone(),
    );
    let subscription = service
        .get_subscription(SUBSCRIPTION_ID)
        .expect("reopen subscription");
    let source_event_id = subscription
        .last_event_id
        .clone()
        .expect("delivered source event");
    let run_id = subscription
        .last_run_id
        .clone()
        .expect("queued subscription run");
    let delivery = repository
        .subscription_delivery(SUBSCRIPTION_ID, &source_event_id)
        .expect("reopen subscription delivery")
        .expect("accepted subscription delivery");
    assert_eq!(delivery.run_id, run_id);
    let queued = service.get_run(&run_id).expect("reopen subscription run");
    assert_eq!(queued.status, WorkflowStatus::Queued);
    assert_eq!(queued.trigger_kind, Some(WorkflowTriggerKind::Subscription));
    assert_eq!(queued.trigger_id.as_deref(), Some(SUBSCRIPTION_ID));
    assert_eq!(
        queued.trigger_occurrence.as_deref(),
        Some(source_event_id.as_str())
    );
    assert!(
        service
            .tick_subscriptions_now()
            .await
            .expect("repeat subscription tick")
            .is_empty(),
        "recovery must not queue the committed source event twice"
    );
    assert!(effects.calls().is_empty());
    let completed = service
        .drain()
        .await
        .expect("drain recovered subscription run");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].run_id, run_id);
    assert_eq!(completed[0].status, WorkflowStatus::Completed);
    assert_eq!(service.list_runs(10).expect("list runs").len(), 1);
    journal
        .verify()
        .expect("verify recovered subscription journal");
    drop(service);
    drop(repository);
    drop(journal);

    let reopened = process_kill_journal(directory.path());
    reopened
        .verify()
        .expect("verify reopened subscription journal");
    let repository = EventSourcedWorkflowRepository::new(reopened);
    let replayed_delivery = repository
        .subscription_delivery(SUBSCRIPTION_ID, &source_event_id)
        .expect("reopened subscription delivery")
        .expect("subscription delivery");
    assert_eq!(replayed_delivery, delivery);
    let runs = repository.runs(10).expect("reopened runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn process_kill_after_external_effect_recovers_without_unsafe_replay() {
    if let Some(mode) = std::env::var_os("COLOSSUS_WORKFLOW_PROCESS_KILL_CHILD") {
        let root = PathBuf::from(
            std::env::var_os("COLOSSUS_WORKFLOW_PROCESS_KILL_ROOT")
                .expect("process-kill child root"),
        );
        let marker = PathBuf::from(
            std::env::var_os("COLOSSUS_WORKFLOW_PROCESS_KILL_MARKER")
                .expect("process-kill child marker"),
        );
        let run_id = std::env::var("COLOSSUS_WORKFLOW_PROCESS_KILL_RUN_ID")
            .expect("process-kill child run id");
        process_kill_child(
            &root,
            &marker,
            &run_id,
            mode.to_str().expect("process-kill mode is UTF-8"),
        )
        .await;
        return;
    }

    for (
        mode,
        run_id,
        expected_marker,
        expected_step,
        expected_execution,
        expected_phase,
        retry_allowed,
    ) in [
        (
            "non-idempotent",
            "018f0000-0000-7000-8000-000000000001",
            Some("mutation.run:1\n"),
            "mutate",
            "mutate",
            "primary",
            Some(false),
        ),
        (
            "idempotent",
            "018f0000-0000-7000-8000-000000000002",
            Some("mutation.run:1\n"),
            "mutate",
            "mutate",
            "primary",
            Some(true),
        ),
        (
            "compensation",
            "018f0000-0000-7000-8000-000000000003",
            Some("rollback.run:2\n"),
            "rollback",
            "rollback",
            "compensation",
            Some(false),
        ),
        (
            "completed-step",
            "018f0000-0000-7000-8000-000000000004",
            None,
            "durable",
            "durable",
            "primary",
            None,
        ),
        (
            "parallel",
            "018f0000-0000-7000-8000-000000000005",
            Some("parallel.run:3\n"),
            "mutate",
            "branches.branch[1]/mutate",
            "primary",
            Some(true),
        ),
        (
            "subworkflow-link",
            "018f0000-0000-7000-8000-000000000006",
            Some("workflow.start:1\n"),
            "child-call",
            "child-call",
            "primary",
            Some(true),
        ),
        (
            "nested-child",
            "018f0000-0000-7000-8000-000000000007",
            Some("nested.run:1\n"),
            "child-call",
            "child-call",
            "primary",
            Some(true),
        ),
    ] {
        let directory = tempdir().expect("process-kill directory");
        let marker = directory.path().join("external-effect.log");
        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "tests::process_kill_after_external_effect_recovers_without_unsafe_replay",
                "--nocapture",
            ])
            .env("COLOSSUS_WORKFLOW_PROCESS_KILL_CHILD", mode)
            .env("COLOSSUS_WORKFLOW_PROCESS_KILL_ROOT", directory.path())
            .env("COLOSSUS_WORKFLOW_PROCESS_KILL_MARKER", &marker)
            .env("COLOSSUS_WORKFLOW_PROCESS_KILL_RUN_ID", run_id)
            .output()
            .expect("spawn process-kill child");
        assert!(
            !output.status.success(),
            "process-kill child unexpectedly succeeded: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(expected_marker) = expected_marker {
            assert_eq!(
                fs::read_to_string(&marker).expect("durable external-effect marker"),
                expected_marker,
                "the child must terminate only after the simulated effect is durable"
            );
        } else {
            assert!(!marker.exists());
        }

        let journal = process_kill_journal(directory.path());
        journal.verify().expect("verify journal after process kill");
        let repository: Arc<dyn WorkflowRepository> =
            Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
        let effects = Arc::new(RecordingEffects::default());
        let service = WorkflowService::new(Arc::clone(&journal), repository, effects.clone());
        let recovered = service.recover_interrupted().expect("recover killed run");
        assert_eq!(recovered.len(), if mode == "nested-child" { 2 } else { 1 });
        assert_eq!(
            service.get_run(run_id).expect("recovered parent").status,
            colossus_contracts::WorkflowStatus::Interrupted
        );
        let events = journal
            .read_stream(&format!("workflow-run:{run_id}"))
            .expect("recovered workflow events");
        let unknown = events
            .iter()
            .filter(|event| event.event_type == "workflow.step.outcome_unknown.v1")
            .collect::<Vec<_>>();
        if let Some(retry_allowed) = retry_allowed {
            assert_eq!(unknown.len(), 1);
            let unknown = journal
                .decrypt_payload(unknown[0])
                .expect("unknown-outcome payload");
            assert_eq!(unknown["step_id"], expected_step);
            assert_eq!(unknown["execution_id"], expected_execution);
            assert_eq!(unknown["phase"], expected_phase);
            assert_eq!(
                unknown["attempt"],
                match mode {
                    "compensation" => 2,
                    "parallel" => 3,
                    _ => 1,
                }
            );
            assert_eq!(unknown["retry_allowed"], retry_allowed);
        } else {
            assert!(unknown.is_empty());
        }
        assert!(
            service
                .recover_interrupted()
                .expect("idempotent recovery")
                .is_empty()
        );
        assert!(
            service
                .drain()
                .await
                .expect("drain after recovery")
                .is_empty()
        );

        if mode == "idempotent" {
            let completed = service.resume_run(run_id).await.expect("safe retry");
            assert_eq!(
                completed.status,
                colossus_contracts::WorkflowStatus::Completed
            );
            let calls = effects.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].attempt, 2);
            let expected_idempotency = format!("durable-key:{run_id}:mutate");
            assert_eq!(
                calls[0].idempotency.as_deref(),
                Some(expected_idempotency.as_str())
            );
        } else if mode == "parallel" {
            let completed = service
                .resume_run(run_id)
                .await
                .expect("safe scoped parallel retry");
            assert_eq!(
                completed.status,
                colossus_contracts::WorkflowStatus::Completed
            );
            let calls = effects.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].action, "parallel.run");
            assert_eq!(calls[0].step_id, expected_execution);
            assert_eq!(calls[0].attempt, 5);
            let expected_idempotency = format!("parallel-key:{run_id}:{expected_execution}");
            assert_eq!(
                calls[0].idempotency.as_deref(),
                Some(expected_idempotency.as_str())
            );
            let events = journal
                .read_stream(&format!("workflow-run:{run_id}"))
                .expect("parallel events after resume");
            let prior_completion_count = events
                .iter()
                .filter(|event| event.event_type == "workflow.step.completed.v1")
                .map(|event| journal.decrypt_payload(event).expect("completion payload"))
                .filter(|payload| {
                    payload
                        .get("execution_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("branches.branch[0]/before")
                })
                .count();
            assert_eq!(prior_completion_count, 1);
        } else if mode == "subworkflow-link" {
            let before_resume = journal
                .read_stream(&format!("workflow-run:{run_id}"))
                .expect("linked parent events");
            let link = before_resume
                .iter()
                .find(|event| event.event_type == "workflow.subworkflow.linked.v1")
                .expect("durable child link");
            let child_run_id =
                journal.decrypt_payload(link).expect("child link payload")["child_run_id"]
                    .as_str()
                    .expect("child run id")
                    .to_owned();
            let completed = service
                .resume_run(run_id)
                .await
                .expect("resume linked child without relaunch");
            assert_eq!(
                completed.status,
                colossus_contracts::WorkflowStatus::Completed
            );
            assert!(effects.calls().is_empty());
            assert_eq!(service.list_runs(10).expect("parent and child").len(), 2);
            assert_eq!(
                service
                    .get_run(&child_run_id)
                    .expect("recreated child")
                    .status,
                colossus_contracts::WorkflowStatus::Completed
            );
            assert_eq!(
                completed.outputs.as_ref().expect("parent outputs")["child-call"]["run_id"],
                child_run_id
            );
        } else if mode == "nested-child" {
            let parent_events = journal
                .read_stream(&format!("workflow-run:{run_id}"))
                .expect("nested parent events");
            let link = parent_events
                .iter()
                .find(|event| event.event_type == "workflow.subworkflow.linked.v1")
                .expect("nested child link");
            let child_run_id = journal
                .decrypt_payload(link)
                .expect("nested child link payload")["child_run_id"]
                .as_str()
                .expect("nested child run id")
                .to_owned();
            assert_eq!(
                service
                    .get_run(&child_run_id)
                    .expect("interrupted child")
                    .status,
                colossus_contracts::WorkflowStatus::Interrupted
            );
            let child_events = journal
                .read_stream(&format!("workflow-run:{child_run_id}"))
                .expect("nested child events");
            let child_unknown = child_events
                .iter()
                .find(|event| event.event_type == "workflow.step.outcome_unknown.v1")
                .expect("nested child unknown outcome");
            let child_unknown = journal
                .decrypt_payload(child_unknown)
                .expect("nested child unknown payload");
            assert_eq!(child_unknown["step_id"], "nested-mutate");
            assert_eq!(child_unknown["execution_id"], "nested-mutate");
            assert_eq!(child_unknown["attempt"], 1);
            assert_eq!(child_unknown["retry_allowed"], true);

            let premature_parent = service
                .resume_run(run_id)
                .await
                .expect_err("parent must not fail or advance before child recovery");
            assert!(premature_parent.to_string().contains(&child_run_id));
            assert!(
                premature_parent
                    .to_string()
                    .contains("resumed before parent")
            );
            assert_eq!(
                service
                    .get_run(run_id)
                    .expect("still interrupted parent")
                    .status,
                colossus_contracts::WorkflowStatus::Interrupted
            );
            let child = service
                .resume_run(&child_run_id)
                .await
                .expect("resume idempotent nested child");
            assert_eq!(child.status, colossus_contracts::WorkflowStatus::Completed);
            let calls = effects.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].action, "nested.run");
            assert_eq!(calls[0].attempt, 2);
            let expected_idempotency = format!("nested-key:{child_run_id}:nested-mutate");
            assert_eq!(
                calls[0].idempotency.as_deref(),
                Some(expected_idempotency.as_str())
            );
            let parent = service
                .resume_run(run_id)
                .await
                .expect("resume parent after child");
            assert_eq!(parent.status, colossus_contracts::WorkflowStatus::Completed);
            assert_eq!(
                parent.outputs.as_ref().expect("nested parent outputs")["child-call"]["run_id"],
                child_run_id
            );
            assert_eq!(effects.calls().len(), 1);
        } else if mode == "completed-step" {
            let completed = service
                .resume_run(run_id)
                .await
                .expect("resume after durable completion");
            assert_eq!(
                completed.status,
                colossus_contracts::WorkflowStatus::Completed
            );
            assert_eq!(
                completed.outputs.as_ref().expect("completed outputs")["durable"]["persisted"],
                true
            );
            assert!(effects.calls().is_empty());
        } else {
            assert!(
                service
                    .resume_run(run_id)
                    .await
                    .expect_err("unsafe retry must fail")
                    .to_string()
                    .contains("cannot be retried")
            );
            assert!(effects.calls().is_empty());
        }
        journal.verify().expect("verify recovered workflow journal");
        let expected_status = if matches!(mode, "non-idempotent" | "compensation") {
            colossus_contracts::WorkflowStatus::Interrupted
        } else {
            colossus_contracts::WorkflowStatus::Completed
        };
        drop(service);
        drop(journal);

        let reopened = process_kill_journal(directory.path());
        reopened.verify().expect("verify reopened workflow journal");
        let repository = EventSourcedWorkflowRepository::new(reopened);
        assert_eq!(
            repository
                .run(run_id)
                .expect("reopened run")
                .expect("run")
                .status,
            expected_status
        );
        if matches!(mode, "subworkflow-link" | "nested-child") {
            assert_eq!(
                repository
                    .runs(10)
                    .expect("reopened parent and child")
                    .len(),
                2
            );
        }
    }
}

#[tokio::test]
async fn crash_recovery_records_unknown_and_never_auto_retries() {
    const NON_IDEMPOTENT: &str = r#"
apiVersion: colossus.dev/v1alpha1
kind: Workflow
metadata:
  name: crashy
  version: 1.0.0
  description: Crash recovery
inputs: { type: object }
outputs: { type: object }
capabilities: [workflow.execute]
maxConcurrency: 1
stepBudget: 2
steps:
  - type: tool
    id: mutate
    tool: mutation.run
    arguments: {}
    idempotency: null
"#;
    let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
    let repository: Arc<dyn WorkflowRepository> =
        Arc::new(EventSourcedWorkflowRepository::new(Arc::clone(&journal)));
    let service = WorkflowService::new(
        Arc::clone(&journal),
        repository,
        Arc::new(DenyWorkflowEffects),
    );
    service
        .register_definition(NON_IDEMPOTENT, "test")
        .expect("register");
    let queued = service
        .queue_run("crashy", "1.0.0", json!({}))
        .expect("queue");
    service
        .append_run_event(&queued.run_id, "workflow.run.started.v1", json!({}))
        .expect("claim");
    service
        .append_run_event(
            &queued.run_id,
            "workflow.step.started.v1",
            json!({"step_id": "mutate", "attempt": 1}),
        )
        .expect("started effect");
    let recovered = service.recover_interrupted().expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].status,
        colossus_contracts::WorkflowStatus::Interrupted
    );
    let events = journal
        .read_stream(&format!("workflow-run:{}", queued.run_id))
        .expect("events");
    assert!(
        events
            .iter()
            .any(|event| { event.event_type == "workflow.step.outcome_unknown.v1" })
    );
    assert!(
        service
            .resume_run(&queued.run_id)
            .await
            .expect_err("unsafe retry")
            .to_string()
            .contains("cannot be retried")
    );
    assert!(service.drain().await.expect("drain").is_empty());

    let completed_before_crash = service
        .queue_run("crashy", "1.0.0", json!({}))
        .expect("second queue");
    service
        .append_run_event(
            &completed_before_crash.run_id,
            "workflow.run.started.v1",
            json!({}),
        )
        .expect("second claim");
    service
        .append_run_event(
            &completed_before_crash.run_id,
            "workflow.step.started.v1",
            json!({"step_id": "mutate", "attempt": 1}),
        )
        .expect("second start");
    service
        .append_run_event(
            &completed_before_crash.run_id,
            "workflow.step.completed.v1",
            json!({"step_id": "mutate", "root_index": 0, "output": {}}),
        )
        .expect("durable completion");
    service.recover_interrupted().expect("second recover");
    let completed_events = journal
        .read_stream(&format!("workflow-run:{}", completed_before_crash.run_id))
        .expect("second events");
    assert!(
        !completed_events
            .iter()
            .any(|event| { event.event_type == "workflow.step.outcome_unknown.v1" })
    );
}
