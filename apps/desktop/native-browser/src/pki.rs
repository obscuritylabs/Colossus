//! Native software-certificate import. Store scope is wider than browser profiles.
//!
//! This API receives only native inputs. Password/key bytes have no serialization
//! or diagnostic surface. Imports do not establish Chromium identity-use proof.

use rustls_pki_types::{CertificateDer, pem::PemObject as _};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use x509_parser::prelude::{FromDer as _, X509Certificate};

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
        client_identity_selection_ready: false,
        acceptance_pending: true,
        message: if supported {
            "Imports affect this operating-system user's certificate store and can affect other applications. Browser profiles do not isolate this trust. Native import and Chromium key-use acceptance are pending."
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
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut result, byte| {
            let _ = write!(result, "{byte:02x}");
            result
        })
}

fn validate_ca(bytes: &[u8]) -> Result<Vec<u8>, PkiError> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(PkiError::InvalidCertificate);
    }
    let certificates = if bytes.starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| PkiError::InvalidCertificate)?
    } else {
        vec![CertificateDer::from(bytes)]
    };
    if certificates.len() != 1 {
        return Err(PkiError::InvalidCertificate);
    }
    let der = certificates[0].as_ref();
    let (remaining, certificate) =
        X509Certificate::from_der(der).map_err(|_| PkiError::InvalidCertificate)?;
    if !remaining.is_empty()
        || !certificate.validity().is_valid()
        || !certificate
            .basic_constraints()
            .map_err(|_| PkiError::InvalidCertificate)?
            .is_some_and(|value| value.value.ca)
        || certificate
            .key_usage()
            .map_err(|_| PkiError::InvalidCertificate)?
            .is_some_and(|usage| !usage.value.key_cert_sign())
    {
        return Err(PkiError::InvalidCertificate);
    }
    Ok(der.to_vec())
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
