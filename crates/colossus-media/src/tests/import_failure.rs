use super::*;
use colossus_contracts::{EventEnvelope, ProjectionWorkItem, SignedCheckpoint};
use colossus_ports::{StoreError, VerificationReport};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct RejectLastImageEvent {
    inner: InMemoryEventJournal,
    single_appends: AtomicUsize,
}

impl EventJournal for RejectLastImageEvent {
    fn append(&self, mut event: NewEvent) -> Result<EventEnvelope, StoreError> {
        // The old per-image implementation commits the first image, then fails.
        if self.single_appends.fetch_add(1, Ordering::Relaxed) == 1 {
            event.expected_stream_version = 1;
        }
        self.inner.append(event)
    }

    fn append_batch(&self, mut events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        // A known transaction failure on the final image must roll back the batch.
        events
            .last_mut()
            .expect("nonempty image batch")
            .expected_stream_version = 1;
        self.inner.append_batch(events)
    }

    fn read_stream(&self, id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream(id)
    }
    fn read_stream_from(
        &self,
        id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_from(id, after, limit)
    }
    fn read_stream_backwards(
        &self,
        id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_backwards(id, before, limit)
    }
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.inner.list_stream_ids(prefix, after, limit)
    }
    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_global(from, limit)
    }
    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from, limit)
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

#[test]
fn a_later_storage_failure_retains_no_partial_image_batch() {
    let journal = RejectLastImageEvent::default();
    let before = journal.head().unwrap();
    let bytes = encoded(ImageFormat::Png);
    assert!(
        import_released_images(
            &journal,
            Actor {
                actor_type: ActorType::Application,
                id: "app:image-owner".into()
            },
            ExecutionContext::default(),
            vec![
                ("first.png".into(), bytes.clone()),
                ("second.png".into(), bytes)
            ],
        )
        .is_err()
    );
    assert_eq!(journal.head().unwrap(), before);
    assert!(
        journal
            .list_stream_ids("artifact:", None, 16)
            .unwrap()
            .is_empty()
    );
    assert!(journal.read_projection_work(1, 16).unwrap().is_empty());
}
