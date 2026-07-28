use reqwest::{Certificate, ClientBuilder};
use rustls::{
    RootCertStore,
    pki_types::{CertificateDer, pem::PemObject as _},
};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    fs::File,
    io::{Read as _, Take},
    path::Path,
    sync::Arc,
};
use thiserror::Error;

const MAX_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CA_CERTIFICATES: usize = 256;

/// Validated additional trust anchors shared by Colossus-owned network clients.
///
/// These roots augment each client's built-in public roots. Adapters with an explicit
/// exclusive trust policy, such as a pinned OPA or PostgreSQL configuration, retain
/// that stricter policy.
#[derive(Clone, Default)]
pub struct AdditionalRootCertificates {
    reqwest: Arc<[Certificate]>,
    rustls: Arc<[CertificateDer<'static>]>,
}

impl AdditionalRootCertificates {
    /// Read and validate one bounded PEM CA certificate bundle.
    pub fn from_pem_bundle_path(path: impl AsRef<Path>) -> Result<Self, TlsTrustError> {
        let file = File::open(path).map_err(|_| TlsTrustError::Unreadable)?;
        let mut bytes = Vec::new();
        let mut bounded: Take<File> = file.take(MAX_CA_BUNDLE_BYTES + 1);
        bounded
            .read_to_end(&mut bytes)
            .map_err(|_| TlsTrustError::Unreadable)?;
        if bytes.len() as u64 > MAX_CA_BUNDLE_BYTES {
            return Err(TlsTrustError::TooLarge);
        }
        Self::from_pem_bundle(&bytes)
    }

    /// Validate an in-memory PEM CA certificate bundle.
    pub fn from_pem_bundle(pem: &[u8]) -> Result<Self, TlsTrustError> {
        if pem.len() as u64 > MAX_CA_BUNDLE_BYTES {
            return Err(TlsTrustError::TooLarge);
        }
        let reqwest = Certificate::from_pem_bundle(pem).map_err(|_| TlsTrustError::Invalid)?;
        if reqwest.is_empty() {
            return Err(TlsTrustError::Empty);
        }
        if reqwest.len() > MAX_CA_CERTIFICATES {
            return Err(TlsTrustError::TooMany);
        }
        let rustls = CertificateDer::pem_slice_iter(pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TlsTrustError::Invalid)?;
        if rustls.len() != reqwest.len() {
            return Err(TlsTrustError::Invalid);
        }
        let mut validation_store = RootCertStore::empty();
        for certificate in &rustls {
            validation_store
                .add(certificate.clone())
                .map_err(|_| TlsTrustError::Invalid)?;
        }
        Ok(Self {
            reqwest: reqwest.into(),
            rustls: rustls.into(),
        })
    }

    /// Add these roots to a reqwest client builder without removing built-in roots.
    pub fn configure_reqwest(&self, mut builder: ClientBuilder) -> ClientBuilder {
        for certificate in self.reqwest.iter() {
            builder = builder.add_root_certificate(certificate.clone());
        }
        builder
    }

    /// Add these roots to an existing rustls root store.
    pub fn add_to_rustls(&self, roots: &mut RootCertStore) -> Result<(), TlsTrustError> {
        for certificate in self.rustls.iter() {
            roots
                .add(certificate.clone())
                .map_err(|_| TlsTrustError::Invalid)?;
        }
        Ok(())
    }

    /// Whether no additional roots were configured.
    pub fn is_empty(&self) -> bool {
        self.reqwest.is_empty()
    }

    /// Number of configured additional roots.
    pub fn len(&self) -> usize {
        self.reqwest.len()
    }

    /// Return stable SHA-256 fingerprints of the validated DER certificates.
    ///
    /// Fingerprints are safe trust-anchor metadata; the original bundle bytes and
    /// source path remain native-only.
    pub fn fingerprints_sha256(&self) -> Vec<String> {
        self.rustls
            .iter()
            .map(|certificate| hex::encode(Sha256::digest(certificate.as_ref())))
            .collect()
    }
}

impl fmt::Debug for AdditionalRootCertificates {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdditionalRootCertificates")
            .field("certificate_count", &self.len())
            .finish()
    }
}

/// Invalid or unavailable outbound TLS trust configuration.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TlsTrustError {
    /// The configured bundle could not be opened or read.
    #[error("CA certificate bundle is unreadable")]
    Unreadable,
    /// The configured bundle exceeded the startup bound.
    #[error("CA certificate bundle exceeds 4 MiB")]
    TooLarge,
    /// The configured bundle was not valid PEM-encoded CA material.
    #[error("CA certificate bundle is invalid")]
    Invalid,
    /// The configured bundle contained no certificates.
    #[error("CA certificate bundle contains no certificates")]
    Empty,
    /// The configured bundle exceeded the certificate-count bound.
    #[error("CA certificate bundle contains more than 256 certificates")]
    TooMany,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};

    fn ca_pem(name: &str) -> String {
        let mut params = CertificateParams::new(vec![name.into()]).expect("CA parameters");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .self_signed(&KeyPair::generate().expect("CA key"))
            .expect("CA certificate")
            .pem()
    }

    #[test]
    fn pem_bundle_is_bounded_validated_and_redacted_from_debug() {
        let bundle = format!("{}\n{}", ca_pem("one.example"), ca_pem("two.example"));
        let roots =
            AdditionalRootCertificates::from_pem_bundle(bundle.as_bytes()).expect("valid bundle");
        assert_eq!(roots.len(), 2);
        assert_eq!(roots.fingerprints_sha256().len(), 2);
        assert!(
            roots
                .fingerprints_sha256()
                .iter()
                .all(|fingerprint| fingerprint.len() == 64)
        );
        assert!(!roots.is_empty());
        assert_eq!(
            format!("{roots:?}"),
            "AdditionalRootCertificates { certificate_count: 2 }"
        );
        roots
            .configure_reqwest(reqwest::Client::builder())
            .build()
            .expect("reqwest accepts validated roots");
        let mut rustls = RootCertStore::empty();
        roots
            .add_to_rustls(&mut rustls)
            .expect("rustls accepts validated roots");
        assert_eq!(rustls.len(), 2);
    }

    #[test]
    fn invalid_empty_and_oversized_bundles_fail_closed() {
        assert_eq!(
            AdditionalRootCertificates::from_pem_bundle(b"not a certificate")
                .expect_err("invalid bundle"),
            TlsTrustError::Empty
        );
        assert_eq!(
            AdditionalRootCertificates::from_pem_bundle(&vec![
                b'x';
                (MAX_CA_BUNDLE_BYTES + 1) as usize
            ])
            .expect_err("oversized bundle"),
            TlsTrustError::TooLarge
        );
    }
}
