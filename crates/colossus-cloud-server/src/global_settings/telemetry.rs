//! Instance collector policy cannot grant disclosure of personal conversation content.
use crate::{http::Result, server::State};
use axum::{Json, Router, extract::State as Extract, http::HeaderMap, routing::post};
use colossus_cloud::{CloudError, CloudResult};
use colossus_runtime::{JournalPayloadMode, ObservabilityConfig};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticInput {
    revision: u64,
}

#[derive(Serialize)]
struct DiagnosticResult {
    status: &'static str,
    message: &'static str,
    revision: u64,
}

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new().route(
        "/api/admin/hosted-settings/telemetry/test",
        post(diagnostic),
    )
}

async fn diagnostic(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(input): Json<DiagnosticInput>,
) -> Result<Json<DiagnosticResult>> {
    state.auth.admin(&headers, true).await?;
    let host = state.hosted.as_deref().ok_or(CloudError::NotFound)?;
    let current = super::catalog(host)?;
    if input.revision != current.revision
        || state
            .telemetry
            .configure(&current.observability, current.revision)
            .state
            != "active"
    {
        return Err(CloudError::Conflict.into());
    }
    let (diagnostics, permit) = state.telemetry.diagnostic()?;
    // SDK exporter timeouts bound the background flush. Keep the permit there even
    // if the HTTP deadline expires or the caller disconnects.
    let probe = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        diagnostics.force_flush()
    });
    let result = tokio::time::timeout(Duration::from_secs(20), probe).await;
    let (status, message) = match result {
        Ok(Ok(report)) if report.ready => (
            "passed",
            "The applied collector acknowledged the diagnostic flush. This does not guarantee future delivery.",
        ),
        _ => (
            "failed",
            "The applied collector did not acknowledge the diagnostic flush within the request deadline.",
        ),
    };
    Ok(Json(DiagnosticResult {
        status,
        message,
        revision: current.revision,
    }))
}

pub(crate) fn validate(config: &ObservabilityConfig) -> CloudResult<()> {
    config.validate().map_err(|_| CloudError::InvalidArgument)?;
    if config.logs.journal_payloads == JournalPayloadMode::Full
        || config.logs.acknowledge_sensitive_content
        || config.resource_attributes.iter().any(|(key, value)| {
            key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
                || matches!(
                    key.as_str(),
                    "colossus.project.id" | "colossus.application.id" | "enduser.id"
                )
        })
    {
        return Err(CloudError::InvalidArgument);
    }
    if let Some(endpoint) = config.otlp.endpoint.as_deref() {
        let url = url::Url::parse(endpoint).map_err(|_| CloudError::InvalidArgument)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
        {
            return Err(CloudError::InvalidArgument);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_collector_rejects_private_content_and_credentialed_destinations() {
        let mut config = ObservabilityConfig::default();
        assert!(validate(&config).is_ok());
        config.logs.journal_payloads = JournalPayloadMode::Full;
        config.logs.acknowledge_sensitive_content = true;
        config.logs.otlp = true;
        assert_eq!(validate(&config), Err(CloudError::InvalidArgument));
        config.logs.journal_payloads = JournalPayloadMode::Disabled;
        config.logs.acknowledge_sensitive_content = false;
        for endpoint in [
            "https://user:secret@collector.example/",
            "https://collector.example/?token=secret",
            "https://collector.example/#secret",
            "file:///tmp/collector",
        ] {
            config.otlp.endpoint = Some(endpoint.into());
            assert_eq!(validate(&config), Err(CloudError::InvalidArgument));
        }
        config.otlp.endpoint = Some("https://collector.example/".into());
        assert!(validate(&config).is_ok());
        config
            .resource_attributes
            .insert("colossus.project.id".into(), "spoofed".into());
        assert_eq!(validate(&config), Err(CloudError::InvalidArgument));
    }
}
