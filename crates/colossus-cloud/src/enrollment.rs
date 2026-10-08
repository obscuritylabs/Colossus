use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Single-use enrollment invitation, bound to administrator-reviewed placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    /// Hash of the high-entropy invitation, never its redeemable plaintext.
    pub token_hash: String,
    /// Fixed owning project.
    pub project_id: String,
    /// Fixed node identity.
    pub node_id: String,
    /// Administrator-reviewed display label.
    pub label: String,
    /// Additional role ceiling, independent of the local grant.
    pub roles: BTreeSet<String>,
    /// Unix second expiry enforced at redemption.
    pub expires_at: u64,
    /// Exact enrolled certificate hash after redemption.
    pub redeemed_certificate: Option<String>,
    /// CSR hash bound to the consumed invitation for exact retry reconciliation.
    pub redeemed_csr: Option<String>,
    /// Public issued certificate retained to reconcile a lost enrollment response.
    pub certificate_pem: Option<String>,
}

/// Host-validated issuance, carrying no private signing or connector key.
pub struct CertificateRedemption {
    /// Exact issued leaf hash.
    pub fingerprint: String,
    /// Exact verified CSR hash.
    pub csr_sha256: String,
    /// Issued public leaf PEM.
    pub certificate_pem: String,
}

/// Exact identity authenticated by the existing mTLS leaf during certificate renewal.
pub struct RenewalIdentity<'a> {
    /// Owning project.
    pub project_id: &'a str,
    /// Fixed enrolled node.
    pub node_id: &'a str,
    /// Independently bound local runtime.
    pub instance_id: &'a str,
    /// SHA-256 of the presented client leaf.
    pub previous_fingerprint: &'a str,
    /// Durable native-generated renewal identity.
    pub renewal_id: &'a str,
}
