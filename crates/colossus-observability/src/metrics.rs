use crate::{attributes, instruments, operations};
use opentelemetry::{KeyValue, global};
use std::time::Instant;
use tracing::Span;

/// RAII span for the distinguishable planning phase of a Plan Mode invocation.
pub struct PlanObservation {
    span: Span,
    error_type: Option<&'static str>,
}

impl PlanObservation {
    /// Start a `plan {role}` internal span beneath the active agent invocation.
    pub fn start(role: &str) -> Self {
        Self {
            span: tracing::info_span!(
                target: "colossus.gen_ai",
                "plan",
                otel.name = %format_args!("plan {role}"),
                otel.kind = "internal",
                otel.status_code = tracing::field::Empty,
                error.type = tracing::field::Empty,
                gen_ai.operation.name = "plan",
                gen_ai.agent.name = role,
                gen_ai.conversation.id = tracing::field::Empty,
                colossus.run.id = tracing::field::Empty,
                colossus.application.id = tracing::field::Empty,
                enduser.id = tracing::field::Empty,
            ),
            error_type: Some("_OTHER"),
        }
    }

    /// Span used as the explicit parent for model and tool work in Plan Mode.
    #[must_use]
    pub const fn span(&self) -> &Span {
        &self.span
    }

    /// Attach durable run and conversation correlation after allocation.
    pub fn record_correlation(&self, run_id: &str, conversation_id: &str) {
        self.span.record("colossus.run.id", run_id);
        self.span.record("gen_ai.conversation.id", conversation_id);
    }

    /// Attach authenticated application and optional asserted end-user identity.
    pub fn record_identity(&self, application_id: Option<&str>, end_user_id: Option<&str>) {
        if let Some(application_id) = application_id {
            self.span.record("colossus.application.id", application_id);
        }
        if let Some(end_user_id) = end_user_id {
            self.span.record("enduser.id", end_user_id);
        }
    }

    /// Mark planning successful before the invocation returns.
    pub fn success(&mut self) {
        self.error_type = None;
    }
}

impl Drop for PlanObservation {
    fn drop(&mut self) {
        self.span.record(
            "otel.status_code",
            if self.error_type.is_some() {
                "ERROR"
            } else {
                "OK"
            },
        );
        if let Some(error_type) = self.error_type {
            self.span.record("error.type", error_type);
        }
    }
}

/// RAII measurement for an agent invocation, including early-return failures.
pub struct AgentObservation {
    agent_name: String,
    started: Instant,
    inference_calls: u64,
    tool_calls: u64,
    error_type: Option<String>,
    span: Span,
}

impl AgentObservation {
    /// Start measuring the current instrumented agent span.
    pub fn start(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            started: Instant::now(),
            inference_calls: 0,
            tool_calls: 0,
            error_type: Some("_OTHER".into()),
            span: Span::current(),
        }
    }

    /// Count one provider inference operation.
    pub const fn inference_call(&mut self) {
        self.inference_calls = self.inference_calls.saturating_add(1);
    }

    /// Count one requested tool operation.
    pub const fn tool_call(&mut self) {
        self.tool_calls = self.tool_calls.saturating_add(1);
    }

    /// Mark the invocation successful.
    pub fn success(&mut self) {
        self.error_type = None;
    }

    /// Attach a low-cardinality failure class before returning.
    pub fn failure(&mut self, error_type: impl Into<String>) {
        self.error_type = Some(error_type.into());
    }
}

impl Drop for AgentObservation {
    fn drop(&mut self) {
        let status = if self.error_type.is_some() {
            "ERROR"
        } else {
            "OK"
        };
        self.span.record("otel.status_code", status);
        if let Some(error_type) = self.error_type.as_deref() {
            self.span.record("error.type", error_type);
        }
        record_agent(
            &self.agent_name,
            self.started.elapsed().as_secs_f64(),
            self.inference_calls,
            self.tool_calls,
            self.error_type.as_deref(),
        );
    }
}

/// Provider token counts available after one model operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModelTokenUsage {
    /// Billable or reported input tokens.
    pub input: Option<u64>,
    /// Billable or reported output tokens.
    pub output: Option<u64>,
}

/// Low-cardinality data recorded for one model operation.
pub struct ModelMetric<'a> {
    /// Provider semantic-convention name.
    pub provider: &'a str,
    /// Requested model.
    pub request_model: &'a str,
    /// Provider-reported response model, when available.
    pub response_model: Option<&'a str>,
    /// Low-cardinality terminal error class.
    pub error_type: Option<&'a str>,
    /// Complete operation duration in seconds.
    pub duration_seconds: f64,
    /// Time to the first streamed chunk in seconds.
    pub first_chunk_seconds: Option<f64>,
    /// Time between successive output chunks.
    pub output_chunk_intervals: &'a [f64],
    /// Reported token usage.
    pub tokens: ModelTokenUsage,
}

/// Record standard GenAI client metrics for one completed provider operation.
pub fn record_model(metric: &ModelMetric<'_>) {
    let meter = global::meter("colossus.gen_ai");
    let mut attributes = vec![
        KeyValue::new(attributes::OPERATION_NAME, operations::CHAT),
        KeyValue::new(attributes::PROVIDER_NAME, metric.provider.to_owned()),
        KeyValue::new(attributes::REQUEST_MODEL, metric.request_model.to_owned()),
    ];
    if let Some(response_model) = metric.response_model {
        attributes.push(KeyValue::new(
            attributes::RESPONSE_MODEL,
            response_model.to_owned(),
        ));
    }
    if let Some(error_type) = metric.error_type {
        attributes.push(KeyValue::new(attributes::ERROR_TYPE, error_type.to_owned()));
    }
    meter
        .f64_histogram(instruments::CLIENT_OPERATION_DURATION)
        .with_unit("s")
        .build()
        .record(metric.duration_seconds, &attributes);
    if let Some(first_chunk_seconds) = metric.first_chunk_seconds {
        meter
            .f64_histogram(instruments::CLIENT_TIME_TO_FIRST_CHUNK)
            .with_unit("s")
            .build()
            .record(first_chunk_seconds, &attributes);
    }
    let chunk_histogram = meter
        .f64_histogram(instruments::CLIENT_TIME_PER_OUTPUT_CHUNK)
        .with_unit("s")
        .build();
    for interval in metric.output_chunk_intervals {
        chunk_histogram.record(*interval, &attributes);
    }
    let token_histogram = meter
        .u64_histogram(instruments::CLIENT_TOKEN_USAGE)
        .with_unit("{token}")
        .build();
    if let Some(input) = metric.tokens.input {
        let mut token_attributes = attributes.clone();
        token_attributes.push(KeyValue::new("gen_ai.token.type", "input"));
        token_histogram.record(input, &token_attributes);
    }
    if let Some(output) = metric.tokens.output {
        let mut token_attributes = attributes;
        token_attributes.push(KeyValue::new("gen_ai.token.type", "output"));
        token_histogram.record(output, &token_attributes);
    }
}

/// Record standard GenAI metrics for one in-process agent invocation.
pub fn record_agent(
    agent_name: &str,
    duration_seconds: f64,
    inference_calls: u64,
    tool_calls: u64,
    error_type: Option<&str>,
) {
    let meter = global::meter("colossus.gen_ai");
    let mut attributes = vec![KeyValue::new(attributes::AGENT_NAME, agent_name.to_owned())];
    if let Some(error_type) = error_type {
        attributes.push(KeyValue::new(attributes::ERROR_TYPE, error_type.to_owned()));
    }
    meter
        .f64_histogram(instruments::INVOKE_AGENT_DURATION)
        .with_unit("s")
        .build()
        .record(duration_seconds, &attributes);
    meter
        .u64_histogram(instruments::INVOKE_AGENT_INFERENCE_CALLS)
        .with_unit("{inference_call}")
        .build()
        .record(inference_calls, &attributes);
    meter
        .u64_histogram(instruments::INVOKE_AGENT_TOOL_CALLS)
        .with_unit("{tool_call}")
        .build()
        .record(tool_calls, &attributes);
}

/// Record standard GenAI tool duration without correlation identifiers.
pub fn record_tool(tool_name: &str, duration_seconds: f64, error_type: Option<&str>) {
    let meter = global::meter("colossus.gen_ai");
    let mut attributes = vec![KeyValue::new(attributes::TOOL_NAME, tool_name.to_owned())];
    if let Some(error_type) = error_type {
        attributes.push(KeyValue::new(attributes::ERROR_TYPE, error_type.to_owned()));
    }
    meter
        .f64_histogram(instruments::EXECUTE_TOOL_DURATION)
        .with_unit("s")
        .build()
        .record(duration_seconds, &attributes);
}

/// Record standard GenAI workflow duration without the workflow run identifier.
pub fn record_workflow(workflow_name: &str, duration_seconds: f64, error_type: Option<&str>) {
    let meter = global::meter("colossus.gen_ai");
    let mut attributes = vec![KeyValue::new(
        attributes::WORKFLOW_NAME,
        workflow_name.to_owned(),
    )];
    if let Some(error_type) = error_type {
        attributes.push(KeyValue::new(attributes::ERROR_TYPE, error_type.to_owned()));
    }
    meter
        .f64_histogram(instruments::INVOKE_WORKFLOW_DURATION)
        .with_unit("s")
        .build()
        .record(duration_seconds, &attributes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    #[test]
    fn in_memory_export_contains_standard_instruments_without_correlation_dimensions() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        global::set_meter_provider(provider.clone());

        record_agent("primary", 1.0, 2, 3, None);
        record_tool("memory.search", 0.2, Some("denied"));
        record_workflow("release", 4.0, None);
        record_model(&ModelMetric {
            provider: "openai",
            request_model: "gpt-test",
            response_model: None,
            error_type: None,
            duration_seconds: 0.5,
            first_chunk_seconds: Some(0.1),
            output_chunk_intervals: &[0.02],
            tokens: ModelTokenUsage {
                input: Some(10),
                output: Some(5),
            },
        });
        provider.force_flush().expect("flush in-memory metrics");

        let exported = exporter.get_finished_metrics().expect("exported metrics");
        let metrics = exported
            .iter()
            .flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics())
            .collect::<Vec<_>>();
        let names = metrics
            .iter()
            .map(|metric| metric.name())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            names,
            [
                instruments::CLIENT_TOKEN_USAGE,
                instruments::CLIENT_OPERATION_DURATION,
                instruments::CLIENT_TIME_TO_FIRST_CHUNK,
                instruments::CLIENT_TIME_PER_OUTPUT_CHUNK,
                instruments::INVOKE_AGENT_DURATION,
                instruments::INVOKE_AGENT_INFERENCE_CALLS,
                instruments::INVOKE_AGENT_TOOL_CALLS,
                instruments::EXECUTE_TOOL_DURATION,
                instruments::INVOKE_WORKFLOW_DURATION,
            ]
            .into_iter()
            .collect()
        );

        let forbidden = [
            "gen_ai.conversation.id",
            "enduser.id",
            "gen_ai.response.id",
            "gen_ai.tool.call.id",
            "colossus.run.id",
            "colossus.workflow.run.id",
            "colossus.message.sequence",
        ];
        for metric in metrics {
            let assert_attributes = |attributes: Vec<&KeyValue>| {
                assert!(
                    attributes
                        .iter()
                        .all(|attribute| { !forbidden.contains(&attribute.key.as_str()) })
                );
            };
            match metric.data() {
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => {
                    for point in histogram.data_points() {
                        assert_attributes(point.attributes().collect());
                    }
                }
                AggregatedMetrics::U64(MetricData::Histogram(histogram)) => {
                    for point in histogram.data_points() {
                        assert_attributes(point.attributes().collect());
                    }
                }
                other => panic!("GenAI metric must use a histogram: {other:?}"),
            }
        }
    }
}
