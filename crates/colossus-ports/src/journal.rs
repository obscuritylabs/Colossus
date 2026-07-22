use super::*;

/// Maximum number of stream events returned by one ranged journal read.
///
/// Adapters must enforce this ceiling even when a caller supplies a larger
/// `limit`, so untrusted cursors cannot induce unbounded allocation or reads.
pub const MAX_STREAM_READ_BATCH: usize = 1_024;

/// Authoritative immutable event store.
pub trait EventJournal: Send + Sync {
    /// Append one event atomically.
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError>;

    /// Append events in one transaction and in the supplied order.
    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read a stream in ascending version order.
    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read stream events after an exclusive version cursor in ascending order.
    ///
    /// A zero `limit` returns no events. Larger limits are clamped to
    /// [`MAX_STREAM_READ_BATCH`].
    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read stream events newest-first below an optional exclusive version cursor.
    ///
    /// `None` starts at the current stream tail. A zero `limit` returns no events.
    /// Larger limits are clamped to [`MAX_STREAM_READ_BATCH`].
    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read global events from a one-based sequence, bounded by `limit`.
    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError>;

    /// Read projection outbox items in ascending sequence order.
    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError>;

    /// Return the durable global sequence and record hash at the journal head.
    fn head(&self) -> Result<(u64, String), StoreError>;

    /// Decrypt an event payload after policy has authorized disclosure.
    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError>;

    /// Verify encryption, hashes, sequence, secure anchor, and checkpoints.
    fn verify(&self) -> Result<VerificationReport, StoreError>;

    /// Return whether writes are blocked due to failed verification.
    fn is_recovery_mode(&self) -> bool;

    /// Create a signed checkpoint at the current chain head.
    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError>;
}
