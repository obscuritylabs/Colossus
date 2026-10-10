use std::fmt;

use colossus_contracts::{
    BrowserCapabilities, BrowserMode, BrowserOrigin, BrowserProfileSelection, BrowserSessionBinding,
};
use colossus_ports::BrowserDriverError;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

/// Fresh native-only authentication material delivered over supervised bootstrap memory.
/// This key deliberately implements neither `Serialize` nor `Debug` nor `Clone`.
pub struct BrowserBridgeKey(pub(crate) Zeroizing<[u8; 32]>);

impl BrowserBridgeKey {
    /// Create unpredictable per-host material. Never reuse it across supervised restarts.
    pub fn generate() -> Result<Self, BrowserDriverError> {
        let mut bytes = Zeroizing::new([0; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| BrowserDriverError::Unavailable)?;
        Ok(Self(bytes))
    }

    /// Receive exact native bootstrap bytes by move, retaining their zeroizing owner.
    /// The supervisor must own their provenance; no ordinary serializable key DTO exists.
    pub fn from_bootstrap(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    /// Derive a distinct native presentation-channel key without exposing this key.
    /// Presentation authenticates its own enrollment, direction and bounded sequences.
    pub fn derive_presentation_key(&self) -> Zeroizing<[u8; 32]> {
        // HMAC accepts every SHA-256 key length; this input is exactly 32 bytes.
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.0.as_ref())
            .unwrap_or_else(|_| unreachable!("fixed SHA-256 HMAC key length"));
        mac.update(b"colossus-browser-presentation-key-v1\0");
        Zeroizing::new(mac.finalize().into_bytes().into())
    }
}

/// Immutable native enrollment; it is never a renderer or public application request.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserBridgeEnrollment {
    /// Exact runtime, workspace, application and conversation/workflow host ownership.
    pub binding: BrowserSessionBinding,
    /// Dedicated process placement; this is independently accepted by the supervisor.
    pub mode: BrowserMode,
    /// Exact supervisor-owned cache selection; no caller supplies its native path.
    pub profile: BrowserProfileSelection,
    /// Immutable origin envelope applied to the entire native process tree.
    pub allowed_origins: Vec<BrowserOrigin>,
    /// Fresh 128-bit native enrollment identity, distinct across restarts.
    pub instance_nonce: [u8; 16],
    /// Digest of the supervisor-verified installed component and containment policy.
    pub component_digest: [u8; 32],
    /// Supervisor-accepted cancellation semantics; true requires a fresh session after cancel.
    pub cancellation_closes_context: bool,
    /// Supervisor-accepted capability ceiling; never accepted from child self-report.
    pub capabilities: BrowserCapabilities,
}

impl BrowserBridgeEnrollment {
    /// Check strict metadata bounds before native allocation or launch effects.
    /// This validates shape; only the trusted supervisor proves installation/containment.
    pub fn validate(&self) -> Result<(), BrowserDriverError> {
        crate::validation::binding(&self.binding)?;
        if self.instance_nonce == [0; 16] || self.component_digest == [0; 32] {
            return Err(BrowserDriverError::Unavailable);
        }
        if self.allowed_origins.is_empty()
            || self.allowed_origins.len() > 32
            || self
                .allowed_origins
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.allowed_origins.len()
            || !self.capabilities.modes.contains(&self.mode)
        {
            return Err(BrowserDriverError::Unavailable);
        }
        crate::validation::capabilities(&self.capabilities)
    }
}

impl fmt::Debug for BrowserBridgeEnrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserBridgeEnrollment")
            .finish_non_exhaustive()
    }
}
