//! Stable, independently keyed TLS identity for the loopback public API.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use getrandom::fill;
#[cfg(test)]
use rcgen::BasicConstraints as CertificateBasicConstraints;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519, SerialNumber, date_time_ymd,
};
use rustls::{
    ServerConfig,
    client::verify_server_name,
    pki_types::{
        CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, pem::PemObject as _,
    },
    server::ParsedCertificate,
};
use sha2::{Digest as _, Sha256};
use std::{fmt, sync::Arc};
use thiserror::Error;
use x509_parser::parse_x509_certificate;
use zeroize::Zeroizing;

const KEY_SEED_BYTES: usize = 32;
const MAX_CERTIFICATE_PEM_BYTES: usize = 256 * 1024;
const MAX_PRIVATE_KEY_PEM_BYTES: usize = 64 * 1024;
const REQUIRED_SUBJECT_ALT_NAMES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
const ED25519_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// Dedicated seed for the public API TLS identity.
///
/// This value must come from an API-specific platform secret. It must not be
/// derived from the journal signing key, worker IPC key, bearer-authentication
/// root, or any provider credential. The seed is redacted from debug output and
/// zeroized on drop.
pub struct TlsKeySeed(Zeroizing<[u8; KEY_SEED_BYTES]>);

impl TlsKeySeed {
    /// Wrap an independently loaded 32-byte seed.
    pub fn new(seed: [u8; KEY_SEED_BYTES]) -> Self {
        Self(Zeroizing::new(seed))
    }

    /// Generate a new independent seed that the caller must persist securely.
    pub fn random() -> Result<Self, TlsIdentityError> {
        let mut seed = Zeroizing::new([0_u8; KEY_SEED_BYTES]);
        fill(seed.as_mut()).map_err(|_| TlsIdentityError::RandomUnavailable)?;
        Ok(Self(seed))
    }

    fn expose(&self) -> &[u8; KEY_SEED_BYTES] {
        &self.0
    }
}

impl fmt::Debug for TlsKeySeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TlsKeySeed([REDACTED])")
    }
}

/// Validated single-leaf TLS certificate and private key for the loopback API server.
///
/// The private key is retained in zeroizing memory until it is consumed by tonic.
/// This type is deliberately not `Clone`.
pub struct TlsIdentity {
    certificate_pem: Vec<u8>,
    private_key_pem: Zeroizing<Vec<u8>>,
    certificate_sha256: String,
}

impl TlsIdentity {
    /// Deterministically generate the stable local certificate from a dedicated seed.
    ///
    /// The certificate contains exact SAN entries for `localhost`, `127.0.0.1`,
    /// and `::1`. Reusing the same seed produces the same DER fingerprint; rotating
    /// the seed deliberately rotates the identity and descriptor pin.
    pub fn from_seed(seed: TlsKeySeed) -> Result<Self, TlsIdentityError> {
        let (certificate_pem, private_key_pem) =
            generate_materials(seed.expose(), &REQUIRED_SUBJECT_ALT_NAMES)?;
        Self::from_pem(certificate_pem, private_key_pem)
    }

    /// Validate and construct an identity from separately supplied PEM material.
    ///
    /// The PEM must contain exactly one end-entity certificate with an explicit
    /// `BasicConstraints CA=false`, cover every required loopback SAN, and match the
    /// private key. The caller should load the private key directly into a
    /// `Zeroizing<Vec<u8>>` from an API-specific platform secret store.
    pub fn from_pem(
        certificate_pem: Vec<u8>,
        private_key_pem: Zeroizing<Vec<u8>>,
    ) -> Result<Self, TlsIdentityError> {
        let leaf = validate_pem_identity(&certificate_pem, private_key_pem.as_ref())?;
        let certificate_sha256 = lowercase_hex(&Sha256::digest(leaf.as_ref()));
        Ok(Self {
            certificate_pem,
            private_key_pem,
            certificate_sha256,
        })
    }

    /// PEM containing exactly one pinned end-entity certificate.
    pub fn certificate_pem(&self) -> &[u8] {
        &self.certificate_pem
    }

    /// Lowercase SHA-256 fingerprint of the leaf certificate DER.
    pub fn certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }

    /// Consume this material into the actual TLS 1.3-only acceptor configuration.
    pub(crate) fn into_rustls_server_config(self) -> Result<Arc<ServerConfig>, TlsIdentityError> {
        let certificates = CertificateDer::pem_slice_iter(&self.certificate_pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TlsIdentityError::InvalidCertificatePem)?;
        if certificates.len() != 1 {
            return Err(TlsIdentityError::InvalidCertificatePem);
        }
        let mut keys = PrivateKeyDer::pem_slice_iter(self.private_key_pem.as_ref());
        let private_key = keys
            .next()
            .ok_or(TlsIdentityError::InvalidPrivateKeyPem)?
            .map_err(|_| TlsIdentityError::InvalidPrivateKeyPem)?;
        if keys.next().is_some() {
            return Err(TlsIdentityError::InvalidPrivateKeyPem);
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| TlsIdentityError::InvalidIdentity)?
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|_| TlsIdentityError::InvalidIdentity)?;
        config.alpn_protocols = vec![b"h2".to_vec()];
        Ok(Arc::new(config))
    }
}

impl fmt::Debug for TlsIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TlsIdentity")
            .field("certificate_sha256", &self.certificate_sha256)
            .field("certificate_pem_bytes", &self.certificate_pem.len())
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

/// Failure to generate or validate local TLS identity material.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TlsIdentityError {
    /// The operating system random source did not provide a seed.
    #[error("secure random generation for the TLS identity is unavailable")]
    RandomUnavailable,
    /// Deterministic certificate generation failed.
    #[error("could not generate the local TLS identity")]
    GenerationFailed,
    /// The certificate PEM was absent, oversized, mixed with key material, or malformed.
    #[error("TLS certificate PEM is invalid")]
    InvalidCertificatePem,
    /// The key PEM was absent, oversized, mixed with certificate material, or malformed.
    #[error("TLS private-key PEM is invalid")]
    InvalidPrivateKeyPem,
    /// The certificate does not cover every exact loopback server name.
    #[error("TLS certificate is missing a required loopback subject alternative name")]
    MissingLoopbackSubjectAlternativeName,
    /// The private key and leaf certificate do not form a valid server identity.
    #[error("TLS certificate and private key do not form a valid server identity")]
    InvalidIdentity,
}

fn generate_materials(
    seed: &[u8; KEY_SEED_BYTES],
    subject_alt_names: &[&str],
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), TlsIdentityError> {
    generate_materials_with_is_ca(seed, subject_alt_names, IsCa::ExplicitNoCa)
}

fn generate_materials_with_is_ca(
    seed: &[u8; KEY_SEED_BYTES],
    subject_alt_names: &[&str],
    is_ca: IsCa,
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), TlsIdentityError> {
    let mut private_key_der = Zeroizing::new(Vec::with_capacity(
        ED25519_PKCS8_PREFIX.len() + KEY_SEED_BYTES,
    ));
    private_key_der.extend_from_slice(&ED25519_PKCS8_PREFIX);
    private_key_der.extend_from_slice(seed);
    let pkcs8 = PrivatePkcs8KeyDer::from(private_key_der.as_slice());
    let key_pair = Zeroizing::new(
        KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &PKCS_ED25519)
            .map_err(|_| TlsIdentityError::GenerationFailed)?,
    );

    let mut params = CertificateParams::new(
        subject_alt_names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| TlsIdentityError::GenerationFailed)?;
    params.not_before = date_time_ymd(2025, 1, 1);
    params.not_after = date_time_ymd(2050, 1, 1);
    params.is_ca = is_ca;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    if matches!(is_ca, IsCa::Ca(_)) {
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    }
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, "Colossus Local API");
    params.distinguished_name = distinguished_name;

    let digest = Sha256::digest(key_pair.public_key_raw());
    let mut serial = digest[..20].to_vec();
    serial[0] &= 0x7f;
    if serial.iter().all(|byte| *byte == 0) {
        serial[19] = 1;
    }
    params.serial_number = Some(SerialNumber::from_slice(&serial));

    let certificate = params
        .self_signed(&*key_pair)
        .map_err(|_| TlsIdentityError::GenerationFailed)?;
    let certificate_pem = certificate.pem().into_bytes();
    let private_key_pem = encode_private_key_pem(private_key_der.as_ref());
    Ok((certificate_pem, private_key_pem))
}

fn validate_pem_identity(
    certificate_pem: &[u8],
    private_key_pem: &[u8],
) -> Result<CertificateDer<'static>, TlsIdentityError> {
    if certificate_pem.is_empty()
        || certificate_pem.len() > MAX_CERTIFICATE_PEM_BYTES
        || contains_bytes(certificate_pem, b"PRIVATE KEY")
    {
        return Err(TlsIdentityError::InvalidCertificatePem);
    }
    if private_key_pem.is_empty()
        || private_key_pem.len() > MAX_PRIVATE_KEY_PEM_BYTES
        || contains_bytes(private_key_pem, b"CERTIFICATE")
    {
        return Err(TlsIdentityError::InvalidPrivateKeyPem);
    }

    let certificates = CertificateDer::pem_slice_iter(certificate_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsIdentityError::InvalidCertificatePem)?;
    let [leaf] = certificates.as_slice() else {
        return Err(TlsIdentityError::InvalidCertificatePem);
    };
    let leaf = leaf.clone();
    validate_end_entity_certificate(&leaf)?;

    let mut keys = PrivateKeyDer::pem_slice_iter(private_key_pem);
    let private_key = keys
        .next()
        .ok_or(TlsIdentityError::InvalidPrivateKeyPem)?
        .map_err(|_| TlsIdentityError::InvalidPrivateKeyPem)?;
    if keys.next().is_some() {
        return Err(TlsIdentityError::InvalidPrivateKeyPem);
    }

    validate_loopback_subject_alt_names(&leaf)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| TlsIdentityError::InvalidIdentity)?
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|_| TlsIdentityError::InvalidIdentity)?;
    Ok(leaf)
}

pub(crate) fn validate_end_entity_certificate(
    certificate: &CertificateDer<'_>,
) -> Result<(), TlsIdentityError> {
    let (remaining, parsed) = parse_x509_certificate(certificate.as_ref())
        .map_err(|_| TlsIdentityError::InvalidCertificatePem)?;
    if !remaining.is_empty() {
        return Err(TlsIdentityError::InvalidCertificatePem);
    }
    let basic_constraints = parsed
        .tbs_certificate
        .basic_constraints()
        .map_err(|_| TlsIdentityError::InvalidCertificatePem)?
        .ok_or(TlsIdentityError::InvalidCertificatePem)?;
    if basic_constraints.value.ca {
        return Err(TlsIdentityError::InvalidCertificatePem);
    }
    Ok(())
}

fn validate_loopback_subject_alt_names(
    certificate: &CertificateDer<'_>,
) -> Result<(), TlsIdentityError> {
    let parsed = ParsedCertificate::try_from(certificate)
        .map_err(|_| TlsIdentityError::InvalidCertificatePem)?;
    for required_name in REQUIRED_SUBJECT_ALT_NAMES {
        let server_name =
            ServerName::try_from(required_name).map_err(|_| TlsIdentityError::GenerationFailed)?;
        verify_server_name(&parsed, &server_name)
            .map_err(|_| TlsIdentityError::MissingLoopbackSubjectAlternativeName)?;
    }
    Ok(())
}

fn encode_private_key_pem(private_key_der: &[u8]) -> Zeroizing<Vec<u8>> {
    let encoded = Zeroizing::new(STANDARD.encode(private_key_der));
    let mut pem = Zeroizing::new(Vec::with_capacity(encoded.len() + 64));
    pem.extend_from_slice(b"-----BEGIN PRIVATE KEY-----\n");
    for line in encoded.as_bytes().chunks(64) {
        pem.extend_from_slice(line);
        pem.push(b'\n');
    }
    pem.extend_from_slice(b"-----END PRIVATE KEY-----\n");
    pem
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_stable_fingerprint_and_different_seed_rotates_it() {
        let first = TlsIdentity::from_seed(TlsKeySeed::new([7_u8; KEY_SEED_BYTES]))
            .expect("first identity");
        let same =
            TlsIdentity::from_seed(TlsKeySeed::new([7_u8; KEY_SEED_BYTES])).expect("same identity");
        let rotated = TlsIdentity::from_seed(TlsKeySeed::new([8_u8; KEY_SEED_BYTES]))
            .expect("rotated identity");

        assert_eq!(first.certificate_sha256(), same.certificate_sha256());
        assert_eq!(first.certificate_pem(), same.certificate_pem());
        assert_ne!(first.certificate_sha256(), rotated.certificate_sha256());
        assert_eq!(first.certificate_sha256().len(), 64);
        assert!(
            first
                .certificate_sha256()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn generated_identity_validates_all_required_loopback_names() {
        let identity = TlsIdentity::from_seed(TlsKeySeed::new([9_u8; KEY_SEED_BYTES]))
            .expect("generated identity");
        let certificates = CertificateDer::pem_slice_iter(identity.certificate_pem())
            .collect::<Result<Vec<_>, _>>()
            .expect("certificate PEM");
        let leaf = certificates.first().expect("leaf");
        validate_loopback_subject_alt_names(leaf).expect("all loopback SANs");
    }

    #[test]
    fn supplied_pem_missing_an_ip_san_is_rejected() {
        let (certificate_pem, private_key_pem) =
            generate_materials(&[3_u8; KEY_SEED_BYTES], &["localhost"]).expect("test material");
        assert!(matches!(
            TlsIdentity::from_pem(certificate_pem, private_key_pem),
            Err(TlsIdentityError::MissingLoopbackSubjectAlternativeName)
        ));
    }

    #[test]
    fn supplied_pem_with_mismatched_private_key_is_rejected() {
        let (certificate_pem, _) =
            generate_materials(&[4_u8; KEY_SEED_BYTES], &REQUIRED_SUBJECT_ALT_NAMES)
                .expect("certificate");
        let (_, private_key_pem) =
            generate_materials(&[5_u8; KEY_SEED_BYTES], &REQUIRED_SUBJECT_ALT_NAMES)
                .expect("other key");
        assert!(matches!(
            TlsIdentity::from_pem(certificate_pem, private_key_pem),
            Err(TlsIdentityError::InvalidIdentity)
        ));
    }

    #[test]
    fn certificate_chains_and_ca_certificates_are_rejected() {
        let (certificate_pem, private_key_pem) =
            generate_materials(&[6_u8; KEY_SEED_BYTES], &REQUIRED_SUBJECT_ALT_NAMES)
                .expect("leaf material");
        let mut chain = certificate_pem.clone();
        chain.extend_from_slice(&certificate_pem);
        assert!(matches!(
            TlsIdentity::from_pem(chain, private_key_pem),
            Err(TlsIdentityError::InvalidCertificatePem)
        ));

        let (ca_pem, ca_key) = generate_materials_with_is_ca(
            &[10_u8; KEY_SEED_BYTES],
            &REQUIRED_SUBJECT_ALT_NAMES,
            IsCa::Ca(CertificateBasicConstraints::Unconstrained),
        )
        .expect("CA material");
        assert!(matches!(
            TlsIdentity::from_pem(ca_pem, ca_key),
            Err(TlsIdentityError::InvalidCertificatePem)
        ));
    }

    #[test]
    fn malformed_or_mixed_pem_is_rejected_without_echoing_material() {
        assert!(matches!(
            TlsIdentity::from_pem(
                b"not a certificate".to_vec(),
                Zeroizing::new(b"not a key".to_vec())
            ),
            Err(TlsIdentityError::InvalidCertificatePem)
        ));
        assert!(matches!(
            TlsIdentity::from_pem(
                b"-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----\n".to_vec(),
                Zeroizing::new(b"not a key".to_vec())
            ),
            Err(TlsIdentityError::InvalidCertificatePem)
        ));
    }

    #[test]
    fn debug_output_redacts_seed_and_private_key() {
        let seed = TlsKeySeed::new([0x5a_u8; KEY_SEED_BYTES]);
        assert_eq!(format!("{seed:?}"), "TlsKeySeed([REDACTED])");
        let identity = TlsIdentity::from_seed(seed).expect("identity");
        let debug = format!("{identity:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("PRIVATE KEY"));
        assert!(!debug.contains("Wlpa"));
    }
}
