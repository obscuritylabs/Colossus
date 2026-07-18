use super::*;

/// Disposable search projection for canonical memory identifiers.
#[async_trait]
pub trait MemoryIndex: Send + Sync {
    /// Last durably applied global journal sequence.
    fn position(&self) -> Result<u64, StoreError>;

    /// Persist the last fully applied global journal sequence.
    async fn set_position(&self, position: u64) -> Result<(), StoreError>;

    /// Idempotently add/update an indexed record using the source event id.
    async fn upsert(
        &self,
        event_id: &str,
        memory_id: &str,
        text: &str,
        metadata: &Value,
        embedding: Option<&[f32]>,
    ) -> Result<(), StoreError>;

    /// Idempotently remove an indexed record.
    async fn remove(&self, event_id: &str, memory_id: &str) -> Result<(), StoreError>;

    /// Return candidate identifiers and scores only.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError>;

    /// Return bounded readiness and lag metadata.
    async fn status(&self) -> Result<Value, StoreError>;

    /// Rebuild from canonical records supplied by the caller.
    async fn rebuild(&self, records: &[(String, String, Value)]) -> Result<(), StoreError>;
}

/// Policy-aware relevant-memory retrieval used by context composition.
#[async_trait]
pub trait MemoryRetriever: Send + Sync {
    /// Return canonical active records authorized for this session and query.
    async fn relevant(
        &self,
        query: &str,
        session_id: &str,
        context: ExecutionContext,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;
}

/// Provider for caller-generated embedding vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed bounded input text.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, StoreError>;
}
