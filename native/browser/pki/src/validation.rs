use rustls_pki_types::{CertificateDer, pem::PemObject as _};
use sha2::{Digest as _, Sha256};
use x509_parser::prelude::{FromDer as _, X509Certificate};

/// Invalid public certificate; never carries paths, native errors, or secrets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationError;

/// SHA-256 of the exact public certificate DER.
pub fn fingerprint(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut result, byte| {
            let _ = write!(result, "{byte:02x}");
            result
        })
}

/// Validate one bounded, currently valid CA certificate before explicit trust mutation.
pub fn ca_der(bytes: &[u8]) -> Result<Vec<u8>, ValidationError> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Err(ValidationError);
    }
    let certificates = if bytes.starts_with(b"-----BEGIN") {
        CertificateDer::pem_slice_iter(bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ValidationError)?
    } else {
        vec![CertificateDer::from(bytes)]
    };
    if certificates.len() != 1 {
        return Err(ValidationError);
    }
    let der = certificates[0].as_ref();
    let (remaining, certificate) = X509Certificate::from_der(der).map_err(|_| ValidationError)?;
    if !remaining.is_empty()
        || !certificate.validity().is_valid()
        || !certificate
            .basic_constraints()
            .map_err(|_| ValidationError)?
            .is_some_and(|value| value.value.ca)
        || certificate
            .key_usage()
            .map_err(|_| ValidationError)?
            .is_some_and(|value| !value.value.key_cert_sign())
    {
        return Err(ValidationError);
    }
    Ok(der.to_vec())
}

/// Exclude ineligible native client identities; malformed DER fails the entire request.
pub fn identity_validity(der: &[u8]) -> Result<Option<(i64, i64)>, ValidationError> {
    if der.is_empty() || der.len() > 65_536 {
        return Err(ValidationError);
    }
    let (remaining, certificate) = X509Certificate::from_der(der).map_err(|_| ValidationError)?;
    if !remaining.is_empty() {
        return Err(ValidationError);
    }
    let eligible = certificate.validity().is_valid()
        && !certificate
            .basic_constraints()
            .map_err(|_| ValidationError)?
            .is_some_and(|value| value.value.ca)
        && certificate
            .key_usage()
            .map_err(|_| ValidationError)?
            .is_none_or(|value| value.value.digital_signature())
        && certificate
            .extended_key_usage()
            .map_err(|_| ValidationError)?
            .is_none_or(|value| value.value.client_auth);
    Ok(eligible.then(|| {
        (
            certificate.validity().not_before.timestamp(),
            certificate.validity().not_after.timestamp(),
        )
    }))
}
