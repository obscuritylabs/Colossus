//! Request-scoped, payload-free observations. No discovery or policy bypass lives here.

use colossus_contracts::{McpDiagnosticCode, McpDiagnosticFailure, McpDiagnosticStage};
use std::sync::{Arc, Mutex};

/// Bounded observations scoped to one authorized health check, including its HTTP tasks.
#[derive(Clone, Debug, Default)]
pub struct McpDiagnosticCapture(Arc<Mutex<Observation>>);

#[derive(Clone, Debug, Default)]
struct Observation {
    stage: McpDiagnosticStage,
    failure: Option<McpDiagnosticFailure>,
}

tokio::task_local! {
    static ACTIVE_CAPTURE: McpDiagnosticCapture;
}

impl McpDiagnosticCapture {
    /// Observe only this discovery future. HTTP clients carry clones into rmcp tasks.
    pub async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        ACTIVE_CAPTURE.scope(self.clone(), future).await
    }

    pub(super) fn current() -> Self {
        ACTIVE_CAPTURE.try_with(Clone::clone).unwrap_or_default()
    }

    pub(super) fn stage(&self, stage: McpDiagnosticStage) {
        let mut observation = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        observation.stage = stage;
        observation.failure = None;
    }

    pub(super) fn fail(&self, code: McpDiagnosticCode, http_status: Option<u16>) {
        let mut observation = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Preserve the first concrete cause when rmcp later reports a closed channel.
        observation
            .failure
            .get_or_insert(McpDiagnosticFailure { code, http_status });
    }

    /// Read only locally generated categories and numeric status; never source errors.
    pub fn snapshot(&self) -> (McpDiagnosticStage, Option<McpDiagnosticFailure>) {
        let observation = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (observation.stage, observation.failure.clone())
    }
}

pub(super) fn process_failure(error: &colossus_policy::ExecutionError) -> McpDiagnosticCode {
    // This exact marker is generated locally by ProcessExecutor, never subprocess output.
    match error {
        colossus_policy::ExecutionError::Failed(message)
            if message == "sandboxed process exceeded its timeout" =>
        {
            McpDiagnosticCode::Timeout
        }
        _ => McpDiagnosticCode::Process,
    }
}

pub(super) fn request_failure(error: &reqwest::Error) -> McpDiagnosticCode {
    use std::error::Error as _;
    if error.is_timeout() {
        return McpDiagnosticCode::Timeout;
    }
    let mut source = error.source();
    // Bound traversal and inspect types only. Error text can contain URLs and secrets.
    for _ in 0..16 {
        let Some(cause) = source else { break };
        if cause.downcast_ref::<rustls::Error>().is_some() {
            return McpDiagnosticCode::Tls;
        }
        // io::Error::source skips the wrapped error itself. Follow get_ref()
        // one layer at a time, including nested io::Errors around rustls.
        source = cause
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| cause.source());
    }
    if error.is_connect() {
        McpDiagnosticCode::Connect
    } else {
        McpDiagnosticCode::Transport
    }
}
