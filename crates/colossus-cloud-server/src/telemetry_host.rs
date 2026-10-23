//! One instance collector, owned from startup until all hosted work has drained.
use colossus_cloud::{CloudError, CloudResult};
use colossus_observability::{ObservabilityConfig, ObservabilityDiagnostics, ObservabilityGuard};
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TelemetryStatus {
    pub state: &'static str,
    pub configured_revision: u64,
    pub applied_revision: u64,
}

pub(crate) struct CloudTelemetry {
    applied: ObservabilityConfig,
    applied_revision: u64,
    enabled: bool,
    guard: Mutex<Option<ObservabilityGuard>>,
    diagnostic_permits: Arc<tokio::sync::Semaphore>,
}

impl CloudTelemetry {
    pub(crate) fn start(config: ObservabilityConfig, revision: u64) -> Result<Self, &'static str> {
        crate::global_settings::telemetry::validate(&config)
            .map_err(|_| "instance telemetry configuration invalid")?;
        let guard = ObservabilityGuard::install(&config)
            .map_err(|_| "instance telemetry exporters unavailable")?;
        Ok(Self {
            applied: config,
            applied_revision: revision,
            enabled: guard.is_some(),
            guard: Mutex::new(guard),
            diagnostic_permits: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    /// Persisted changes take effect at the next process startup, never mid-run.
    pub(crate) fn configure(
        &self,
        desired: &ObservabilityConfig,
        revision: u64,
    ) -> TelemetryStatus {
        let same = desired == &self.applied;
        TelemetryStatus {
            state: if !same {
                "restart_required"
            } else if self.enabled {
                "active"
            } else {
                "disabled"
            },
            configured_revision: revision,
            // Unrelated catalog edits with identical telemetry are already satisfied.
            applied_revision: if same {
                revision
            } else {
                self.applied_revision
            },
        }
    }

    pub(crate) fn applied_configuration(&self) -> &ObservabilityConfig {
        &self.applied
    }

    /// A dropped HTTP request cannot start another probe while its flush is running.
    pub(crate) fn diagnostic(
        &self,
    ) -> CloudResult<(ObservabilityDiagnostics, tokio::sync::OwnedSemaphorePermit)> {
        if !self.applied.enabled
            || !(self.applied.traces.enabled
                || self.applied.metrics.enabled
                || self.applied.logs.otlp)
        {
            return Err(CloudError::Conflict);
        }
        let permit = self
            .diagnostic_permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| CloudError::ResourceExhausted)?;
        let guard = self.guard.lock().map_err(|_| CloudError::Storage)?;
        let diagnostics = guard.as_ref().ok_or(CloudError::Conflict)?.diagnostics();
        Ok((diagnostics, permit))
    }

    /// Called after HTTP/gRPC and hosted execution have finished their bounded drain.
    pub(crate) fn shutdown(&self) {
        if let Ok(mut guard) = self.guard.lock()
            && let Some(guard) = guard.take()
        {
            guard.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_collector_changes_require_restart_and_unrelated_edits_do_not() {
        let config = ObservabilityConfig::default();
        let host = CloudTelemetry::start(config.clone(), 4).unwrap();
        assert_eq!(host.configure(&config, 5).state, "disabled");
        assert_eq!(host.configure(&config, 5).applied_revision, 5);
        let mut changed = config.clone();
        changed.enabled = true;
        changed.traces.enabled = true;
        changed.otlp.endpoint = Some("https://collector.example/".into());
        let status = host.configure(&changed, 6);
        assert_eq!(status.state, "restart_required");
        assert_eq!(status.configured_revision, 6);
        assert_eq!(status.applied_revision, 4);
        assert!(!host.applied_configuration().enabled);
        host.shutdown();
    }
}
