use super::*;
use crate::{CertificateRedemption, validate_identifier, validation::bounded_fingerprint};
use serde::Deserialize;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Renewal {
    id: String,
    previous_fingerprint: String,
    csr_sha256: String,
    certificate_pem: String,
    certificate_sha256: String,
    issued_at: u64,
}

impl CloudRepository {
    /// Rotate a fixed node's leaf beneath its existing mTLS authority. The key stays
    /// native. Exact retries reconcile a lost reply, including after host restart.
    pub async fn renew_certificate(
        &self,
        identity: crate::RenewalIdentity<'_>,
        certificate: CertificateRedemption,
        now: u64,
    ) -> CloudResult<String> {
        let project = identity.project_id;
        let node_id = identity.node_id;
        let instance_id = identity.instance_id;
        let previous_fingerprint = identity.previous_fingerprint;
        let renewal_id = identity.renewal_id;
        validate_identifier(renewal_id)?;
        bounded_fingerprint(previous_fingerprint)?;
        bounded_fingerprint(&certificate.fingerprint)?;
        bounded_fingerprint(&certificate.csr_sha256)?;
        if certificate.certificate_pem.len() > 16384 {
            return Err(CloudError::InvalidArgument);
        }
        let mut node = self.node(project, node_id).await?;
        if node.revoked || node.instance_id != instance_id {
            return Err(CloudError::PermissionDenied);
        }
        let renewal_stream = format!("cloud.renewal:{project}:{node_id}");
        let prior = match self.read::<Renewal>(&renewal_stream).await {
            Ok(value) => Some(value),
            Err(CloudError::NotFound) => None,
            Err(error) => return Err(error),
        };
        if let Some((prior, _)) = &prior {
            if prior.id == renewal_id {
                return if prior.previous_fingerprint == previous_fingerprint
                    && prior.csr_sha256 == certificate.csr_sha256
                    && node.certificate_sha256 == prior.certificate_sha256
                {
                    Ok(prior.certificate_pem.clone())
                } else {
                    Err(CloudError::Conflict)
                };
            }
            if now < prior.issued_at.saturating_add(60) {
                return Err(CloudError::ResourceExhausted);
            }
        }
        if node.certificate_sha256 != previous_fingerprint {
            return Err(CloudError::PermissionDenied);
        }
        let renewal = Renewal {
            id: renewal_id.into(),
            previous_fingerprint: previous_fingerprint.into(),
            csr_sha256: certificate.csr_sha256,
            certificate_pem: certificate.certificate_pem.clone(),
            certificate_sha256: certificate.fingerprint.clone(),
            issued_at: now,
        };
        let revision = node.revision;
        node.revision += 1;
        node.certificate_sha256 = certificate.fingerprint;
        self.commit(vec![
            self.event(
                node_id,
                format!("cloud.node:{project}:{node_id}"),
                revision,
                "cloud.node.certificate-renewed.v1",
                &node,
            )?,
            self.event(
                node_id,
                renewal_stream,
                prior.as_ref().map_or(0, |(_, version)| *version),
                "cloud.renewal.issued.v1",
                &renewal,
            )?,
        ])
        .await?;
        Ok(certificate.certificate_pem)
    }
}
