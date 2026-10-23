use super::*;
use std::{collections::BTreeMap, fmt, sync::Mutex};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt as _},
    registry::LookupSpan,
    util::SubscriberInitExt as _,
};

#[derive(Clone, Debug)]
struct Captured {
    name: &'static str,
    fields: BTreeMap<String, String>,
    scope: Vec<&'static str>,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Captured>>>);

#[derive(Default)]
struct Fields(BTreeMap<String, String>);
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, _id: &Id, _context: Context<'_, S>) {
        let name = attributes.metadata().name();
        if !name.starts_with("colossus.hosted.run.") {
            return;
        }
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        self.0.lock().unwrap().push(Captured {
            name,
            fields: fields.0,
            scope: Vec::new(),
        });
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        if event.metadata().target() != "colossus.lifecycle" {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        if !fields
            .0
            .get("event_name")
            .is_some_and(|name| name.starts_with("colossus.hosted.run."))
        {
            return;
        }
        let scope = context.event_scope(event).map_or_else(Vec::new, |scope| {
            scope
                .from_root()
                .map(|span| span.metadata().name())
                .collect()
        });
        self.0.lock().unwrap().push(Captured {
            name: "lifecycle",
            fields: fields.0,
            scope,
        });
    }
}

pub(super) async fn trusted_project_follows_spawned_and_recovered_work(runtime: Arc<Runtime>) {
    const PROJECT: &str = "trusted-hosted-project";
    const PRIVATE_PROMPT: &str = "private-prompt-must-not-enter-lifecycle-telemetry";
    const ASSERTED_USER: &str = "caller-asserted-other-project";
    let capture = Capture::default();
    let _subscriber = tracing_subscriber::registry()
        .with(capture.clone())
        .set_default();
    let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
    let service = RuntimeAgentRunApi::new(
        Arc::clone(&runtime),
        interactions,
        "primary",
        "Project telemetry conformance",
    )
    .with_telemetry_project_id(PROJECT)
    .expect("trusted composition label");
    let owner = caller(
        &format!("app:project-telemetry-{}", Uuid::now_v7().simple()),
        "project-telemetry",
    );
    let mut fresh_request = request(
        &format!("project-telemetry-new-{}", Uuid::now_v7().simple()),
        PRIVATE_PROMPT,
    );
    fresh_request.end_user_id = Some(ASSERTED_USER.into());
    let fresh = service
        .create_run(&owner, fresh_request)
        .await
        .expect("create hosted run")
        .run;
    assert_eq!(
        wait_terminal(&service, &owner, &fresh.id).await.status,
        RunStatus::Completed
    );
    wait_inactive(&service).await;

    let repository = EventSourcedRunRepository::new(runtime.journal());
    let mut queued_request = request(
        &format!("project-telemetry-recovery-{}", Uuid::now_v7().simple()),
        PRIVATE_PROMPT,
    );
    queued_request.end_user_id = Some(ASSERTED_USER.into());
    let queued = NewRun::from_request(
        Uuid::now_v7().to_string(),
        Uuid::now_v7().to_string(),
        "primary",
        &queued_request,
    )
    .expect("recoverable request");
    let orphan = repository
        .create_run(&owner, &queued_request, &queued)
        .expect("durable queued run")
        .value;
    service
        .get_run(
            &owner,
            GetRunRequest {
                run_id: orphan.id.clone(),
            },
        )
        .await
        .expect("recover hosted run");
    assert_eq!(
        wait_terminal(&service, &owner, &orphan.id).await.status,
        RunStatus::Completed
    );
    wait_inactive(&service).await;

    let records = capture.0.lock().unwrap();
    for (run_id, source) in [(&fresh.id, "submitted"), (&orphan.id, "recovered")] {
        assert!(records.iter().any(|record| {
            record.name == "colossus.hosted.run.execute"
                && record.fields.get("colossus.run.id") == Some(run_id)
                && record
                    .fields
                    .get("colossus.execution.source")
                    .map(String::as_str)
                    == Some(source)
        }));
        for name in [
            "colossus.hosted.run.execution_started",
            "colossus.hosted.run.execution_finished",
        ] {
            let event = records
                .iter()
                .find(|record| {
                    record.fields.get("event_name").map(String::as_str) == Some(name)
                        && record.fields.get("colossus.run.id") == Some(run_id)
                })
                .expect("execution lifecycle event");
            assert!(
                event.scope.contains(&"colossus.hosted.run.execute"),
                "project span must instrument the spawned future itself"
            );
        }
    }
    assert!(
        records
            .iter()
            .any(|record| record.name == "colossus.hosted.run.recover"
                && record.fields.get("colossus.run.id") == Some(&orphan.id))
    );
    assert!(
        records
            .iter()
            .any(|record| record.fields.get("event_name").map(String::as_str)
                == Some("colossus.hosted.run.recovery_started"))
    );
    for record in records.iter() {
        assert_eq!(
            record.fields.get("colossus.project.id").map(String::as_str),
            Some(PROJECT)
        );
        assert!(
            record
                .fields
                .values()
                .all(|value| !value.contains(PRIVATE_PROMPT) && !value.contains(ASSERTED_USER))
        );
    }
}
