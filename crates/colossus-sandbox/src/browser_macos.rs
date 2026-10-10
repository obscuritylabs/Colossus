//! Fail-closed macOS composition gate for a separately owned Chromium host.
//!
//! The human CEF preview and direct-child Darwin process ownership do not establish
//! the whole-process network and cleanup boundary required by `BrowserHostFactory`.
//! This type gives trusted composition one explicit Mac owner to retain while making
//! the missing native acceptance visible. It cannot be constructed in an available
//! state and never allocates a process, profile, channel, or network lease.

use async_trait::async_trait;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_contracts::BrowserCapabilities;
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use std::sync::Arc;

const CURRENT_BLOCKERS: [MacosBrowserRequirement; 6] = [
    MacosBrowserRequirement::WholeProcessTreeOwner,
    MacosBrowserRequirement::InheritedNetworkContainment,
    MacosBrowserRequirement::RequestOriginEnforcement,
    MacosBrowserRequirement::OwnedCertificateCustody,
    MacosBrowserRequirement::NativePresentationAcceptance,
    MacosBrowserRequirement::SignedDistributionAcceptance,
];

/// Independently accepted platform requirements that remain before Mac launch.
///
/// A direct-child PID, process group, BSD session, CEF callback, proxy, human
/// preview, or Debug signature cannot satisfy any of these requirements alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacosBrowserRequirement {
    /// Kernel-backed ownership and acknowledged exit of the browser and every helper.
    WholeProcessTreeOwner,
    /// Positively enforced direct TCP, UDP, QUIC, WebRTC, loopback and DNS denial.
    InheritedNetworkContainment,
    /// Exact HTTPS request-origin enforcement, including coalesced connections.
    RequestOriginEnforcement,
    /// Isolated private CA and reviewed client-identity import, use and removal.
    OwnedCertificateCustody,
    /// Authenticated compositor, input, IME, handoff and viewer lifecycle acceptance.
    NativePresentationAcceptance,
    /// Publisher-reviewed component inventory, signatures and installed-host evidence.
    SignedDistributionAcceptance,
}

/// Current native readiness of the Mac browser host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacosBrowserReadiness {
    blockers: &'static [MacosBrowserRequirement],
}

impl MacosBrowserReadiness {
    /// Requirements for which no independently accepted receipt exists.
    #[must_use]
    pub fn blockers(self) -> &'static [MacosBrowserRequirement] {
        self.blockers
    }

    /// Whether native composition may advertise or allocate the browser host.
    #[must_use]
    pub fn is_available(self) -> bool {
        self.blockers.is_empty()
    }
}

/// Retained Mac browser owner that deliberately cannot launch before acceptance.
///
/// There is intentionally no constructor accepting booleans, paths, process IDs,
/// audit-session IDs, or self-reported child evidence. A future accepted native
/// owner must introduce an unforgeable receipt and replace this gate as one reviewed
/// change. Until then Core can retain this value without accidentally enabling Mac
/// automation.
#[derive(Clone, Copy, Debug)]
pub struct MacosBrowserSupervisor;

impl MacosBrowserSupervisor {
    /// Construct the current fail-closed native owner.
    #[must_use]
    pub fn unavailable() -> Self {
        Self
    }

    /// Return the exact independently unaccepted requirements.
    #[must_use]
    pub fn readiness(self) -> MacosBrowserReadiness {
        MacosBrowserReadiness {
            blockers: &CURRENT_BLOCKERS,
        }
    }
}

#[async_trait]
impl BrowserHostFactory for MacosBrowserSupervisor {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }

    async fn launch(
        &self,
        _request: &BrowserDriverOpenRequest,
        _control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        Err(BrowserDriverError::Unavailable)
    }

    async fn reap_failed_launch(
        &self,
        _request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        // `launch` performs no allocation before returning unavailable. There is no
        // uncertain native obligation to reconcile through this gate.
        Ok(())
    }
}
