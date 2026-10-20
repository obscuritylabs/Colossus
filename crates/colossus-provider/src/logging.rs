//! Provider diagnostics contain operational metadata, never HTTP or model bodies.

use crate::{EffectRequest, ExecutionError, ProviderKind, QuarantinedEffectResult};
use std::time::Instant;

pub(super) struct ProviderOperation<'a> {
    provider: &'static str,
    profile: &'a str,
    run_id: &'a str,
    operation: &'static str,
    started: Instant,
    finished: bool,
}

impl<'a> ProviderOperation<'a> {
    pub(super) fn begin(
        provider: ProviderKind,
        profile: &'a str,
        request: &'a EffectRequest,
        streaming: bool,
    ) -> Self {
        let operation = if request.action == "provider.models" {
            "catalog"
        } else {
            "generation"
        };
        let result = Self {
            provider: provider.as_str(),
            profile: safe_identifier(profile),
            run_id: safe_identifier(request.context.run_id.as_deref().unwrap_or_default()),
            operation,
            started: Instant::now(),
            finished: false,
        };
        tracing::info!(
            target: "colossus.provider",
            event_name = "provider.started",
            provider = result.provider,
            provider_profile = result.profile,
            run_id = result.run_id,
            operation,
            streaming,
        );
        result
    }

    pub(super) fn finish(mut self, result: &Result<QuarantinedEffectResult, ExecutionError>) {
        let elapsed_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result {
            Ok(_) => tracing::info!(
                target: "colossus.provider",
                event_name = "provider.completed",
                provider = self.provider,
                provider_profile = self.profile,
                run_id = self.run_id,
                operation = self.operation,
                elapsed_ms,
            ),
            Err(error) => {
                let (category, status, unknown) = failure_metadata(error);
                tracing::warn!(
                    target: "colossus.provider",
                    event_name = "provider.failed",
                    provider = self.provider,
                    provider_profile = self.profile,
                    run_id = self.run_id,
                    operation = self.operation,
                    error_type = category,
                    http_status = status,
                    outcome_unknown = unknown,
                    elapsed_ms,
                );
            }
        }
        self.finished = true;
    }
}

impl Drop for ProviderOperation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            tracing::warn!(
                target: "colossus.provider",
                event_name = "provider.interrupted",
                provider = self.provider,
                provider_profile = self.profile,
                run_id = self.run_id,
                operation = self.operation,
                outcome_unknown = true,
            );
        }
    }
}

pub(super) fn safe_identifier(value: &str) -> &str {
    if value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':'))
    {
        value
    } else {
        "redacted"
    }
}

fn failure_metadata(error: &ExecutionError) -> (&'static str, Option<u16>, bool) {
    match error {
        ExecutionError::ProviderRejected(failure) => {
            (failure.reason.code(), failure.http_status, false)
        }
        ExecutionError::HttpStatus { status, .. } => ("http_status", Some(*status), false),
        ExecutionError::Recoverable { http_status, .. } => ("recoverable", *http_status, false),
        ExecutionError::OutcomeUnknown(_) => ("outcome_unknown", None, true),
        ExecutionError::ReleaseDenied(_) => ("release_denied", None, false),
        ExecutionError::Failed(_) => ("adapter_failed", None, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_never_copy_provider_controlled_text_or_urls_to_log_metadata() {
        let private = "https://user:password@private.test/path?key=secret";
        for error in [
            ExecutionError::Failed(private.into()),
            ExecutionError::OutcomeUnknown(private.into()),
            ExecutionError::ReleaseDenied(private.into()),
            ExecutionError::Recoverable {
                code: private.into(),
                message: private.into(),
                http_status: Some(429),
                retry_after_ms: None,
            },
        ] {
            let metadata = failure_metadata(&error);
            assert!(!metadata.0.contains("private"));
            assert!(!metadata.0.contains("secret"));
        }
        assert_eq!(safe_identifier(private), "redacted");
        assert_eq!(safe_identifier("anthropic-main"), "anthropic-main");
    }
}
