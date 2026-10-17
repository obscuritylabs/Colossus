use super::tests::{caller, publication};
use super::*;
use colossus_contracts::{
    Actor, ActorType, BrowserScope, EventClassification, ExecutionContext, NewEvent,
};
use serde_json::json;

fn record(
    journal: &dyn EventJournal,
    stream: &str,
    kind: &str,
    actor: Actor,
    payload: serde_json::Value,
) {
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: stream.into(),
            expected_stream_version: 0,
            classification: EventClassification::Workflow,
            event_type: kind.into(),
            actor,
            context: ExecutionContext::default(),
            payload,
        })
        .unwrap();
}
fn actor(kind: ActorType, id: &str) -> Actor {
    Actor {
        actor_type: kind,
        id: id.into(),
    }
}
fn origin(id: &str) -> serde_json::Value {
    json!({"owner":actor(ActorType::Application,id),"session_id":null,"run_id":null})
}
fn workflow(id: &str) -> BrowserArtifactPublication {
    let mut request = publication(id);
    request.binding.scope = BrowserScope::Workflow { id: id.into() };
    request
}

#[tokio::test]
async fn workflow_capture_uses_canonical_host_application_owner_and_denies_foreign_reads() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    record(
        journal.as_ref(),
        "workflow-run:w1",
        "workflow.run.queued.v1",
        actor(ActorType::Application, "app:real"),
        json!({"origin":origin("app:real"),"inputs":{"origin":origin("app:forged")}}),
    );
    let image = ReleasedBrowserArtifactPublisher
        .publish(Arc::clone(&journal), workflow("w1"))
        .await
        .unwrap();
    let artifacts = EventSourcedArtifactApi::new(journal);
    assert!(
        artifacts
            .get(&caller("app:real"), &image.artifact_id)
            .await
            .is_ok()
    );
    assert!(
        artifacts
            .get(&caller("app:forged"), &image.artifact_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn scheduled_child_uses_first_registered_schedule_origin_after_parent_recursion() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    record(
        journal.as_ref(),
        "workflow-schedule:s1",
        "workflow.schedule.registered.v1",
        actor(ActorType::Application, "app:scheduled"),
        json!({"origin":origin("app:scheduled"),"record":{"inputs":{"owner":"app:forged"}}}),
    );
    record(
        journal.as_ref(),
        "workflow-run:parent",
        "workflow.run.queued.v1",
        actor(ActorType::System, "scheduler"),
        json!({"trigger_kind":"schedule","trigger_id":"s1"}),
    );
    record(
        journal.as_ref(),
        "workflow-run:child",
        "workflow.run.queued.v1",
        actor(ActorType::System, "child"),
        json!({"parent_run_id":"parent"}),
    );
    let image = ReleasedBrowserArtifactPublisher
        .publish(Arc::clone(&journal), workflow("child"))
        .await
        .unwrap();
    assert!(
        EventSourcedArtifactApi::new(journal)
            .get(&caller("app:scheduled"), &image.artifact_id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn forged_origin_legacy_system_records_binding_mismatch_and_cycles_cannot_claim_owner() {
    let journal: Arc<dyn EventJournal> =
        Arc::new(colossus_testkit::InMemoryEventJournal::default());
    record(
        journal.as_ref(),
        "workflow-run:forged",
        "workflow.run.queued.v1",
        actor(ActorType::Application, "app:real"),
        json!({"origin":origin("app:foreign")}),
    );
    record(
        journal.as_ref(),
        "workflow-run:legacy",
        "workflow.run.queued.v1",
        actor(ActorType::System, "legacy"),
        json!({"inputs":{"origin":origin("app:foreign")}}),
    );
    record(
        journal.as_ref(),
        "workflow-run:cycle",
        "workflow.run.queued.v1",
        actor(ActorType::System, "cycle"),
        json!({"parent_run_id":"cycle"}),
    );
    for id in ["forged", "legacy", "cycle"] {
        assert!(
            ReleasedBrowserArtifactPublisher
                .publish(Arc::clone(&journal), workflow(id))
                .await
                .is_err()
        );
    }
    let mut mismatch = workflow("legacy");
    mismatch.binding.application_id = "foreign-workflow".into();
    assert!(
        ReleasedBrowserArtifactPublisher
            .publish(Arc::clone(&journal), mismatch)
            .await
            .is_err()
    );
    let mut application_mismatch = workflow("legacy");
    application_mismatch.binding.application_id = "app:real".into();
    assert!(
        ReleasedBrowserArtifactPublisher
            .publish(Arc::clone(&journal), application_mismatch)
            .await
            .is_err()
    );
    assert!(
        journal
            .list_stream_ids("artifact:", None, 100)
            .unwrap()
            .is_empty()
    );
}
