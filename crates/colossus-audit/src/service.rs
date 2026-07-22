use super::*;

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
        StoreError::WriterLeaseHeld => (false, "audit_export.writer_lease_held"),
        StoreError::WorkspaceIdentityChanged => (false, "audit_export.workspace_identity_changed"),
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
