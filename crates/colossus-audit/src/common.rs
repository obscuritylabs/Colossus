use super::*;

/// Stable consumer identity for external audit evidence.
pub const AUDIT_EXPORT_CONSUMER: &str = "audit.export-v1";
/// Actor identity used to prevent recursively exporting export lifecycle events.
pub const AUDIT_EXPORT_ACTOR: &str = "audit-exporter";
pub(super) const MAX_BATCH: usize = 256;
pub(super) const MAX_EVIDENCE_BYTES: usize = 256 * 1024;

#[cfg(test)]
pub(super) fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_AUDIT_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}

pub(super) fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

pub(super) fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

/// Build a ciphertext-free evidence record from one canonical envelope.
#[must_use]
pub fn evidence(event: &EventEnvelope) -> AuditEvidence {
    AuditEvidence {
        schema_version: 1,
        event_version: event.event_version,
        event_id: event.event_id.clone(),
        global_sequence: event.global_sequence,
        stream_id: event.stream_id.clone(),
        stream_version: event.stream_version,
        classification: event.classification,
        event_type: event.event_type.clone(),
        actor: event.actor.clone(),
        context: event.context.clone(),
        occurred_at: event.occurred_at.clone(),
        payload_key_id: event.payload.key_id.clone(),
        payload_algorithm: event.payload.algorithm.clone(),
        payload_plaintext_hash: event.payload.plaintext_hash.clone(),
        previous_hash: event.previous_hash.clone(),
        record_hash: event.record_hash.clone(),
    }
}
