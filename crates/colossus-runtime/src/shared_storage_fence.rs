//! Every runtime persistence operation remains bound to live distributed ownership.
use colossus_contracts::{
    EventEnvelope, NewEvent, ProjectionBatch, ProjectionWorkItem, SignedCheckpoint,
};
use colossus_ports::{
    EventJournal, ProjectionStore, RuntimeExecutionFence, StoreError, VerificationReport,
};
use serde_json::Value;
use std::sync::Arc;

pub(super) struct FencedJournal {
    inner: Arc<dyn EventJournal>,
    fence: Arc<dyn RuntimeExecutionFence>,
}
impl FencedJournal {
    pub(super) fn new(inner: Arc<dyn EventJournal>, fence: Arc<dyn RuntimeExecutionFence>) -> Self {
        Self { inner, fence }
    }
}
impl EventJournal for FencedJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.fence.verify()?;
        self.inner.append(event)
    }
    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.fence.verify()?;
        self.inner.append_batch(events)
    }
    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.fence.verify()?;
        self.inner.read_stream(stream_id)
    }
    fn read_stream_from(
        &self,
        stream_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.fence.verify()?;
        self.inner.read_stream_from(stream_id, after, limit)
    }
    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.fence.verify()?;
        self.inner.read_stream_backwards(stream_id, before, limit)
    }
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.fence.verify()?;
        self.inner.list_stream_ids(prefix, after, limit)
    }
    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.fence.verify()?;
        self.inner.read_global(from, limit)
    }
    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.fence.verify()?;
        self.inner.read_projection_work(from, limit)
    }
    fn head(&self) -> Result<(u64, String), StoreError> {
        self.fence.verify()?;
        self.inner.head()
    }
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.fence.verify()?;
        self.inner.decrypt_payload(event)
    }
    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.fence.verify()?;
        self.inner.verify()
    }
    fn is_recovery_mode(&self) -> bool {
        self.fence.verify().is_err() || self.inner.is_recovery_mode()
    }
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.fence.verify()?;
        self.inner.checkpoint()
    }
}

pub(super) struct FencedProjections {
    inner: Arc<dyn ProjectionStore>,
    fence: Arc<dyn RuntimeExecutionFence>,
}
impl FencedProjections {
    pub(super) fn new(
        inner: Arc<dyn ProjectionStore>,
        fence: Arc<dyn RuntimeExecutionFence>,
    ) -> Self {
        Self { inner, fence }
    }
}
impl ProjectionStore for FencedProjections {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        self.fence.verify()?;
        self.inner.position(projection)
    }
    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        self.fence.verify()?;
        self.inner.get(projection, key)
    }
    fn list(
        &self,
        projection: &str,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.fence.verify()?;
        self.inner.list(projection, prefix, limit)
    }
    fn list_after(
        &self,
        projection: &str,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        self.fence.verify()?;
        self.inner.list_after(projection, prefix, after, limit)
    }
    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        self.fence.verify()?;
        self.inner.apply(batch)
    }
    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        self.fence.verify()?;
        self.inner.apply_all(batches)
    }
    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        self.fence.verify()?;
        self.inner.reset(projection)
    }
}

#[cfg(test)]
mod tests;
