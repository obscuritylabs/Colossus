//! Native software-certificate import. Store scope is wider than browser profiles.
//!
//! This API receives only native inputs. Password/key bytes have no serialization
//! or diagnostic surface. Imports do not establish Chromium identity-use proof.

use serde::Serialize;

pub(crate) mod selection;
pub use selection::{IdentityCandidate, IdentityRequest};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Non-secret native provisioning metadata only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // Independent native capability projection.
pub struct PkiStatus {
    pub scope: &'static str,
    pub ca_import_available: bool,
    pub pfx_import_available: bool,
    pub client_identity_review_available: bool,
    pub client_identity_selection_ready: bool,
    pub acceptance_pending: bool,
    pub message: &'static str,
}

/// A completed native-store import, not proof of a successful mTLS handshake.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReceipt {
    pub scope: &'static str,
    pub fingerprints_sha256: Vec<String>,
    pub identity_selection_ready: bool,
}

/// Categorical failures never include selected paths or secret/native error text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PkiError {
    Unsupported,
    InvalidCertificate,
    InvalidIdentity,
    Unavailable,
    StoreChangeUnknown,
}

impl std::fmt::Display for PkiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "Native browser certificate import is unavailable on this platform.",
            Self::InvalidCertificate => "Choose one valid CA certificate in PEM or DER format.",
            Self::InvalidIdentity => "The PKCS#12 identity or its password could not be validated.",
            Self::Unavailable => "The native certificate store could not complete the import.",
            Self::StoreChangeUnknown => "The native store may have changed before import failed. Review the user certificate store before retrying.",
        })
    }
}
impl std::error::Error for PkiError {}

/// Actual compiled platform importer status. Native use still requires acceptance.
#[must_use]
pub fn status() -> PkiStatus {
    let supported = cfg!(any(windows, target_os = "macos"));
    PkiStatus {
        scope: if supported {
            "operating_system_user"
        } else {
            "unsupported"
        },
        ca_import_available: supported,
        pfx_import_available: supported,
        client_identity_review_available: supported,
        client_identity_selection_ready: false,
        acceptance_pending: true,
        message: if supported {
            "Imports affect this operating-system user's certificate store and can affect other applications. Browser profiles do not isolate this trust. The Chromium preview supports native review for each pending identity request; installed native key-use acceptance is pending."
        } else {
            "This Desktop build has no native browser certificate importer. Colossus network credentials do not provision Chromium trust or client identities."
        },
    }
}

/// Import one reviewed CA into the user's native trust store.
///
/// # Errors
/// Rejects non-CA/expired/bounded input and reports categorical native failures.
pub fn import_ca(bytes: &[u8]) -> Result<ImportReceipt, PkiError> {
    let certificate = validate_ca(bytes)?;
    #[cfg(windows)]
    windows::import_ca(&certificate)?;
    #[cfg(target_os = "macos")]
    macos::import_ca(&certificate)?;
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = certificate;
        Err(PkiError::Unsupported)
    }
    #[cfg(any(windows, target_os = "macos"))]
    Ok(receipt(vec![fingerprint(&certificate)]))
}

/// Validate a selected native CA and return its DER fingerprint for final review.
///
/// # Errors
/// Rejects invalid, expired, non-CA, or oversized certificates without mutation.
pub fn ca_fingerprint(bytes: &[u8]) -> Result<String, PkiError> {
    validate_ca(bytes).map(|der| fingerprint(&der))
}

/// Import a native-entered PKCS#12 identity into the user's native key store.
/// Windows imports do not request exportable keys or overwrite existing keys.
///
/// # Errors
/// Rejects empty/oversized input, wrong password, and categorical store failures.
pub fn import_pfx(bytes: &[u8], password: &str) -> Result<ImportReceipt, PkiError> {
    if bytes.is_empty()
        || bytes.len() > 4 * 1024 * 1024
        || password.len() > 64 * 1024
        || password.contains('\0')
    {
        return Err(PkiError::InvalidIdentity);
    }
    #[cfg(windows)]
    return windows::import_pfx(bytes, password).map(receipt);
    #[cfg(target_os = "macos")]
    return macos::import_pfx(bytes, password).map(receipt);
    #[cfg(not(any(windows, target_os = "macos")))]
    Err(PkiError::Unsupported)
}

#[cfg(any(windows, target_os = "macos"))]
fn receipt(fingerprints: Vec<String>) -> ImportReceipt {
    ImportReceipt {
        scope: "operating_system_user",
        fingerprints_sha256: fingerprints,
        identity_selection_ready: false,
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    colossus_native_browser_pki::fingerprint(bytes)
}

fn validate_ca(bytes: &[u8]) -> Result<Vec<u8>, PkiError> {
    colossus_native_browser_pki::ca_der(bytes).map_err(|_| PkiError::InvalidCertificate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_reject_oversized_or_malformed_inputs_before_native_effects() {
        assert!(matches!(
            import_ca(b"not a certificate"),
            Err(PkiError::InvalidCertificate)
        ));
        assert!(matches!(
            import_pfx(&[], ""),
            Err(PkiError::InvalidIdentity)
        ));
        assert!(matches!(
            import_pfx(b"x", "bad\0password"),
            Err(PkiError::InvalidIdentity)
        ));
        assert_eq!(fingerprint(b"x").len(), 64);
    }
    #[test]
    fn native_status_never_claims_profile_isolation_or_mtls_readiness() {
        let status = status();
        assert!(!status.client_identity_selection_ready);
        assert!(status.acceptance_pending);
        assert_ne!(status.scope, "browser_profile");
    }
}
