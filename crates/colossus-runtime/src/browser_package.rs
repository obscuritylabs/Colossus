//! Shared compiler-sealed publisher verification and native lifetime composition.
//!
//! CLI and managed sidecar embed independently sealed release bytes. Runtime YAML,
//! inherited application bootstrap, model arguments and ambient environment never
//! nominate a browser component, certificate store or publisher key.
#[cfg(target_os = "linux")]
pub mod files;
pub mod manifest;

use crate::RuntimeBrowserHost;
use colossus_ports::BrowserDriverError;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{path::Path, sync::Arc};

/// Categorical installation failure; component paths and credentials are unreleased.
#[derive(Debug, thiserror::Error)]
pub enum BrowserPackageError {
    /// Compiler-sealed package or owner-private state failed independent verification.
    #[error("installed browser package verification failed")]
    Invalid,
    /// Native cleanup remains unresolved; retain the exact owner and retry.
    #[error("installed browser cleanup outcome unknown")]
    CleanupUnknown,
}

/// Separately retained native lifetime owner; a runtime driver is not a shutdown receipt.
pub struct InstalledBrowserOwner {
    host: RuntimeBrowserHost,
    #[cfg(target_os = "linux")]
    supervisor: Arc<colossus_sandbox::OciBrowserSupervisor>,
    #[cfg(target_os = "linux")]
    shutdown_confirmed: AtomicBool,
}
impl InstalledBrowserOwner {
    /// Clone only the already verified worker/runtime driver binding.
    #[must_use]
    pub fn host(&self) -> RuntimeBrowserHost {
        self.host.clone()
    }
    /// Revoke and positively reap the exact process/profile/installation owner.
    pub async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        #[cfg(target_os = "linux")]
        {
            self.supervisor.shutdown().await?;
            self.shutdown_confirmed.store(true, Ordering::Release);
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Ok(())
    }
}
impl Drop for InstalledBrowserOwner {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if !self.shutdown_confirmed.load(Ordering::Acquire) {
            linux::retain_cleanup(Arc::clone(&self.supervisor));
        }
    }
}

/// Discover only the fixed adjacent payload sealed into the calling executable.
/// The caller supplies bytes embedded at compile time, never a runtime selector.
/// Unsealed and unsupported distributions remain unavailable.
pub async fn discover(
    compiler_manifest: &'static [u8],
    state_parent: &Path,
) -> Result<Option<Arc<InstalledBrowserOwner>>, BrowserPackageError> {
    #[cfg(target_os = "linux")]
    {
        linux::discover(compiler_manifest, state_parent).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (compiler_manifest, state_parent);
        Ok(None)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use colossus_home::ConfinedRoot;
    use colossus_sandbox::{OciBrowserConfig, OciBrowserLimits, OciBrowserSupervisor};
    use std::{path::PathBuf, time::Duration};

    pub(super) fn retain_cleanup(supervisor: Arc<OciBrowserSupervisor>) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            // Keep exact native ownership across cancellation/unknown cleanup.
            // Drop never becomes an explicit successful shutdown receipt.
            runtime.spawn(async move {
                while supervisor.shutdown().await.is_err() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }
    }

    struct StartupOwner(Option<Arc<OciBrowserSupervisor>>);
    impl Drop for StartupOwner {
        fn drop(&mut self) {
            if let Some(supervisor) = self.0.take() {
                retain_cleanup(supervisor);
            }
        }
    }

    pub(super) async fn discover(
        compiler_manifest: &'static [u8],
        state_parent: &Path,
    ) -> Result<Option<Arc<InstalledBrowserOwner>>, BrowserPackageError> {
        if compiler_manifest.is_empty() || std::env::consts::ARCH != "x86_64" {
            return Ok(None);
        }
        let executable = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .map_err(|_| BrowserPackageError::Invalid)?;
        let root = executable
            .parent()
            .ok_or(BrowserPackageError::Invalid)?
            .join("browser");
        let (root, release) = tokio::task::spawn_blocking(move || {
            files::verify(&root, compiler_manifest).map(|release| (root, release))
        })
        .await
        .map_err(|_| BrowserPackageError::Invalid)?
        .map_err(|_| BrowserPackageError::Invalid)?;
        let state_root = ConfinedRoot::bind(state_parent)
            .and_then(|root| root.prepare_directory(Path::new("browser-owned")))
            .map_err(|_| BrowserPackageError::Invalid)?;
        let mut limits = OciBrowserLimits::default();
        limits.egress.lifetime =
            Duration::from_millis(u64::from(release.capabilities.limits.max_lease_ms));
        let manifest_sha256: [u8; 32] = hex::decode(&release.component_manifest_sha256)
            .map_err(|_| BrowserPackageError::Invalid)?
            .try_into()
            .map_err(|_| BrowserPackageError::Invalid)?;
        let factory = match OciBrowserSupervisor::install(OciBrowserConfig {
            docker: PathBuf::from("/usr/bin/docker"),
            image: release.image_id,
            component_root: root.join("component"),
            manifest_sha256,
            state_root,
            limits,
            capabilities: release.capabilities,
            pki: None,
            pki_enrollment: None,
            profile_store: None,
            presentation: false,
        })
        .await
        {
            Ok(factory) => Arc::new(factory),
            // No image loading, Docker installation or capability promotion.
            Err(BrowserDriverError::Unavailable) => return Ok(None),
            Err(BrowserDriverError::OutcomeUnknown) => {
                return Err(BrowserPackageError::CleanupUnknown);
            }
            Err(_) => return Err(BrowserPackageError::Invalid),
        };
        let mut startup_owner = StartupOwner(Some(Arc::clone(&factory)));
        let host = match RuntimeBrowserHost::from_supervised_factory(factory.clone()) {
            Ok(host) => host,
            Err(_) => {
                factory
                    .shutdown()
                    .await
                    .map_err(|_| BrowserPackageError::CleanupUnknown)?;
                startup_owner.0.take();
                return Err(BrowserPackageError::Invalid);
            }
        };
        startup_owner.0.take();
        Ok(Some(Arc::new(InstalledBrowserOwner {
            host,
            supervisor: factory,
            shutdown_confirmed: AtomicBool::new(false),
        })))
    }
}
