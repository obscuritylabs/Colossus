//! Explicit trust for the initial HTTPS invitation exchange.
use reqwest::{Certificate, ClientBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Public roots by default, or one explicitly supplied certificate trust anchor.
///
/// This applies only to HTTPS enrollment. The returned connector CA separately
/// authenticates subsequent mutual-TLS connections and certificate renewal.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentTrust {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    certificate_der: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    certificate_sha256: Option<String>,
}

impl EnrollmentTrust {
    /// Maximum accepted public PEM file size, including whitespace.
    pub const MAX_PEM_BYTES: usize = 16_384;

    /// Retain the client's bundled public certificate roots.
    pub fn public_roots() -> Self {
        Self::default()
    }

    /// Add exactly one public X.509 trust anchor without disabling TLS validation.
    ///
    /// Self-signed server leaves are accepted as deliberate trust anchors, as
    /// well as CA certificates. Hostname and validity checks still occur in TLS.
    pub fn from_pem(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > Self::MAX_PEM_BYTES {
            return Err("enrollment trust certificate exceeds bound");
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "invalid enrollment trust certificate")?
            .trim();
        if !text.starts_with("-----BEGIN CERTIFICATE-----") {
            return Err("invalid enrollment trust certificate");
        }
        let (remaining, pem) = x509_parser::pem::parse_x509_pem(text.as_bytes())
            .map_err(|_| "invalid enrollment trust certificate")?;
        if pem.label != "CERTIFICATE" || !remaining.iter().all(u8::is_ascii_whitespace) {
            return Err("enrollment trust requires exactly one certificate");
        }
        Self::from_der(&pem.contents)
    }

    fn from_der(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > Self::MAX_PEM_BYTES {
            return Err("enrollment trust certificate exceeds bound");
        }
        let (remaining, _) = x509_parser::parse_x509_certificate(bytes)
            .map_err(|_| "invalid enrollment trust certificate")?;
        if !remaining.is_empty() {
            return Err("invalid enrollment trust certificate");
        }
        Certificate::from_der(bytes).map_err(|_| "invalid enrollment trust certificate")?;
        Ok(Self {
            certificate_der: Some(bytes.to_vec()),
            certificate_sha256: Some(hex::encode(Sha256::digest(bytes))),
        })
    }

    pub(super) fn validate(&self) -> Result<(), &'static str> {
        match (&self.certificate_der, &self.certificate_sha256) {
            (None, None) => Ok(()),
            (Some(der), Some(_)) if Self::from_der(der)?.eq(self) => Ok(()),
            _ => Err("invalid enrollment trust binding"),
        }
    }

    pub(super) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder, &'static str> {
        self.validate()?;
        match &self.certificate_der {
            Some(der) => Ok(builder.add_root_certificate(
                Certificate::from_der(der).map_err(|_| "invalid enrollment trust certificate")?,
            )),
            None => Ok(builder),
        }
    }
}
