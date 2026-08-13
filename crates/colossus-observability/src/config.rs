use serde::{Deserialize, Serialize};
#[cfg(any(feature = "host-exporters", test))]
use std::time::Duration;
use std::{collections::BTreeMap, net::IpAddr};
use thiserror::Error;
use url::Url;

#[cfg(any(feature = "host-exporters", test))]
const DEFAULT_OTLP_GRPC_ENDPOINT: &str = "http://127.0.0.1:4317";
#[cfg(any(feature = "host-exporters", test))]
const DEFAULT_OTLP_HTTP_ENDPOINT: &str = "http://127.0.0.1:4318";
const DEFAULT_EXPORT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_METRIC_INTERVAL_MS: u64 = 60_000;
const MAX_RESOURCE_ATTRIBUTES: usize = 32;
const MAX_RESOURCE_ATTRIBUTE_BYTES: usize = 256;

/// Strict opt-in live observability configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Master switch. Environment settings cannot enable a disabled configuration.
    #[serde(default)]
    pub enabled: bool,
    /// OpenTelemetry service name.
    #[serde(default = "default_service_name")]
    pub service_name: String,
    /// Process-wide resource attributes, never per-run identifiers.
    #[serde(default)]
    pub resource_attributes: BTreeMap<String, String>,
    /// Trace signal settings.
    #[serde(default)]
    pub traces: TraceSignalConfig,
    /// Metric signal settings.
    #[serde(default)]
    pub metrics: MetricSignalConfig,
    /// Log signal settings.
    #[serde(default)]
    pub logs: LogSignalConfig,
    /// Shared OTLP transport settings.
    #[serde(default)]
    pub otlp: OtlpConfig,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: default_service_name(),
            resource_attributes: BTreeMap::new(),
            traces: TraceSignalConfig::default(),
            metrics: MetricSignalConfig::default(),
            logs: LogSignalConfig::default(),
            otlp: OtlpConfig::default(),
        }
    }
}

/// Trace export and sampling settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceSignalConfig {
    /// Export spans over OTLP.
    #[serde(default)]
    pub enabled: bool,
    /// Parent-based trace-id sampling ratio.
    #[serde(default = "default_sample_ratio")]
    pub sample_ratio: f64,
}

impl Default for TraceSignalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_ratio: default_sample_ratio(),
        }
    }
}

/// Metric export settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricSignalConfig {
    /// Export metrics over OTLP.
    #[serde(default)]
    pub enabled: bool,
    /// Periodic export interval.
    #[serde(default = "default_metric_interval_ms")]
    pub export_interval_ms: u64,
}

impl Default for MetricSignalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            export_interval_ms: default_metric_interval_ms(),
        }
    }
}

/// Structured log export and sensitive journal payload settings.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogSignalConfig {
    /// Export structured tracing events as OpenTelemetry logs.
    #[serde(default)]
    pub otlp: bool,
    /// Write newline-delimited structured tracing events to stdout.
    #[serde(default)]
    pub stdout_json: bool,
    /// Journal event detail released to live log sinks.
    #[serde(default)]
    pub journal_payloads: JournalPayloadMode,
    /// Required acknowledgement for plaintext durable payload disclosure.
    #[serde(default)]
    pub acknowledge_sensitive_content: bool,
}

/// Journal content released into live logs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalPayloadMode {
    /// Do not emit journal event log records.
    #[default]
    Disabled,
    /// Emit envelope and correlation metadata only.
    Metadata,
    /// Emit metadata plus the complete plaintext durable payload.
    Full,
}

/// Shared OpenTelemetry Protocol exporter settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct OtlpConfig {
    /// Optional collector endpoint. Standard signal-specific environment variables win.
    pub endpoint: Option<String>,
    /// OTLP transport encoding.
    #[serde(default)]
    pub protocol: OtlpProtocol,
    /// Per-export timeout.
    #[serde(default = "default_export_timeout_ms")]
    pub timeout_ms: u64,
    /// Explicitly allow plaintext export to a non-loopback collector.
    #[serde(default)]
    pub acknowledge_insecure_transport: bool,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            protocol: OtlpProtocol::Grpc,
            timeout_ms: default_export_timeout_ms(),
            acknowledge_insecure_transport: false,
        }
    }
}

/// Supported OTLP encodings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtlpProtocol {
    /// OTLP over gRPC.
    #[default]
    Grpc,
    /// OTLP protobuf over HTTP.
    HttpProtobuf,
}

/// Configuration or host-initialization failure.
#[derive(Debug, Error)]
pub enum ObservabilityError {
    /// Strict configuration validation failed.
    #[error("invalid observability configuration: {0}")]
    Configuration(String),
    /// An exporter or subscriber could not be constructed.
    #[error("observability initialization failed: {0}")]
    Initialization(String),
}

impl ObservabilityConfig {
    /// Validate bounds and disclosure acknowledgements without contacting a collector.
    pub fn validate(&self) -> Result<(), ObservabilityError> {
        if self.service_name.is_empty() || self.service_name.len() > 128 {
            return Err(configuration("serviceName must contain 1..=128 bytes"));
        }
        if !self.traces.sample_ratio.is_finite() || !(0.0..=1.0).contains(&self.traces.sample_ratio)
        {
            return Err(configuration("traces.sampleRatio must be within 0.0..=1.0"));
        }
        if !(1_000..=300_000).contains(&self.metrics.export_interval_ms) {
            return Err(configuration(
                "metrics.exportIntervalMs must be within 1000..=300000",
            ));
        }
        if !(100..=120_000).contains(&self.otlp.timeout_ms) {
            return Err(configuration("otlp.timeoutMs must be within 100..=120000"));
        }
        if self.resource_attributes.len() > MAX_RESOURCE_ATTRIBUTES
            || self.resource_attributes.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_RESOURCE_ATTRIBUTE_BYTES
                    || value.len() > MAX_RESOURCE_ATTRIBUTE_BYTES
            })
        {
            return Err(configuration(
                "resourceAttributes must contain at most 32 nonempty keys and 256-byte values",
            ));
        }
        if self.logs.journal_payloads == JournalPayloadMode::Full
            && !self.logs.acknowledge_sensitive_content
        {
            return Err(configuration(
                "logs.journalPayloads: full requires logs.acknowledgeSensitiveContent: true",
            ));
        }
        if self.logs.acknowledge_sensitive_content
            && self.logs.journal_payloads != JournalPayloadMode::Full
        {
            return Err(configuration(
                "logs.acknowledgeSensitiveContent is valid only with journalPayloads: full",
            ));
        }
        if self.logs.journal_payloads != JournalPayloadMode::Disabled
            && !self.logs.otlp
            && !self.logs.stdout_json
        {
            return Err(configuration(
                "journal payload logging requires logs.otlp or logs.stdoutJson",
            ));
        }
        if self.enabled
            && !self.traces.enabled
            && !self.metrics.enabled
            && !self.logs.otlp
            && !self.logs.stdout_json
        {
            return Err(configuration(
                "enabled observability requires at least one signal sink",
            ));
        }
        if let Some(endpoint) = self.otlp.endpoint.as_deref() {
            validate_endpoint(endpoint, self.otlp.acknowledge_insecure_transport)?;
        }
        Ok(())
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn sdk_disabled(&self) -> bool {
        !self.enabled
            || std::env::var("OTEL_SDK_DISABLED")
                .is_ok_and(|value| value.eq_ignore_ascii_case("true"))
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn endpoint_for(&self, signal: Signal) -> Result<String, ObservabilityError> {
        self.endpoint_for_with(signal, |name| std::env::var(name).ok())
    }

    #[cfg(any(feature = "host-exporters", test))]
    fn endpoint_for_with(
        &self,
        signal: Signal,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<String, ObservabilityError> {
        let protocol = self.protocol_for_with(signal, &environment);
        let signal_endpoint = environment(signal.endpoint_environment());
        let base_endpoint = environment("OTEL_EXPORTER_OTLP_ENDPOINT")
            .or_else(|| self.otlp.endpoint.clone())
            .unwrap_or_else(|| match protocol {
                OtlpProtocol::Grpc => DEFAULT_OTLP_GRPC_ENDPOINT.into(),
                OtlpProtocol::HttpProtobuf => DEFAULT_OTLP_HTTP_ENDPOINT.into(),
            });
        let endpoint = match signal_endpoint {
            Some(endpoint) => endpoint,
            None if protocol == OtlpProtocol::HttpProtobuf => {
                append_http_signal_path(&base_endpoint, signal.http_path())?
            }
            None => base_endpoint,
        };
        validate_endpoint(&endpoint, self.otlp.acknowledge_insecure_transport)?;
        Ok(endpoint)
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn protocol_for(&self, signal: Signal) -> OtlpProtocol {
        self.protocol_for_with(signal, |name| std::env::var(name).ok())
    }

    #[cfg(any(feature = "host-exporters", test))]
    fn protocol_for_with(
        &self,
        signal: Signal,
        environment: impl Fn(&str) -> Option<String>,
    ) -> OtlpProtocol {
        environment(signal.protocol_environment())
            .or_else(|| environment("OTEL_EXPORTER_OTLP_PROTOCOL"))
            .as_deref()
            .and_then(parse_protocol)
            .unwrap_or(self.otlp.protocol)
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn timeout_for(&self, signal: Signal) -> Duration {
        let timeout_ms = std::env::var(signal.timeout_environment())
            .ok()
            .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_TIMEOUT").ok())
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (100..=120_000).contains(value))
            .unwrap_or(self.otlp.timeout_ms);
        Duration::from_millis(timeout_ms)
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn metric_interval(&self) -> Duration {
        let interval_ms = std::env::var("OTEL_METRIC_EXPORT_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1_000..=300_000).contains(value))
            .unwrap_or(self.metrics.export_interval_ms);
        Duration::from_millis(interval_ms)
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn service_name(&self) -> String {
        std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| self.service_name.clone())
    }

    #[cfg(any(feature = "host-exporters", test))]
    pub(crate) fn sample_ratio(&self) -> f64 {
        let sampler = std::env::var("OTEL_TRACES_SAMPLER").ok();
        if sampler
            .as_deref()
            .is_some_and(|value| matches!(value, "traceidratio" | "parentbased_traceidratio"))
        {
            return std::env::var("OTEL_TRACES_SAMPLER_ARG")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|ratio| ratio.is_finite() && (0.0..=1.0).contains(ratio))
                .unwrap_or(self.traces.sample_ratio);
        }
        self.traces.sample_ratio
    }
}

#[cfg(any(feature = "host-exporters", test))]
#[derive(Clone, Copy)]
pub(crate) enum Signal {
    Traces,
    Metrics,
    Logs,
}

#[cfg(any(feature = "host-exporters", test))]
impl Signal {
    const fn endpoint_environment(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            Self::Metrics => "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            Self::Logs => "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
        }
    }

    const fn protocol_environment(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
            Self::Metrics => "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
            Self::Logs => "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
        }
    }

    const fn timeout_environment(self) -> &'static str {
        match self {
            Self::Traces => "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
            Self::Metrics => "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT",
            Self::Logs => "OTEL_EXPORTER_OTLP_LOGS_TIMEOUT",
        }
    }

    const fn http_path(self) -> &'static str {
        match self {
            Self::Traces => "/v1/traces",
            Self::Metrics => "/v1/metrics",
            Self::Logs => "/v1/logs",
        }
    }
}

#[cfg(any(feature = "host-exporters", test))]
fn append_http_signal_path(
    endpoint: &str,
    signal_path: &str,
) -> Result<String, ObservabilityError> {
    let mut url = Url::parse(endpoint).map_err(|_| configuration("OTLP endpoint must be a URL"))?;
    let prefix = url.path().trim_end_matches('/');
    url.set_path(&format!("{prefix}{signal_path}"));
    Ok(url.to_string())
}

fn validate_endpoint(endpoint: &str, acknowledge_insecure: bool) -> Result<(), ObservabilityError> {
    let url = Url::parse(endpoint).map_err(|_| configuration("OTLP endpoint must be a URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(configuration(
            "OTLP endpoint must be an http or https origin",
        ));
    }
    if url.scheme() == "http" && !loopback(&url) && !acknowledge_insecure {
        return Err(configuration(
            "plaintext OTLP to a non-loopback endpoint requires acknowledgeInsecureTransport: true",
        ));
    }
    Ok(())
}

fn loopback(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    })
}

#[cfg(any(feature = "host-exporters", test))]
fn parse_protocol(value: &str) -> Option<OtlpProtocol> {
    match value {
        "grpc" => Some(OtlpProtocol::Grpc),
        "http/protobuf" | "http_protobuf" => Some(OtlpProtocol::HttpProtobuf),
        _ => None,
    }
}

fn configuration(message: impl Into<String>) -> ObservabilityError {
    ObservabilityError::Configuration(message.into())
}

fn default_service_name() -> String {
    "colossus".into()
}

const fn default_sample_ratio() -> f64 {
    1.0
}

const fn default_metric_interval_ms() -> u64 {
    DEFAULT_METRIC_INTERVAL_MS
}

const fn default_export_timeout_ms() -> u64 {
    DEFAULT_EXPORT_TIMEOUT_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_defaults_release_no_signals_or_payloads() {
        let config = ObservabilityConfig::default();
        config.validate().expect("disabled config");
        assert!(!config.enabled);
        assert!(!config.traces.enabled);
        assert!(!config.metrics.enabled);
        assert_eq!(config.logs.journal_payloads, JournalPayloadMode::Disabled);
    }

    #[test]
    fn full_payloads_require_explicit_acknowledgement() {
        let config = ObservabilityConfig {
            enabled: true,
            logs: LogSignalConfig {
                stdout_json: true,
                journal_payloads: JournalPayloadMode::Full,
                ..LogSignalConfig::default()
            },
            ..ObservabilityConfig::default()
        };
        assert!(config.validate().is_err());
        let acknowledged = ObservabilityConfig {
            logs: LogSignalConfig {
                acknowledge_sensitive_content: true,
                ..config.logs.clone()
            },
            ..config
        };
        acknowledged.validate().expect("acknowledged full payloads");
    }

    #[test]
    fn non_loopback_plaintext_requires_acknowledgement() {
        let config = ObservabilityConfig {
            otlp: OtlpConfig {
                endpoint: Some("http://collector.example:4317".into()),
                ..OtlpConfig::default()
            },
            ..ObservabilityConfig::default()
        };
        assert!(config.validate().is_err());
        let acknowledged = ObservabilityConfig {
            otlp: OtlpConfig {
                acknowledge_insecure_transport: true,
                ..config.otlp
            },
            ..config
        };
        acknowledged
            .validate()
            .expect("acknowledged insecure transport");
    }

    #[test]
    fn strict_yaml_rejects_unknown_fields() {
        assert!(
            serde_json::from_value::<ObservabilityConfig>(serde_json::json!({
                "enabled": false,
                "surprise": true
            }))
            .is_err()
        );
    }

    #[test]
    fn signal_environment_precedes_generic_environment_and_yaml() {
        let config = ObservabilityConfig {
            otlp: OtlpConfig {
                endpoint: Some("https://yaml.example:4317".into()),
                protocol: OtlpProtocol::Grpc,
                ..OtlpConfig::default()
            },
            ..ObservabilityConfig::default()
        };
        let environment = |name: &str| match name {
            "OTEL_EXPORTER_OTLP_ENDPOINT" => Some("https://generic.example:4317".into()),
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT" => {
                Some("https://traces.example:4318/v1/traces".into())
            }
            "OTEL_EXPORTER_OTLP_PROTOCOL" => Some("grpc".into()),
            "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL" => Some("http/protobuf".into()),
            _ => None,
        };
        assert_eq!(
            config
                .endpoint_for_with(Signal::Traces, environment)
                .expect("environment endpoint"),
            "https://traces.example:4318/v1/traces"
        );
        assert_eq!(
            config.protocol_for_with(Signal::Traces, environment),
            OtlpProtocol::HttpProtobuf
        );
        assert_eq!(
            config
                .endpoint_for_with(Signal::Metrics, environment)
                .expect("generic environment endpoint"),
            "https://generic.example:4317"
        );
    }

    #[test]
    fn http_generic_and_yaml_endpoints_receive_the_standard_signal_path() {
        let config = ObservabilityConfig {
            otlp: OtlpConfig {
                endpoint: Some("https://collector.example/otel".into()),
                protocol: OtlpProtocol::HttpProtobuf,
                ..OtlpConfig::default()
            },
            ..ObservabilityConfig::default()
        };
        assert_eq!(
            config
                .endpoint_for_with(Signal::Logs, |_| None)
                .expect("HTTP log endpoint"),
            "https://collector.example/otel/v1/logs"
        );
    }
}
