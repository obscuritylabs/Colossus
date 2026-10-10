//! Dedicated Linux browser process trees in a networkless OCI namespace.
//!
//! Only trusted native composition constructs this factory. No model or renderer
//! can nominate an engine, image, component path, profile, socket, or credentials.
//! Publication acceptance remains independent: developer probes cannot enable a
//! shipped capability. CONNECT retains end-to-end TLS; native origin checks must
//! also reject encrypted authority changes and HTTP/2 origin coalescing.

mod engine;
mod installation;
mod launch;
mod lifecycle;
mod pki;
mod pki_enrollment;
mod process_identity;
mod relay;
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_contracts::{BrowserCapabilities, BrowserSessionId};
use colossus_home::ConfinedRoot;
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use tokio::sync::Mutex;

use crate::BrowserEgressLimits;
pub use lifecycle::OciBrowserShutdownReceipt;
use lifecycle::Resources;
pub use pki::{OciBrowserIdentity, OciBrowserPki, OciClientIdentityBinding};
pub use pki_enrollment::{
    OciBrowserPkiAuthorization, OciBrowserPkiEnrollmentProvider, OciBrowserPkiRegistration,
    OciBrowserPkiRegistry, OciBrowserPkiScopeAuthorization,
};

/// Whole-process-tree resource ceilings, independently bounded by runtime policy.
#[derive(Clone, Copy, Debug)]
pub struct OciBrowserLimits {
    /// Container memory ceiling (256 MiB..=4 GiB).
    pub memory_bytes: u64,
    /// CPU quota in thousandths of one CPU (100..=4000).
    pub cpu_millis: u32,
    /// Container process/thread ceiling (32..=512).
    pub max_processes: u16,
    /// Ephemeral profile/NSS tmpfs size (16 MiB..=512 MiB).
    /// Diagnostic persistent cache bind mounts require a separate disk quota.
    pub profile_bytes: u64,
    /// Private scratch tmpfs size (16 MiB..=512 MiB).
    pub temporary_bytes: u64,
    /// Monotonic network lease and individual connection ceilings.
    pub egress: BrowserEgressLimits,
}

impl Default for OciBrowserLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 1024 * 1024 * 1024,
            cpu_millis: 2000,
            max_processes: 256,
            profile_bytes: 128 * 1024 * 1024,
            temporary_bytes: 128 * 1024 * 1024,
            egress: BrowserEgressLimits {
                max_connections: 64,
                lifetime: Duration::from_secs(30 * 60),
                connection_timeout: Duration::from_secs(60),
                max_connection_bytes: 32 * 1024 * 1024,
            },
        }
    }
}

/// Private, publisher-verified installation inputs supplied by native composition.
///
/// `manifest_sha256` must come from an independently verified publisher release
/// manifest, never from the adjacent component inventory's own claims. Installation
/// copies and hashes every inventory file into owner-private state before mounting
/// it read-only. The native Docker daemon must share the supervisor's PID namespace
/// sufficiently to prove exact `SO_PEERCRED` identity; otherwise launch fails closed.
pub struct OciBrowserConfig {
    /// Administrator-selected absolute Docker executable; Podman is not accepted.
    pub docker: PathBuf,
    /// Installed immutable Docker image ID, `sha256:` followed by 64 lowercase hex digits.
    pub image: String,
    /// Canonical component inventory root, never a personal browser installation.
    pub component_root: PathBuf,
    /// Independently trusted SHA-256 of the bounded component inventory bytes.
    pub manifest_sha256: [u8; 32],
    /// Existing canonical owner-private runtime state directory.
    pub state_root: PathBuf,
    /// Whole-container and network ceilings.
    pub limits: OciBrowserLimits,
    /// Accepted publisher/native capability ceiling, independently constrained by evidence.
    pub capabilities: BrowserCapabilities,
    /// Explicit native-owned PKI material; never model configuration or ambient stores.
    pub pki: Option<OciBrowserPki>,
    /// Native import consent resolved against each complete Core-generated enrollment.
    /// Mutually exclusive with pre-bound `pki`; never a model-selected identity.
    pub pki_enrollment: Option<Arc<dyn OciBrowserPkiEnrollmentProvider>>,
    /// Explicit native workspace/application profile store for developer acceptance.
    /// Production rejects stores until storage confidentiality and disk quota are
    /// accepted. Absent stores reject persistent requests before allocating a host.
    pub profile_store: Option<Arc<crate::BrowserProfileStore>>,
    /// Optional private read-only OSR channel for trusted native diagnostics.
    /// This does not admit Desktop mode or human input on Linux.
    pub presentation: bool,
}

struct Installation {
    root: ConfinedRoot,
    component: PathBuf,
    seccomp: PathBuf,
    docker_config: PathBuf,
    docker: PathBuf,
    docker_identity: engine::ExecutableIdentity,
    diagnostic_owner: bool,
    image: String,
    digest: [u8; 32],
    uid: u32,
    gid: u32,
    limits: OciBrowserLimits,
    capabilities: BrowserCapabilities,
    pki: Option<OciBrowserPki>,
    pki_enrollment: Option<Arc<dyn OciBrowserPkiEnrollmentProvider>>,
    profile_store: Option<Arc<crate::BrowserProfileStore>>,
    presentation: bool,
    artifacts: Arc<installation::StageGuard>,
}

/// Trusted factory retaining every dedicated host and unresolved cleanup obligation.
///
/// Cancellation terminally fences this host. Resuming work requires a fresh session
/// and newly reviewed enrollment; an old process's authority is never renewed.
pub struct OciBrowserSupervisor {
    installation: Arc<Installation>,
    sessions: Arc<Mutex<BTreeMap<BrowserSessionId, Arc<Mutex<Resources>>>>>,
    stopped: Arc<AtomicBool>,
}

impl OciBrowserSupervisor {
    /// Retain profile storage denial authority from this same verified installation.
    /// Runtime composition adds these roots independently of model policy/grants.
    #[must_use]
    pub fn protected_profile_roots(&self) -> Vec<ConfinedRoot> {
        self.installation
            .profile_store
            .as_ref()
            .map_or_else(Vec::new, |store| vec![store.protected_root()])
    }
    /// Clone the sole private read-only presenter for this exact enrolled request.
    /// Native composition alone uses this seam; no renderer/model channel, key,
    /// handle or nominated session is exposed. Cleanup revokes every clone.
    pub async fn presentation(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(colossus_browser_presentation::PresentationClient, [u8; 32]), BrowserDriverError>
    {
        if self.stopped.load(Ordering::Acquire) || !self.installation.presentation {
            return Err(BrowserDriverError::Unavailable);
        }
        let resources = self
            .sessions
            .lock()
            .await
            .get(&request.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let resources = resources.lock().await;
        if &resources.request != request {
            return Err(BrowserDriverError::Denied);
        }
        if resources.closed || resources.fenced.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Stale);
        }
        if resources.startup_phase != "bridge_ready" {
            return Err(BrowserDriverError::Unavailable);
        }
        resources
            .presentation
            .clone()
            .ok_or(BrowserDriverError::Unavailable)
    }
    /// Verify and privately stage a publisher-accepted production installation.
    pub async fn install(config: OciBrowserConfig) -> Result<Self, BrowserDriverError> {
        Self::install_inner(config, false).await
    }

    /// Stage an explicit developer acceptance installation with unaccepted modes.
    /// This diagnostic constructor is absent from release builds; its capabilities
    /// must never be installed in ordinary application discovery or a release manifest.
    #[cfg(debug_assertions)]
    pub async fn install_for_acceptance(
        config: OciBrowserConfig,
    ) -> Result<Self, BrowserDriverError> {
        Self::install_inner(config, true).await
    }

    async fn install_inner(
        config: OciBrowserConfig,
        diagnostic: bool,
    ) -> Result<Self, BrowserDriverError> {
        let installation =
            tokio::task::spawn_blocking(move || installation::stage(config, diagnostic))
                .await
                .map_err(|_| BrowserDriverError::Unavailable)??;
        if let Err(error) = engine::verify_image(&installation).await {
            installation::diagnostic_phase(diagnostic, "image_verification_failed");
            installation::discard(&installation)?;
            return Err(error);
        }
        installation::diagnostic_phase(diagnostic, "image_verified");
        Ok(Self {
            installation: Arc::new(installation),
            sessions: Arc::new(Mutex::new(BTreeMap::new())),
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Stop every retained process tree and drain all private egress obligations.
    /// Unknown cleanup is retained for explicit reconciliation; dropping the factory
    /// is never an acknowledged shutdown and cannot prove daemon-side process exit.
    pub async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        self.stopped.store(true, Ordering::Release);
        let resources: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let mut result = Ok(());
        for resources in resources {
            if resources.lock().await.cleanup().await.is_err() {
                result = Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        if result.is_ok() {
            installation::discard(&self.installation)?;
        }
        result
    }

    /// Native diagnostic phases for explicit debug acceptance. No IDs, paths,
    /// URLs, native protocol frames, certificates or secret bytes are exposed.
    #[cfg(debug_assertions)]
    pub async fn acceptance_launch_stages(
        &self,
    ) -> Vec<(&'static str, bool, OciBrowserShutdownReceipt)> {
        let resources: Vec<_> = self.sessions.lock().await.values().cloned().collect();
        let mut stages = Vec::with_capacity(resources.len());
        for resources in resources {
            let resources = resources.lock().await;
            stages.push((
                resources.startup_phase,
                resources.closed,
                resources.shutdown_receipt,
            ));
        }
        stages
    }
}

#[async_trait]
impl BrowserHostFactory for OciBrowserSupervisor {
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
        let request = request.clone();
        let control = control.clone();
        let stopped = Arc::clone(&self.stopped);
        // An independently owned task completes/reconciles launch even if a caller
        // drops its future after Docker has allocated resources.
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
        let resources = self.sessions.lock().await.get(&request.session_id).cloned();
        let Some(resources) = resources else {
            return Ok(());
        };
        let mut resources = resources.lock().await;
        if resources.binding != request.binding || resources.request != *request {
            return Err(BrowserDriverError::Denied);
        }
        resources.cleanup().await
    }
}
