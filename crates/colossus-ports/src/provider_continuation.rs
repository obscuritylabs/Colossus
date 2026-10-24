use crate::StoreError;
use colossus_contracts::{ExecutionContext, ProviderContinuation};

/// Bounded private provider state. Quarantined staging is process-local; protected
/// admission and settlement writes join the corresponding session transactions.
pub trait ProviderContinuationRepository: Send + Sync {
    /// Load the latest settled state for a session.
    fn load(&self, session_id: &str) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Stage adapter output without making it reusable or durable.
    fn stage(&self, state: ProviderContinuation) -> Result<(), StoreError>;
    /// Consume a staged candidate after post-policy release; never reuse a candidate.
    fn take_staged(&self, id: &str) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Read a released candidate, including a protected candidate admitted before effects.
    fn candidate(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Prepare protected candidate admission for the session pending-marker transaction.
    fn admission_events(
        &self,
        session_id: &str,
        id: &str,
        context: &ExecutionContext,
    ) -> Result<Vec<crate::NewEvent>, StoreError>;
    /// Prepare protected settlement for the canonical assistant/tool-message transaction.
    fn settlement_events(
        &self,
        state: &ProviderContinuation,
        context: &ExecutionContext,
    ) -> Result<Vec<crate::NewEvent>, StoreError>;
    /// Latest historical settled replay hidden by a legacy retirement marker.
    /// Callers must validate canonical and route bindings before restoring it.
    fn retired_replay(&self, session_id: &str) -> Result<Option<ProviderContinuation>, StoreError>;
    /// Persist a verified settled candidate, using protected storage only.
    fn save(
        &self,
        state: ProviderContinuation,
        context: &ExecutionContext,
    ) -> Result<(), StoreError>;
    /// Retire completed or explicitly invalidated state without deleting history.
    fn clear(&self, session_id: &str, context: &ExecutionContext) -> Result<(), StoreError>;
}
