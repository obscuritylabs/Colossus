use super::*;

/// Supplies journal encryption keys without a plaintext fallback.
pub trait KeyProvider: Send + Sync {
    /// Active key identifier and exactly 32 bytes of key material.
    fn active_key(&self) -> Result<(String, [u8; 32]), StoreError>;

    /// Resolve historical key material by identifier.
    fn key_by_id(&self, key_id: &str) -> Result<[u8; 32], StoreError>;

    /// Persist an independently protected sequence/hash anchor.
    fn store_anchor(&self, sequence: u64, hash: &str) -> Result<(), StoreError>;

    /// Load the last independently protected sequence/hash anchor.
    fn load_anchor(&self) -> Result<Option<(u64, String)>, StoreError>;
}

/// Signs and verifies immutable chain checkpoints.
pub trait CheckpointSigner: Send + Sync {
    /// Stable public key identifier.
    fn key_id(&self) -> &str;

    /// Sign the canonical checkpoint message.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, StoreError>;

    /// Verify a checkpoint signature.
    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), StoreError>;
}

/// Disposable projection state.
pub trait ProjectionStore: Send + Sync {
    /// Last globally applied sequence for a named projection.
    fn position(&self, projection: &str) -> Result<u64, StoreError>;

    /// Load one projection-local record.
    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError>;

    /// List bounded projection-local records in key order.
    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError>;

    /// Atomically apply mutations and advance an optimistic projection position.
    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError>;

    /// Delete a projection so it can be rebuilt.
    fn reset(&self, projection: &str) -> Result<(), StoreError>;
}

/// Durable multi-consumer queue over journal projection work.
///
/// Each consumer owns an independent optimistic checkpoint. Acknowledgment is
/// intentionally separate from reading so an external adapter failure leaves
/// the work visible after a retry or restart.
pub trait ExternalWorkQueue: Send + Sync {
    /// Last durably acknowledged global sequence for one consumer.
    fn position(&self, consumer: &str) -> Result<u64, StoreError>;

    /// Read bounded pending work after the consumer's durable position.
    fn pending(&self, consumer: &str, limit: usize) -> Result<Vec<ProjectionWorkItem>, StoreError>;

    /// Acknowledge exactly the next item using optimistic concurrency.
    fn acknowledge(
        &self,
        consumer: &str,
        expected_position: u64,
        item: &ProjectionWorkItem,
    ) -> Result<u64, StoreError> {
        self.acknowledge_batch(consumer, expected_position, std::slice::from_ref(item))
    }

    /// Atomically acknowledge one non-empty contiguous batch.
    fn acknowledge_batch(
        &self,
        consumer: &str,
        expected_position: u64,
        items: &[ProjectionWorkItem],
    ) -> Result<u64, StoreError>;

    /// Reset one consumer checkpoint so all journal work is replayed.
    fn reset(&self, consumer: &str) -> Result<(), StoreError>;

    /// Load durable retry state when the failure still applies to pending work.
    fn retry_state(&self, consumer: &str) -> Result<Option<ExternalWorkRetryState>, StoreError>;

    /// Persist one failed attempt and compute bounded exponential backoff.
    fn record_failure(
        &self,
        consumer: &str,
        item: Option<&ProjectionWorkItem>,
        failed_at: &str,
        retryable: bool,
        error_code: &str,
        error: &str,
    ) -> Result<ExternalWorkRetryState, StoreError>;

    /// Clear retry state after successful adapter progress or operator reset.
    fn clear_failure(&self, consumer: &str) -> Result<(), StoreError>;
}
