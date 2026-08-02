use super::*;

/// Maximum number of stream events returned by one ranged journal read.
///
/// Adapters must enforce this ceiling even when a caller supplies a larger
/// `limit`, so untrusted cursors cannot induce unbounded allocation or reads.
pub const MAX_STREAM_READ_BATCH: usize = 1_024;

/// Maximum stream identifiers returned by one indexed discovery page.
pub const MAX_STREAM_LIST_BATCH: usize = 1_024;

/// Collect stream identifiers through bounded indexed pages.
///
/// Repository adapters use this for internal aggregate discovery. Individual
/// journal calls remain bounded even when a repository must inspect every
/// matching aggregate before applying its own status and result limits.
pub fn collect_stream_ids(
    journal: &dyn EventJournal,
    prefix: &str,
) -> Result<Vec<String>, StoreError> {
    let mut streams = Vec::new();
    let mut after = None::<String>;
    loop {
        let page = journal.list_stream_ids(prefix, after.as_deref(), MAX_STREAM_LIST_BATCH)?;
        if page.len() > MAX_STREAM_LIST_BATCH {
            return Err(StoreError::Verification(
                "journal stream discovery exceeded its page bound".into(),
            ));
        }
        if page.is_empty() {
            break;
        }
        let mut previous = after.as_deref();
        for stream_id in &page {
            if !stream_id.starts_with(prefix)
                || previous.is_some_and(|previous| stream_id.as_str() <= previous)
            {
                return Err(StoreError::Verification(
                    "journal stream discovery returned an invalid ordered page".into(),
                ));
            }
            previous = Some(stream_id);
        }
        after = page.last().cloned();
        streams.extend(page);
    }
    Ok(streams)
}

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

    /// List matching stream identifiers in ascending lexical order.
    ///
    /// `after` is an optional exclusive stream-id cursor and must share the
    /// requested prefix. A zero limit returns no identifiers. Larger limits are
    /// clamped to [`MAX_STREAM_LIST_BATCH`].
    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError>;

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
