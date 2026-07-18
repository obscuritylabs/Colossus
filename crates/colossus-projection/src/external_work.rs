use super::*;

const EXTERNAL_WORK_PREFIX: &str = "external-work:";
const EXTERNAL_WORK_RETRY_PREFIX: &str = "external-work-retry:";
const EXTERNAL_WORK_RETRY_KEY: &str = "state";
const MAX_EXTERNAL_WORK_BATCH: usize = 4_096;
const MAX_RETRY_STATE_ATTEMPTS: usize = 8;
const MAX_RETRY_ERROR_BYTES: usize = 2_048;
const MAX_RETRY_BACKOFF_SECONDS: i64 = 300;

fn external_work_projection(consumer: &str) -> Result<String, StoreError> {
    if consumer.is_empty()
        || consumer.len() > 128
        || !consumer
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(StoreError::Adapter(
            "external-work consumer must be 1-128 ASCII letters, digits, '.', '_', or '-'".into(),
        ));
    }
    Ok(format!("{EXTERNAL_WORK_PREFIX}{consumer}"))
}

fn external_work_retry_projection(consumer: &str) -> Result<String, StoreError> {
    external_work_projection(consumer)?;
    Ok(format!("{EXTERNAL_WORK_RETRY_PREFIX}{consumer}"))
}

fn retry_state_from_value(value: Value) -> Result<ExternalWorkRetryState, StoreError> {
    serde_json::from_value(value).map_err(|error| {
        StoreError::Verification(format!("external-work retry state is invalid: {error}"))
    })
}

fn retry_timestamp(value: &str) -> Result<OffsetDateTime, StoreError> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        StoreError::Adapter("external-work failure timestamp must be UTC RFC3339".into())
    })?;
    if timestamp.offset() != UtcOffset::UTC {
        return Err(StoreError::Adapter(
            "external-work failure timestamp must be UTC RFC3339".into(),
        ));
    }
    Ok(timestamp)
}

fn retry_backoff(attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(9);
    let seconds = 1_i64
        .checked_shl(exponent)
        .unwrap_or(MAX_RETRY_BACKOFF_SECONDS)
        .min(MAX_RETRY_BACKOFF_SECONDS);
    Duration::seconds(seconds)
}

/// Durable per-consumer checkpoints over the journal's atomic projection outbox.
pub struct JournalExternalWorkQueue {
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn ProjectionStore>,
}

impl JournalExternalWorkQueue {
    /// Compose the queue from the authoritative journal and a durable checkpoint store.
    #[must_use]
    pub fn new(journal: Arc<dyn EventJournal>, store: Arc<dyn ProjectionStore>) -> Self {
        Self { journal, store }
    }

    fn verify_item(&self, item: &ProjectionWorkItem) -> Result<(), StoreError> {
        let durable = self
            .journal
            .read_projection_work(item.global_sequence, 1)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                StoreError::Verification(format!(
                    "external-work outbox sequence {} is missing",
                    item.global_sequence
                ))
            })?;
        if durable != *item {
            return Err(StoreError::Verification(format!(
                "external-work outbox sequence {} does not match its durable item",
                item.global_sequence
            )));
        }
        Ok(())
    }
}

impl ExternalWorkQueue for JournalExternalWorkQueue {
    fn position(&self, consumer: &str) -> Result<u64, StoreError> {
        self.store.position(&external_work_projection(consumer)?)
    }

    fn pending(&self, consumer: &str, limit: usize) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        if limit == 0 || limit > MAX_EXTERNAL_WORK_BATCH {
            return Err(StoreError::Adapter(format!(
                "external-work batch limit must be between 1 and {MAX_EXTERNAL_WORK_BATCH}"
            )));
        }
        let position = self.position(consumer)?;
        let work = self
            .journal
            .read_projection_work(position.saturating_add(1), limit)?;
        let mut expected = position.saturating_add(1);
        for item in &work {
            if item.global_sequence != expected {
                return Err(StoreError::Verification(format!(
                    "external-work consumer {consumer} expected sequence {expected}, got {}",
                    item.global_sequence
                )));
            }
            self.verify_item(item)?;
            expected = expected.saturating_add(1);
        }
        Ok(work)
    }

    fn acknowledge_batch(
        &self,
        consumer: &str,
        expected_position: u64,
        items: &[ProjectionWorkItem],
    ) -> Result<u64, StoreError> {
        if items.is_empty() || items.len() > MAX_EXTERNAL_WORK_BATCH {
            return Err(StoreError::Adapter(format!(
                "external-work acknowledgment batch must contain 1-{MAX_EXTERNAL_WORK_BATCH} items"
            )));
        }
        let projection = external_work_projection(consumer)?;
        let mut next = expected_position.saturating_add(1);
        for item in items {
            if item.global_sequence != next {
                return Err(StoreError::Adapter(format!(
                    "external-work acknowledgment expected sequence {next}, got {}",
                    item.global_sequence
                )));
            }
            self.verify_item(item)?;
            next = next.saturating_add(1);
        }
        let through_sequence = items
            .last()
            .map(|item| item.global_sequence)
            .ok_or_else(|| StoreError::Adapter("external-work batch is empty".into()))?;
        self.store.apply(ProjectionBatch {
            projection,
            expected_position,
            through_sequence,
            mutations: Vec::new(),
        })?;
        Ok(through_sequence)
    }

    fn reset(&self, consumer: &str) -> Result<(), StoreError> {
        self.store.reset(&external_work_projection(consumer)?)?;
        self.store.reset(&external_work_retry_projection(consumer)?)
    }

    fn retry_state(&self, consumer: &str) -> Result<Option<ExternalWorkRetryState>, StoreError> {
        let position = self.position(consumer)?;
        let projection = external_work_retry_projection(consumer)?;
        let Some(value) = self.store.get(&projection, EXTERNAL_WORK_RETRY_KEY)? else {
            return Ok(None);
        };
        let state = retry_state_from_value(value)?;
        let first_failed_at = OffsetDateTime::parse(&state.first_failed_at, &Rfc3339);
        let last_failed_at = OffsetDateTime::parse(&state.last_failed_at, &Rfc3339);
        let next_retry_at = state
            .next_retry_at
            .as_deref()
            .map(|value| OffsetDateTime::parse(value, &Rfc3339))
            .transpose();
        let timestamps_valid = matches!(
            (&first_failed_at, &last_failed_at, &next_retry_at),
            (Ok(first), Ok(last), Ok(next))
                if first.offset() == UtcOffset::UTC
                    && last.offset() == UtcOffset::UTC
                    && next.as_ref().is_none_or(|value| value.offset() == UtcOffset::UTC)
                    && first <= last
        );
        if state.consumer != consumer
            || state.global_sequence == 0
            || state.attempts == 0
            || state.event_id.as_deref().is_some_and(str::is_empty)
            || state.retryable != state.next_retry_at.is_some()
            || state.error_code.is_empty()
            || state.error_code.len() > 128
            || !state
                .error_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || state.error.is_empty()
            || state.error.len() > MAX_RETRY_ERROR_BYTES
            || !timestamps_valid
        {
            return Err(StoreError::Verification(
                "external-work retry state failed semantic validation".into(),
            ));
        }
        Ok((state.global_sequence > position).then_some(state))
    }

    fn record_failure(
        &self,
        consumer: &str,
        item: Option<&ProjectionWorkItem>,
        failed_at: &str,
        retryable: bool,
        error_code: &str,
        error: &str,
    ) -> Result<ExternalWorkRetryState, StoreError> {
        let failed_at_value = retry_timestamp(failed_at)?;
        if error_code.is_empty()
            || error_code.len() > 128
            || !error_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || error.is_empty()
            || error.len() > MAX_RETRY_ERROR_BYTES
        {
            return Err(StoreError::Adapter(
                "external-work failure code or redacted diagnostic is invalid".into(),
            ));
        }
        let work_position = self.position(consumer)?;
        if let Some(item) = item {
            if item.global_sequence != work_position.saturating_add(1) {
                return Err(StoreError::Adapter(
                    "external-work failure must refer to the next pending item".into(),
                ));
            }
            self.verify_item(item)?;
        }
        let global_sequence = item.map_or_else(
            || work_position.saturating_add(1),
            |item| item.global_sequence,
        );
        if global_sequence <= work_position {
            return Err(StoreError::Adapter(
                "external-work failure must refer to pending work".into(),
            ));
        }
        let event_id = item.map(|item| item.event_id.clone());
        let projection = external_work_retry_projection(consumer)?;
        for _ in 0..MAX_RETRY_STATE_ATTEMPTS {
            let retry_position = self.store.position(&projection)?;
            let existing = self
                .store
                .get(&projection, EXTERNAL_WORK_RETRY_KEY)?
                .map(retry_state_from_value)
                .transpose()?;
            let same_work = existing
                .as_ref()
                .is_some_and(|state| state.global_sequence == global_sequence);
            let attempts = if same_work {
                existing
                    .as_ref()
                    .map_or(1, |state| state.attempts.saturating_add(1))
            } else {
                1
            };
            let first_failed_at = if same_work {
                existing.as_ref().map_or_else(
                    || failed_at.to_owned(),
                    |state| state.first_failed_at.clone(),
                )
            } else {
                failed_at.to_owned()
            };
            let next_retry_at = retryable
                .then(|| failed_at_value + retry_backoff(attempts))
                .map(|timestamp| {
                    timestamp
                        .format(&Rfc3339)
                        .map_err(|error| StoreError::Adapter(error.to_string()))
                })
                .transpose()?;
            let state = ExternalWorkRetryState {
                consumer: consumer.into(),
                global_sequence,
                event_id: event_id.clone(),
                attempts,
                retryable,
                first_failed_at,
                last_failed_at: failed_at.into(),
                next_retry_at,
                error_code: error_code.into(),
                error: error.into(),
            };
            let result = self.store.apply(ProjectionBatch {
                projection: projection.clone(),
                expected_position: retry_position,
                through_sequence: retry_position.saturating_add(1),
                mutations: vec![ProjectionMutation::Upsert {
                    key: EXTERNAL_WORK_RETRY_KEY.into(),
                    value: serde_json::to_value(&state)
                        .map_err(|error| StoreError::Adapter(error.to_string()))?,
                }],
            });
            match result {
                Ok(()) => return Ok(state),
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(StoreError::Adapter(
            "external-work retry state remained contended".into(),
        ))
    }

    fn clear_failure(&self, consumer: &str) -> Result<(), StoreError> {
        let projection = external_work_retry_projection(consumer)?;
        for _ in 0..MAX_RETRY_STATE_ATTEMPTS {
            let retry_position = self.store.position(&projection)?;
            if self
                .store
                .get(&projection, EXTERNAL_WORK_RETRY_KEY)?
                .is_none()
            {
                return Ok(());
            }
            let result = self.store.apply(ProjectionBatch {
                projection: projection.clone(),
                expected_position: retry_position,
                through_sequence: retry_position.saturating_add(1),
                mutations: vec![ProjectionMutation::Delete {
                    key: EXTERNAL_WORK_RETRY_KEY.into(),
                }],
            });
            match result {
                Ok(()) => return Ok(()),
                Err(StoreError::Conflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(StoreError::Adapter(
            "external-work retry state remained contended".into(),
        ))
    }
}
