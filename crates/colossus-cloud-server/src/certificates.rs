use crate::config::Config;
use rcgen::{
    CertificateParams, CertificateSigningRequestParams, ExtendedKeyUsagePurpose, Issuer, KeyPair,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

pub(crate) struct CertificateAuthority {
    issuer: Issuer<'static, KeyPair>,
    pub pem: String,
}
impl CertificateAuthority {
    pub fn load(config: &Config) -> Result<Self, &'static str> {
        let pem = std::fs::read_to_string(&config.ca_certificate)
            .map_err(|_| "cannot load connector CA")?;
        let key = Zeroizing::new(
            std::fs::read_to_string(&config.ca_key).map_err(|_| "cannot load connector CA key")?,
        );
        let issuer =
            Issuer::from_ca_cert_pem(&pem, KeyPair::from_pem(&key).map_err(|_| "invalid CA key")?)
                .map_err(|_| "invalid CA certificate")?;
        Ok(Self { issuer, pem })
    }
    pub fn sign(&self, csr: &str) -> Result<(String, String), &'static str> {
        if csr.len() > 16384 {
            return Err("invalid certificate request");
        }
        let mut request = CertificateSigningRequestParams::from_pem(csr)
            .map_err(|_| "invalid certificate request")?;
        // The host supplies all extensions; an untrusted CSR cannot obtain CA authority.
        let now = OffsetDateTime::now_utc();
        request.params = CertificateParams::default();
        request.params.not_before = now - Duration::minutes(5);
        request.params.not_after = now + Duration::days(30);
        request.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = request
            .signed_by(&self.issuer)
            .map_err(|_| "cannot issue client certificate")?;
        Ok((cert.pem(), hex::encode(Sha256::digest(cert.der()))))
    }
}
