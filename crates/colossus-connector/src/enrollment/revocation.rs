use super::*;
use colossus_cloud_protocol::v1alpha1::{
    EnrollmentIdentity, runtime_connection_client::RuntimeConnectionClient,
};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

impl EnrollmentStore {
    /// Revoke this exact native enrollment using its own client certificate.
    /// Persist the acknowledgement locally; a lost acknowledgement can be retried.
    pub async fn revoke(&self) -> Result<(), &'static str> {
        let mut record = self.read_record().await?;
        if record.rotation.is_some() {
            self.renew(false).await?;
            record = self.read_record().await?;
        }
        let config = record.config.as_ref().ok_or("connector is not enrolled")?;
        if config.revoked {
            return Ok(());
        }
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(&config.ca_pem))
            .identity(Identity::from_pem(
                &config.certificate_pem,
                record.pending.client_key_pem.as_bytes(),
            ));
        let endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|_| "invalid cloud endpoint")?
            .tls_config(tls)
            .map_err(|_| "invalid connector identity")?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15));
        let channel = tokio::time::timeout(Duration::from_secs(10), endpoint.connect())
            .await
            .map_err(|_| "revocation channel startup timeout; retry the same enrollment")?
            .map_err(|_| "revocation transport unavailable; retry the same enrollment")?;
        RuntimeConnectionClient::new(channel)
            .max_decoding_message_size(65536)
            .max_encoding_message_size(65536)
            .revoke_enrollment(EnrollmentIdentity {
                project_id: config.project_id.clone(),
                node_id: config.node_id.clone(),
                instance_id: config.instance_id.clone(),
            })
            .await
            .map_err(|_| "cloud revocation rejected; retry or revoke in the web fleet")?;
        record
            .config
            .as_mut()
            .ok_or("connector is not enrolled")?
            .revoked = true;
        self.save_async(&record).await
    }

    async fn read_record(&self) -> Result<Record, &'static str> {
        let store = self.clone();
        let stored = tokio::task::spawn_blocking(move || store.vault.read(&store.key))
            .await
            .map_err(|_| "enrollment vault unavailable")?
            .map_err(|_| "enrollment vault unavailable")?
            .ok_or("connector is not enrolled")?;
        serde_json::from_slice(stored.expose()).map_err(|_| "invalid enrollment record")
    }
}
