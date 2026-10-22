//! Bounded public metadata for native-only pending TLS client identity requests.
//! These values never authorize a model, web guest or ordinary viewer to use a key.

use colossus_contracts::BrowserOrigin;
use serde::{Deserialize, Serialize};

use crate::{MAX_CONTROL_BYTES, PresentationError};

/// Explicit policy from the separately authenticated native Desktop owner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClientIdentityPolicy {
    /// Retain every eligible identity for an explicit trusted Desktop decision.
    #[default]
    Review,
    /// Queue the sole eligible identity; multiple identities still require review.
    AutomaticIfUnique,
}

/// One exact native pending request. These numeric IDs must never cross web IPC.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentityKey {
    /// Native tab in the exact supervised host.
    pub tab: u64,
    /// Native tab generation at the TLS request.
    pub generation: u64,
    /// One-use native TLS callback request.
    pub request_id: u64,
    /// Native operation generation; zero also precedes the first admitted writer.
    pub control_generation: u64,
}
impl ClientIdentityKey {
    /// Reject absent native identities before dispatch.
    pub fn validate(&self) -> Result<(), PresentationError> {
        if self.tab == 0 || self.generation == 0 || self.request_id == 0 {
            Err(PresentationError::Invalid)
        } else {
            Ok(())
        }
    }
}

/// Public identity metadata from Chromium's issuer-pruned native certificate list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentityCandidate {
    /// SHA-256 of the exact eligible public certificate DER.
    pub fingerprint_sha256: String,
    /// Short plain-text public subject label; never HTML or private-key metadata.
    pub label: String,
}
impl ClientIdentityCandidate {
    /// Enforce bounded public display data before any trusted UI receives it.
    pub fn validate(&self) -> Result<(), PresentationError> {
        validate_fingerprint(&self.fingerprint_sha256)?;
        if self.label.is_empty()
            || self.label.len() > 128
            || self.label.trim() != self.label
            || self.label.chars().any(|value| {
                value.is_control()
                    || matches!(value, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            || serde_json::to_vec(&self.label)
                .map_err(|_| PresentationError::Invalid)?
                .len()
                > 128
        {
            Err(PresentationError::Invalid)
        } else {
            Ok(())
        }
    }
}

/// One expiring native handshake, independent of a visible presentation lease.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientIdentityChallenge {
    /// Exact native one-use request; the host rechecks it at consumption.
    pub key: ClientIdentityKey,
    /// Canonical credential-free HTTPS origin supplied by Chromium.
    pub origin: BrowserOrigin,
    /// Eligible public candidates, preserving their native host indexes privately.
    pub candidates: Vec<ClientIdentityCandidate>,
    /// Remaining native lifetime; polling never renews it.
    pub remaining_ms: u32,
}
impl ClientIdentityChallenge {
    /// Validate all display metadata, request identity and aggregate wire size.
    pub fn validate(&self) -> Result<(), PresentationError> {
        self.key.validate()?;
        if !self.origin.as_str().starts_with("https://")
            || self.origin.as_str().len() > 4096
            || self.remaining_ms == 0
            || self.remaining_ms > 45_000
            || self.candidates.is_empty()
            || self.candidates.len() > 64
        {
            return Err(PresentationError::Invalid);
        }
        for (index, candidate) in self.candidates.iter().enumerate() {
            candidate.validate()?;
            if self.candidates[..index]
                .iter()
                .any(|prior| prior.fingerprint_sha256 == candidate.fingerprint_sha256)
            {
                return Err(PresentationError::Invalid);
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| PresentationError::Invalid)?
            .len()
            > MAX_CONTROL_BYTES - 1024
        {
            return Err(PresentationError::LimitExceeded);
        }
        Ok(())
    }
}

pub(crate) fn validate_fingerprint(value: &str) -> Result<(), PresentationError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(PresentationError::Invalid)
    }
}

#[cfg(test)]
mod channel_tests;
#[cfg(test)]
mod tests;
