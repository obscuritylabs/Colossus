use super::*;

impl EventJournal for PostgresEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.call(move |journal| journal.append(event))
    }
    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.call(move |journal| journal.append_batch(events))
    }
    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        let id = stream_id.to_owned();
        self.call(move |journal| journal.read_stream(&id))
    }
    fn read_stream_from(
        &self,
        stream_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let id = stream_id.to_owned();
        self.call(move |journal| journal.read_stream_from(&id, after, limit))
    }
    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let id = stream_id.to_owned();
        self.call(move |journal| journal.read_stream_backwards(&id, before, limit))
    }
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        let prefix = prefix.to_owned();
        let after = after.map(str::to_owned);
        self.call(move |journal| journal.list_stream_ids(&prefix, after.as_deref(), limit))
    }
    fn read_global(&self, from: u64, limit: usize) -> Result<Vec<EventEnvelope>, StoreError> {
        self.call(move |journal| journal.read_global(from, limit))
    }
    fn read_projection_work(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.call(move |journal| journal.read_projection_work(from, limit))
    }
    fn head(&self) -> Result<(u64, String), StoreError> {
        self.call(EventJournal::head)
    }
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let event = event.clone();
        self.call(move |journal| journal.decrypt_payload(&event))
    }
    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.call(EventJournal::verify)
    }
    fn is_recovery_mode(&self) -> bool {
        self.call(|journal| Ok(journal.is_recovery_mode()))
            .unwrap_or(true)
    }
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.call(EventJournal::checkpoint)
    }
}

impl ProjectionStore for PostgresEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        let projection = projection.to_owned();
        self.call(move |journal| journal.position(&projection))
    }
    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        let projection = projection.to_owned();
        let key = key.to_owned();
        self.call(move |journal| journal.get(&projection, &key))
    }
    fn list(
        &self,
        projection: &str,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        let projection = projection.to_owned();
        let prefix = prefix.to_owned();
        self.call(move |journal| journal.list(&projection, &prefix, limit))
    }
    fn list_after(
        &self,
        projection: &str,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        let projection = projection.to_owned();
        let prefix = prefix.to_owned();
        let after = after.map(str::to_owned);
        self.call(move |journal| journal.list_after(&projection, &prefix, after.as_deref(), limit))
    }
    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        self.call(move |journal| journal.apply(batch))
    }
    fn apply_all(&self, batches: &[ProjectionBatch]) -> Result<(), StoreError> {
        let batches = batches.to_vec();
        self.call(move |journal| journal.apply_all(&batches))
    }
    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        let projection = projection.to_owned();
        self.call(move |journal| journal.reset(&projection))
    }
}
