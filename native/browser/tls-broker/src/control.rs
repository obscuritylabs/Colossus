//! Control of an independently admitted, already-owned private Unix stream.
//! This layer performs no process or socket-peer admission itself.

mod codec;
mod framing;
mod service;

use crate::{Bootstrap, BrokerLimits, BrokerState};
use std::fmt;
use zeroize::Zeroizing;

#[cfg(all(target_os = "macos", feature = "macos-native-custody-development"))]
pub use service::serve_control_with_native_registry;
pub use service::{
    BrokerClient, ControlExit, ControlExitReason, DrainEvidence, serve_control,
    serve_control_with_local_identity,
};

/// Immutable native envelope, delivered only after independent socket-peer admission.
/// There is no path, file descriptor, environment, signer or client-identity field.
pub struct BootstrapEnvelope {
    /// Authority and reviewed public roots supplied by trusted native composition.
    pub bootstrap: Bootstrap,
    /// Immutable resource ceilings for this one generation.
    pub limits: BrokerLimits,
    /// Presealed nonzero port on both numeric loopback addresses.
    pub port: u16,
}

/// Public trust and private proxy authentication returned on the admitted channel.
/// The credential is zeroized on drop and omitted from Debug and diagnostics.
pub struct BrokerReady {
    pub(crate) port: u16,
    pub(crate) credential: Zeroizing<String>,
    pub(crate) ca_der: Vec<u8>,
    pub(crate) leaf_der: Vec<u8>,
    pub(crate) leaf_sha256: [u8; 32],
}

impl BrokerReady {
    /// Exact numeric dual-loopback listener port, without a discovery fallback.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Secret proxy bootstrap for the separately contained browser host only.
    pub fn credential(&self) -> &str {
        self.credential.as_str()
    }

    /// Public downstream CA DER; upstream trust roots are never returned.
    pub fn ca_der(&self) -> &[u8] {
        &self.ca_der
    }

    /// Public downstream leaf DER for the enrolled exact HTTPS origin.
    pub fn leaf_der(&self) -> &[u8] {
        &self.leaf_der
    }

    /// SHA-256 of the public leaf, supplemental to normal CA/hostname verification.
    pub fn leaf_sha256(&self) -> &[u8; 32] {
        &self.leaf_sha256
    }
}

impl fmt::Debug for BrokerReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrokerReady").finish_non_exhaustive()
    }
}

/// Categorical control failures, without peer-provided bytes or native secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    InvalidEnvelope,
    Authentication,
    Closed,
    Interrupted,
    Rejected,
}

pub(crate) enum Message {
    BootstrapPki(BootstrapEnvelope, crate::PkiSelection),
    Bootstrap(BootstrapEnvelope),
    Status,
    Revoke,
    Shutdown,
    Ready(BrokerReady),
    State(BrokerState),
    Revoked,
    ShutdownAck,
}
