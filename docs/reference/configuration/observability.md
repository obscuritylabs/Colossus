---
title: Live observability
description: Configure opt-in OpenTelemetry traces, metrics, and structured journal logs.
audience: operator
type: reference
---

# Live observability

`observability` is the opt-in live OpenTelemetry plane. It does not replace the
journal-derived `telemetry` commands, does not replay historical events, and does not
change canonical storage or run results when an exporter is unavailable.

The complete disabled default is:

```yaml
observability:
  enabled: false
  serviceName: colossus
  resourceAttributes: {}
  traces:
    enabled: false
    sampleRatio: 1.0
  metrics:
    enabled: false
    exportIntervalMs: 60000
  logs:
    otlp: false
    stdoutJson: false
    journalPayloads: disabled
    acknowledgeSensitiveContent: false
  otlp:
    endpoint: null
    protocol: grpc
    timeoutMs: 10000
    acknowledgeInsecureTransport: false
```

`enabled: true` requires at least one trace, metric, OTLP-log, or stdout-log sink.
Signals remain YAML-controlled; environment variables cannot enable them. When tracing
is enabled, sampling is parent-based at `1.0` unless explicitly changed.

## Fields

| Field | Default | Constraint |
| --- | --- | --- |
| `enabled` | `false` | Master switch; `OTEL_SDK_DISABLED=true` may still disable the SDK |
| `serviceName` | `colossus` | 1–128 bytes; `OTEL_SERVICE_NAME` overrides it |
| `resourceAttributes` | `{}` | At most 32 entries; nonempty keys and values at most 256 bytes |
| `traces.enabled` | `false` | Export GenAI, RPC, and runtime spans over OTLP |
| `traces.sampleRatio` | `1.0` | Finite value from `0.0` through `1.0` |
| `metrics.enabled` | `false` | Export standard GenAI histograms over OTLP |
| `metrics.exportIntervalMs` | `60000` | `1000..=300000` milliseconds |
| `logs.otlp` | `false` | Export structured tracing records over OTLP |
| `logs.stdoutJson` | `false` | Write newline-delimited JSON through a bounded nonblocking queue |
| `logs.journalPayloads` | `disabled` | `disabled`, `metadata`, or `full` |
| `logs.acknowledgeSensitiveContent` | `false` | Must be `true` exactly when journal payload mode is `full` |
| `otlp.endpoint` | Loopback OTLP default | Absolute `http` or `https` URL |
| `otlp.protocol` | `grpc` | `grpc` or `http_protobuf` |
| `otlp.timeoutMs` | `10000` | `100..=120000` milliseconds |
| `otlp.acknowledgeInsecureTransport` | `false` | Required for plaintext, non-loopback OTLP |

The gRPC defaults are `http://127.0.0.1:4317`; HTTP/protobuf defaults to
`http://127.0.0.1:4318`. HTTP/protobuf appends `/v1/traces`, `/v1/metrics`, or
`/v1/logs` to the shared YAML or generic environment endpoint; signal-specific endpoint
variables are treated as complete URLs. Plaintext loopback is allowed for local development.
Plaintext export anywhere else fails configuration validation unless
`acknowledgeInsecureTransport: true` is present.

## Standard environment overrides

Standard OpenTelemetry variables override matching exporter values after YAML has
selected the allowed signals:

- `OTEL_EXPORTER_OTLP_ENDPOINT` and signal-specific `..._TRACES_ENDPOINT`,
  `..._METRICS_ENDPOINT`, and `..._LOGS_ENDPOINT`.
- `OTEL_EXPORTER_OTLP_PROTOCOL` and the signal-specific protocol variables.
- `OTEL_EXPORTER_OTLP_TIMEOUT` and the signal-specific timeout variables.
- `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG`,
  `OTEL_METRIC_EXPORT_INTERVAL`, `OTEL_SERVICE_NAME`, and
  `OTEL_RESOURCE_ATTRIBUTES`.

Signal-specific values take precedence over generic values, which take precedence over
YAML. An environment endpoint is subject to the same URL and insecure-transport checks
as a YAML endpoint. Environment variables cannot enable stdout, allow insecure
transport, or acknowledge sensitive content.

## Journal log disclosure

`metadata` emits one structured record only after each durable single or batch append
succeeds. It contains envelope and event identity, classification, actor type, and
durable correlation metadata, but omits plaintext payloads.

`full` additionally releases the complete plaintext durable event payload to every
enabled log sink. It requires both:

```yaml
logs:
  journalPayloads: full
  acknowledgeSensitiveContent: true
```

This can expose prompts, released model output, tool arguments and results, artifacts,
PII including `enduser.id`, and released reasoning summaries. Hidden reasoning and
credentials are not released because they are not durable released payloads. Treat the
collector and stdout destination as part of the sensitive-data boundary.

Exporter failure, a full bounded queue, or a blocked stdout reader may drop live
records but cannot fail a journal append or agent run. Large content is never attached
to spans or metrics.

## Host ownership

Only the long-running `worker` installs the process-global subscriber. An embedded
`Runtime` never installs global telemetry and may be used under a subscriber selected
by its host. The worker drains public work before bounded provider flush and process
exit.

CLI and TUI clients automatically use the worker when its authenticated endpoint exists
for the same canonical workspace. Runs submitted by those clients are therefore traced
and exported by the worker. Start the worker before the TUI when local OTLP export is
required:

```bash
colossus --config .colossus/config.yaml worker
# In another terminal:
colossus --config .colossus/config.yaml tui
```

Without an active worker, the standalone CLI and TUI use an embedded runtime. The stock
CLI host does not install an OTLP subscriber for that fallback, so it does not export
live signals even when `observability.enabled` is `true`. An embedding application may
install its own compatible `tracing` subscriber. Worker startup spans measure database
and recovery work when the worker opens; attaching a TUI to an already-running worker
does not open another runtime or emit another startup trace.

See [OpenTelemetry implementation](../../develop/observability.md) for the signal and
propagation contract and the [LGTM example](https://github.com/obscuritylabs/Colossus/tree/main/examples/observability)
for a development-only collector.
