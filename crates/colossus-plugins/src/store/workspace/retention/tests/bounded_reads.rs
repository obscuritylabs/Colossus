use super::*;
use colossus_contracts::{EventEnvelope, ProjectionWorkItem, SignedCheckpoint};
use colossus_ports::VerificationReport;

/// Fail if inventory or collection enumerates history or reads an evicted receipt.
pub(super) fn assert_inventory_and_collection_skip_history(store: &PluginStore) {
    store
        .with_write(|repository| {
            let allowed = repository
                .workspace_cache()?
                .expect("workspace cache")
                .entries
                .into_iter()
                .map(|(digest, entry)| {
                    EventSourcedPluginRepository::installation_stream(&entry.name, &digest)
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            let journal = Arc::new(RetainedOnlyJournal {
                inner: Arc::clone(&repository.journal),
                allowed,
            });
            let bounded = EventSourcedPluginRepository::new(journal);
            assert_eq!(bounded.list_plugins(10_000)?.len(), RECENT_PER_SOURCE);
            assert!(store.gc_locked(&bounded)?.is_empty());
            Ok(())
        })
        .expect("bounded inventory and GC after 31 distinct versions");
}

struct RetainedOnlyJournal {
    inner: Arc<dyn EventJournal>,
    allowed: BTreeSet<String>,
}

impl EventJournal for RetainedOnlyJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.inner.append(event)
    }
    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.append_batch(events)
    }
    fn read_stream(&self, stream: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        if stream.starts_with("plugin:") || stream.starts_with("plugin-active:") {
            return Err(adapter("unbounded plugin stream read"));
        }
        self.inner.read_stream(stream)
    }
    fn read_stream_from(
        &self,
        stream: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream_from(stream, after, limit)
    }
    fn read_stream_backwards(
        &self,
        stream: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        if stream.starts_with("plugin:") && !self.allowed.contains(stream) {
            return Err(adapter("read of a superseded workspace receipt"));
        }
        if limit != 1 || before.is_some() {
            return Err(adapter("plugin inventory must use bounded tail reads"));
        }
        self.inner.read_stream_backwards(stream, before, limit)
    }
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        if prefix == "plugin:" {
            return Err(adapter("enumeration of lifetime plugin history"));
        }
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
