//! Durable, policy-bound export of redacted audit evidence.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    Actor, ActorType, AuditEvidence, CredentialReference, EventEnvelope, ExecutionContext,
    ExternalWorkRetryState,
};
use colossus_policy::{EffectExecutor, EffectGateway, GatewayError, effect_request};
use colossus_ports::{AuditExporter, EventJournal, ExternalWorkQueue, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

/// Stable consumer identity for external audit evidence.
pub const AUDIT_EXPORT_CONSUMER: &str = "audit.export-v1";
/// Actor identity used to prevent recursively exporting export lifecycle events.
pub const AUDIT_EXPORT_ACTOR: &str = "audit-exporter";
const MAX_BATCH: usize = 256;
const MAX_EVIDENCE_BYTES: usize = 256 * 1024;

#[cfg(test)]
fn crash_at_test_fault(point: &str) {
    if std::env::var("COLOSSUS_AUDIT_TEST_CRASH_POINT").as_deref() == Ok(point) {
        std::process::abort();
    }
}

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn now() -> Result<String, StoreError> {
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

/// Filesystem exporter whose writes always cross the effect gateway.
pub struct GatewayDirectoryAuditExporter {
    root: std::path::PathBuf,
    gateway: Arc<EffectGateway>,
    executor: Arc<dyn EffectExecutor>,
}

impl GatewayDirectoryAuditExporter {
    /// Bind an existing canonical directory to a permit-requiring filesystem executor.
    pub fn new(
        root: impl AsRef<Path>,
        gateway: Arc<EffectGateway>,
        executor: Arc<dyn EffectExecutor>,
    ) -> Result<Self, StoreError> {
        let root = std::fs::canonicalize(root).map_err(adapter)?;
        if !root.is_dir() {
            return Err(StoreError::Adapter(
                "audit export root must be an existing directory".into(),
            ));
        }
        Ok(Self {
            root,
            gateway,
            executor,
        })
    }

    fn target(&self, evidence: &AuditEvidence) -> std::path::PathBuf {
        self.root.join(format!(
            "{:020}-{}.json",
            evidence.global_sequence, evidence.event_id
        ))
    }
}

#[async_trait]
impl AuditExporter for GatewayDirectoryAuditExporter {
    fn kind(&self) -> &'static str {
        "directory-json"
    }

    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError> {
        let mut encoded = serde_json::to_string(evidence).map_err(adapter)?;
        encoded.push('\n');
        if encoded.len() > MAX_EVIDENCE_BYTES {
            return Err(StoreError::Adapter(
                "redacted audit evidence exceeds 256 KiB".into(),
            ));
        }
        let target = self.target(evidence);
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::System,
                id: AUDIT_EXPORT_ACTOR.into(),
            },
            "audit.export.write",
            target.to_string_lossy(),
            json!({
                "operation": "write",
                "mode": "overwrite",
                "text": encoded,
                "display_path": target.file_name().map(|name| name.to_string_lossy()),
            }),
        );
        request.capabilities = vec!["audit.export.write".into()];
        request.context = ExecutionContext {
            correlation_id: format!("audit-export:{}", evidence.event_id),
            causation_id: Some(evidence.event_id.clone()),
            ..ExecutionContext::default()
        };
        self.gateway
            .execute(request, self.executor.as_ref())
            .await
            .map(|_| ())
            .map_err(|error| match error {
                GatewayError::OutcomeUnknown(message) => StoreError::OutcomeUnknown(message),
                error => StoreError::Adapter(error.to_string()),
            })
    }
}

/// HTTPS create-only exporter for a remote retention-locked or WORM object endpoint.
pub struct GatewayWormAuditExporter {
    endpoint: Url,
    credential_reference: Option<String>,
    gateway: Arc<EffectGateway>,
    executor: Arc<dyn EffectExecutor>,
}

impl GatewayWormAuditExporter {
    /// Bind a trailing-slash HTTPS collection endpoint to the permit-bearing HTTP executor.
    pub fn new(
        endpoint: &str,
        credential_reference: Option<String>,
        gateway: Arc<EffectGateway>,
        executor: Arc<dyn EffectExecutor>,
    ) -> Result<Self, StoreError> {
        let endpoint = Url::parse(endpoint)
            .map_err(|_| StoreError::Adapter("WORM audit endpoint is invalid".into()))?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !endpoint.path().ends_with('/')
            || endpoint.cannot_be_a_base()
        {
            return Err(StoreError::Adapter(
                "WORM audit endpoint must be a credential-free trailing-slash HTTPS URL".into(),
            ));
        }
        if credential_reference
            .as_deref()
            .is_some_and(|reference| !valid_environment_reference(reference))
        {
            return Err(StoreError::Adapter(
                "WORM audit credential must be an env:VARIABLE reference".into(),
            ));
        }
        Ok(Self {
            endpoint,
            credential_reference,
            gateway,
            executor,
        })
    }

    fn target(&self, evidence: &AuditEvidence, content_hash: &str) -> Result<Url, StoreError> {
        let mut target = self.endpoint.clone();
        target
            .path_segments_mut()
            .map_err(|_| StoreError::Adapter("WORM audit object URL is invalid".into()))?
            .push(&format!(
                "{:020}-{}-{content_hash}.json",
                evidence.global_sequence, evidence.event_id
            ));
        Ok(target)
    }
}

#[async_trait]
impl AuditExporter for GatewayWormAuditExporter {
    fn kind(&self) -> &'static str {
        "https-create-only-worm-json"
    }

    async fn export(&self, evidence: &AuditEvidence) -> Result<(), StoreError> {
        let mut encoded = serde_json::to_vec(evidence).map_err(adapter)?;
        encoded.push(b'\n');
        if encoded.len() > MAX_EVIDENCE_BYTES {
            return Err(StoreError::Adapter(
                "redacted audit evidence exceeds 256 KiB".into(),
            ));
        }
        let content_hash = hex::encode(Sha256::digest(&encoded));
        let target = self.target(evidence, &content_hash)?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::System,
                id: AUDIT_EXPORT_ACTOR.into(),
            },
            "audit.export.worm.write",
            target.as_str(),
            json!({
                "method": "PUT",
                "create_only": true,
                "body_base64": BASE64.encode(encoded),
                "content_sha256": content_hash,
            }),
        );
        request.capabilities = vec!["audit.export.worm.write".into()];
        request.credential_references = self
            .credential_reference
            .iter()
            .map(|reference| CredentialReference {
                reference: reference.clone(),
                value_hash: None,
            })
            .collect();
        request.context = ExecutionContext {
            correlation_id: format!("audit-export:{}", evidence.event_id),
            causation_id: Some(evidence.event_id.clone()),
            ..ExecutionContext::default()
        };
        self.gateway
            .execute(request, self.executor.as_ref())
            .await
            .map(|_| ())
            .map_err(|error| match error {
                GatewayError::OutcomeUnknown(message) => StoreError::OutcomeUnknown(message),
                error => StoreError::Adapter(error.to_string()),
            })
    }
}

fn valid_environment_reference(reference: &str) -> bool {
    reference.strip_prefix("env:").is_some_and(|name| {
        let mut bytes = name.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    })
}

/// Bounded readiness for one configured audit-export consumer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditExportStatus {
    /// Whether an exporter is configured.
    pub configured: bool,
    /// Stable exporter kind when configured.
    pub exporter: Option<String>,
    /// Durable consumer identity.
    pub consumer: String,
    /// Last acknowledged journal sequence.
    pub position: u64,
    /// Current authoritative journal head.
    pub journal_head: u64,
    /// Pending global sequences.
    pub lag: u64,
    /// Whether all configured export work is current and retryable.
    pub ready: bool,
    /// Durable retry or operator-block state.
    pub retry: Option<ExternalWorkRetryState>,
}

/// Result of one bounded export operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditExportReport {
    /// Outbox entries examined.
    pub examined: u64,
    /// Evidence records delivered to the sink.
    pub exported: u64,
    /// Exporter's own lifecycle entries intentionally skipped.
    pub skipped: u64,
    /// Readiness after the operation.
    pub status: AuditExportStatus,
}

/// Durable journal-to-exporter application service.
pub struct AuditExportService {
    journal: Arc<dyn EventJournal>,
    queue: Arc<dyn ExternalWorkQueue>,
    exporter: Option<Arc<dyn AuditExporter>>,
}

impl AuditExportService {
    /// Compose an optional exporter over the shared durable work queue.
    #[must_use]
    pub fn new(
        journal: Arc<dyn EventJournal>,
        queue: Arc<dyn ExternalWorkQueue>,
        exporter: Option<Arc<dyn AuditExporter>>,
    ) -> Self {
        Self {
            journal,
            queue,
            exporter,
        }
    }

    /// Return durable exporter position, lag, and retry state.
    pub fn status(&self) -> Result<AuditExportStatus, StoreError> {
        let (journal_head, _) = self.journal.head()?;
        let position = self.queue.position(AUDIT_EXPORT_CONSUMER)?;
        let retry = self.queue.retry_state(AUDIT_EXPORT_CONSUMER)?;
        let configured = self.exporter.is_some();
        Ok(AuditExportStatus {
            configured,
            exporter: self
                .exporter
                .as_ref()
                .map(|exporter| exporter.kind().into()),
            consumer: AUDIT_EXPORT_CONSUMER.into(),
            position,
            journal_head,
            lag: journal_head.saturating_sub(position),
            ready: !configured || (position == journal_head && retry.is_none()),
            retry,
        })
    }

    fn retry_gate(&self) -> Result<(), StoreError> {
        let Some(state) = self.queue.retry_state(AUDIT_EXPORT_CONSUMER)? else {
            return Ok(());
        };
        if !state.retryable {
            return Err(StoreError::OutcomeUnknown(format!(
                "audit export is blocked at sequence {} after {} attempt(s); operator reset required",
                state.global_sequence, state.attempts
            )));
        }
        if let Some(next_retry_at) = state.next_retry_at.as_deref() {
            let next_retry = OffsetDateTime::parse(next_retry_at, &Rfc3339).map_err(|_| {
                StoreError::Verification("audit export retry timestamp is invalid".into())
            })?;
            if OffsetDateTime::now_utc() < next_retry {
                return Err(StoreError::Adapter(format!(
                    "audit export retry is deferred until {next_retry_at}"
                )));
            }
        }
        Ok(())
    }

    /// Export one bounded outbox batch.
    pub async fn run_once(&self, limit: usize) -> Result<AuditExportReport, StoreError> {
        let Some(exporter) = self.exporter.as_ref() else {
            return Ok(AuditExportReport {
                examined: 0,
                exported: 0,
                skipped: 0,
                status: self.status()?,
            });
        };
        if limit == 0 || limit > MAX_BATCH {
            return Err(StoreError::Adapter(format!(
                "audit export batch must be in 1..={MAX_BATCH}"
            )));
        }
        self.retry_gate()?;
        let work = self.queue.pending(AUDIT_EXPORT_CONSUMER, limit)?;
        let mut exported = 0_u64;
        let mut skipped = 0_u64;
        for item in &work {
            let event = self
                .journal
                .read_global(item.global_sequence, 1)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "audit export sequence {} has no journal event",
                        item.global_sequence
                    ))
                })?;
            if event.event_id != item.event_id || event.global_sequence != item.global_sequence {
                return Err(StoreError::Verification(format!(
                    "audit export sequence {} does not match its journal event",
                    item.global_sequence
                )));
            }
            if event.actor.actor_type == ActorType::System && event.actor.id == AUDIT_EXPORT_ACTOR {
                skipped = skipped.saturating_add(1);
                continue;
            }
            if let Err(error) = exporter.export(&evidence(&event)).await {
                let (retryable, code) = export_retry_classification(&error);
                let diagnostic = bounded_error(&error);
                self.queue.record_failure(
                    AUDIT_EXPORT_CONSUMER,
                    Some(item),
                    &now()?,
                    retryable,
                    code,
                    &diagnostic,
                )?;
                return Err(error);
            }
            #[cfg(test)]
            crash_at_test_fault("after_export_before_ack");
            exported = exported.saturating_add(1);
        }
        if !work.is_empty() {
            let position = self.queue.position(AUDIT_EXPORT_CONSUMER)?;
            self.queue
                .acknowledge_batch(AUDIT_EXPORT_CONSUMER, position, &work)?;
            self.queue.clear_failure(AUDIT_EXPORT_CONSUMER)?;
        }
        Ok(AuditExportReport {
            examined: u64::try_from(work.len()).map_err(adapter)?,
            exported,
            skipped,
            status: self.status()?,
        })
    }

    /// Drain bounded batches until current or the round budget is exhausted.
    pub async fn drain(
        &self,
        batch_limit: usize,
        max_rounds: usize,
    ) -> Result<AuditExportReport, StoreError> {
        if max_rounds == 0 {
            return Err(StoreError::Adapter(
                "audit export drain rounds must be greater than zero".into(),
            ));
        }
        let mut report = AuditExportReport {
            examined: 0,
            exported: 0,
            skipped: 0,
            status: self.status()?,
        };
        for _ in 0..max_rounds {
            let next = self.run_once(batch_limit).await?;
            report.examined = report.examined.saturating_add(next.examined);
            report.exported = report.exported.saturating_add(next.exported);
            report.skipped = report.skipped.saturating_add(next.skipped);
            report.status = next.status;
            if report.status.ready || next.examined == 0 {
                break;
            }
        }
        Ok(report)
    }

    /// Reset the consumer and retry state for operator-authorized replay.
    pub fn reset(&self) -> Result<AuditExportStatus, StoreError> {
        self.queue.reset(AUDIT_EXPORT_CONSUMER)?;
        self.status()
    }
}

fn export_retry_classification(error: &StoreError) -> (bool, &'static str) {
    match error {
        StoreError::Conflict { .. } => (true, "audit_export.conflict"),
        StoreError::KeyUnavailable(_) => (true, "audit_export.key_unavailable"),
        StoreError::Adapter(_) => (true, "audit_export.adapter"),
        StoreError::NotFound(_) => (false, "audit_export.not_found"),
        StoreError::Verification(_) => (false, "audit_export.verification"),
        StoreError::OutcomeUnknown(_) => (false, "audit_export.outcome_unknown"),
        StoreError::RecoveryMode => (false, "audit_export.recovery_mode"),
    }
}

fn bounded_error(error: &StoreError) -> String {
    const MAX_BYTES: usize = 2_048;
    let source = error.to_string();
    let mut bounded = String::with_capacity(source.len().min(MAX_BYTES));
    for character in source.chars() {
        if bounded.len().saturating_add(character.len_utf8()) > MAX_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded
}

#[cfg(test)]
mod tests;
