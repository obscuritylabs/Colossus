//! Native Windows AppContainer/Job/WFP supervision of the private CEF host.
mod installation;
mod launch;
mod lifecycle;
mod owner;

use async_trait::async_trait;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_browser_presentation::PresentationClient;
use colossus_contracts::{BrowserCapabilities, BrowserSessionId};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

use crate::BrowserEgressLimits;

/// Native publisher-verification output. Never accept these paths/digests from a
/// renderer, tool call or model. The manifest digest must come from independently
/// verified publisher release metadata, including its native capability matrix.
pub struct WindowsBrowserConfig {
    /// Installed fixed host component, with LPAC read-only engine ACLs.
    pub component_root: PathBuf,
    /// Independently publisher-verified exact inventory digest.
    pub manifest_sha256: [u8; 32],
    /// Existing owner-private state root, unrelated to engine installation.
    pub state_root: PathBuf,
    /// Publisher/platform accepted ceiling; private OS certificate stores unsupported.
    pub capabilities: BrowserCapabilities,
    /// Aggregate Job memory ceiling (256 MiB..=4 GiB).
    pub memory_bytes: u64,
    /// Job process ceiling (32..=512).
    pub max_processes: u32,
    /// Immutable destination/connection lifetime ceilings.
    pub egress: BrowserEgressLimits,
}

/// Exact supervised attachment; runtime keeps the independent automation owner.
pub struct WindowsBrowserPresentation {
    /// Separately authenticated native-only presentation channel.
    pub client: PresentationClient,
    /// Exact enrollment digest required by the native pixel consumer.
    pub enrollment_digest: [u8; 32],
}

/// Native-owned teardown category; EOF and process IDs cannot establish this receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsBrowserShutdownReceipt {
    /// Authenticated CEF shutdown acknowledged and the full Job exited without termination.
    GracefulCefShutdown,
    /// Full Job exit proved after native termination or without an authenticated CEF receipt.
    ForcedJobExit,
}

/// One independently owned native process tree per immutable runtime session.
pub struct WindowsBrowserSupervisor {
    installation: Arc<installation::Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<lifecycle::Resources>>>>>,
    stopped: Arc<AtomicBool>,
}
impl WindowsBrowserSupervisor {
    /// Clone the separately authenticated presenter for this exact enrolled owner.
    /// This native-only seam does not allocate a host or grant browser control.
    /// Cleanup fences the retained session and disconnects every channel clone.
    pub async fn presentation(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        let resource = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let resource = resource.lock().await;
        if self.stopped.load(Ordering::Acquire)
            || resource.fenced.load(Ordering::Acquire)
            || resource.closed
        {
            return Err(BrowserDriverError::Stale);
        }
        if resource.request != *request {
            return Err(BrowserDriverError::Denied);
        }
        let presentation = resource
            .presentation
            .as_ref()
            .ok_or(BrowserDriverError::Unavailable)?;
        Ok((presentation.client.clone(), presentation.enrollment_digest))
    }
    /// Bind every installed byte and independently accepted native capability.
    pub async fn install(config: WindowsBrowserConfig) -> Result<Self, BrowserDriverError> {
        Self::install_inner(config, false).await
    }
    /// Explicit native development acceptance only; absent from release builds.
    /// A diagnostic instance must never enter ordinary application discovery.
    #[cfg(debug_assertions)]
    pub async fn install_for_acceptance(
        config: WindowsBrowserConfig,
    ) -> Result<Self, BrowserDriverError> {
        Self::install_inner(config, true).await
    }
    async fn install_inner(
        config: WindowsBrowserConfig,
        diagnostic: bool,
    ) -> Result<Self, BrowserDriverError> {
        let installation =
            tokio::task::spawn_blocking(move || installation::bind(config, diagnostic))
                .await
                .map_err(|_| BrowserDriverError::Unavailable)??;
        Ok(Self {
            installation: Arc::new(installation),
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }
    /// Move the single native-admitted attachment for this exact retained session.
    /// This performs no launch or control takeover; a second attachment is denied.
    pub async fn take_presentation(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<WindowsBrowserPresentation, BrowserDriverError> {
        let resource = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Denied)?;
        let mut resource = resource.lock().await;
        if self.stopped.load(Ordering::Acquire)
            || resource.fenced.load(Ordering::Acquire)
            || resource.closed
            || resource.request != *request
        {
            return Err(BrowserDriverError::Denied);
        }
        resource
            .presentation
            .take()
            .ok_or(BrowserDriverError::Denied)
    }
    /// Retain unknown native/process/profile cleanup for explicit trusted retry.
    pub async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        self.stopped.store(true, Ordering::Release);
        let resources: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let mut result = Ok(());
        for resource in resources {
            if resource.lock().await.cleanup().await.is_err() {
                result = Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        result
    }

    /// Categorical retained teardown evidence for explicit native acceptance.
    pub async fn shutdown_receipts(
        &self,
    ) -> Vec<(BrowserSessionId, Option<WindowsBrowserShutdownReceipt>)> {
        let resources: Vec<_> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, resource)| (id.clone(), Arc::clone(resource)))
            .collect();
        let mut receipts = Vec::with_capacity(resources.len());
        for (id, resource) in resources {
            receipts.push((id, resource.lock().await.receipt));
        }
        receipts
    }
}
#[async_trait]
impl BrowserHostFactory for WindowsBrowserSupervisor {
    fn capabilities(&self) -> BrowserCapabilities {
        if self.stopped.load(Ordering::Acquire) {
            BrowserCapabilities::unavailable()
        } else {
            self.installation.capabilities.clone()
        }
    }
    async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        let installation = Arc::clone(&self.installation);
        let sessions = Arc::clone(&self.sessions);
        let stopped = Arc::clone(&self.stopped);
        let request = request.clone();
        let control = control.clone();
        // Independent task owns partial allocation even when caller cancellation
        // drops the waiting future. The map retains every uncertain obligation.
        tokio::spawn(async move {
            launch::owned(installation, sessions, stopped, request, control).await
        })
        .await
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
    }
    async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        let resource = self.sessions.lock().await.get(&request.session_id).cloned();
        if let Some(resource) = resource {
            let mut resource = resource.lock().await;
            if resource.request != *request {
                return Err(BrowserDriverError::Denied);
            }
            resource.cleanup().await?;
        }
        Ok(())
    }
}
