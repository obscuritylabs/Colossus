use colossus_api::{ApiError, ApiErrorReason, ApiResult, RunStatus};
use std::{sync::Arc, time::Duration};
use tracing::Span;

/// Correlation selected only by trusted hosted composition, never by a run request.
#[derive(Clone, Default)]
pub(super) struct RunTelemetry {
    project_id: Option<Arc<str>>,
}

#[derive(Clone, Copy)]
pub(super) enum ExecutionSource {
    Submitted,
    Recovered,
}
impl ExecutionSource {
    const fn label(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Recovered => "recovered",
        }
    }
}

impl RunTelemetry {
    pub(super) fn for_project(project_id: String) -> ApiResult<Self> {
        if project_id.is_empty()
            || project_id.len() > 256
            || matches!(project_id.as_str(), "." | "..")
            || !project_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
            })
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "telemetry.project_id",
                "hosted telemetry requires a bounded opaque project identifier",
            ));
        }
        Ok(Self {
            project_id: Some(project_id.into()),
        })
    }

    pub(super) fn execution_span(&self, run_id: &str, source: ExecutionSource) -> Span {
        let Some(project_id) = self.project_id.as_deref() else {
            return Span::none();
        };
        tracing::info_span!(
            target: "colossus.gen_ai",
            "colossus.hosted.run.execute",
            "colossus.project.id" = project_id,
            "colossus.run.id" = run_id,
            "colossus.execution.source" = source.label(),
        )
    }

    pub(super) fn recovery_span(&self, run_id: &str) -> Span {
        let Some(project_id) = self.project_id.as_deref() else {
            return Span::none();
        };
        tracing::info_span!(
            target: "colossus.gen_ai",
            "colossus.hosted.run.recover",
            "colossus.project.id" = project_id,
            "colossus.run.id" = run_id,
        )
    }

    pub(super) fn execution_started(&self, run_id: &str, source: ExecutionSource) {
        if let Some(project_id) = self.project_id.as_deref() {
            tracing::info!(
                target: "colossus.lifecycle",
                event_name = "colossus.hosted.run.execution_started",
                "colossus.project.id" = project_id,
                "colossus.run.id" = run_id,
                "colossus.execution.source" = source.label(),
            );
        }
    }

    pub(super) fn execution_finished(
        &self,
        run_id: &str,
        status: Option<RunStatus>,
        terminal_persisted: bool,
        duration: Duration,
    ) {
        if let Some(project_id) = self.project_id.as_deref() {
            tracing::info!(
                target: "colossus.lifecycle",
                event_name = "colossus.hosted.run.execution_finished",
                "colossus.project.id" = project_id,
                "colossus.run.id" = run_id,
                status = status.map_or("unknown", status_label),
                terminal_persisted,
                duration_ms = duration.as_secs_f64() * 1_000.0,
            );
        }
    }

    pub(super) fn recovery_started(&self, run_id: &str, status: RunStatus) {
        if let Some(project_id) = self.project_id.as_deref() {
            tracing::info!(
                target: "colossus.lifecycle",
                event_name = "colossus.hosted.run.recovery_started",
                "colossus.project.id" = project_id,
                "colossus.run.id" = run_id,
                status = status_label(status),
            );
        }
    }
}

const fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Waiting => "waiting",
        RunStatus::Cancelling => "cancelling",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
        RunStatus::OutcomeUnknown => "outcome_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::RunTelemetry;

    #[test]
    fn project_correlation_rejects_payloads_paths_and_unbounded_values() {
        for value in [
            "",
            ".",
            "..",
            "../project",
            "project\nother",
            "project name",
            "https://private.example",
        ] {
            assert!(RunTelemetry::for_project(value.into()).is_err());
        }
        assert!(RunTelemetry::for_project("a".repeat(257)).is_err());
        assert!(
            RunTelemetry::for_project("opaque-project_1.namespace:service@instance".into()).is_ok()
        );
        assert!(RunTelemetry::default().project_id.is_none());
    }
}
