use super::*;
use colossus_contracts::SecureAnchorStatus;

/// Bounded public attestation envelope; it contains no journal payload or key.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedAnchorSnapshot {
    /// Authority protocol version.
    pub version: u16,
    /// Monotonic CAS revision, including tombstones.
    pub revision: u64,
    /// Last independently protected journal attestation.
    pub anchor: Option<SecureAnchor>,
}

/// Exact conditional update accepted by the independent authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedAnchorUpdate {
    /// Authority protocol version.
    pub version: u16,
    /// Revision reviewed by the writer.
    pub expected_revision: u64,
    /// Proposed monotonic journal attestation.
    pub anchor: SecureAnchor,
}

pub(super) fn namespace(value: &str) -> Result<(), ChatError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ChatError::InvalidConfiguration);
    }
    Ok(())
}

/// Preserve the independent authority's monotonic sequence/hash invariant.
/// A full journal verification may restore a quarantined attestation at its
/// already attested boundary, matching the existing protected journal contract.
pub fn validate_shared_anchor_advance(
    previous: Option<&SecureAnchor>,
    next: &SecureAnchor,
) -> Result<(), ChatError> {
    if !(1..=2).contains(&next.format_version)
        || next.hash.len() != 64
        || !next.hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        || next.verification_profile.as_ref().is_some_and(|profile| {
            profile.trim().is_empty()
                || profile.len() > 256
                || profile.chars().any(char::is_control)
        })
    {
        return Err(ChatError::InvalidConfiguration);
    }
    if let Some(previous) = previous
        && (next.sequence < previous.sequence
            || (next.sequence == previous.sequence && next.hash != previous.hash)
            || (previous.status == SecureAnchorStatus::Quarantined
                && next.status == SecureAnchorStatus::Verified
                && next.verification_profile.is_none()))
    {
        return Err(ChatError::StateUnavailable);
    }
    Ok(())
}
