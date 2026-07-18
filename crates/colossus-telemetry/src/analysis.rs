use super::*;

pub(super) fn event_run_id(event: &EventEnvelope) -> Option<String> {
    event.context.run_id.clone()
}

pub(super) fn resolve_run_id<'a>(
    ids: impl Iterator<Item = &'a String>,
    value: &str,
) -> Result<String, StoreError> {
    if value.trim().is_empty() {
        return Err(StoreError::NotFound("empty telemetry run id".into()));
    }
    let ids = ids.cloned().collect::<Vec<_>>();
    if ids.iter().any(|id| id == value) {
        return Ok(value.into());
    }
    if value.len() < 4 {
        return Err(StoreError::NotFound(format!("telemetry run {value}")));
    }
    let matches = ids
        .into_iter()
        .filter(|id| id.starts_with(value))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [run_id] => Ok(run_id.clone()),
        [] => Err(StoreError::NotFound(format!("telemetry run {value}"))),
        _ => Err(StoreError::Adapter(format!(
            "ambiguous telemetry run id prefix {value}"
        ))),
    }
}

pub(super) fn metadata_record(run_id: &str, event: &EventEnvelope) -> TelemetryEventRecord {
    TelemetryEventRecord {
        sequence: event.global_sequence,
        event_id: event.event_id.clone(),
        run_id: run_id.into(),
        event_type: event.event_type.clone(),
        classification: event.classification,
        actor_type: event.actor.actor_type,
        actor_id: event.actor.id.clone(),
        context: event.context.clone(),
        created_at: event.occurred_at.clone(),
        payload_hash: event.payload.plaintext_hash.clone(),
        encrypted_payload_bytes: event.payload.ciphertext.len(),
    }
}

pub(super) fn summarize(
    journal: &dyn EventJournal,
    events: &[EventEnvelope],
) -> Result<RunTelemetrySummary, StoreError> {
    let first = events
        .first()
        .ok_or_else(|| StoreError::Adapter("cannot summarize an empty telemetry run".into()))?;
    let last = events.last().unwrap_or(first);
    let run_id = event_run_id(first)
        .ok_or_else(|| StoreError::Verification("telemetry event lost run identity".into()))?;
    let mut summary = RunTelemetrySummary {
        run_id,
        session_id: first.context.session_id.clone(),
        started_at: first.occurred_at.clone(),
        last_event_at: last.occurred_at.clone(),
        duration_seconds: duration_seconds(&first.occurred_at, &last.occurred_at),
        events: events.len(),
        event_types: BTreeMap::new(),
        model_output_chars: 0,
        tool_calls: 0,
        tool_errors: 0,
        approval_requests: 0,
        auto_approvals: 0,
        risk_assessments: 0,
        research_events: 0,
        subagent_events: 0,
        context_compactions: 0,
        error_events: 0,
        final_outputs: 0,
        provider_input_tokens: 0,
        provider_output_tokens: 0,
        provider_total_tokens: 0,
        provider_cached_input_tokens: 0,
        provider_reasoning_tokens: 0,
    };
    let mut delta_output_chars = 0_usize;
    let mut final_output_chars = 0_usize;
    for event in events {
        *summary
            .event_types
            .entry(event.event_type.clone())
            .or_default() += 1;
        match event.event_type.as_str() {
            "model.delta.v1" => {
                delta_output_chars =
                    delta_output_chars.saturating_add(payload_text_chars(journal, event, "text")?);
            }
            "final.output.v1" => {
                summary.final_outputs = summary.final_outputs.saturating_add(1);
                final_output_chars =
                    final_output_chars.saturating_add(payload_text_chars(journal, event, "text")?);
            }
            "tool.call.requested.v1" => {
                summary.tool_calls = summary.tool_calls.saturating_add(1);
            }
            "tool.call.completed.v1" => {
                if payload_i64(journal, event, "exit_code")?.is_some_and(|code| code != 0) {
                    summary.tool_errors = summary.tool_errors.saturating_add(1);
                }
            }
            "provider.usage.v1" => {
                let usage = payload(journal, event)?;
                summary.provider_input_tokens = summary.provider_input_tokens.saturating_add(
                    usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                summary.provider_output_tokens = summary.provider_output_tokens.saturating_add(
                    usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                summary.provider_total_tokens = summary.provider_total_tokens.saturating_add(
                    usage
                        .get("total_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                summary.provider_cached_input_tokens =
                    summary.provider_cached_input_tokens.saturating_add(
                        usage
                            .get("cached_input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
                summary.provider_reasoning_tokens =
                    summary.provider_reasoning_tokens.saturating_add(
                        usage
                            .get("reasoning_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    );
            }
            "approval.granted.v1" | "approval.denied.v1" | "approval.error.v1" => {
                summary.approval_requests = summary.approval_requests.saturating_add(1);
                if event.event_type == "approval.granted.v1"
                    && payload_string(journal, event, "approved_by")?.is_some_and(|value| {
                        value.contains("full-access") || value.contains("auto")
                    })
                {
                    summary.auto_approvals = summary.auto_approvals.saturating_add(1);
                }
            }
            _ => {}
        }
        if event.event_type == "context.prepared.v1"
            && payload_bool(journal, event, "compacted")?.unwrap_or(false)
        {
            summary.context_compactions = summary.context_compactions.saturating_add(1);
        }
        if event.event_type.starts_with("risk.") {
            summary.risk_assessments = summary.risk_assessments.saturating_add(1);
        }
        if event.event_type.starts_with("research.") {
            summary.research_events = summary.research_events.saturating_add(1);
        }
        if event.event_type.starts_with("subagent.") || event.context.subagent_id.is_some() {
            summary.subagent_events = summary.subagent_events.saturating_add(1);
        }
        if is_error_event(&event.event_type) {
            summary.error_events = summary.error_events.saturating_add(1);
        }
    }
    summary.model_output_chars = if delta_output_chars > 0 {
        delta_output_chars
    } else {
        final_output_chars
    };
    Ok(summary)
}

pub(super) fn is_error_event(event_type: &str) -> bool {
    event_type == "error.v1"
        || event_type.contains("failed")
        || event_type.contains("outcome_unknown")
        || event_type.contains("release_denied")
}

pub(super) fn payload(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
) -> Result<Value, StoreError> {
    journal.decrypt_payload(event)
}

pub(super) fn payload_text_chars(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<usize, StoreError> {
    Ok(payload(journal, event)?
        .get(field)
        .and_then(Value::as_str)
        .map_or(0, |text| text.chars().count()))
}

pub(super) fn payload_i64(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<i64>, StoreError> {
    Ok(payload(journal, event)?.get(field).and_then(Value::as_i64))
}

pub(super) fn payload_bool(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<bool>, StoreError> {
    Ok(payload(journal, event)?.get(field).and_then(Value::as_bool))
}

pub(super) fn payload_string(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<String>, StoreError> {
    Ok(payload(journal, event)?
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned))
}

pub(super) fn duration_seconds(started_at: &str, ended_at: &str) -> f64 {
    let Ok(started) = OffsetDateTime::parse(started_at, &Rfc3339) else {
        return 0.0;
    };
    let Ok(ended) = OffsetDateTime::parse(ended_at, &Rfc3339) else {
        return 0.0;
    };
    let duration = ended - started;
    if duration.is_negative() {
        0.0
    } else {
        duration.as_seconds_f64()
    }
}
