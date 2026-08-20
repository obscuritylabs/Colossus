use std::{fmt, sync::Arc};

use async_trait::async_trait;

use crate::{
    Backend, NativeSidecarFailure, NativeSidecarStatus, SdkError, SdkResult,
    SidecarBootstrapConfig, SidecarLifecycle, SidecarOptions,
};
use tokio::sync::watch;

/// Configuration inspection is unavailable on unsupported native platforms.
pub async fn inspect_sidecar_configuration(
    _executable: &crate::VerifiedExecutable,
    _yaml: String,
) -> SdkResult<colossus_sidecar_protocol::ConfigurationInspectionResponse> {
    Err(SdkError::InvalidConfiguration(
        "configuration inspection is unsupported on this platform",
    ))
}

/// Portable API stub for platforms that do not yet have a native sidecar launcher.
pub struct NativeSidecarLifecycle {
    bootstrap: Arc<SidecarBootstrapConfig>,
    status: watch::Sender<NativeSidecarStatus>,
}

impl NativeSidecarLifecycle {
    /// Create a lifecycle that reports the platform support boundary at startup.
    pub fn new(bootstrap: SidecarBootstrapConfig) -> Self {
        let (status, _) = watch::channel(NativeSidecarStatus::Starting);
        Self {
            bootstrap: Arc::new(bootstrap),
            status,
        }
    }

    /// Return the lifecycle's current secret-free supervision state.
    pub fn status(&self) -> NativeSidecarStatus {
        *self.status.borrow()
    }

    /// Subscribe to supervision state changes without exposing process or transport data.
    pub fn subscribe_status(&self) -> watch::Receiver<NativeSidecarStatus> {
        self.status.subscribe()
    }
}

impl fmt::Debug for NativeSidecarLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSidecarLifecycle")
            .field("bootstrap", &self.bootstrap)
            .field("status", &self.status())
            .finish()
    }
}

#[async_trait]
impl SidecarLifecycle for NativeSidecarLifecycle {
    async fn start_verified(&self, _options: &SidecarOptions) -> SdkResult<Arc<dyn Backend>> {
        self.status.send_replace(NativeSidecarStatus::Failed(
            NativeSidecarFailure::SupervisionFailed,
        ));
        Err(SdkError::InvalidConfiguration(
            "native managed sidecars are currently supported only on Unix",
        ))
    }
}
