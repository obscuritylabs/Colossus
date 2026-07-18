use super::*;

/// Assert the common stable-kind and idempotent-delivery contract for an audit sink.
pub async fn assert_audit_exporter_conformance(
    exporter: &dyn AuditExporter,
    evidence: &AuditEvidence,
) {
    assert!(!exporter.kind().trim().is_empty());
    exporter
        .export(evidence)
        .await
        .expect("first conformance export");
    exporter
        .export(evidence)
        .await
        .expect("idempotent conformance replay");
}
