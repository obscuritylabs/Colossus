use super::*;

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
            provider_input_tokens: 0,
            provider_output_tokens: 0,
            provider_total_tokens: 0,
            provider_cached_input_tokens: 0,
            provider_reasoning_tokens: 0,
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
            metrics.provider_input_tokens = metrics
                .provider_input_tokens
                .saturating_add(summary.provider_input_tokens);
            metrics.provider_output_tokens = metrics
                .provider_output_tokens
                .saturating_add(summary.provider_output_tokens);
            metrics.provider_total_tokens = metrics
                .provider_total_tokens
                .saturating_add(summary.provider_total_tokens);
            metrics.provider_cached_input_tokens = metrics
                .provider_cached_input_tokens
                .saturating_add(summary.provider_cached_input_tokens);
            metrics.provider_reasoning_tokens = metrics
                .provider_reasoning_tokens
                .saturating_add(summary.provider_reasoning_tokens);
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
