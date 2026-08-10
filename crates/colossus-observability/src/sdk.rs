use crate::{
    ObservabilityConfig, ObservabilityError, OtlpProtocol, Signal, install_trace_context_propagator,
};
use opentelemetry::{Key, KeyValue, global, trace::TracerProvider as _};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{
    Resource,
    logs::SdkLoggerProvider,
    metrics::{Aggregation, Instrument, PeriodicReader, SdkMeterProvider, Stream},
    trace::{Sampler, SdkTracerProvider},
};
use std::time::Duration;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    Layer as _, filter::filter_fn, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

/// Host-owned OpenTelemetry SDK providers and non-blocking stdout guard.
///
/// The worker must retain this value until all application work has drained and then
/// call [`Self::shutdown`] to flush bounded exporter queues.
pub struct ObservabilityGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
    stdout_guard: Option<WorkerGuard>,
    shutdown_timeout: Duration,
}

impl ObservabilityGuard {
    /// Install one process-global subscriber and the selected SDK providers.
    ///
    /// A disabled configuration returns `Ok(None)` and does not claim global state.
    pub fn install(config: &ObservabilityConfig) -> Result<Option<Self>, ObservabilityError> {
        config.validate()?;
        if config.sdk_disabled() {
            return Ok(None);
        }

        let resource = resource(config);
        let tracer_provider = config
            .traces
            .enabled
            .then(|| build_tracer_provider(config, resource.clone()))
            .transpose()?;
        let meter_provider = config
            .metrics
            .enabled
            .then(|| build_meter_provider(config, resource.clone()))
            .transpose()?;
        let logger_provider = config
            .logs
            .otlp
            .then(|| build_logger_provider(config, resource))
            .transpose()?;

        if let Some(provider) = meter_provider.as_ref() {
            global::set_meter_provider(provider.clone());
        }
        if let Some(provider) = tracer_provider.as_ref() {
            global::set_tracer_provider(provider.clone());
        }
        install_trace_context_propagator();

        // Keep exporter transport diagnostics out of the signals they are exporting.
        let trace_layer = tracer_provider.as_ref().map(|provider| {
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("colossus.gen_ai"))
                .with_filter(filter_fn(|metadata| {
                    is_colossus_trace_target(metadata.target())
                }))
        });
        let log_layer = logger_provider
            .as_ref()
            .map(OpenTelemetryTracingBridge::new)
            .map(|layer| layer.with_filter(filter_fn(is_colossus_log_metadata)));
        let (stdout_writer, stdout_guard) = if config.logs.stdout_json {
            let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());
            (Some(writer), Some(guard))
        } else {
            (None, None)
        };
        let stdout_layer = stdout_writer.map(|writer| {
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false)
                .with_writer(writer)
                .with_filter(filter_fn(is_colossus_log_metadata))
        });

        tracing_subscriber::registry()
            .with(trace_layer)
            .with(log_layer)
            .with(stdout_layer)
            .try_init()
            .map_err(|error| ObservabilityError::Initialization(error.to_string()))?;

        Ok(Some(Self {
            tracer_provider,
            meter_provider,
            logger_provider,
            stdout_guard,
            shutdown_timeout: Duration::from_millis(config.otlp.timeout_ms),
        }))
    }

    /// Flush and shut down all providers after application work has drained.
    pub fn shutdown(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(provider) = self.logger_provider.take() {
            let _ = provider.shutdown_with_timeout(self.shutdown_timeout);
        }
        if let Some(provider) = self.meter_provider.take() {
            let _ = provider.shutdown_with_timeout(self.shutdown_timeout);
        }
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown_with_timeout(self.shutdown_timeout);
        }
        drop(self.stdout_guard.take());
    }
}

fn is_colossus_trace_target(target: &str) -> bool {
    matches!(target, "colossus.gen_ai" | "colossus.rpc")
}

fn is_colossus_journal_target(target: &str) -> bool {
    target == "colossus.journal"
}

fn is_colossus_log_metadata(metadata: &tracing::Metadata<'_>) -> bool {
    // Per-layer filters are contextual, so parent spans must be admitted even though
    // the log sinks only emit the journal events nested inside them.
    if metadata.is_span() {
        is_colossus_trace_target(metadata.target())
    } else {
        is_colossus_journal_target(metadata.target())
    }
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        self.finish();
    }
}

fn build_tracer_provider(
    config: &ObservabilityConfig,
    resource: Resource,
) -> Result<SdkTracerProvider, ObservabilityError> {
    let endpoint = config.endpoint_for(Signal::Traces)?;
    let exporter = match config.protocol_for(Signal::Traces) {
        OtlpProtocol::Grpc => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Traces))
            .build(),
        OtlpProtocol::HttpProtobuf => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Traces))
            .build(),
    }
    .map_err(initialization)?;
    Ok(SdkTracerProvider::builder()
        .with_resource(resource)
        .with_sampler(configured_sampler(config))
        .with_batch_exporter(exporter)
        .build())
}

fn build_meter_provider(
    config: &ObservabilityConfig,
    resource: Resource,
) -> Result<SdkMeterProvider, ObservabilityError> {
    let endpoint = config.endpoint_for(Signal::Metrics)?;
    let exporter = match config.protocol_for(Signal::Metrics) {
        OtlpProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Metrics))
            .build(),
        OtlpProtocol::HttpProtobuf => opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Metrics))
            .build(),
    }
    .map_err(initialization)?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(config.metric_interval())
        .build();
    Ok(SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(reader)
        .with_view(genai_metric_view)
        .build())
}

pub(crate) fn genai_metric_view(instrument: &Instrument) -> Option<Stream> {
    const CLIENT_DURATION_BUCKETS: &[f64] = &[
        0.01, 0.02, 0.04, 0.08, 0.16, 0.32, 0.64, 1.28, 2.56, 5.12, 10.24, 20.48, 40.96, 81.92,
    ];
    const WORKFLOW_DURATION_BUCKETS: &[f64] = &[
        1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1_800.0, 3_600.0, 7_200.0,
    ];
    const AGENT_DURATION_BUCKETS: &[f64] = &[
        0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8, 25.6, 51.2, 102.4, 204.8, 409.6,
    ];
    const CALL_COUNT_BUCKETS: &[f64] = &[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
    const TOKEN_BUCKETS: &[f64] = &[
        1.0,
        4.0,
        16.0,
        64.0,
        256.0,
        1_024.0,
        4_096.0,
        16_384.0,
        65_536.0,
        262_144.0,
        1_048_576.0,
        4_194_304.0,
        16_777_216.0,
        67_108_864.0,
    ];
    let (boundaries, allowed_attributes) = match instrument.name() {
        crate::instruments::CLIENT_TOKEN_USAGE => (
            TOKEN_BUCKETS,
            &[
                crate::attributes::OPERATION_NAME,
                crate::attributes::PROVIDER_NAME,
                "gen_ai.token.type",
                crate::attributes::REQUEST_MODEL,
                crate::attributes::RESPONSE_MODEL,
            ][..],
        ),
        crate::instruments::CLIENT_OPERATION_DURATION
        | crate::instruments::CLIENT_TIME_TO_FIRST_CHUNK
        | crate::instruments::CLIENT_TIME_PER_OUTPUT_CHUNK => (
            CLIENT_DURATION_BUCKETS,
            &[
                crate::attributes::OPERATION_NAME,
                crate::attributes::PROVIDER_NAME,
                crate::attributes::REQUEST_MODEL,
                crate::attributes::RESPONSE_MODEL,
                crate::attributes::ERROR_TYPE,
            ][..],
        ),
        crate::instruments::INVOKE_AGENT_DURATION => (
            AGENT_DURATION_BUCKETS,
            &[crate::attributes::AGENT_NAME, crate::attributes::ERROR_TYPE][..],
        ),
        crate::instruments::INVOKE_AGENT_INFERENCE_CALLS
        | crate::instruments::INVOKE_AGENT_TOOL_CALLS => {
            (CALL_COUNT_BUCKETS, &[crate::attributes::AGENT_NAME][..])
        }
        crate::instruments::EXECUTE_TOOL_DURATION => (
            CLIENT_DURATION_BUCKETS,
            &[crate::attributes::TOOL_NAME, crate::attributes::ERROR_TYPE][..],
        ),
        crate::instruments::INVOKE_WORKFLOW_DURATION => (
            WORKFLOW_DURATION_BUCKETS,
            &[
                crate::attributes::WORKFLOW_NAME,
                crate::attributes::ERROR_TYPE,
            ][..],
        ),
        _ => return None,
    };
    Stream::builder()
        .with_aggregation(Aggregation::ExplicitBucketHistogram {
            boundaries: boundaries.to_vec(),
            record_min_max: true,
        })
        .with_allowed_attribute_keys(
            allowed_attributes
                .iter()
                .map(|key| Key::from_static_str(key)),
        )
        .with_cardinality_limit(2_000)
        .build()
        .ok()
}

fn build_logger_provider(
    config: &ObservabilityConfig,
    resource: Resource,
) -> Result<SdkLoggerProvider, ObservabilityError> {
    let endpoint = config.endpoint_for(Signal::Logs)?;
    let exporter = match config.protocol_for(Signal::Logs) {
        OtlpProtocol::Grpc => opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Logs))
            .build(),
        OtlpProtocol::HttpProtobuf => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .with_timeout(config.timeout_for(Signal::Logs))
            .build(),
    }
    .map_err(initialization)?;
    Ok(SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build())
}

fn resource(config: &ObservabilityConfig) -> Resource {
    let mut attributes = config
        .resource_attributes
        .iter()
        .map(|(key, value)| KeyValue::new(key.clone(), value.clone()))
        .collect::<Vec<_>>();
    if let Ok(environment) = std::env::var("OTEL_RESOURCE_ATTRIBUTES") {
        for entry in environment.split(',') {
            if let Some((key, value)) = entry.split_once('=')
                && !key.is_empty()
            {
                attributes.retain(|attribute| attribute.key.as_str() != key);
                attributes.push(KeyValue::new(key.to_owned(), value.to_owned()));
            }
        }
    }
    Resource::builder()
        .with_service_name(config.service_name())
        .with_attributes(attributes)
        .build()
}

fn initialization(error: impl std::fmt::Display) -> ObservabilityError {
    ObservabilityError::Initialization(error.to_string())
}

fn configured_sampler(config: &ObservabilityConfig) -> Sampler {
    let ratio = config.sample_ratio();
    match std::env::var("OTEL_TRACES_SAMPLER").as_deref() {
        Ok("always_on") => Sampler::AlwaysOn,
        Ok("always_off") => Sampler::AlwaysOff,
        Ok("traceidratio") => Sampler::TraceIdRatioBased(ratio),
        Ok("parentbased_always_on") => Sampler::ParentBased(Box::new(Sampler::AlwaysOn)),
        Ok("parentbased_always_off") => Sampler::ParentBased(Box::new(Sampler::AlwaysOff)),
        Ok("parentbased_traceidratio") | Err(_) | Ok(_) => {
            Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(ratio)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_colossus_journal_target, is_colossus_log_metadata, is_colossus_trace_target};
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
    use opentelemetry_sdk::{
        logs::{InMemoryLogExporter, SdkLoggerProvider},
        trace::{InMemorySpanExporter, SdkTracerProvider},
    };
    use tracing_subscriber::{Layer as _, layer::SubscriberExt as _};

    #[test]
    fn exporter_layers_only_accept_owned_signal_targets() {
        assert!(is_colossus_trace_target("colossus.gen_ai"));
        assert!(is_colossus_trace_target("colossus.rpc"));
        assert!(!is_colossus_trace_target("colossus.journal"));
        assert!(!is_colossus_trace_target("h2::proto::streams"));
        assert!(!is_colossus_trace_target("opentelemetry_otlp"));

        assert!(is_colossus_journal_target("colossus.journal"));
        assert!(!is_colossus_journal_target("colossus.gen_ai"));
        assert!(!is_colossus_journal_target("tonic::transport"));
    }

    #[test]
    fn journal_log_filter_preserves_events_inside_agent_spans() {
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer_exporter = InMemorySpanExporter::default();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(tracer_exporter.clone())
            .build();
        let trace_layer = tracing_opentelemetry::layer()
            .with_tracer(tracer_provider.tracer("colossus.gen_ai"))
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                is_colossus_trace_target(metadata.target())
            }));
        let log_layer = OpenTelemetryTracingBridge::new(&provider).with_filter(
            tracing_subscriber::filter::filter_fn(is_colossus_log_metadata),
        );
        let subscriber = tracing_subscriber::registry()
            .with(trace_layer)
            .with(log_layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(target: "colossus.gen_ai", "invoke_agent");
            let _entered = span.enter();
            tracing::info!(
                target: "colossus.journal",
                event_name = "colossus.journal.appended",
                message = "metadata"
            );
            tracing::info!(target: "h2::proto::streams", message = "must be dropped");
        });

        provider.force_flush().expect("flush logs");
        tracer_provider.force_flush().expect("flush spans");
        let records = exporter.get_emitted_logs().expect("exported logs");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].record.target().map(ToString::to_string),
            Some("colossus.journal".into())
        );
        assert_eq!(
            tracer_exporter
                .get_finished_spans()
                .expect("exported spans")
                .len(),
            1
        );
    }
}
