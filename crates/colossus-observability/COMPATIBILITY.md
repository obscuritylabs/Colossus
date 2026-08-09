# Observability compatibility pins

The lockfile and compliance tests for this crate were reviewed against:

| Component | Pinned revision/version |
| --- | --- |
| OpenTelemetry GenAI semantic conventions | `46d43c8949afb53765a202e89f4534eeb75ca3fa` |
| `opentelemetry` | `0.32.0` |
| `opentelemetry_sdk` | `0.32.1` |
| `opentelemetry-otlp` | `0.32.0` |
| `opentelemetry-appender-tracing` | `0.32.0` |
| `tracing-opentelemetry` | `0.33.0` |
| `tracing` | `0.1.44` |
| `tracing-subscriber` | `0.3.23` |
| `tracing-appender` | `0.2.5` |

Update this manifest, `Cargo.lock`, `src/conventions.rs`, the metric views, compliance
tests, and `docs/develop/observability.md` together.
