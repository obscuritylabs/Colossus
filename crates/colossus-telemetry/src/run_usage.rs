use crate::TelemetryService;
use colossus_contracts::{EventEnvelope, ProviderUsage, RunTokenUsage};
use colossus_ports::{MAX_STREAM_READ_BATCH, StoreError};

impl TelemetryService {
    /// Sum provider reports from one exact session-bound canonical run stream.
    ///
    /// The query freezes the durable tail and reads bounded indexed pages through
    /// that version. It exposes no prompt, output, or provider-private payloads.
    /// Absent reports remain unknown; journal coverage is not a billing guarantee.
    pub fn run_token_usage(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<RunTokenUsage, StoreError> {
        if session_id.is_empty() || run_id.is_empty() {
            return Err(invalid_usage_history());
        }
        let stream_id = format!("run:{run_id}");
        let mut tail = self.journal.read_stream_backwards(&stream_id, None, 1)?;
        if tail.is_empty() {
            return Err(StoreError::NotFound("run token usage".into()));
        }
        if tail.len() != 1 {
            return Err(invalid_usage_history());
        }
        let tail = tail.pop().ok_or_else(invalid_usage_history)?;
        validate_identity(&tail, &stream_id, session_id, run_id)?;
        let through_version = tail.stream_version;
        if through_version == 0 {
            return Err(invalid_usage_history());
        }
        let mut after = 0_u64;
        let mut reports = 0_u64;
        let mut usage = None::<ProviderUsage>;
        while after < through_version {
            let remaining = through_version - after;
            let limit = usize::try_from(remaining)
                .unwrap_or(MAX_STREAM_READ_BATCH)
                .min(MAX_STREAM_READ_BATCH);
            let page = self.journal.read_stream_from(&stream_id, after, limit)?;
            if page.is_empty() || page.len() > limit {
                return Err(invalid_usage_history());
            }
            for event in page {
                validate_identity(&event, &stream_id, session_id, run_id)?;
                let expected = after.checked_add(1).ok_or_else(invalid_usage_history)?;
                if event.stream_version != expected || event.stream_version > through_version {
                    return Err(invalid_usage_history());
                }
                if (expected == 1) != (event.event_type == "run.started.v1") {
                    return Err(invalid_usage_history());
                }
                if event.event_type == "provider.usage.v1" {
                    let report: ProviderUsage =
                        serde_json::from_value(self.journal.decrypt_payload(&event)?)
                            .map_err(|_| invalid_usage_history())?;
                    usage = Some(match usage {
                        None => report,
                        Some(total) => total.checked_add(&report).ok_or_else(usage_overflow)?,
                    });
                    reports = reports.checked_add(1).ok_or_else(usage_overflow)?;
                }
                if event.stream_version == through_version && event != tail {
                    return Err(invalid_usage_history());
                }
                after = event.stream_version;
            }
        }
        Ok(RunTokenUsage {
            run_id: run_id.into(),
            session_id: session_id.into(),
            through_version,
            reports,
            usage,
        })
    }
}

fn validate_identity(
    event: &EventEnvelope,
    stream_id: &str,
    session_id: &str,
    run_id: &str,
) -> Result<(), StoreError> {
    if event.schema_version != 1
        || event.event_version != 1
        || event.stream_id != stream_id
        || event.context.run_id.as_deref() != Some(run_id)
        || event.context.session_id.as_deref() != Some(session_id)
    {
        return Err(invalid_usage_history());
    }
    Ok(())
}

fn invalid_usage_history() -> StoreError {
    StoreError::Verification("run token usage history is incomplete or inconsistent".into())
}

fn usage_overflow() -> StoreError {
    StoreError::Verification("reported run token usage exceeds the accounting bound".into())
}
