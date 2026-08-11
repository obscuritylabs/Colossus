//! Startup OpenTelemetry acceptance coverage isolated from parallel callsite-cache tests.

use colossus_policy::DenyApproval;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions};
use opentelemetry::{
    Value as OtelValue,
    trace::{SpanKind, Status, TracerProvider as _},
};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt as _;

#[test]
fn runtime_startup_emits_parented_storage_and_recovery_phase_spans() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("runtime-startup-test")));
    let root = tempfile::tempdir().expect("workspace");
    let config = RuntimeConfig::offline_template(root.path().join("state.redb"));
    let options = RuntimeOpenOptions::for_workspace(root.path()).expect("workspace options");

    let subscriber_guard = tracing::subscriber::set_default(subscriber);
    let runtime = Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options)
        .expect("runtime startup");
    drop(runtime);
    drop(subscriber_guard);
    provider.force_flush().expect("flush spans");

    let spans = exporter.get_finished_spans().expect("finished spans");
    let startup = spans
        .iter()
        .find(|span| span.name == "colossus.runtime.open")
        .expect("runtime startup span");
    assert_eq!(startup.span_kind, SpanKind::Internal);
    assert_eq!(startup.status, Status::Ok);
    assert!(startup.end_time >= startup.start_time);
    assert!(startup.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "colossus.storage.adapter" && attribute.value.as_str() == "redb"
    }));
    assert!(startup.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "colossus.storage.startup_verification"
            && attribute.value.as_str() == "incremental"
    }));
    assert!(startup.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "colossus.runtime.recovery_mode"
            && attribute.value == OtelValue::Bool(false)
    }));

    for name in [
        "colossus.runtime.workspace.acquire",
        "colossus.runtime.storage.open",
        "colossus.runtime.projections.catch_up",
        "colossus.runtime.effects.recover",
        "colossus.runtime.research.recover",
        "colossus.runtime.workflows.recover",
    ] {
        let phase = spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("missing startup phase {name}"));
        assert_eq!(phase.span_kind, SpanKind::Internal);
        assert_eq!(phase.status, Status::Ok);
        assert_eq!(phase.parent_span_id, startup.span_context.span_id());
        assert!(phase.start_time >= startup.start_time);
        assert!(phase.end_time <= startup.end_time);
    }
}
