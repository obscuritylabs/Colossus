use crate::{
    BrowserError,
    state::{State, lock, owned},
    validation,
};
use colossus_contracts::*;
use colossus_ports::{BrowserDriver, BrowserDriverControl, BrowserDriverError};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod capture;
mod control;
mod execute;
mod execute_accept;
mod lifecycle;
mod native_handoff;
mod transfer;

pub use capture::CapturedBrowserScreenshot;
pub use native_handoff::NativeBrowserHandoff;
pub use transfer::CapturedBrowserDownload;

/// Bounded caller-owned session manager, independent of MCP transport lifetime.
///
/// Inputs containing bindings/actors must come from authenticated composition, never
/// model or renderer ownership claims. Callers enter only after normal effect permits
/// have been verified/consumed. Results remain staged until runtime release policy.
pub struct BrowserCoordinator {
    driver: Arc<dyn BrowserDriver>,
    capabilities: BrowserCapabilities,
    state: Arc<Mutex<State>>,
}

impl BrowserCoordinator {
    /// Install a supervised engine and narrow its bounds to coordinator hard ceilings.
    pub fn new(driver: Arc<dyn BrowserDriver>, limits: BrowserLimits) -> Self {
        let mut capabilities = driver.capabilities();
        capabilities.limits = validation::limits(limits, &capabilities.limits);
        if capabilities
            .engine_version
            .as_ref()
            .is_some_and(|v| v.len() > 128)
            || capabilities.actions.len() > 32
            || capabilities.modes.len() > 2
        {
            capabilities = BrowserCapabilities::unavailable();
        }
        Self {
            driver,
            capabilities,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    /// Installed capability evidence and effective bounds, never an authority grant.
    pub fn capabilities(&self) -> BrowserCapabilities {
        self.capabilities.clone()
    }

    /// Bounded discovery filtered by the complete authenticated binding.
    pub fn list(
        &self,
        binding: &BrowserSessionBinding,
    ) -> Result<Vec<BrowserSessionSummary>, BrowserError> {
        validation::binding(binding)?;
        Ok(lock(&self.state)?
            .sessions
            .values()
            .filter(|s| &s.binding == binding)
            .map(|s| s.summary.clone())
            .collect())
    }

    /// Resolve only an exact caller-owned opaque session; foreign handles appear absent.
    pub fn get(
        &self,
        binding: &BrowserSessionBinding,
        id: &BrowserSessionId,
    ) -> Result<BrowserSessionSummary, BrowserError> {
        validation::binding(binding)?;
        let mut state = lock(&self.state)?;
        Ok(owned(&mut state, binding, id)?.summary.clone())
    }

    async fn await_driver<T>(
        &self,
        future: impl Future<Output = Result<T, BrowserDriverError>>,
        control: &BrowserDriverControl,
        timeout_ms: u32,
    ) -> Result<T, BrowserError> {
        tokio::select! {
            result = tokio::time::timeout(Duration::from_millis(u64::from(timeout_ms)), future) => {
                match result {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(BrowserDriverError::OutcomeUnknown)) | Err(_) => Err(BrowserError::OutcomeUnknown),
                    Ok(Err(error)) => Err(BrowserError::Driver(error)),
                }
            }
            () = wait_cancelled(control) => Err(BrowserError::OutcomeUnknown),
        }
    }
}

async fn wait_cancelled(control: &BrowserDriverControl) {
    while !control.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn token(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::now_v7().simple())
}

fn session_id() -> Result<BrowserSessionId, BrowserError> {
    BrowserSessionId::parse(token("bs_")).map_err(|_| BrowserError::InvalidArguments)
}

fn tab_id() -> Result<BrowserTabId, BrowserError> {
    BrowserTabId::parse(token("bt_")).map_err(|_| BrowserError::InvalidArguments)
}

fn document_id() -> Result<BrowserDocumentId, BrowserError> {
    BrowserDocumentId::parse(token("bd_")).map_err(|_| BrowserError::InvalidArguments)
}

fn snapshot_id() -> Result<BrowserSnapshotId, BrowserError> {
    BrowserSnapshotId::parse(token("bn_")).map_err(|_| BrowserError::InvalidArguments)
}

fn lease_id() -> Result<BrowserControlLeaseId, BrowserError> {
    BrowserControlLeaseId::parse(token("bl_")).map_err(|_| BrowserError::InvalidArguments)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}
