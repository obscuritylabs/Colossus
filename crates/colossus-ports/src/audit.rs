use super::*;

/// Bounded redacted audit export sink.
#[async_trait]
pub trait AuditExporter: Send + Sync {
    /// Stable adapter kind for readiness output.
    fn kind(&self) -> &'static str;

    /// Idempotently export one immutable redacted evidence record.
    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError>;
}
