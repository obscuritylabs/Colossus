use crate::JournalPayloadMode;
use colossus_contracts::{EventEnvelope, NewEvent, ProjectionWorkItem, SignedCheckpoint};
use colossus_ports::{EventJournal, StoreError, VerificationReport};
use serde_json::{Value, json};
use std::sync::Arc;

/// Best-effort live-log decorator for the authoritative event journal.
///
/// Records are emitted only after the inner append succeeds. Logging failures are
/// handled by subscriber/exporter queues and cannot change the returned journal result.
pub struct ObservedEventJournal {
    inner: Arc<dyn EventJournal>,
    payload_mode: JournalPayloadMode,
}

impl ObservedEventJournal {
    /// Decorate one canonical journal without changing its read or write behavior.
    pub fn new(inner: Arc<dyn EventJournal>, payload_mode: JournalPayloadMode) -> Self {
        Self {
            inner,
            payload_mode,
        }
    }

    fn emit(&self, envelope: &EventEnvelope, event: &NewEvent) {
        if self.payload_mode == JournalPayloadMode::Disabled {
            return;
        }
        let record = journal_log_record(envelope, event, self.payload_mode);
        let record_json = serde_json::to_string(&record)
            .unwrap_or_else(|_| "{\"error\":\"journal log serialization failed\"}".into());
        tracing::info!(
            target: "colossus.journal",
            event_name = "colossus.journal.appended",
            event_id = %envelope.event_id,
            event_type = %envelope.event_type,
            stream_id = %envelope.stream_id,
            global_sequence = envelope.global_sequence,
            actor_type = ?envelope.actor.actor_type,
            colossus_run_id = envelope.context.run_id.as_deref().unwrap_or(""),
            gen_ai_conversation_id = envelope.context.session_id.as_deref().unwrap_or(""),
            message = %record_json,
        );
    }
}

impl EventJournal for ObservedEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        let envelope = self.inner.append(event.clone())?;
        self.emit(&envelope, &event);
        Ok(envelope)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let envelopes = self.inner.append_batch(events.clone())?;
        for (envelope, event) in envelopes.iter().zip(&events) {
            self.emit(envelope, event);
        }
        Ok(envelopes)
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

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.inner.list_stream_ids(prefix, after, limit)
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

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
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

fn journal_log_record(
    envelope: &EventEnvelope,
    event: &NewEvent,
    mode: JournalPayloadMode,
) -> Value {
    let mut record = json!({
        "schema_version": 1,
        "event_id": envelope.event_id,
        "global_sequence": envelope.global_sequence,
        "stream_id": envelope.stream_id,
        "stream_version": envelope.stream_version,
        "classification": envelope.classification,
        "event_type": envelope.event_type,
        "actor_type": envelope.actor.actor_type,
        "context": envelope.context,
        "occurred_at": envelope.occurred_at,
    });
    if mode == JournalPayloadMode::Full {
        record["payload"] = event.payload.clone();
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext};
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tracing_subscriber::{Layer, layer::SubscriberExt as _};

    #[derive(Clone)]
    struct JournalEventCounter(Arc<AtomicUsize>);

    impl<S> Layer<S> for JournalEventCounter
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if event.metadata().target() == "colossus.journal" {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn event(expected_stream_version: u64) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: "run:test".into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: "test.event.v1".into(),
            actor: Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            context: ExecutionContext {
                correlation_id: "run-test".into(),
                run_id: Some("run-test".into()),
                ..ExecutionContext::default()
            },
            payload: json!({"secret": "visible only in full mode"}),
        }
    }

    #[test]
    fn metadata_record_omits_payload_and_full_record_preserves_it() {
        let journal = InMemoryEventJournal::default();
        let source = event(0);
        let envelope = journal.append(source.clone()).expect("append");
        let metadata = journal_log_record(&envelope, &source, JournalPayloadMode::Metadata);
        let full = journal_log_record(&envelope, &source, JournalPayloadMode::Full);
        assert!(metadata.get("payload").is_none());
        assert_eq!(full["payload"], source.payload);
    }

    #[test]
    fn decorated_single_and_batch_appends_preserve_canonical_results() {
        let records = Arc::new(AtomicUsize::new(0));
        let subscriber =
            tracing_subscriber::registry().with(JournalEventCounter(Arc::clone(&records)));
        let inner: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let journal = ObservedEventJournal::new(Arc::clone(&inner), JournalPayloadMode::Metadata);
        let (first, batch) = tracing::subscriber::with_default(subscriber, || {
            let first = journal.append(event(0)).expect("single append");
            let batch = journal
                .append_batch(vec![event(1), event(2)])
                .expect("batch append");
            (first, batch)
        });
        assert_eq!(first.global_sequence, 1);
        assert_eq!(batch.len(), 2);
        assert_eq!(inner.read_stream("run:test").expect("read").len(), 3);
        assert_eq!(records.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn failed_append_emits_no_live_log_record() {
        let records = Arc::new(AtomicUsize::new(0));
        let subscriber =
            tracing_subscriber::registry().with(JournalEventCounter(Arc::clone(&records)));
        let inner: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let journal = ObservedEventJournal::new(inner, JournalPayloadMode::Full);
        tracing::subscriber::with_default(subscriber, || {
            journal
                .append(event(1))
                .expect_err("wrong expected version must fail");
        });
        assert_eq!(records.load(Ordering::Relaxed), 0);
    }
}
