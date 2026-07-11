//! Metadata-only operational telemetry derived from persisted journal events.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    EventEnvelope, RunTelemetryDetail, RunTelemetrySummary, TelemetryEventRecord, TelemetryMetrics,
};
use colossus_ports::{EventJournal, StoreError};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_SCAN_EVENTS: u64 = 100_000;
const MAX_RUNS: usize = 1_000;
const MAX_DETAIL_EVENTS: usize = 10_000;

/// Read-only telemetry queries over the authoritative encrypted journal.
pub struct TelemetryService {
    journal: Arc<dyn EventJournal>,
}

impl TelemetryService {
    /// Bind telemetry to persisted event envelopes rather than rendered transcripts.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    /// List recent metadata-only run summaries.
    pub fn list_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RunTelemetrySummary>, StoreError> {
        let mut grouped = self.grouped()?;
        let mut summaries = grouped
            .values_mut()
            .map(|events| summarize(self.journal.as_ref(), events))
            .collect::<Result<Vec<_>, _>>()?;
        summaries.retain(|summary| {
            session_id.is_none_or(|session_id| summary.session_id.as_deref() == Some(session_id))
        });
        summaries.sort_by(|left, right| {
            right
                .last_event_at
                .cmp(&left.last_event_at)
                .then_with(|| right.run_id.cmp(&left.run_id))
        });
        summaries.truncate(limit.clamp(1, MAX_RUNS));
        Ok(summaries)
    }

    /// Resolve a full run id or unique prefix and return a bounded metadata timeline.
    pub fn get_run(
        &self,
        id_or_prefix: &str,
        limit: usize,
    ) -> Result<RunTelemetryDetail, StoreError> {
        let grouped = self.grouped()?;
        let run_id = resolve_run_id(grouped.keys(), id_or_prefix)?;
        let events = grouped
            .get(&run_id)
            .ok_or_else(|| StoreError::NotFound(format!("telemetry run {id_or_prefix}")))?;
        let summary = summarize(self.journal.as_ref(), events)?;
        let bounded = limit.clamp(1, MAX_DETAIL_EVENTS);
        let truncated = events.len() > bounded;
        let records = events
            .iter()
            .take(bounded)
            .map(|event| metadata_record(&run_id, event))
            .collect();
        Ok(RunTelemetryDetail {
            summary,
            records,
            truncated,
        })
    }

    /// Aggregate metadata-only counters over bounded recent runs.
    pub fn metrics(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<TelemetryMetrics, StoreError> {
        let summaries = self.list_runs(session_id, limit)?;
        let mut metrics = TelemetryMetrics {
            run_count: summaries.len(),
            event_count: 0,
            average_duration_seconds: 0.0,
            max_duration_seconds: 0.0,
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
            event_types: BTreeMap::new(),
        };
        let mut duration_total = 0.0;
        for summary in summaries {
            duration_total += summary.duration_seconds;
            metrics.max_duration_seconds =
                metrics.max_duration_seconds.max(summary.duration_seconds);
            metrics.event_count = metrics.event_count.saturating_add(summary.events);
            metrics.model_output_chars = metrics
                .model_output_chars
                .saturating_add(summary.model_output_chars);
            metrics.tool_calls = metrics.tool_calls.saturating_add(summary.tool_calls);
            metrics.tool_errors = metrics.tool_errors.saturating_add(summary.tool_errors);
            metrics.approval_requests = metrics
                .approval_requests
                .saturating_add(summary.approval_requests);
            metrics.auto_approvals = metrics
                .auto_approvals
                .saturating_add(summary.auto_approvals);
            metrics.risk_assessments = metrics
                .risk_assessments
                .saturating_add(summary.risk_assessments);
            metrics.research_events = metrics
                .research_events
                .saturating_add(summary.research_events);
            metrics.subagent_events = metrics
                .subagent_events
                .saturating_add(summary.subagent_events);
            metrics.context_compactions = metrics
                .context_compactions
                .saturating_add(summary.context_compactions);
            metrics.error_events = metrics.error_events.saturating_add(summary.error_events);
            metrics.final_outputs = metrics.final_outputs.saturating_add(summary.final_outputs);
            for (event_type, count) in summary.event_types {
                let total = metrics.event_types.entry(event_type).or_default();
                *total = total.saturating_add(count);
            }
        }
        if metrics.run_count > 0 {
            metrics.average_duration_seconds = duration_total / metrics.run_count as f64;
        }
        Ok(metrics)
    }

    fn grouped(&self) -> Result<BTreeMap<String, Vec<EventEnvelope>>, StoreError> {
        let (head, _) = self.journal.head()?;
        if head == 0 {
            return Ok(BTreeMap::new());
        }
        let from = head
            .saturating_sub(MAX_SCAN_EVENTS.saturating_sub(1))
            .max(1);
        let mut grouped = BTreeMap::<String, Vec<EventEnvelope>>::new();
        let mut cursor = from;
        loop {
            let events = self.journal.read_global(cursor, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if let Some(run_id) = event_run_id(event) {
                    grouped.entry(run_id).or_default().push(event.clone());
                }
            }
            cursor = events
                .last()
                .map_or(cursor, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 || cursor > head {
                break;
            }
        }
        Ok(grouped)
    }
}

fn event_run_id(event: &EventEnvelope) -> Option<String> {
    event.context.run_id.clone()
}

fn resolve_run_id<'a>(
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

fn metadata_record(run_id: &str, event: &EventEnvelope) -> TelemetryEventRecord {
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

fn summarize(
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
    };
    for event in events {
        *summary
            .event_types
            .entry(event.event_type.clone())
            .or_default() += 1;
        match event.event_type.as_str() {
            "model.delta.v1" => {
                summary.model_output_chars = summary
                    .model_output_chars
                    .saturating_add(payload_text_chars(journal, event, "text")?);
            }
            "final.output.v1" => {
                summary.final_outputs = summary.final_outputs.saturating_add(1);
                summary.model_output_chars = summary
                    .model_output_chars
                    .saturating_add(payload_text_chars(journal, event, "text")?);
            }
            "tool.call.requested.v1" => {
                summary.tool_calls = summary.tool_calls.saturating_add(1);
            }
            "tool.call.completed.v1" => {
                if payload_i64(journal, event, "exit_code")?.is_some_and(|code| code != 0) {
                    summary.tool_errors = summary.tool_errors.saturating_add(1);
                }
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
    Ok(summary)
}

fn is_error_event(event_type: &str) -> bool {
    event_type == "error.v1"
        || event_type.contains("failed")
        || event_type.contains("outcome_unknown")
        || event_type.contains("release_denied")
}

fn payload(journal: &dyn EventJournal, event: &EventEnvelope) -> Result<Value, StoreError> {
    journal.decrypt_payload(event)
}

fn payload_text_chars(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<usize, StoreError> {
    Ok(payload(journal, event)?
        .get(field)
        .and_then(Value::as_str)
        .map_or(0, |text| text.chars().count()))
}

fn payload_i64(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<i64>, StoreError> {
    Ok(payload(journal, event)?.get(field).and_then(Value::as_i64))
}

fn payload_bool(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<bool>, StoreError> {
    Ok(payload(journal, event)?.get(field).and_then(Value::as_bool))
}

fn payload_string(
    journal: &dyn EventJournal,
    event: &EventEnvelope,
    field: &str,
) -> Result<Option<String>, StoreError> {
    Ok(payload(journal, event)?
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn duration_seconds(started_at: &str, ended_at: &str) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::{TelemetryService, duration_seconds};
    use colossus_contracts::{Actor, ActorType, EventClassification, ExecutionContext, NewEvent};
    use colossus_ports::EventJournal;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn append(journal: &dyn EventJournal, version: u64, event_type: &str, payload: Value) {
        journal
            .append(NewEvent {
                event_version: 1,
                stream_id: "run:run-telemetry".into(),
                expected_stream_version: version,
                classification: EventClassification::Domain,
                event_type: event_type.into(),
                actor: Actor {
                    actor_type: ActorType::System,
                    id: "test".into(),
                },
                context: ExecutionContext {
                    correlation_id: "run-telemetry".into(),
                    session_id: Some("session-1".into()),
                    run_id: Some("run-telemetry".into()),
                    ..ExecutionContext::default()
                },
                payload,
            })
            .expect("append");
    }

    #[test]
    fn derives_counts_without_returning_payload_content() {
        let journal = Arc::new(InMemoryEventJournal::default());
        append(
            journal.as_ref(),
            0,
            "model.delta.v1",
            json!({"text": "secret prompt text"}),
        );
        append(
            journal.as_ref(),
            1,
            "tool.call.requested.v1",
            json!({"name": "echo"}),
        );
        append(
            journal.as_ref(),
            2,
            "tool.call.completed.v1",
            json!({"exit_code": 7, "output": "secret tool output"}),
        );
        append(
            journal.as_ref(),
            3,
            "context.prepared.v1",
            json!({"compacted": true}),
        );
        append(
            journal.as_ref(),
            4,
            "research.run_updated.v1",
            json!({"record": "hidden"}),
        );
        append(
            journal.as_ref(),
            5,
            "final.output.v1",
            json!({"text": "done"}),
        );
        let service = TelemetryService::new(journal);
        let summary = service
            .list_runs(Some("session-1"), 20)
            .expect("runs")
            .remove(0);
        assert_eq!(summary.events, 6);
        assert_eq!(summary.model_output_chars, 22);
        assert_eq!(summary.tool_calls, 1);
        assert_eq!(summary.tool_errors, 1);
        assert_eq!(summary.context_compactions, 1);
        assert_eq!(summary.research_events, 1);
        assert_eq!(summary.final_outputs, 1);
        let detail = service.get_run("run-tele", 3).expect("detail");
        assert!(detail.truncated);
        assert_eq!(detail.records.len(), 3);
        let rendered = serde_json::to_string(&detail).expect("JSON");
        assert!(!rendered.contains("secret prompt text"));
        assert!(!rendered.contains("secret tool output"));
        let metrics = service.metrics(None, 100).expect("metrics");
        assert_eq!(metrics.run_count, 1);
        assert_eq!(metrics.event_count, 6);
    }

    #[test]
    fn duration_uses_timestamps_and_never_goes_negative() {
        assert_eq!(
            duration_seconds("2026-07-11T00:00:00Z", "2026-07-11T00:00:01.500Z"),
            1.5
        );
        assert_eq!(
            duration_seconds("2026-07-11T00:00:02Z", "2026-07-11T00:00:01Z"),
            0.0
        );
    }
}
