//! Distinct Desktop admission authority, separate from worker/TUI and browser-host keys.
use crate::{Configure, PresentationError};
use colossus_contracts::{BrowserControlLease, BrowserSessionId, BrowserUrl};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[cfg(test)]
mod tests;
mod transport;
pub use transport::{AdmissionChannel, client_handshake, server_handshake};

/// Immutable native parent enrollment, reconstructed from attested inherited bootstrap.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBrowserEnrollment {
    /// Fresh supervising Desktop lifecycle identity.
    pub generation: [u8; 16],
    /// Exact managed sidecar instance.
    pub instance: [u8; 16],
    /// Primary bootstrap application's immutable identity.
    pub application_id: String,
    /// Attested kernel workspace identity derivation version.
    pub workspace_version: u16,
    /// Attested kernel workspace identity digest, never a model-supplied path.
    pub workspace_digest: [u8; 32],
    /// Positively authenticated supervising native process.
    pub parent_process_id: u32,
}
impl NativeBrowserEnrollment {
    /// Fixed short Unix endpoint, avoiding application state paths beyond sockaddr bounds.
    /// Native composition must validate or create the returned parent as owner-private.
    #[cfg(unix)]
    pub fn unix_endpoint(&self, user_id: u32) -> Result<std::path::PathBuf, PresentationError> {
        let root = if cfg!(target_os = "macos") {
            "/private/tmp"
        } else {
            "/tmp"
        };
        let digest = self.digest()?;
        let leaf = digest[..20]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(std::path::Path::new(root)
            .join(format!("colossus-native-browser-{user_id}"))
            .join(format!("b-{leaf}.sock")))
    }
    /// Reject missing native identities before admitting an endpoint or key.
    pub fn validate(&self) -> Result<(), PresentationError> {
        if self.generation == [0; 16]
            || self.instance == [0; 16]
            || self.workspace_digest == [0; 32]
            || !(1..=3).contains(&self.workspace_version)
            || self.parent_process_id == 0
            || self.application_id.is_empty()
            || self.application_id.len() > 256
            || self.application_id.chars().any(char::is_control)
        {
            return Err(PresentationError::Invalid);
        }
        Ok(())
    }
    /// Fixed authority digest used only by the private admission handshake.
    pub fn digest(&self) -> Result<[u8; 32], PresentationError> {
        self.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"colossus-desktop-browser-authority-v1\0");
        hash.update(serde_json::to_vec(self).map_err(|_| PresentationError::Invalid)?);
        Ok(hash.finalize().into())
    }
}

/// Native GUI intent, without executable, profile, binding, credential or receipt inputs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBrowserOpen {
    /// Existing app-owned conversation, or request creation of a canonical new conversation.
    pub conversation_id: Option<String>,
    /// Explicit human address; its exact origin bounds this new browser context.
    pub url: BrowserUrl,
    /// Logical viewport width.
    pub width: u32,
    /// Logical viewport height.
    pub height: u32,
    /// Native display scale multiplied by one thousand.
    pub scale_milli: u32,
    /// Fresh native viewport generation.
    pub viewport_generation: u64,
    /// Short native visibility heartbeat bound.
    pub lease_ms: u16,
}
impl NativeBrowserOpen {
    /// Bound intent before allocation; opaque ownership is separately runtime-derived.
    pub fn validate(&self) -> Result<(), PresentationError> {
        if self
            .conversation_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(PresentationError::Invalid);
        }
        // Reuse exactly the native presenter's scale, dimensions and heartbeat limits.
        if self.viewport_generation == 0
            || !(500..=4000).contains(&self.scale_milli)
            || self.width == 0
            || self.height == 0
            || self.width > 4096
            || self.height > 4096
            || !(1..=1500).contains(&self.lease_ms)
        {
            return Err(PresentationError::LimitExceeded);
        }
        let width = (u64::from(self.width) * u64::from(self.scale_milli)).div_ceil(1000);
        let height = (u64::from(self.height) * u64::from(self.scale_milli)).div_ceil(1000);
        if width > 4096 || height > 4096 || width * height * 4 > crate::MAX_FRAME_BYTES as u64 {
            return Err(PresentationError::LimitExceeded);
        }
        Ok(())
    }
}

/// Exactly one authenticated admission operation; presentation uses its own derived key.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NativeBrowserRequest {
    /// Check accepted Embedded availability without allocating a host.
    Probe,
    /// Explicit native human admission to an app-owned canonical conversation.
    Open(NativeBrowserOpen),
    /// Reap the exact native admission and its private context.
    Close {
        /// Exact opaque native admission owned by this enrolled application.
        session: BrowserSessionId,
    },
    /// Drop this native viewer; close a Human context, preserve an agent-owned page.
    Detach {
        /// Exact native admission whose viewer belongs to this authority.
        session: BrowserSessionId,
    },
    /// Attach an already registered run through Runtime's ordinary effect gateway.
    Handoff {
        /// Exact native human context to transfer.
        session: BrowserSessionId,
        /// Existing registered run; provenance is reconstructed by Runtime.
        run_id: String,
        /// Requested bounded writer lease.
        lease_ms: u32,
    },
}
impl NativeBrowserRequest {
    /// Validate bounded intent; no wire request can nominate runtime provenance.
    pub fn validate(&self) -> Result<(), PresentationError> {
        match self {
            Self::Open(request) => request.validate(),
            Self::Handoff {
                run_id, lease_ms, ..
            } if run_id.is_empty()
                || run_id.len() > 256
                || run_id.chars().any(char::is_control)
                || *lease_ms == 0 =>
            {
                Err(PresentationError::Invalid)
            }
            _ => Ok(()),
        }
    }
}

/// Categorical admission result with native-only opaque ownership and no host key.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NativeBrowserReply {
    /// Installed platform proof or Embedded capabilities are absent.
    Unavailable,
    /// Credential, app/session ownership, active run or policy denied admission.
    Denied,
    /// Accepted platform Embedded support is available; no host was launched.
    Available,
    /// Human ownership was admitted and the same stream becomes a native relay.
    Opened {
        /// Exact canonical conversation selected or created by Runtime.
        conversation_id: String,
        /// Runtime-derived opaque session and initial native placement.
        configure: Configure,
        /// Exact retained host enrollment, authenticated by this native stream.
        enrollment_digest: [u8; 32],
    },
    /// Positively acknowledged full context cleanup.
    Closed,
    /// Native viewer is quiescent; an agent-owned context remains run-owned.
    Detached,
    /// Gateway-authorized run writer after native fence and atomic Core adoption.
    Granted {
        /// Server-held run writer after gateway authorization and native fence.
        lease: BrowserControlLease,
        /// Exact postgrant read-only target; viewport is the next fresh GUI configuration.
        configure: Configure,
        /// Authenticated retained host enrollment for the unchanged native relay.
        enrollment_digest: [u8; 32],
    },
    /// An effect or cleanup may have happened; retain ownership and do not replay it.
    OutcomeUnknown,
}
