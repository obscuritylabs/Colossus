//! Public-certificate validation for one native, expiring handshake review.
//!
//! These native-only values are deliberately not deserializable or serializable.
//! A renderer can request review, but cannot supply an origin, certificate, or key.

#![cfg_attr(
    not(colossus_cef_linked),
    allow(
        dead_code,
        reason = "An unlinked preview has the same native request API but cannot receive CEF handshakes."
    )
)]

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{PkiError, fingerprint};

pub(crate) const MAX_CANDIDATES: usize = 64;
pub(crate) const MAX_CERTIFICATE_BYTES: usize = 65_536;
const REVIEW_TIMEOUT: Duration = Duration::from_mins(2);

/// An eligible public leaf identity offered by Chromium's native store lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityCandidate {
    pub(crate) index: i32,
    not_before: i64,
    not_after: i64,
    /// SHA-256 of the exact public DER certificate, never its private key.
    pub fingerprint_sha256: String,
}

/// One exact native handshake request. Closing/revoking the tab invalidates it.
#[derive(Clone, Debug)]
pub struct IdentityRequest {
    pub(crate) id: u64,
    pub(crate) epoch: u64,
    /// Canonical credential-free HTTPS origin reported by Chromium.
    pub origin: String,
    /// Valid, non-CA identities in the server's native candidate list.
    pub candidates: Vec<IdentityCandidate>,
    expires_at: Instant,
}

impl IdentityRequest {
    pub(crate) fn new(
        id: u64,
        epoch: u64,
        origin: &str,
        certificates: &[&[u8]],
    ) -> Result<Self, PkiError> {
        if id == 0 || certificates.is_empty() || certificates.len() > MAX_CANDIDATES {
            return Err(PkiError::InvalidIdentity);
        }
        exact_https_origin(origin)?;
        let mut candidates = Vec::with_capacity(certificates.len());
        for (index, der) in certificates.iter().enumerate() {
            // A malformed native list fails closed; valid but ineligible leafs
            // are excluded without renumbering Chromium's candidate indexes.
            if der.is_empty() || der.len() > MAX_CERTIFICATE_BYTES {
                return Err(PkiError::InvalidIdentity);
            }
            if let Some((not_before, not_after)) = eligible(der)? {
                let fingerprint_sha256 = fingerprint(der);
                if candidates.iter().any(|candidate: &IdentityCandidate| {
                    candidate.fingerprint_sha256 == fingerprint_sha256
                }) {
                    return Err(PkiError::InvalidIdentity);
                }
                candidates.push(IdentityCandidate {
                    index: i32::try_from(index).map_err(|_| PkiError::InvalidIdentity)?,
                    not_before,
                    not_after,
                    fingerprint_sha256,
                });
            }
        }
        if candidates.is_empty() {
            return Err(PkiError::InvalidIdentity);
        }
        Ok(Self {
            id,
            epoch,
            origin: origin.into(),
            candidates,
            expires_at: Instant::now() + REVIEW_TIMEOUT,
        })
    }

    /// Native dialogs must dismiss expired reviews without selecting a key.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub(crate) fn candidate_index(&self, reviewed_fingerprint: &str) -> Option<i32> {
        if self.is_expired() {
            return None;
        }
        let now =
            i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
        self.candidates
            .iter()
            .find(|candidate| {
                candidate.fingerprint_sha256 == reviewed_fingerprint
                    && candidate.not_before <= now
                    && now < candidate.not_after
            })
            .map(|candidate| candidate.index)
    }
}

fn exact_https_origin(origin: &str) -> Result<(), PkiError> {
    if origin.len() > 8192 {
        return Err(PkiError::InvalidIdentity);
    }
    let value = url::Url::parse(origin).map_err(|_| PkiError::InvalidIdentity)?;
    if value.scheme() != "https"
        || value.host_str().is_none()
        || !value.username().is_empty()
        || value.password().is_some()
        || value.query().is_some()
        || value.fragment().is_some()
        || value.origin().ascii_serialization() != origin
    {
        return Err(PkiError::InvalidIdentity);
    }
    Ok(())
}

fn eligible(der: &[u8]) -> Result<Option<(i64, i64)>, PkiError> {
    colossus_native_browser_pki::identity_validity(der).map_err(|_| PkiError::InvalidIdentity)
}

#[cfg(test)]
mod tests;
