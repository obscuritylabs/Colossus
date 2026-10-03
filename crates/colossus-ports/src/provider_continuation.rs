use crate::StoreError;
use colossus_contracts::{ExecutionContext, ProviderContinuation};

/// Bounded private Responses state. Staging is process-local; only settled turns persist.
pub trait ProviderContinuationRepository: Send + Sync {
    /// Load the latest settled state for a session.
    fn load(&self, session_id: &str) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Stage adapter output without making it reusable or durable.
    fn stage(&self, state: ProviderContinuation) -> Result<(), StoreError>;
    /// Consume a staged candidate after post-policy release; never reuse a candidate.
    fn take_staged(&self, id: &str) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Persist a verified settled candidate, using protected storage only.
    fn save(
        &self,
        state: ProviderContinuation,
        context: &ExecutionContext,
    ) -> Result<(), StoreError>;
    /// Retire state after an incomplete or uncertain turn without deleting history.
    fn clear(&self, session_id: &str, context: &ExecutionContext) -> Result<(), StoreError>;
}
