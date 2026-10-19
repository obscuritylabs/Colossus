use colossus_ports::ComputerDriver;
use std::{fmt, sync::Arc};

/// Verified native desktop driver selected only by trusted host composition.
/// No helper executable, process identity, or transport key comes from configuration or the model.
#[derive(Clone)]
pub struct RuntimeComputerHost {
    pub(super) driver: Arc<dyn ComputerDriver>,
}

impl RuntimeComputerHost {
    /// Bind a supervisor verified by the native CLI or managed sidecar host.
    #[must_use]
    pub fn new(driver: Arc<dyn ComputerDriver>) -> Self {
        Self { driver }
    }

    pub(crate) fn capabilities(&self) -> colossus_contracts::ComputerCapabilities {
        self.driver.capabilities()
    }
}

impl fmt::Debug for RuntimeComputerHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeComputerHost")
            .finish_non_exhaustive()
    }
}

impl crate::Runtime {
    /// Bind the trusted post-authorized computer PNG publisher. Existing composition wins.
    pub fn install_computer_artifact_publisher(
        &self,
        publisher: Arc<dyn colossus_ports::ComputerArtifactPublisher>,
    ) {
        if let Some(computer) = &self.computer
            && let Ok(mut installed) = computer.artifacts.lock()
            && installed.is_none()
        {
            *installed = Some(publisher);
        }
    }

    /// Credential-free verified native computer availability; disabled means no fallback.
    pub fn computer_capabilities(&self) -> colossus_contracts::ComputerCapabilities {
        self.computer.as_ref().map_or_else(
            || colossus_contracts::ComputerCapabilities {
                available: false,
                platform: std::env::consts::OS.into(),
                limits: Default::default(),
                unavailable_reason: Some("disabled_or_unavailable".into()),
            },
            |computer| computer.capabilities(),
        )
    }

    /// Enumeration also owns a helper and therefore pins an active runtime until cleanup.
    pub fn computer_active_work(&self) -> bool {
        self.computer
            .as_ref()
            .is_some_and(|computer| computer.runs.lock().map_or(true, |runs| !runs.is_empty()))
    }

    /// Synchronously revoke all native writer authority during trusted disable or shutdown.
    pub fn stop_computer_sessions(&self) {
        if let Some(computer) = &self.computer {
            computer.cancel_all();
        }
    }

    /// Acknowledge bounded native helper cleanup without closing the user's applications.
    pub async fn drain_computer_sessions(&self) -> Result<(), colossus_ports::ComputerDriverError> {
        if let Some(computer) = &self.computer {
            computer.drain().await?;
        }
        Ok(())
    }
}
