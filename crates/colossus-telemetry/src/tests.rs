use super::{TelemetryService, duration_seconds};
use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
use colossus_ports::EventJournal;
use colossus_testkit::InMemoryEventJournal;
use serde_json::{Value, json};
use std::sync::Arc;

fn append(journal: &dyn EventJournal, version: u64, event_type: &str, payload: Value) {
    journal
        .append(NewEvent {
            event_version: 1,
            stream_id: "run:run-telemetry".into(),
            expected_stream_version: version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "run-telemetry".into(),
                session_id: Some("session-1".into()),
                run_id: Some("run-telemetry".into()),
                ..ExecutionContext::default()
            },
            payload,
        })
        .expect("append");
}

#[test]
fn derives_counts_without_returning_payload_content() {
    let journal = Arc::new(InMemoryEventJournal::default());
    append(
        journal.as_ref(),
        0,
        "model.delta.v1",
        json!({"text": "secret prompt text"}),
    );
    append(
        journal.as_ref(),
        1,
        "tool.call.requested.v1",
        json!({"name": "echo"}),
    );
    append(
        journal.as_ref(),
        2,
        "tool.call.completed.v1",
        json!({"exit_code": 7, "output": "secret tool output"}),
    );
    append(
        journal.as_ref(),
        3,
        "context.prepared.v1",
        json!({"compacted": true}),
    );
    append(
        journal.as_ref(),
        4,
        "research.run_updated.v1",
        json!({"record": "hidden"}),
    );
    append(
        journal.as_ref(),
        5,
        "final.output.v1",
        json!({"text": "done"}),
    );
    append(
        journal.as_ref(),
        6,
        "provider.usage.v1",
        json!({
            "input_tokens": 10,
            "output_tokens": 4,
            "total_tokens": 14,
            "cached_input_tokens": 3,
            "reasoning_tokens": 2,
        }),
    );
    let service = TelemetryService::new(journal);
    let summary = service
        .list_runs(Some("session-1"), 20)
        .expect("runs")
        .remove(0);
    assert_eq!(summary.events, 7);
    assert_eq!(summary.model_output_chars, 18);
    assert_eq!(summary.tool_calls, 1);
    assert_eq!(summary.tool_errors, 1);
    assert_eq!(summary.context_compactions, 1);
    assert_eq!(summary.research_events, 1);
    assert_eq!(summary.final_outputs, 1);
    assert_eq!(summary.provider_input_tokens, 10);
    assert_eq!(summary.provider_output_tokens, 4);
    assert_eq!(summary.provider_total_tokens, 14);
    assert_eq!(summary.provider_cached_input_tokens, 3);
    assert_eq!(summary.provider_reasoning_tokens, 2);
    let detail = service.get_run("run-tele", 3).expect("detail");
    assert!(detail.truncated);
    assert_eq!(detail.records.len(), 3);
    let rendered = serde_json::to_string(&detail).expect("JSON");
    assert!(!rendered.contains("secret prompt text"));
    assert!(!rendered.contains("secret tool output"));
    let metrics = service.metrics(None, 100).expect("metrics");
    assert_eq!(metrics.run_count, 1);
    assert_eq!(metrics.event_count, 7);
    assert_eq!(metrics.provider_total_tokens, 14);
}

#[test]
fn duration_uses_timestamps_and_never_goes_negative() {
    assert_eq!(
        duration_seconds("2026-07-11T00:00:00Z", "2026-07-11T00:00:01.500Z"),
        1.5
    );
    assert_eq!(
        duration_seconds("2026-07-11T00:00:02Z", "2026-07-11T00:00:01Z"),
        0.0
    );
}
