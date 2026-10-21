use std::{
    fmt,
    sync::{Arc, Mutex},
};

use serde_json::{Map, Value, json};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

#[derive(Default)]
struct EventFields(Map<String, Value>);

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
}

struct EventCapture(Arc<Mutex<Vec<Map<String, Value>>>>);

impl Subscriber for EventCapture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "colossus.desktop"
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0.lock().expect("captured events").push(fields.0);
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

#[test]
fn startup_log_uses_the_tauri_product_version_instead_of_the_private_crate_version() {
    let mut context = crate::app_context::create();
    context.package_info_mut().version = "0.11.9".parse().expect("product version");
    let expected = context.package_info().version.to_string();
    assert_ne!(expected, env!("CARGO_PKG_VERSION"));
    let events = Arc::new(Mutex::new(Vec::new()));

    tracing::subscriber::with_default(EventCapture(events.clone()), || {
        super::started(context.package_info());
    });

    let events = events.lock().expect("captured events");
    assert_eq!(events.len(), 1);
    assert_eq!(
        json!(events[0]),
        json!({
            "event_name": "desktop.started",
            "application_version": expected,
            "platform": std::env::consts::OS,
            "process_id": std::process::id(),
        })
    );
}
