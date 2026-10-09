use super::*;
use colossus_cloud_protocol::v1alpha1::{
    CertificateRenewal, runtime_connection_client::RuntimeConnectionClient,
};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub(super) struct Rotation {
    client_key_pem: String,
    csr_pem: String,
    renewal_id: String,
}
pub(crate) fn certificate_due(pem: &str) -> Result<bool, &'static str> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| "invalid connector certificate")?;
    let certificate = pem
        .parse_x509()
        .map_err(|_| "invalid connector certificate")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "invalid system clock")?
        .as_secs();
    Ok(certificate.validity().not_after.timestamp() <= now as i64 + 7 * 24 * 60 * 60)
}
impl EnrollmentStore {
    /// Renew at seven days remaining, or explicitly rotate before expiry. Persisting
    /// the next key/CSR before RPC makes a lost response recoverable with the old leaf.
    pub async fn renew(&self, force: bool) -> Result<(), &'static str> {
        let store = self.clone();
        let stored = tokio::task::spawn_blocking(move || store.vault.read(&store.key))
            .await
            .map_err(|_| "enrollment vault unavailable")?
            .map_err(|_| "enrollment vault unavailable")?
            .ok_or("connector is not enrolled")?;
        let mut record: Record =
            serde_json::from_slice(stored.expose()).map_err(|_| "invalid enrollment record")?;
        let config = record
            .config
            .as_ref()
            .ok_or("connector is not enrolled")?
            .clone();
        if config.revoked {
            return Err("cloud enrollment was revoked");
        }
        if record.rotation.is_none() && !force && !certificate_due(&config.certificate_pem)? {
            return Ok(());
        }
        if record.rotation.is_none() {
            let key = KeyPair::generate().map_err(|_| "connector key generation failed")?;
            let csr = CertificateParams::default()
                .serialize_request(&key)
                .map_err(|_| "certificate request failed")?
                .pem()
                .map_err(|_| "certificate request failed")?;
            record.rotation = Some(Rotation {
                client_key_pem: key.serialize_pem(),
                csr_pem: csr,
                renewal_id: uuid::Uuid::now_v7().simple().to_string(),
            });
            self.save_async(&record).await?;
        }
        let rotation = record
            .rotation
            .as_ref()
            .ok_or("renewal state unavailable")?;
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(config.ca_pem))
            .identity(Identity::from_pem(
                config.certificate_pem,
                record.pending.client_key_pem.as_bytes(),
            ));
        let endpoint = Endpoint::from_shared(config.endpoint)
            .map_err(|_| "invalid cloud endpoint")?
            .tls_config(tls)
            .map_err(|_| "invalid connector identity")?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15));
        let channel = tokio::time::timeout(Duration::from_secs(10), endpoint.connect())
            .await
            .map_err(|_| "certificate renewal channel startup timeout")?
            .map_err(|_| "certificate renewal transport unavailable")?;
        let certificate = RuntimeConnectionClient::new(channel)
            .max_decoding_message_size(65536)
            .max_encoding_message_size(65536)
            .renew_certificate(CertificateRenewal {
                project_id: config.project_id,
                node_id: config.node_id,
                instance_id: config.instance_id,
                renewal_id: rotation.renewal_id.clone(),
                csr_pem: rotation.csr_pem.clone(),
            })
            .await
            .map_err(|error| match error.code() {
                tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                    "cloud enrollment was revoked"
                }
                _ => "certificate renewal rejected; retry the same exchange",
            })?
            .into_inner()
            .certificate_pem;
        if certificate_due(&certificate)? {
            return Err("renewed certificate lifetime invalid");
        }
        let (_, pem) = x509_parser::pem::parse_x509_pem(certificate.as_bytes())
            .map_err(|_| "invalid renewed certificate")?;
        let leaf = pem
            .parse_x509()
            .map_err(|_| "invalid renewed certificate")?;
        let key = KeyPair::from_pem(&rotation.client_key_pem).map_err(|_| "invalid renewal key")?;
        if leaf.public_key().subject_public_key.data.as_ref() != key.public_key_raw() {
            return Err("renewed certificate does not bind the requested key");
        }
        if let Some(config) = &mut record.config {
            config.certificate_pem = certificate;
        }
        record.pending.client_key_pem.zeroize();
        record.pending.client_key_pem = rotation.client_key_pem.clone();
        record.rotation = None;
        self.save_async(&record).await
    }
}
